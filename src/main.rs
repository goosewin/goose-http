use std::error::Error;

use goose_http::{
    Server, cache,
    common::StatusCode,
    headers::header_keys,
    range,
    request::{Request, RequestTarget},
    response::{Response, ResponseBody},
    routing::{Router, router},
};

const HELLO_BODY: &[u8] = b"Hello from Goose HTTP\n";

fn demo_router() -> Router {
    router()
        .get("/", handle_get)
        .options("/", handle_options)
        .options("*", handle_options)
        .trace("/", trace_request)
        .trace("*", trace_request)
        .build()
}

fn handle_get(request: Request) -> Response {
    let body = HELLO_BODY;
    let mut response = Response::new(StatusCode::OK);
    response
        .headers_mut()
        .insert(header_keys::CONTENT_TYPE, "text/plain; charset=utf-8");
    response
        .headers_mut()
        .insert(header_keys::ETAG, "\"demo-etag\"");
    response
        .headers_mut()
        .insert(header_keys::LAST_MODIFIED, "Wed, 21 Oct 2015 07:28:00 GMT");
    cache::ensure_age_header(response.headers_mut());

    if let Some(range_header) = request.header(header_keys::RANGE) {
        if let Ok(specs) = range::parse_range_header(range_header) {
            let ranges = range::compute_satisfiable_ranges(&specs, body.len() as u64);
            if ranges.is_empty() {
                let mut partial = Response::new(StatusCode::RANGE_NOT_SATISFIABLE);
                partial.headers_mut().insert(
                    header_keys::CONTENT_RANGE,
                    range::format_unsatisfied_range(body.len() as u64),
                );
                partial.set_body(ResponseBody::Empty);
                return partial;
            }

            let range = ranges[0];
            let slice = &body[range.start as usize..=range.end as usize];
            let mut partial = Response::new(StatusCode::PARTIAL_CONTENT);
            partial.headers_mut().insert(
                header_keys::CONTENT_RANGE,
                range::format_content_range(range, body.len() as u64),
            );
            partial
                .headers_mut()
                .insert(header_keys::ACCEPT_RANGES, "bytes");
            partial.set_body_static(slice);
            return partial;
        }
    }

    if let Some(etag) = request.if_none_match() {
        if etag == "\"demo-etag\"" || etag == "*" {
            let mut not_modified = Response::new(StatusCode::NOT_MODIFIED);
            not_modified
                .headers_mut()
                .insert(header_keys::ETAG, "\"demo-etag\"");
            not_modified
                .headers_mut()
                .insert(header_keys::LAST_MODIFIED, "Wed, 21 Oct 2015 07:28:00 GMT");
            return not_modified;
        }
    }

    response.set_body_static(body);
    response
}

fn handle_options(request: Request) -> Response {
    let mut response = Response::new(StatusCode::OK);
    response
        .headers_mut()
        .insert(header_keys::ALLOW, "GET, HEAD, OPTIONS, TRACE");
    if matches!(request.target(), RequestTarget::Asterisk) {
        response.set_body(ResponseBody::Empty);
    }
    response
}

fn trace_request(request: Request) -> Response {
    let mut response = Response::new(StatusCode::OK);
    response
        .headers_mut()
        .insert(header_keys::CONTENT_TYPE, "message/http");
    let mut echo = String::new();
    echo.push_str(&format!(
        "{} {} HTTP/1.1\r\n",
        request.method().as_str(),
        match request.target() {
            RequestTarget::Origin(s) | RequestTarget::Absolute(s) | RequestTarget::Authority(s) =>
                s,
            RequestTarget::Asterisk => "*",
        }
    ));
    for (name, values) in request.headers().iter() {
        for value in values {
            echo.push_str(name.as_str());
            echo.push_str(": ");
            echo.push_str(value);
            echo.push_str("\r\n");
        }
    }
    echo.push_str("\r\n");
    response.set_body_bytes(echo);
    response
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = Server::builder()
        .with_addr("127.0.0.1:8080")
        .with_handler(demo_router())
        .build();

    println!("Goose HTTP demo running on {}", server.addr());
    server.run().await?;
    Ok(())
}
