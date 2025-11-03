//! HTTP request representation.
//!
//! The concrete implementation will mirror RFC 9110 semantics, including
//! support for request methods, target forms, and header access. This scaffold
//! provides the basic structure needed by other modules during development.

use std::time::SystemTime;

use bytes::Bytes;

use crate::{
    body::Body,
    common::{HttpVersion, Method},
    date,
    headers::{HeaderName, Headers, header_keys},
};

/// HTTP request target representation (RFC 9112 Section 3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestTarget {
    /// origin-form (most common): `"/path?query"`.
    Origin(String),
    /// absolute-form: full URI for proxies.
    Absolute(String),
    /// authority-form used with CONNECT: `"host:port"`.
    Authority(String),
    /// asterisk-form used with OPTIONS.
    Asterisk,
}

impl RequestTarget {
    /// Create an origin-form target.
    pub fn origin(path: impl Into<String>) -> Self {
        RequestTarget::Origin(path.into())
    }

    /// Returns a borrowed string representation.
    pub fn as_str(&self) -> &str {
        match self {
            RequestTarget::Origin(s) | RequestTarget::Absolute(s) | RequestTarget::Authority(s) => {
                s.as_str()
            }
            RequestTarget::Asterisk => "*",
        }
    }
}

/// Represents an HTTP request.
#[derive(Debug, Clone)]
pub struct Request {
    method: Method,
    target: RequestTarget,
    version: HttpVersion,
    headers: Headers,
    body: Body,
    payload: Bytes,
}

impl Request {
    /// Create a new request using HTTP/1.1 as the default version.
    pub fn new(method: Method, target: RequestTarget) -> Self {
        Self {
            method,
            target,
            version: HttpVersion::HTTP_1_1,
            headers: Headers::new(),
            body: Body::Empty,
            payload: Bytes::new(),
        }
    }

    /// Construct a request builder for incremental configuration.
    pub fn builder(method: Method, target: RequestTarget) -> RequestBuilder {
        RequestBuilder::new(method, target)
    }

    /// Return the request method.
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// Return the request target.
    pub fn target(&self) -> &RequestTarget {
        &self.target
    }

    /// Return the HTTP version.
    pub fn version(&self) -> HttpVersion {
        self.version
    }

    /// Borrow the header map.
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Retrieve the first value for the specified header name.
    pub fn header(&self, name: impl Into<HeaderName>) -> Option<&str> {
        self.headers.get(name)
    }

    /// Returns true if a header with the specified name is present.
    pub fn contains_header(&self, name: impl Into<HeaderName>) -> bool {
        self.headers.contains(name)
    }

    /// Borrow the body representation.
    pub fn body(&self) -> &Body {
        &self.body
    }

    /// Borrow the buffered body bytes.
    pub fn body_bytes(&self) -> &Bytes {
        &self.payload
    }

    /// Borrow the header map mutably.
    pub fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// Set the HTTP version for the request.
    pub fn set_version(&mut self, version: HttpVersion) {
        self.version = version;
    }

    /// Set the body representation.
    pub fn set_body(&mut self, body: Body) {
        self.body = body;
    }

    /// Replace the buffered body bytes.
    pub fn set_body_bytes(&mut self, bytes: impl Into<Bytes>) {
        self.payload = bytes.into();
    }

    /// Consume the request and return the buffered body bytes.
    pub fn into_body_bytes(self) -> Bytes {
        self.payload
    }

    /// Remove and return the buffered body bytes, leaving an empty buffer.
    pub fn take_body_bytes(&mut self) -> Bytes {
        std::mem::take(&mut self.payload)
    }

    /// Whether the request includes `Expect: 100-continue` in its header section.
    pub fn expect_100_continue(&self) -> bool {
        self.header(header_keys::EXPECT)
            .map(|value| {
                value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("100-continue"))
            })
            .unwrap_or(false)
    }

    /// Determine whether the request prefers the connection to close.
    pub fn wants_close(&self) -> bool {
        self.header(header_keys::CONNECTION).map_or(false, |value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("close"))
        })
    }

    /// Retrieve the If-None-Match header value trimmed of whitespace.
    pub fn if_none_match(&self) -> Option<&str> {
        self.header(header_keys::IF_NONE_MATCH)
            .map(|value| value.trim())
    }

    /// Retrieve the If-Match header value trimmed of whitespace.
    pub fn if_match(&self) -> Option<&str> {
        self.header(header_keys::IF_MATCH).map(|value| value.trim())
    }

    /// Retrieve the If-Unmodified-Since value as `SystemTime`.
    pub fn if_unmodified_since(&self) -> Option<SystemTime> {
        self.header(header_keys::IF_UNMODIFIED_SINCE)
            .and_then(|value| date::parse_http_date(value.trim()))
    }

    /// Retrieve the If-Modified-Since value as `SystemTime`.
    pub fn if_modified_since(&self) -> Option<SystemTime> {
        self.header(header_keys::IF_MODIFIED_SINCE)
            .and_then(|value| date::parse_http_date(value.trim()))
    }
}

/// Builder for `Request` values.
#[derive(Debug, Clone)]
pub struct RequestBuilder {
    method: Method,
    target: RequestTarget,
    version: HttpVersion,
    headers: Headers,
    body: Body,
    payload: Bytes,
}

impl RequestBuilder {
    fn new(method: Method, target: RequestTarget) -> Self {
        Self {
            method,
            target,
            version: HttpVersion::HTTP_1_1,
            headers: Headers::new(),
            body: Body::Empty,
            payload: Bytes::new(),
        }
    }

    /// Override the HTTP version.
    pub fn version(mut self, version: HttpVersion) -> Self {
        self.version = version;
        self
    }

    /// Insert or replace a header.
    pub fn header(mut self, name: impl Into<HeaderName>, value: impl Into<String>) -> Self {
        self.headers.insert(name, value);
        self
    }

    /// Set the body representation.
    pub fn body(mut self, body: Body) -> Self {
        self.body = body;
        self
    }

    /// Buffer body bytes that will be attached to the request.
    pub fn body_bytes(mut self, bytes: impl Into<Bytes>) -> Self {
        self.payload = bytes.into();
        self
    }

    /// Build the request.
    pub fn build(self) -> Request {
        Request {
            method: self.method,
            target: self.target,
            version: self.version,
            headers: self.headers,
            body: self.body,
            payload: self.payload,
        }
    }
}
