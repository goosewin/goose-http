//! HTTP header utilities and case-insensitive storage.
//!
//! This module provides the foundational header map implementation used across
//! the Goose HTTP server to store and retrieve header fields while preserving
//! insertion order and supporting multi-value semantics.

use std::collections::HashMap;
use std::fmt;

/// Represents an HTTP/1.1 header name with case-insensitive comparison.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeaderName(String);

impl HeaderName {
    /// Create a new header name (internally lowercased for case-insensitive comparison).
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        HeaderName(name.to_lowercase())
    }

    /// Get the header name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for HeaderName {
    fn from(s: &str) -> Self {
        HeaderName::new(s)
    }
}

impl From<String> for HeaderName {
    fn from(s: String) -> Self {
        HeaderName::new(s)
    }
}

impl fmt::Display for HeaderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Common HTTP header field names as string constants for efficient lookups.
pub mod header_keys {
    /// Content-Type header (RFC 9110 Section 8.3).
    pub const CONTENT_TYPE: &str = "content-type";
    /// Content-Encoding header (RFC 9110 Section 8.4).
    pub const CONTENT_ENCODING: &str = "content-encoding";
    /// Content-Language header (RFC 9110 Section 12.5.4).
    pub const CONTENT_LANGUAGE: &str = "content-language";
    /// Content-Location header (RFC 9110 Section 8.5).
    pub const CONTENT_LOCATION: &str = "content-location";

    /// Request and response framing headers.
    pub const CONTENT_LENGTH: &str = "content-length";
    pub const HOST: &str = "host";
    pub const TRANSFER_ENCODING: &str = "transfer-encoding";
    pub const CONNECTION: &str = "connection";
    pub const DATE: &str = "date";
    pub const EXPECT: &str = "expect";

    /// Authentication and caching related headers.
    pub const AUTHORIZATION: &str = "authorization";
    pub const CACHE_CONTROL: &str = "cache-control";
    pub const ETAG: &str = "etag";
    pub const EXPIRES: &str = "expires";
    pub const IF_MATCH: &str = "if-match";
    pub const IF_MODIFIED_SINCE: &str = "if-modified-since";
    pub const IF_UNMODIFIED_SINCE: &str = "if-unmodified-since";
    pub const IF_NONE_MATCH: &str = "if-none-match";
    pub const IF_RANGE: &str = "if-range";
    pub const LAST_MODIFIED: &str = "last-modified";
    pub const AGE: &str = "age";
    pub const VARY: &str = "vary";
    pub const PRAGMA: &str = "pragma";
    pub const RANGE: &str = "range";
    pub const CONTENT_RANGE: &str = "content-range";
    pub const ACCEPT_RANGES: &str = "accept-ranges";
    pub const ALLOW: &str = "allow";

    /// Content negotiation headers.
    pub const ACCEPT: &str = "accept";
    pub const ACCEPT_ENCODING: &str = "accept-encoding";
    pub const ACCEPT_LANGUAGE: &str = "accept-language";

    /// Miscellaneous headers often used in examples.
    pub const USER_AGENT: &str = "user-agent";
    pub const SERVER: &str = "server";
    pub const SET_COOKIE: &str = "set-cookie";
    pub const COOKIE: &str = "cookie";
    pub const LOCATION: &str = "location";
    pub const UPGRADE: &str = "upgrade";
}

/// Represents HTTP headers with support for multiple values per key.
#[derive(Debug, Clone)]
pub struct Headers {
    map: HashMap<HeaderName, Vec<String>>,
    order: Vec<HeaderName>,
}

impl Headers {
    /// Create a new empty header collection.
    pub fn new() -> Self {
        Headers {
            map: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Insert a header, replacing any existing values.
    pub fn insert(&mut self, name: impl Into<HeaderName>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();

        if !self.map.contains_key(&name) {
            self.order.push(name.clone());
        }

        self.map.insert(name, vec![value]);
    }

    /// Append a header value (for headers that can have multiple values).
    pub fn append(&mut self, name: impl Into<HeaderName>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();

        if !self.map.contains_key(&name) {
            self.order.push(name.clone());
        }

        self.map.entry(name).or_insert_with(Vec::new).push(value);
    }

    /// Get the first value for a header (most common use case).
    pub fn get(&self, name: impl Into<HeaderName>) -> Option<&str> {
        let name = name.into();
        self.map
            .get(&name)
            .and_then(|values| values.first().map(|s| s.as_str()))
    }

    /// Get all values for a header.
    pub fn get_all(&self, name: impl Into<HeaderName>) -> Option<&[String]> {
        let name = name.into();
        self.map.get(&name).map(|v| v.as_slice())
    }

    /// Check if a header exists.
    pub fn contains(&self, name: impl Into<HeaderName>) -> bool {
        let name = name.into();
        self.map.contains_key(&name)
    }

    /// Remove a header and return its values.
    pub fn remove(&mut self, name: impl Into<HeaderName>) -> Option<Vec<String>> {
        let name = name.into();
        if let Some(values) = self.map.remove(&name) {
            self.order.retain(|n| n != &name);
            Some(values)
        } else {
            None
        }
    }

    /// Get the number of unique header names.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Check if headers are empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterate over headers in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&HeaderName, &Vec<String>)> {
        self.order
            .iter()
            .filter_map(move |name| self.map.get(name).map(|values| (name, values)))
    }

    /// Clear all headers.
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

impl Default for Headers {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Headers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (name, values) in self.iter() {
            for value in values {
                writeln!(f, "{}: {}", name, value)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_case_insensitive_header_name() {
        let name1 = HeaderName::new("Content-Type");
        let name2 = HeaderName::new("content-type");
        let name3 = HeaderName::new("CONTENT-TYPE");

        assert_eq!(name1, name2);
        assert_eq!(name2, name3);
    }

    #[test]
    fn test_insert_and_get() {
        let mut headers = Headers::new();
        headers.insert("Content-Type", "application/json");

        assert_eq!(headers.get("content-type"), Some("application/json"));
        assert_eq!(headers.get("Content-Type"), Some("application/json"));
    }

    #[test]
    fn test_append_multiple_values() {
        let mut headers = Headers::new();
        headers.append("Accept", "text/html");
        headers.append("Accept", "application/json");

        let values = headers.get_all("accept").unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], "text/html");
        assert_eq!(values[1], "application/json");
    }

    #[test]
    fn test_insert_replaces() {
        let mut headers = Headers::new();
        headers.insert("Content-Type", "text/html");
        headers.insert("Content-Type", "application/json");

        let values = headers.get_all("content-type").unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], "application/json");
    }

    #[test]
    fn test_remove() {
        let mut headers = Headers::new();
        headers.insert("Content-Type", "application/json");

        let removed = headers.remove("content-type");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap(), vec!["application/json"]);
        assert!(headers.get("content-type").is_none());
    }

    #[test]
    fn test_order_preservation() {
        let mut headers = Headers::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Content-Length", "1234");
        headers.insert("Host", "example.com");

        let order: Vec<String> = headers
            .iter()
            .map(|(name, _)| name.as_str().to_string())
            .collect();

        assert_eq!(order, vec!["content-type", "content-length", "host"]);
    }
}
