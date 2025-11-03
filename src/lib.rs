//! Goose HTTP - foundational modules for a spec-compliant HTTP/1.1 server.
//!
//! This library exposes the building blocks for parsing, routing, and
//! responding to HTTP/1.1 requests while adhering to the HTTPbis 2022 update
//! of the specification ([RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.html),
//! [RFC 9111](https://www.rfc-editor.org/rfc/rfc9111.html),
//! [RFC 9112](https://www.rfc-editor.org/rfc/rfc9112.html)).

pub mod body;
pub mod cache;
pub mod common;
pub mod conn;
pub mod date;
pub mod encode;
pub mod headers;
pub mod log;
pub mod parse;
pub mod range;
pub mod request;
pub mod response;
pub mod routing;
pub mod server;
pub mod util;

pub use common::{HttpVersion, Method, StatusCode};
pub use headers::{HeaderName, Headers, header_keys};
pub use request::{Request, RequestBuilder, RequestTarget};
pub use response::{Response, ResponseBody, ResponseBuilder};
pub use routing::{Handler, Router, RouterBuilder, router};
pub use server::{Server, ServerBuilder};
