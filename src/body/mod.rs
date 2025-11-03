//! Abstractions for request and response bodies.
//!
//! The `Body` enum will allow streaming of fixed-size, chunked, or empty
//! payloads. For now we supply a simplified scaffold.

/// Represents the body of an HTTP message.
#[derive(Debug, Clone)]
pub enum Body {
    /// No body is present.
    Empty,
    /// A fixed-length body with the specified number of bytes.
    Fixed(u64),
    /// A chunked transfer-coded body.
    Chunked,
}

impl Default for Body {
    fn default() -> Self {
        Body::Empty
    }
}
