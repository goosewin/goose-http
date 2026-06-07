//! HTTP response serialization utilities.
//!
//! Responsible for emitting response lines, headers, and bodies according to
//! RFC 9112 framing rules, including chunked transfer coding and trailers.

use std::fmt::Write;

use futures_util::StreamExt;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::{
    common::{Method, StatusCode},
    date,
    headers::{Headers, header_keys},
    response::{BoxBodyStream, Response, ResponseBody},
};

/// Directive applied to the `Connection` header for a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionDirective {
    /// Explicitly instruct the client to close the TCP connection.
    Close,
    /// Leave the connection open for further requests (default for HTTP/1.1).
    KeepAlive,
}

/// Errors produced while serialising a response.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("response contains both Content-Length and Transfer-Encoding")]
    ConflictingLengthAndTransferEncoding,
}

/// Writer responsible for serialising responses onto the wire.
pub struct ResponseWriter<'a, W>
where
    W: AsyncWrite + Unpin,
{
    writer: &'a mut W,
}

impl<'a, W> ResponseWriter<'a, W>
where
    W: AsyncWrite + Unpin,
{
    /// Create a new writer referencing the underlying transport.
    pub fn new(writer: &'a mut W) -> Self {
        Self { writer }
    }

    /// Emit an interim `100 Continue` response.
    pub async fn write_continue(&mut self) -> Result<(), EncodeError> {
        self.writer
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .await?;
        Ok(())
    }

    /// Serialise the supplied response according to RFC 9112 framing rules.
    pub async fn write_response(
        &mut self,
        response: &mut Response,
        request_method: &Method,
        directive: ConnectionDirective,
    ) -> Result<(), EncodeError> {
        let version = response.version();
        let status = response.status();
        let status_code = status.as_u16();
        let reason = response.reason_phrase().to_owned();

        let mut body = response.take_body();
        let mut trailers = response.take_trailers();
        let headers = response.headers_mut();

        let has_content_length = headers.contains(header_keys::CONTENT_LENGTH);
        let has_transfer_encoding = headers.contains(header_keys::TRANSFER_ENCODING);
        if has_content_length && has_transfer_encoding {
            return Err(EncodeError::ConflictingLengthAndTransferEncoding);
        }

        if !headers.contains(header_keys::DATE) {
            headers.insert(header_keys::DATE, date::now());
        }

        apply_connection_directive(headers, directive);

        let forbid_content_length = matches!(status_code, 100..=199 | 204 | 304);
        let body_allowed = response_allows_body(status, request_method);

        if !body_allowed {
            if matches!(request_method, Method::Head)
                && let ResponseBody::Full(ref bytes) = body
                && !forbid_content_length
                && !headers.contains(header_keys::CONTENT_LENGTH)
            {
                headers.insert(header_keys::CONTENT_LENGTH, bytes.len().to_string());
            }
            headers.remove(header_keys::TRANSFER_ENCODING);
            if forbid_content_length {
                headers.remove(header_keys::CONTENT_LENGTH);
            } else if !headers.contains(header_keys::CONTENT_LENGTH)
                && !matches!(request_method, Method::Head)
            {
                headers.insert(header_keys::CONTENT_LENGTH, "0");
            }
            trailers = None;
            body = ResponseBody::Empty;
        } else {
            match &body {
                ResponseBody::Empty => {
                    headers.remove(header_keys::TRANSFER_ENCODING);
                    if forbid_content_length {
                        headers.remove(header_keys::CONTENT_LENGTH);
                    } else if !headers.contains(header_keys::CONTENT_LENGTH) {
                        headers.insert(header_keys::CONTENT_LENGTH, "0");
                    }
                    trailers = None;
                }
                ResponseBody::Full(bytes) => {
                    headers.remove(header_keys::TRANSFER_ENCODING);
                    if forbid_content_length {
                        headers.remove(header_keys::CONTENT_LENGTH);
                    } else if !headers.contains(header_keys::CONTENT_LENGTH) {
                        headers.insert(header_keys::CONTENT_LENGTH, bytes.len().to_string());
                    }
                    trailers = None;
                }
                ResponseBody::Stream(_) => {
                    headers.remove(header_keys::CONTENT_LENGTH);
                    ensure_chunked_encoding(headers);
                }
            }
        }

        let mut head = String::new();
        head.push_str(&format!("{} {}", version, status_code));
        if !reason.is_empty() {
            head.push(' ');
            head.push_str(&reason);
        }
        head.push_str("\r\n");

        append_headers(&mut head, headers);

        self.writer.write_all(head.as_bytes()).await?;

        match body {
            ResponseBody::Empty => {}
            ResponseBody::Full(bytes) => {
                if body_allowed {
                    self.writer.write_all(bytes.as_ref()).await?;
                }
            }
            ResponseBody::Stream(mut stream) => {
                write_chunked_body(self.writer, &mut stream, trailers.as_ref()).await?;
            }
        }

        Ok(())
    }

    /// Flush the underlying writer.
    pub async fn flush(&mut self) -> Result<(), EncodeError> {
        self.writer.flush().await.map_err(EncodeError::from)
    }
}

fn apply_connection_directive(headers: &mut Headers, directive: ConnectionDirective) {
    match directive {
        ConnectionDirective::Close => headers.insert(header_keys::CONNECTION, "close"),
        ConnectionDirective::KeepAlive => {}
    }
}

fn response_allows_body(status: StatusCode, method: &Method) -> bool {
    let code = status.as_u16();
    if matches!(code, 100..=199 | 204 | 304) {
        return false;
    }
    !matches!(method, Method::Head)
}

fn append_headers(buffer: &mut String, headers: &Headers) {
    for (name, values) in headers.iter() {
        for value in values {
            buffer.push_str(name.as_str());
            buffer.push_str(": ");
            buffer.push_str(value);
            buffer.push_str("\r\n");
        }
    }
    buffer.push_str("\r\n");
}

fn ensure_chunked_encoding(headers: &mut Headers) {
    let existing = headers
        .get(header_keys::TRANSFER_ENCODING)
        .map(|v| v.to_string());
    if let Some(value) = existing
        && value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case("chunked"))
    {
        return;
    }
    headers.insert(header_keys::TRANSFER_ENCODING, "chunked");
}

async fn write_chunked_body<W>(
    writer: &mut W,
    stream: &mut BoxBodyStream,
    trailers: Option<&Headers>,
) -> Result<(), EncodeError>
where
    W: AsyncWrite + Unpin,
{
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if chunk.is_empty() {
            continue;
        }
        let mut prefix = String::new();
        write!(&mut prefix, "{:X}\r\n", chunk.len()).unwrap();
        writer.write_all(prefix.as_bytes()).await?;
        writer.write_all(chunk.as_ref()).await?;
        writer.write_all(b"\r\n").await?;
    }

    writer.write_all(b"0\r\n").await?;
    if let Some(trailers) = trailers {
        let mut block = String::new();
        append_headers(&mut block, trailers);
        writer.write_all(block.as_bytes()).await?;
    } else {
        writer.write_all(b"\r\n").await?;
    }

    Ok(())
}
