# Goose HTTP

Goose HTTP is a from-scratch implementation of an HTTP/1.1 server written in
Rust. It adheres to the 2022 HTTPbis specification split:

- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 9111: Caching](https://www.rfc-editor.org/rfc/rfc9111.html)
- [RFC 9112: HTTP/1.1](https://www.rfc-editor.org/rfc/rfc9112.html)

The project is intentionally minimal but complete—it implements message
framing, request parsing, conditional requests, Range handling, caching
semantics, connection management, and a fully asynchronous runtime using
Tokio.

## Getting Started

### Prerequisites

- Rust toolchain (1.75+ recommended)
- `cargo` for building and running

### Building

```bash
cargo build
```

### Running the Demo Server

```bash
cargo run
```

The demo server binds to `127.0.0.1:8080` and serves a few sample routes from
`src/main.rs`, showcasing ETag handling, Range requests, and OPTIONS/TRACE
responses.

### Testing

```bash
cargo test
```

Unit tests cover parsers, caching helpers, and range utilities. Integration
tests in `tests/http_flow.rs` spin up the server to verify `Expect:
100-continue`, pipelined responses, and multi-range responses end-to-end.

## Project Layout

- `src/conn/`: Connection state machine, keep-alive & pipelining logic, timeout
  handling, conditional request evaluation.
- `src/parse/`: Request-line, header parsing, and body framing (Content-Length &
  chunked) strictly following RFC 9112.
- `src/encode/`: Response serialization including chunked transfer-coding,
  trailers, and mandatory headers (Date, Connection).
- `src/cache/`: Cache-Control parsing, freshness calculations, Age defaults.
- `src/range/`: Range header parsing and satisfiable range computation.
- `src/request/` & `src/response/`: Typed representations of HTTP messages with
  convenience builders.
- `src/server/`: Tokio accept loop, connection orchestration, configurable
  timeouts and structured logging.
- `src/log/`: Tracing-based logging façade with lazy initialisation.
- `src/main.rs`: Demonstration binary wiring everything together.

## Logging & Diagnostics

Logging is powered by the [`tracing`](https://crates.io/crates/tracing) crate.
Call `goose_http::log::init()` once during application startup (the demo binary
does this automatically). Environment-based filtering is supported via
`RUST_LOG`.

## License

This project is released under the MIT license. See `LICENSE` for details.
