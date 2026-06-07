//! HTTP response representation.
//!
//! Provides the shell for building status lines, headers, and bodies. Detailed
//! semantics (validators, range headers, etc.) will be filled in future tasks.

use std::{fmt, io};

use bytes::Bytes;
use futures_core::Stream;

use crate::{
    common::{HttpVersion, StatusCode},
    headers::{HeaderName, Headers, header_keys},
};

/// Alias for boxed streaming response bodies.
pub type BoxBodyStream = Box<dyn Stream<Item = io::Result<Bytes>> + Send + Unpin>;

/// Represents the body of an HTTP response.
pub enum ResponseBody {
    /// No body content is present.
    Empty,
    /// Entire payload is buffered in memory.
    Full(Bytes),
    /// Payload will be produced lazily via chunked transfer encoding.
    Stream(BoxBodyStream),
}

impl fmt::Debug for ResponseBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResponseBody::Empty => f.write_str("ResponseBody::Empty"),
            ResponseBody::Full(bytes) => f.debug_tuple("ResponseBody::Full").field(bytes).finish(),
            ResponseBody::Stream(_) => f.write_str("ResponseBody::Stream(<stream>)"),
        }
    }
}

impl ResponseBody {
    /// Returns true if the body is empty.
    pub fn is_empty(&self) -> bool {
        matches!(self, ResponseBody::Empty)
    }

    /// Returns the known byte length when determinable.
    pub fn len(&self) -> Option<usize> {
        match self {
            ResponseBody::Empty => Some(0),
            ResponseBody::Full(bytes) => Some(bytes.len()),
            ResponseBody::Stream(_) => None,
        }
    }

    /// Indicates whether the body requires streaming.
    pub fn is_stream(&self) -> bool {
        matches!(self, ResponseBody::Stream(_))
    }
}

/// Represents an HTTP response message.
#[derive(Debug)]
pub struct Response {
    version: HttpVersion,
    status: StatusCode,
    reason: Option<String>,
    headers: Headers,
    body: ResponseBody,
    trailers: Option<Headers>,
}

impl Response {
    /// Create a new response with the provided status code and canonical reason.
    pub fn new(status: StatusCode) -> Self {
        Self {
            version: HttpVersion::HTTP_1_1,
            reason: None,
            status,
            headers: Headers::new(),
            body: ResponseBody::Empty,
            trailers: None,
        }
    }

    /// Construct a builder for the response.
    pub fn builder(status: StatusCode) -> ResponseBuilder {
        ResponseBuilder::new(status)
    }

    /// Return the HTTP version of the response.
    pub fn version(&self) -> HttpVersion {
        self.version
    }

    /// Return the status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Access the reason phrase used for the status line.
    pub fn reason_phrase(&self) -> &str {
        self.reason
            .as_deref()
            .or_else(|| self.status.canonical_reason())
            .unwrap_or("")
    }

    /// Borrow header map immutably.
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Borrow header map mutably.
    pub fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// Borrow the body representation.
    pub fn body(&self) -> &ResponseBody {
        &self.body
    }

    /// Override the status code.
    pub fn set_status(&mut self, status: StatusCode) {
        self.status = status;
    }

    /// Remove and return the body, leaving an empty placeholder behind.
    pub fn take_body(&mut self) -> ResponseBody {
        std::mem::replace(&mut self.body, ResponseBody::Empty)
    }

    /// Replace the body with the provided representation.
    pub fn set_body(&mut self, body: ResponseBody) {
        self.body = body;
    }

    /// Convenience helper for replacing the body with fully buffered bytes.
    pub fn set_body_bytes(&mut self, bytes: impl Into<Bytes>) {
        self.body = ResponseBody::Full(bytes.into());
    }

    /// Replace the body with a static byte slice without copying.
    pub fn set_body_static(&mut self, bytes: &'static [u8]) {
        self.body = ResponseBody::Full(Bytes::from_static(bytes));
    }

    /// Replace the body with a static UTF-8 string without copying.
    pub fn set_body_text_static(&mut self, text: &'static str) {
        self.set_body_static(text.as_bytes());
    }

    /// Prepare the response for a HEAD reply by stripping the body while
    /// preserving the advertised payload length when known.
    pub fn strip_body_for_head(&mut self) {
        if let ResponseBody::Full(bytes) = &self.body
            && !self.headers.contains(header_keys::CONTENT_LENGTH)
        {
            self.headers
                .insert(header_keys::CONTENT_LENGTH, bytes.len().to_string());
        }
        self.body = ResponseBody::Empty;
    }

    /// Borrow the trailers if present.
    pub fn trailers(&self) -> Option<&Headers> {
        self.trailers.as_ref()
    }

    /// Replace the trailers with the provided header set.
    pub fn set_trailers(&mut self, trailers: Headers) {
        self.trailers = Some(trailers);
    }

    /// Remove any trailers from the response and return them.
    pub fn take_trailers(&mut self) -> Option<Headers> {
        self.trailers.take()
    }

    /// Borrow the body mutably.
    pub fn body_mut(&mut self) -> &mut ResponseBody {
        &mut self.body
    }

    /// Override the HTTP version.
    pub fn set_version(&mut self, version: HttpVersion) {
        self.version = version;
    }

    /// Override the reason phrase.
    pub fn set_reason(&mut self, reason: impl Into<String>) {
        self.reason = Some(reason.into());
    }
}

/// Builder pattern for constructing responses.
#[derive(Debug)]
pub struct ResponseBuilder {
    version: HttpVersion,
    status: StatusCode,
    reason: Option<String>,
    headers: Headers,
    body: ResponseBody,
    trailers: Option<Headers>,
}

impl ResponseBuilder {
    fn new(status: StatusCode) -> Self {
        Self {
            version: HttpVersion::HTTP_1_1,
            status,
            reason: None,
            headers: Headers::new(),
            body: ResponseBody::Empty,
            trailers: None,
        }
    }

    /// Override the HTTP version.
    pub fn version(mut self, version: HttpVersion) -> Self {
        self.version = version;
        self
    }

    /// Override the reason phrase.
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Append a header value.
    pub fn header(mut self, name: impl Into<HeaderName>, value: impl Into<String>) -> Self {
        self.headers.insert(name, value);
        self
    }

    /// Set the response body representation.
    pub fn body(mut self, body: ResponseBody) -> Self {
        self.body = body;
        self
    }

    /// Provide a fully buffered body payload.
    pub fn body_bytes(mut self, bytes: impl Into<Bytes>) -> Self {
        self.body = ResponseBody::Full(bytes.into());
        self
    }

    /// Provide a static byte slice body payload without copying.
    pub fn body_static(mut self, bytes: &'static [u8]) -> Self {
        self.body = ResponseBody::Full(Bytes::from_static(bytes));
        self
    }

    /// Provide a static UTF-8 string body payload without copying.
    pub fn text_static(self, text: &'static str) -> Self {
        self.body_static(text.as_bytes())
    }

    /// Attach response trailers that will be emitted after the payload.
    pub fn trailers(mut self, trailers: Headers) -> Self {
        self.trailers = Some(trailers);
        self
    }

    /// Build the response.
    pub fn build(self) -> Response {
        Response {
            version: self.version,
            status: self.status,
            reason: self.reason,
            headers: self.headers,
            body: self.body,
            trailers: self.trailers,
        }
    }
}
