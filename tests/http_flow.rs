use std::{
    net::{SocketAddr, TcpListener},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use goose_http::{
    Server,
    common::StatusCode,
    headers::header_keys,
    log, range,
    request::Request,
    response::Response,
    routing::{Handler, router},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    task::JoinHandle,
    time::sleep,
};

#[tokio::test(flavor = "multi_thread")]
async fn expect_100_continue_flow() -> Result<(), Box<dyn std::error::Error>> {
    log::init();
    let (addr, handle) = spawn_server(|_| {
        let mut response = Response::new(StatusCode::OK);
        response
            .headers_mut()
            .insert(header_keys::CONTENT_TYPE, "text/plain");
        response.set_body_text_static("done");
        response
    })
    .await;
    let mut stream = TcpStream::connect(addr).await?;

    let request_head = "POST /upload HTTP/1.1\r\nHost: localhost\r\nExpect: 100-continue\r\nContent-Length: 4\r\n\r\n";
    stream.write_all(request_head.as_bytes()).await?;

    let interim = read_headers(&mut stream).await?;
    assert!(interim.starts_with("HTTP/1.1 100"));

    stream.write_all(b"test").await?;

    let (final_head, final_body) = read_response(&mut stream).await?;
    assert!(final_head.starts_with("HTTP/1.1 200"));
    assert_eq!(std::str::from_utf8(&final_body)?, "done");

    handle.abort();
    let _ = handle.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn pipelined_requests_are_serialised() -> Result<(), Box<dyn std::error::Error>> {
    log::init();
    let counter = Arc::new(AtomicUsize::new(1));
    let handler_counter = counter.clone();
    let (addr, handle) = spawn_server(move |_| {
        let current = handler_counter.fetch_add(1, Ordering::SeqCst);
        let mut response = Response::new(StatusCode::OK);
        response.set_body_bytes(format!("response-{current}"));
        response
    })
    .await;
    let mut stream = TcpStream::connect(addr).await?;

    let req1 = "GET /first HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n";
    let req2 = "GET /second HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n";
    stream.write_all(req1.as_bytes()).await?;
    stream.write_all(req2.as_bytes()).await?;

    let (_head1, body1) = read_response(&mut stream).await?;
    let (_head2, body2) = read_response(&mut stream).await?;

    assert_eq!(std::str::from_utf8(&body1)?, "response-1");
    assert_eq!(std::str::from_utf8(&body2)?, "response-2");

    handle.abort();
    let _ = handle.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_range_responses_return_multipart_body() -> Result<(), Box<dyn std::error::Error>> {
    log::init();
    let data = Arc::new(b"abcdefghijklmnopqrstuvwxyz".to_vec());
    let handler_data = data.clone();

    let (addr, handle) = spawn_server(move |request: Request| {
        if let Some(range_header) = request.header(header_keys::RANGE) {
            if let Ok(specs) = range::parse_range_header(range_header) {
                let ranges = range::compute_satisfiable_ranges(&specs, handler_data.len() as u64);
                if ranges.len() > 1 {
                    let boundary = "test-boundary";
                    let mut body = String::new();
                    for range in &ranges {
                        let start = range.start as usize;
                        let end = range.end as usize + 1;
                        let slice = &handler_data[start..end];
                        body.push_str(&format!(
                            "--{boundary}\r\nContent-Type: text/plain\r\nContent-Range: bytes {}-{}/{}\r\n\r\n{}\r\n",
                            range.start,
                            range.end,
                            handler_data.len(),
                            std::str::from_utf8(slice).unwrap()
                        ));
                    }
                    body.push_str(&format!("--{boundary}--\r\n"));

                    let mut response = Response::new(StatusCode::PARTIAL_CONTENT);
                    response.headers_mut().insert(
                        header_keys::CONTENT_TYPE,
                        format!("multipart/byteranges; boundary={boundary}"),
                    );
                    response
                        .headers_mut()
                        .insert(header_keys::ACCEPT_RANGES, "bytes");
                    response.set_body_bytes(body);
                    return response;
                }
            }
        }

        let mut response = Response::new(StatusCode::OK);
        response.set_body_bytes(handler_data.as_ref().clone());
        response
    })
    .await;
    let mut stream = TcpStream::connect(addr).await?;

    let request = "GET /multi HTTP/1.1\r\nHost: localhost\r\nRange: bytes=0-3,6-9\r\n\r\n";
    stream.write_all(request.as_bytes()).await?;

    let (headers, body) = read_response(&mut stream).await?;
    assert!(headers.starts_with("HTTP/1.1 206"));
    assert!(headers.contains("multipart/byteranges"));
    let body_str = std::str::from_utf8(&body)?;
    assert!(body_str.contains("Content-Range: bytes 0-3/26"));
    assert!(body_str.contains("Content-Range: bytes 6-9/26"));

    handle.abort();
    let _ = handle.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn router_returns_404_for_unknown_path() -> Result<(), Box<dyn std::error::Error>> {
    log::init();
    let router = router()
        .get("/", |_| {
            let mut response = Response::new(StatusCode::OK);
            response.set_body_text_static("ok");
            response
        })
        .build();

    let (addr, handle) = spawn_server(router).await;
    let mut stream = TcpStream::connect(addr).await?;

    let request = "GET /missing HTTP/1.1\r\nHost: localhost\r\n\r\n";
    stream.write_all(request.as_bytes()).await?;

    let (headers, body) = read_response(&mut stream).await?;
    assert!(headers.starts_with("HTTP/1.1 404"));
    assert!(body.is_empty());

    handle.abort();
    let _ = handle.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn router_returns_405_with_allow_header() -> Result<(), Box<dyn std::error::Error>> {
    log::init();
    let router = router()
        .get("/resource", |_| {
            let mut response = Response::new(StatusCode::OK);
            response.set_body_text_static("ok");
            response
        })
        .build();

    let (addr, handle) = spawn_server(router).await;
    let mut stream = TcpStream::connect(addr).await?;

    let request = "POST /resource HTTP/1.1\r\nHost: localhost\r\n\r\n";
    stream.write_all(request.as_bytes()).await?;

    let (headers, body) = read_response(&mut stream).await?;
    assert!(headers.starts_with("HTTP/1.1 405"));
    assert!(headers.to_ascii_lowercase().contains("allow: get, head"));
    assert!(body.is_empty());

    handle.abort();
    let _ = handle.await;
    Ok(())
}

async fn spawn_server(handler: impl Handler) -> (SocketAddr, JoinHandle<()>) {
    let port = pick_unused_port();
    let addr = format!("127.0.0.1:{port}");
    let socket_addr: SocketAddr = addr.parse().expect("valid socket address");
    let server = Server::builder()
        .with_addr(addr)
        .with_handler(handler)
        .build();

    let handle = tokio::spawn(async move {
        let _ = server.run().await;
    });

    sleep(Duration::from_millis(50)).await;
    (socket_addr, handle)
}

fn pick_unused_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind to ephemeral port")
        .local_addr()
        .expect("get local address")
        .port()
}

async fn read_headers(stream: &mut TcpStream) -> Result<String, std::io::Error> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    while !buffer.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await?;
        buffer.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

async fn read_response(stream: &mut TcpStream) -> Result<(String, Vec<u8>), std::io::Error> {
    let headers = read_headers(stream).await?;
    let length = parse_content_length(&headers).unwrap_or(0);
    let mut body = vec![0u8; length];
    if length > 0 {
        stream.read_exact(&mut body).await?;
    }
    Ok((headers, body))
}

fn parse_content_length(headers: &str) -> Option<usize> {
    headers
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("content-length"))
        .and_then(|line| line.split_once(':'))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
}
