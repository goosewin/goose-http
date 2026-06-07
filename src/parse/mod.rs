//! HTTP/1.1 request parsing and body framing utilities.
//!
//! Implements strict parsing logic for request start-lines and headers in line
//! with RFC 9112, as well as readers for fixed-length and chunked message
//! bodies.

use std::{cmp::min, str::FromStr};

use bytes::{Buf, Bytes, BytesMut};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{
    body::Body,
    common::{HttpVersion, Method},
    headers::{HeaderName, Headers, header_keys},
    request::{Request, RequestTarget},
};

pub(crate) const HEADER_LIMIT: usize = 64 * 1024;
const LINE_LIMIT: usize = 8 * 1024;

/// Indicates whether more bytes are required to parse a complete request head.
pub fn needs_more_head(buffer: &[u8]) -> bool {
    find_headers_end(buffer).is_none()
}

/// Indicates whether an incomplete request head has exceeded the header limit.
pub(crate) fn incomplete_head_exceeds_limit(buffer: &[u8]) -> bool {
    find_headers_end(buffer).is_none() && buffer.len() > HEADER_LIMIT
}

/// Parse a request head (start-line + headers) from the provided buffer.
///
/// Returns the constructed [`Request`], the inferred [`BodyMode`], and the
/// number of bytes consumed from the buffer.
pub fn parse_request_head(buffer: &[u8]) -> Result<(Request, BodyMode, usize), ParseError> {
    let head_end = find_headers_end(buffer).ok_or(ParseError::Incomplete)?;
    if head_end > HEADER_LIMIT {
        return Err(ParseError::HeaderTooLarge);
    }

    let head_bytes = &buffer[..head_end];
    let head_str =
        std::str::from_utf8(head_bytes).map_err(|_| ParseError::InvalidHeaderEncoding)?;

    let mut lines = head_str.split("\r\n");
    let request_line = lines.next().ok_or(ParseError::InvalidRequestLine)?;
    if request_line.len() > LINE_LIMIT {
        return Err(ParseError::RequestLineTooLong);
    }

    let (method, target, version) = parse_request_line(request_line)?;

    let mut headers = Headers::new();
    let mut host_value: Option<String> = None;
    let mut host_seen = false;
    let mut content_length: Option<u64> = None;
    let mut transfer_encodings: Vec<String> = Vec::new();

    for line in lines {
        if line.is_empty() {
            break;
        }
        if line.len() > LINE_LIMIT {
            return Err(ParseError::HeaderLineTooLong);
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(ParseError::ObsoleteLineFolding);
        }

        let (name_str, value_str) = split_header_line(line)?;
        if !is_field_name(name_str) {
            return Err(ParseError::InvalidHeaderName);
        }

        let value = value_str.trim_matches(|c| matches!(c, ' ' | '\t'));
        if value.len() > LINE_LIMIT {
            return Err(ParseError::HeaderLineTooLong);
        }
        if contains_invalid_header_value(value) {
            return Err(ParseError::InvalidHeaderValue);
        }

        let name = HeaderName::new(name_str);
        let name_key = name.as_str();

        match name_key {
            header_keys::HOST => {
                if host_seen {
                    return Err(ParseError::MultipleHostValues);
                }
                host_seen = true;
                if !is_valid_host(value) {
                    return Err(ParseError::InvalidHost);
                }
                host_value = Some(value.to_string());
            }
            header_keys::CONTENT_LENGTH => {
                let length = parse_content_length(value)?;
                if let Some(existing) = content_length {
                    if existing != length {
                        return Err(ParseError::ConflictingContentLength);
                    }
                } else {
                    content_length = Some(length);
                }
            }
            header_keys::TRANSFER_ENCODING => {
                let codings = parse_transfer_encoding(value)?;
                transfer_encodings.extend(codings);
            }
            _ => {}
        }

        headers.append(name, value.to_string());
    }

    if host_value.is_none() {
        return Err(ParseError::MissingHost);
    }

    let body_mode = determine_body_mode(content_length, &transfer_encodings)?;

    let mut request = Request::new(method, target);
    request.set_version(version);
    request.set_body(match body_mode {
        BodyMode::None => Body::Empty,
        BodyMode::Fixed(len) => Body::Fixed(len),
        BodyMode::Chunked => Body::Chunked,
    });
    *request.headers_mut() = headers;

    Ok((request, body_mode, head_end))
}

/// Message body framing modes detected from request headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMode {
    None,
    Fixed(u64),
    Chunked,
}

/// Construct a body reader for the supplied mode.
pub fn body_reader<'a, R>(
    mode: BodyMode,
    reader: &'a mut R,
    buffer: &'a mut BytesMut,
) -> BodyReader<'a, R>
where
    R: AsyncRead + Unpin,
{
    match mode {
        BodyMode::None => BodyReader {
            inner: BodyReaderInner::Empty,
        },
        BodyMode::Fixed(remaining) => BodyReader {
            inner: BodyReaderInner::Fixed(FixedBodyReader {
                reader,
                buffer,
                remaining,
            }),
        },
        BodyMode::Chunked => BodyReader {
            inner: BodyReaderInner::Chunked(ChunkedBodyReader::new(reader, buffer)),
        },
    }
}

/// Reader for request bodies.
pub struct BodyReader<'a, R>
where
    R: AsyncRead + Unpin,
{
    inner: BodyReaderInner<'a, R>,
}

impl<'a, R> BodyReader<'a, R>
where
    R: AsyncRead + Unpin,
{
    /// Read the next available body chunk. Returns `Ok(None)` once the body has
    /// been fully consumed.
    pub async fn read_next(&mut self) -> Result<Option<Bytes>, BodyError> {
        match &mut self.inner {
            BodyReaderInner::Empty => Ok(None),
            BodyReaderInner::Fixed(inner) => inner.read_next().await,
            BodyReaderInner::Chunked(inner) => inner.read_next().await,
        }
    }

    /// Drain the remaining body, discarding any data.
    pub async fn drain(&mut self) -> Result<(), BodyError> {
        while self.read_next().await?.is_some() {}
        Ok(())
    }

    /// Return parsed trailers if chunked decoding completed successfully.
    pub fn trailers(&self) -> Option<&Headers> {
        match &self.inner {
            BodyReaderInner::Chunked(inner) if inner.trailers_complete => Some(&inner.trailers),
            _ => None,
        }
    }

    /// Indicates whether the body reader has consumed all data.
    pub fn is_finished(&self) -> bool {
        match &self.inner {
            BodyReaderInner::Empty => true,
            BodyReaderInner::Fixed(inner) => inner.remaining == 0,
            BodyReaderInner::Chunked(inner) => inner.state == ChunkState::Done,
        }
    }
}

enum BodyReaderInner<'a, R>
where
    R: AsyncRead + Unpin,
{
    Empty,
    Fixed(FixedBodyReader<'a, R>),
    Chunked(ChunkedBodyReader<'a, R>),
}

struct FixedBodyReader<'a, R>
where
    R: AsyncRead + Unpin,
{
    reader: &'a mut R,
    buffer: &'a mut BytesMut,
    remaining: u64,
}

impl<'a, R> FixedBodyReader<'a, R>
where
    R: AsyncRead + Unpin,
{
    async fn read_next(&mut self) -> Result<Option<Bytes>, BodyError> {
        if self.remaining == 0 {
            return Ok(None);
        }

        if !self.buffer.is_empty() {
            let available = min(self.buffer.len() as u64, self.remaining) as usize;
            let chunk = self.buffer.split_to(available).freeze();
            self.remaining -= available as u64;
            return Ok(Some(chunk));
        }

        let to_read = min(self.remaining, 8 * 1024) as usize;
        let mut temp = vec![0_u8; to_read];
        let read = self.reader.read(&mut temp).await?;
        if read == 0 {
            return Err(BodyError::UnexpectedEof);
        }
        self.remaining -= read as u64;
        temp.truncate(read);
        Ok(Some(Bytes::from(temp)))
    }
}

struct ChunkedBodyReader<'a, R>
where
    R: AsyncRead + Unpin,
{
    reader: &'a mut R,
    buffer: &'a mut BytesMut,
    state: ChunkState,
    current_chunk_remaining: u64,
    trailers: Headers,
    trailers_complete: bool,
}

impl<'a, R> ChunkedBodyReader<'a, R>
where
    R: AsyncRead + Unpin,
{
    fn new(reader: &'a mut R, buffer: &'a mut BytesMut) -> Self {
        Self {
            reader,
            buffer,
            state: ChunkState::ReadingSize,
            current_chunk_remaining: 0,
            trailers: Headers::new(),
            trailers_complete: false,
        }
    }

    async fn read_next(&mut self) -> Result<Option<Bytes>, BodyError> {
        loop {
            match self.state {
                ChunkState::ReadingSize => {
                    let line = self.read_line().await?;
                    let size = parse_chunk_size(&line)?;
                    if size == 0 {
                        self.state = ChunkState::ReadingTrailers;
                    } else {
                        self.current_chunk_remaining = size;
                        self.state = ChunkState::ReadingData;
                    }
                }
                ChunkState::ReadingData => {
                    if self.current_chunk_remaining == 0 {
                        self.state = ChunkState::ExpectingCrLf;
                        continue;
                    }

                    if !self.buffer.is_empty() {
                        let available =
                            min(self.buffer.len() as u64, self.current_chunk_remaining) as usize;
                        let chunk = self.buffer.split_to(available).freeze();
                        self.current_chunk_remaining -= available as u64;
                        if self.current_chunk_remaining == 0 {
                            self.state = ChunkState::ExpectingCrLf;
                        }
                        return Ok(Some(chunk));
                    }

                    let to_read = min(self.current_chunk_remaining, 8 * 1024) as usize;
                    let mut temp = vec![0_u8; to_read];
                    let read = self.reader.read(&mut temp).await?;
                    if read == 0 {
                        return Err(BodyError::UnexpectedEof);
                    }
                    self.current_chunk_remaining -= read as u64;
                    temp.truncate(read);
                    if self.current_chunk_remaining == 0 {
                        self.state = ChunkState::ExpectingCrLf;
                    }
                    return Ok(Some(Bytes::from(temp)));
                }
                ChunkState::ExpectingCrLf => {
                    if self.buffer.len() < 2 {
                        let read = self.reader.read_buf(self.buffer).await?;
                        if read == 0 {
                            return Err(BodyError::UnexpectedEof);
                        }
                        continue;
                    }
                    if &self.buffer[..2] != b"\r\n" {
                        return Err(BodyError::InvalidChunk);
                    }
                    self.buffer.advance(2);
                    self.state = ChunkState::ReadingSize;
                }
                ChunkState::ReadingTrailers => {
                    let line = self.read_line().await?;
                    if line.is_empty() {
                        self.state = ChunkState::Done;
                        self.trailers_complete = true;
                        return Ok(None);
                    }
                    let (name_str, value_str) =
                        split_header_line(&line).map_err(BodyError::InvalidTrailer)?;
                    if !is_field_name(name_str) {
                        return Err(BodyError::InvalidTrailer(ParseError::InvalidHeaderName));
                    }
                    let value = value_str.trim_matches(|c| matches!(c, ' ' | '\t'));
                    if contains_invalid_header_value(value) {
                        return Err(BodyError::InvalidTrailer(ParseError::InvalidHeaderValue));
                    }
                    self.trailers
                        .append(HeaderName::new(name_str), value.to_string());
                }
                ChunkState::Done => return Ok(None),
            }
        }
    }

    async fn read_line(&mut self) -> Result<String, BodyError> {
        loop {
            if let Some(pos) = find_crlf(self.buffer) {
                let mut line = self.buffer.split_to(pos + 2);
                line.truncate(pos);
                return String::from_utf8(line.to_vec()).map_err(|_| BodyError::InvalidChunk);
            }
            let read = self.reader.read_buf(self.buffer).await?;
            if read == 0 {
                return Err(BodyError::UnexpectedEof);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkState {
    ReadingSize,
    ReadingData,
    ExpectingCrLf,
    ReadingTrailers,
    Done,
}

/// Errors produced while parsing the request head.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("request head incomplete")]
    Incomplete,
    #[error("request line too long")]
    RequestLineTooLong,
    #[error("invalid request line")]
    InvalidRequestLine,
    #[error("invalid method token")]
    InvalidMethod,
    #[error("invalid HTTP version")]
    InvalidVersion,
    #[error("invalid request target")]
    InvalidRequestTarget,
    #[error("header section exceeds limit")]
    HeaderTooLarge,
    #[error("header line exceeds limit")]
    HeaderLineTooLong,
    #[error("obsolete line folding detected")]
    ObsoleteLineFolding,
    #[error("invalid header name")]
    InvalidHeaderName,
    #[error("invalid header value")]
    InvalidHeaderValue,
    #[error("invalid Host header value")]
    InvalidHost,
    #[error("multiple Host header values are not allowed")]
    MultipleHostValues,
    #[error("required Host header missing")]
    MissingHost,
    #[error("invalid Content-Length value")]
    InvalidContentLength,
    #[error("conflicting Content-Length values")]
    ConflictingContentLength,
    #[error("invalid Transfer-Encoding value")]
    InvalidTransferEncoding,
    #[error("Transfer-Encoding and Content-Length conflict")]
    ConflictingLengthAndTransferEncoding,
    #[error("unsupported Transfer-Encoding")]
    UnsupportedTransferEncoding,
    #[error("invalid header encoding")]
    InvalidHeaderEncoding,
}

/// Errors produced while decoding the message body.
#[derive(Debug, Error)]
pub enum BodyError {
    #[error("unexpected end of stream")]
    UnexpectedEof,
    #[error("invalid chunked body")]
    InvalidChunk,
    #[error("invalid trailer: {0}")]
    InvalidTrailer(ParseError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn parse_request_line(line: &str) -> Result<(Method, RequestTarget, HttpVersion), ParseError> {
    let bytes = line.as_bytes();
    let first_space = bytes
        .iter()
        .position(|&b| b == b' ')
        .ok_or(ParseError::InvalidRequestLine)?;
    let method_str = &line[..first_space];
    if method_str.is_empty() {
        return Err(ParseError::InvalidMethod);
    }

    let rest = &line[first_space + 1..];
    let second_space = rest
        .as_bytes()
        .iter()
        .position(|&b| b == b' ')
        .ok_or(ParseError::InvalidRequestLine)?;
    let target_str = &rest[..second_space];
    if target_str.is_empty() {
        return Err(ParseError::InvalidRequestTarget);
    }

    let version_str = &rest[second_space + 1..];
    if version_str.is_empty() || version_str.contains(' ') {
        return Err(ParseError::InvalidVersion);
    }

    let method = Method::from_str(method_str).map_err(|_| ParseError::InvalidMethod)?;
    let target = parse_request_target(&method, target_str)?;
    let version = HttpVersion::from_str(version_str).map_err(|_| ParseError::InvalidVersion)?;
    if !matches!(version, HttpVersion::Http10 | HttpVersion::Http11) {
        return Err(ParseError::InvalidVersion);
    }

    Ok((method, target, version))
}

fn parse_request_target(method: &Method, target: &str) -> Result<RequestTarget, ParseError> {
    if target == "*" {
        if matches!(method, Method::Options) {
            return Ok(RequestTarget::Asterisk);
        }
        return Err(ParseError::InvalidRequestTarget);
    }

    if target.starts_with('/') {
        return Ok(RequestTarget::origin(target.to_string()));
    }

    if matches!(method, Method::Connect) {
        if is_authority_form(target) {
            return Ok(RequestTarget::Authority(target.to_string()));
        }
        return Err(ParseError::InvalidRequestTarget);
    }

    if target.contains("://") {
        return Ok(RequestTarget::Absolute(target.to_string()));
    }

    Err(ParseError::InvalidRequestTarget)
}

fn split_header_line(line: &str) -> Result<(&str, &str), ParseError> {
    let (name, value) = line.split_once(':').ok_or(ParseError::InvalidHeaderName)?;
    Ok((name, value))
}

fn parse_content_length(value: &str) -> Result<u64, ParseError> {
    if value.is_empty() {
        return Err(ParseError::InvalidContentLength);
    }
    value
        .parse::<u64>()
        .map_err(|_| ParseError::InvalidContentLength)
}

fn parse_transfer_encoding(value: &str) -> Result<Vec<String>, ParseError> {
    let mut codings = Vec::new();
    for coding in value.split(',') {
        let token = coding.trim();
        if token.is_empty() || !is_token(token) {
            return Err(ParseError::InvalidTransferEncoding);
        }
        codings.push(token.to_ascii_lowercase());
    }
    if codings.is_empty() {
        return Err(ParseError::InvalidTransferEncoding);
    }
    Ok(codings)
}

fn determine_body_mode(
    content_length: Option<u64>,
    transfer_encodings: &[String],
) -> Result<BodyMode, ParseError> {
    if !transfer_encodings.is_empty() {
        let chunked_positions: Vec<usize> = transfer_encodings
            .iter()
            .enumerate()
            .filter_map(|(idx, coding)| coding.eq_ignore_ascii_case("chunked").then_some(idx))
            .collect();

        if chunked_positions.is_empty() {
            return Err(ParseError::UnsupportedTransferEncoding);
        }
        if chunked_positions.len() > 1
            || *chunked_positions.last().unwrap() != transfer_encodings.len() - 1
        {
            return Err(ParseError::InvalidTransferEncoding);
        }
        if content_length.is_some() {
            return Err(ParseError::ConflictingLengthAndTransferEncoding);
        }
        return Ok(BodyMode::Chunked);
    }

    if let Some(length) = content_length {
        return Ok(BodyMode::Fixed(length));
    }

    Ok(BodyMode::None)
}

fn find_headers_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| idx + 4)
}

fn find_crlf(buffer: &BytesMut) -> Option<usize> {
    buffer.windows(2).position(|window| window == b"\r\n")
}

fn is_field_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(is_tchar)
}

fn is_token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(is_tchar)
}

const fn is_tchar(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`'
            | b'|' | b'~'
            | b'0'..=b'9'
            | b'A'..=b'Z'
            | b'a'..=b'z'
    )
}

fn contains_invalid_header_value(value: &str) -> bool {
    value.bytes().any(|b| matches!(b, 0..=8 | 10..=31 | 127))
}

fn is_valid_host(value: &str) -> bool {
    if value.is_empty() || value.contains(' ') || value.contains('\t') {
        return false;
    }

    if value.starts_with('[') {
        let Some(end) = value.find(']') else {
            return false;
        };
        let addr = &value[1..end];
        if addr.is_empty() || !addr.chars().all(|c| c.is_ascii_hexdigit() || c == ':') {
            return false;
        }
        let remainder = &value[end + 1..];
        if remainder.is_empty() {
            return true;
        }
        if let Some(port) = remainder.strip_prefix(':') {
            return !port.is_empty() && port.chars().all(|c| c.is_ascii_digit());
        }
        return false;
    }

    let mut parts = value.splitn(2, ':');
    let host = parts.next().unwrap_or("");
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return false;
    }

    if let Some(port) = parts.next()
        && (port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()))
    {
        return false;
    }

    true
}

fn parse_chunk_size(line: &str) -> Result<u64, BodyError> {
    let (size_str, _) = line.split_once(';').unwrap_or((line, ""));
    u64::from_str_radix(size_str.trim(), 16).map_err(|_| BodyError::InvalidChunk)
}

fn is_authority_form(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if value.starts_with('[') {
        if let Some(end) = value.find(']') {
            let port = value[end + 1..].strip_prefix(':');
            return port
                .map(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
                .unwrap_or(false);
        }
        return false;
    }
    let mut parts = value.splitn(2, ':');
    let host = parts.next().unwrap_or("");
    let port = parts.next();
    !host.is_empty()
        && port
            .map(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn detect_headers_end() {
        let data = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
        assert_eq!(find_headers_end(data), Some(data.len()));
    }

    #[test]
    fn parse_simple_request() {
        let data = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let (request, mode, consumed) = parse_request_head(data).unwrap();
        assert_eq!(consumed, data.len());
        assert_eq!(request.method().as_str(), "GET");
        assert_eq!(request.version(), HttpVersion::HTTP_1_1);
        assert_eq!(mode, BodyMode::None);
    }

    #[test]
    fn rejects_invalid_origin_targets_for_non_connect_methods() {
        let bare_target = b"GET noslash HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let authority_target = b"GET example.com:80 HTTP/1.1\r\nHost: example.com\r\n\r\n";

        assert!(matches!(
            parse_request_head(bare_target),
            Err(ParseError::InvalidRequestTarget)
        ));
        assert!(matches!(
            parse_request_head(authority_target),
            Err(ParseError::InvalidRequestTarget)
        ));
    }

    #[test]
    fn accepts_valid_non_origin_target_forms() {
        let absolute = b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let options = b"OPTIONS * HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let connect = b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n";

        assert!(parse_request_head(absolute).is_ok());
        assert!(parse_request_head(options).is_ok());
        assert!(parse_request_head(connect).is_ok());
    }

    #[tokio::test]
    async fn fixed_body_reader_consumes_buffered_bytes() {
        let mut buf = BytesMut::from(&b"hello"[..]);
        let mut reader = tokio::io::empty();
        let mut body = body_reader(BodyMode::Fixed(5), &mut reader, &mut buf);
        let chunk = body.read_next().await.unwrap().unwrap();
        assert_eq!(&chunk[..], b"hello");
        assert!(body.read_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn chunked_reader_parses_small_chunk() {
        let mut buf = BytesMut::from(&b"4\r\nRust\r\n0\r\n\r\n"[..]);
        let mut reader = tokio::io::empty();
        let mut body = body_reader(BodyMode::Chunked, &mut reader, &mut buf);
        let chunk = body.read_next().await.unwrap().unwrap();
        assert_eq!(&chunk[..], b"Rust");
        assert!(body.read_next().await.unwrap().is_none());
    }
}
