//! Core HTTP types shared across request and response handling.
//!
//! Implements strongly typed representations of methods, status codes, and
//! protocol versions in line with RFC 9110.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// HTTP method token (RFC 9110 Section 9).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Delete,
    Connect,
    Options,
    Trace,
    Patch,
    Extension(Box<str>),
}

impl Method {
    /// Returns the canonical string representation of the method.
    pub fn as_str(&self) -> &str {
        match self {
            Method::Get => "GET",
            Method::Head => "HEAD",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Connect => "CONNECT",
            Method::Options => "OPTIONS",
            Method::Trace => "TRACE",
            Method::Patch => "PATCH",
            Method::Extension(token) => token,
        }
    }

    /// True if the method is request-body safe by default (spec semantics).
    pub fn is_idempotent(&self) -> bool {
        matches!(
            self,
            Method::Get
                | Method::Head
                | Method::Put
                | Method::Delete
                | Method::Options
                | Method::Trace
        )
    }

    /// Determine if the method is cacheable by default.
    pub fn is_cacheable(&self) -> bool {
        matches!(self, Method::Get | Method::Head)
    }

    /// Determine if the method is defined to be safe (no state change).
    pub fn is_safe(&self) -> bool {
        matches!(
            self,
            Method::Get | Method::Head | Method::Options | Method::Trace
        )
    }

    fn from_token(token: &str) -> Result<Self, MethodError> {
        match token {
            "GET" => Ok(Method::Get),
            "HEAD" => Ok(Method::Head),
            "POST" => Ok(Method::Post),
            "PUT" => Ok(Method::Put),
            "DELETE" => Ok(Method::Delete),
            "CONNECT" => Ok(Method::Connect),
            "OPTIONS" => Ok(Method::Options),
            "TRACE" => Ok(Method::Trace),
            "PATCH" => Ok(Method::Patch),
            _ => {
                if is_token(token) {
                    Ok(Method::Extension(token.into()))
                } else {
                    Err(MethodError::InvalidToken)
                }
            }
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Method {
    type Err = MethodError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Method::from_token(s)
    }
}

/// Errors that can occur while parsing methods.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MethodError {
    #[error("invalid method token")]
    InvalidToken,
}

/// HTTP protocol version representation (RFC 9112 Section 2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpVersion {
    Http10,
    Http11,
    Other(u8, u8),
}

impl HttpVersion {
    /// Returns the major component of the version.
    pub fn major(self) -> u8 {
        match self {
            HttpVersion::Http10 => 1,
            HttpVersion::Http11 => 1,
            HttpVersion::Other(major, _) => major,
        }
    }

    /// Returns the minor component of the version.
    pub fn minor(self) -> u8 {
        match self {
            HttpVersion::Http10 => 0,
            HttpVersion::Http11 => 1,
            HttpVersion::Other(_, minor) => minor,
        }
    }

    /// Returns a static HTTP/1.1 variant.
    pub const HTTP_1_1: HttpVersion = HttpVersion::Http11;
}

impl fmt::Display for HttpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpVersion::Http10 => f.write_str("HTTP/1.0"),
            HttpVersion::Http11 => f.write_str("HTTP/1.1"),
            HttpVersion::Other(major, minor) => write!(f, "HTTP/{major}.{minor}"),
        }
    }
}

impl FromStr for HttpVersion {
    type Err = VersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("HTTP/") {
            return Err(VersionError::InvalidPrefix);
        }
        let remainder = &s[5..];
        let mut parts = remainder.split('.');
        let major = parts
            .next()
            .ok_or(VersionError::InvalidFormat)?
            .parse::<u8>()
            .map_err(|_| VersionError::InvalidNumber)?;
        let minor = parts
            .next()
            .ok_or(VersionError::InvalidFormat)?
            .parse::<u8>()
            .map_err(|_| VersionError::InvalidNumber)?;
        if parts.next().is_some() {
            return Err(VersionError::InvalidFormat);
        }
        Ok(match (major, minor) {
            (1, 0) => HttpVersion::Http10,
            (1, 1) => HttpVersion::Http11,
            (maj, min) => HttpVersion::Other(maj, min),
        })
    }
}

/// Errors while parsing HTTP versions.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VersionError {
    #[error("invalid HTTP version prefix")]
    InvalidPrefix,
    #[error("invalid HTTP version format")]
    InvalidFormat,
    #[error("invalid HTTP version number")]
    InvalidNumber,
}

/// Status codes (RFC 9110 Section 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StatusCode(u16);

impl StatusCode {
    /// Creates a new status code if it falls within the valid range 100-599.
    pub fn from_u16(code: u16) -> Result<Self, StatusCodeError> {
        if (100..=599).contains(&code) {
            Ok(StatusCode(code))
        } else {
            Err(StatusCodeError::OutOfRange(code))
        }
    }

    /// Returns the inner numeric code.
    pub fn as_u16(self) -> u16 {
        self.0
    }

    /// Returns the canonical reason phrase if known.
    pub fn canonical_reason(self) -> Option<&'static str> {
        match self.0 {
            100 => Some("Continue"),
            101 => Some("Switching Protocols"),
            102 => Some("Processing"),
            103 => Some("Early Hints"),
            200 => Some("OK"),
            201 => Some("Created"),
            202 => Some("Accepted"),
            203 => Some("Non-Authoritative Information"),
            204 => Some("No Content"),
            205 => Some("Reset Content"),
            206 => Some("Partial Content"),
            300 => Some("Multiple Choices"),
            301 => Some("Moved Permanently"),
            302 => Some("Found"),
            303 => Some("See Other"),
            304 => Some("Not Modified"),
            305 => Some("Use Proxy"),
            307 => Some("Temporary Redirect"),
            308 => Some("Permanent Redirect"),
            400 => Some("Bad Request"),
            401 => Some("Unauthorized"),
            402 => Some("Payment Required"),
            403 => Some("Forbidden"),
            404 => Some("Not Found"),
            405 => Some("Method Not Allowed"),
            406 => Some("Not Acceptable"),
            407 => Some("Proxy Authentication Required"),
            408 => Some("Request Timeout"),
            409 => Some("Conflict"),
            410 => Some("Gone"),
            411 => Some("Length Required"),
            412 => Some("Precondition Failed"),
            413 => Some("Content Too Large"),
            414 => Some("URI Too Long"),
            415 => Some("Unsupported Media Type"),
            416 => Some("Range Not Satisfiable"),
            417 => Some("Expectation Failed"),
            418 => Some("I'm a teapot"),
            421 => Some("Misdirected Request"),
            422 => Some("Unprocessable Content"),
            426 => Some("Upgrade Required"),
            428 => Some("Precondition Required"),
            429 => Some("Too Many Requests"),
            431 => Some("Request Header Fields Too Large"),
            451 => Some("Unavailable For Legal Reasons"),
            500 => Some("Internal Server Error"),
            501 => Some("Not Implemented"),
            502 => Some("Bad Gateway"),
            503 => Some("Service Unavailable"),
            504 => Some("Gateway Timeout"),
            505 => Some("HTTP Version Not Supported"),
            511 => Some("Network Authentication Required"),
            _ => None,
        }
    }

    pub const CONTINUE: StatusCode = StatusCode(100);
    pub const OK: StatusCode = StatusCode(200);
    pub const CREATED: StatusCode = StatusCode(201);
    pub const PARTIAL_CONTENT: StatusCode = StatusCode(206);
    pub const NOT_FOUND: StatusCode = StatusCode(404);
    pub const NOT_MODIFIED: StatusCode = StatusCode(304);
    pub const BAD_REQUEST: StatusCode = StatusCode(400);
    pub const REQUEST_TIMEOUT: StatusCode = StatusCode(408);
    pub const METHOD_NOT_ALLOWED: StatusCode = StatusCode(405);
    pub const PRECONDITION_FAILED: StatusCode = StatusCode(412);
    pub const RANGE_NOT_SATISFIABLE: StatusCode = StatusCode(416);
    pub const EXPECTATION_FAILED: StatusCode = StatusCode(417);
    pub const INTERNAL_SERVER_ERROR: StatusCode = StatusCode(500);
    pub const NOT_IMPLEMENTED: StatusCode = StatusCode(501);
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<u16> for StatusCode {
    type Error = StatusCodeError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        StatusCode::from_u16(value)
    }
}

impl From<StatusCode> for u16 {
    fn from(code: StatusCode) -> Self {
        code.0
    }
}

/// Errors that occur when constructing status codes.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StatusCodeError {
    #[error("status code {0} is outside the valid range 100-599")]
    OutOfRange(u16),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_parsing_standard() {
        assert_eq!(Method::from_str("GET").unwrap(), Method::Get);
        assert_eq!(Method::from_str("POST").unwrap(), Method::Post);
    }

    #[test]
    fn method_parsing_extension() {
        let ext = Method::from_str("FOO").unwrap();
        assert!(matches!(ext, Method::Extension(_)));
        assert_eq!(ext.as_str(), "FOO");
    }

    #[test]
    fn method_invalid_token() {
        assert_eq!(Method::from_str("inv alid"), Err(MethodError::InvalidToken));
        assert_eq!(Method::from_str(""), Err(MethodError::InvalidToken));
    }

    #[test]
    fn version_parsing() {
        assert_eq!(
            HttpVersion::from_str("HTTP/1.1").unwrap(),
            HttpVersion::Http11
        );
        assert_eq!(
            HttpVersion::from_str("HTTP/1.0").unwrap(),
            HttpVersion::Http10
        );
        assert_eq!(
            HttpVersion::from_str("HTTP/2.0").unwrap(),
            HttpVersion::Other(2, 0)
        );
        assert!(HttpVersion::from_str("HTTP/1").is_err());
    }

    #[test]
    fn status_code_bounds() {
        assert!(StatusCode::from_u16(99).is_err());
        assert!(StatusCode::from_u16(600).is_err());
        assert_eq!(StatusCode::from_u16(200).unwrap().as_u16(), 200);
    }
}
