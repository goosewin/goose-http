//! Connection state machine for handling HTTP/1.1 keep-alive and pipelining.
//!
//! Each accepted TCP stream is wrapped by [`Connection`] and progressed through
//! the parsing, routing, and response lifecycle while ensuring the wire remains
//! synchronised per RFC 9112 requirements.

use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use bytes::{Buf, Bytes, BytesMut};
use thiserror::Error;
use tokio::{io::AsyncReadExt, net::TcpStream, time};

use crate::{
    body::Body,
    common::{HttpVersion, Method, StatusCode},
    date,
    encode::{ConnectionDirective, EncodeError, ResponseWriter},
    headers::header_keys,
    log,
    parse::{self, BodyError, BodyMode, ParseError},
    request::Request,
    response::{Response, ResponseBody},
    routing::Handler,
};

/// Represents an individual client connection.
pub struct Connection {
    id: u64,
    stream: TcpStream,
    handler: Arc<dyn Handler>,
    buffer: BytesMut,
    state: ConnectionState,
    config: ConnectionConfig,
}

/// Tunable connection behaviour.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub header_read_timeout: Duration,
    pub body_read_timeout: Duration,
    pub idle_timeout: Duration,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            header_read_timeout: Duration::from_secs(5),
            body_read_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(60),
        }
    }
}

/// Categorises timeout failures for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    Header,
    Body,
    Idle,
}

/// High-level phases a connection can occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Awaiting a new request head.
    Idle,
    /// Streaming a request/response pair.
    Streaming,
    /// Preparing to close the underlying transport.
    Closing,
}

impl Connection {
    /// Create a new connection wrapper with a monotonically increasing id.
    pub fn new(
        id: u64,
        stream: TcpStream,
        handler: Arc<dyn Handler>,
        config: ConnectionConfig,
    ) -> Self {
        Self {
            id,
            stream,
            handler,
            buffer: BytesMut::with_capacity(16 * 1024),
            state: ConnectionState::Idle,
            config,
        }
    }

    /// Returns the identifier associated with the connection.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Drive the connection until it terminates, handling pipelined requests in order.
    pub async fn run(mut self) -> Result<(), ConnectionError> {
        let mut processed_requests = false;

        loop {
            self.state = ConnectionState::Idle;

            if parse::incomplete_head_exceeds_limit(&self.buffer[..]) {
                self.respond_with_error(StatusCode::BAD_REQUEST, "Bad Request")
                    .await?;
                self.state = ConnectionState::Closing;
                return Err(ConnectionError::Parse(ParseError::HeaderTooLarge));
            }

            if parse::needs_more_head(&self.buffer[..]) {
                let buffer_empty = self.buffer.is_empty();
                let (timeout_duration, timeout_kind) = if buffer_empty && processed_requests {
                    (self.config.idle_timeout, TimeoutKind::Idle)
                } else {
                    (self.config.header_read_timeout, TimeoutKind::Header)
                };

                match time::timeout(timeout_duration, self.stream.read_buf(&mut self.buffer)).await
                {
                    Ok(Ok(0)) => {
                        if self.buffer.is_empty() {
                            return Ok(());
                        }
                        self.respond_with_error(StatusCode::BAD_REQUEST, "Bad Request")
                            .await?;
                        self.state = ConnectionState::Closing;
                        return Err(ConnectionError::Parse(ParseError::Incomplete));
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => return Err(ConnectionError::Io(error)),
                    Err(_) => {
                        log::warn(&format!(
                            "connection {} timed out during {:?} read",
                            self.id, timeout_kind
                        ));
                        self.respond_with_error(StatusCode::REQUEST_TIMEOUT, "Request timeout")
                            .await?;
                        return Err(ConnectionError::Timeout(timeout_kind));
                    }
                }

                if parse::incomplete_head_exceeds_limit(&self.buffer[..]) {
                    self.respond_with_error(StatusCode::BAD_REQUEST, "Bad Request")
                        .await?;
                    self.state = ConnectionState::Closing;
                    return Err(ConnectionError::Parse(ParseError::HeaderTooLarge));
                }

                continue;
            }

            self.state = ConnectionState::Streaming;

            let (mut request, body_mode, consumed) =
                match parse::parse_request_head(&self.buffer[..]) {
                    Ok(result) => result,
                    Err(err) => {
                        self.respond_with_error(StatusCode::BAD_REQUEST, "Bad Request")
                            .await?;
                        self.state = ConnectionState::Closing;
                        return Err(ConnectionError::Parse(err));
                    }
                };
            self.buffer.advance(consumed);

            let request_method = request.method().clone();
            let request_version = request.version();
            let request_close = should_close_after_request(request_version, request.headers());

            if request.expect_100_continue() {
                let mut writer = ResponseWriter::new(&mut self.stream);
                writer.write_continue().await?;
            }

            if body_mode != BodyMode::None {
                let mut reader = parse::body_reader(body_mode, &mut self.stream, &mut self.buffer);
                let read_result = time::timeout(self.config.body_read_timeout, async move {
                    let mut buf = BytesMut::new();
                    while let Some(chunk) = reader.read_next().await? {
                        buf.extend_from_slice(&chunk);
                    }
                    Ok::<BytesMut, BodyError>(buf)
                })
                .await;

                match read_result {
                    Ok(Ok(bytes)) => {
                        request.set_body_bytes(bytes.freeze());
                    }
                    Ok(Err(err)) => {
                        self.respond_with_error(StatusCode::BAD_REQUEST, "Malformed request body")
                            .await?;
                        self.state = ConnectionState::Closing;
                        return Err(ConnectionError::Body(err));
                    }
                    Err(_) => {
                        log::warn(&format!(
                            "connection {} timed out while reading request body",
                            self.id
                        ));
                        self.respond_with_error(
                            StatusCode::REQUEST_TIMEOUT,
                            "Request body timeout",
                        )
                        .await?;
                        self.state = ConnectionState::Closing;
                        return Err(ConnectionError::Timeout(TimeoutKind::Body));
                    }
                }
            }
            let request_snapshot = request.clone();
            request.set_body(Body::Empty);

            let mut response = self.handler.handle(request);
            if response.status().as_u16() < 400 {
                apply_request_preconditions(&request_snapshot, &mut response);
            }
            let response_close = response_requests_close(&response);

            let mut directive = if request_close || response_close {
                ConnectionDirective::Close
            } else {
                ConnectionDirective::KeepAlive
            };

            if request_version != HttpVersion::Http11 {
                directive = ConnectionDirective::Close;
            }

            let mut writer = ResponseWriter::new(&mut self.stream);
            writer
                .write_response(&mut response, &request_method, directive)
                .await?;
            writer.flush().await?;

            processed_requests = true;

            if matches!(directive, ConnectionDirective::Close) {
                self.state = ConnectionState::Closing;
                return Ok(());
            }
        }
    }

    async fn respond_with_error(
        &mut self,
        status: StatusCode,
        message: &str,
    ) -> Result<(), EncodeError> {
        log::warn(&format!(
            "connection {} sending {}: {}",
            self.id, status, message
        ));
        let mut response = Response::new(status);
        response
            .headers_mut()
            .insert(header_keys::CONTENT_TYPE, "text/plain; charset=utf-8");
        response.set_body_bytes(Bytes::copy_from_slice(message.as_bytes()));

        let mut writer = ResponseWriter::new(&mut self.stream);
        let method = Method::Get;
        writer
            .write_response(&mut response, &method, ConnectionDirective::Close)
            .await?;
        writer.flush().await
    }
}

/// Errors that can arise while servicing a connection.
#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Encode(#[from] EncodeError),
    #[error(transparent)]
    Body(#[from] BodyError),
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error("connection timeout while handling {0:?}")]
    Timeout(TimeoutKind),
}

fn should_close_after_request(version: HttpVersion, headers: &crate::headers::Headers) -> bool {
    match version {
        HttpVersion::Http11 => headers
            .get(header_keys::CONNECTION)
            .is_some_and(|value| contains_token(value, "close")),
        HttpVersion::Http10 => !headers
            .get(header_keys::CONNECTION)
            .is_some_and(|value| contains_token(value, "keep-alive")),
        _ => true,
    }
}

fn response_requests_close(response: &Response) -> bool {
    response
        .headers()
        .get(header_keys::CONNECTION)
        .is_some_and(|value| contains_token(value, "close"))
}

fn contains_token(value: &str, token: &str) -> bool {
    value
        .split(',')
        .any(|part| part.trim().eq_ignore_ascii_case(token))
}

fn apply_request_preconditions(request: &Request, response: &mut Response) {
    let mut decision: Option<StatusCode> = None;
    let response_etag = response.headers().get(header_keys::ETAG);

    if let Some(if_match) = request.if_match()
        && !etag_list_matches(if_match, response_etag, true)
    {
        decision = Some(StatusCode::PRECONDITION_FAILED);
    }

    if decision.is_none()
        && let Some(unmodified_since) = request.if_unmodified_since()
        && let Some(last_modified) = parse_last_modified(response)
        && last_modified > unmodified_since
    {
        decision = Some(StatusCode::PRECONDITION_FAILED);
    }

    if decision.is_none() {
        if let Some(if_none_match) = request.if_none_match() {
            if etag_list_matches(if_none_match, response_etag, false) {
                if matches!(request.method(), Method::Get | Method::Head) {
                    decision = Some(StatusCode::NOT_MODIFIED);
                } else {
                    decision = Some(StatusCode::PRECONDITION_FAILED);
                }
            }
        } else if let Some(if_modified_since) = request.if_modified_since()
            && matches!(request.method(), Method::Get | Method::Head)
            && let Some(last_modified) = parse_last_modified(response)
            && last_modified <= if_modified_since
        {
            decision = Some(StatusCode::NOT_MODIFIED);
        }
    }

    if let Some(status) = decision {
        log::info(&format!(
            "precondition evaluation changed response to {} for {}",
            status,
            request.method().as_str()
        ));
        response.set_status(status);
        response.set_body(ResponseBody::Empty);
        response.take_trailers();
        let headers = response.headers_mut();
        headers.remove(header_keys::CONTENT_LENGTH);
        headers.remove(header_keys::TRANSFER_ENCODING);
        if status == StatusCode::NOT_MODIFIED {
            headers.remove(header_keys::CONTENT_TYPE);
        }
    }
}

fn parse_last_modified(response: &Response) -> Option<SystemTime> {
    response
        .headers()
        .get(header_keys::LAST_MODIFIED)
        .and_then(|value| date::parse_http_date(value.trim()))
}

fn etag_list_matches(value: &str, entity_tag: Option<&str>, strong: bool) -> bool {
    let trimmed = value.trim();
    if trimmed == "*" {
        return entity_tag.is_some();
    }

    let entity = entity_tag.map(|v| v.trim()).filter(|v| !v.is_empty());
    let Some(entity) = entity else {
        return false;
    };

    trimmed
        .split(',')
        .map(|candidate| candidate.trim())
        .filter(|candidate| !candidate.is_empty())
        .any(|candidate| {
            if strong {
                candidate == entity
            } else {
                weak_etag_equal(candidate, entity)
            }
        })
}

fn weak_etag_equal(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }

    let a = a.trim_start_matches("W/");
    let b = b.trim_start_matches("W/");
    a == b
}
