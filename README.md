# Goose HTTP

[![Crates.io](https://img.shields.io/crates/v/goose-http.svg)](https://crates.io/crates/goose-http)

Goose HTTP is a from-scratch implementation of an HTTP/1.1 server written in
Rust. It adheres to the 2022 HTTPbis specification split:

- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 9111: Caching](https://www.rfc-editor.org/rfc/rfc9111.html)
- [RFC 9112: HTTP/1.1](https://www.rfc-editor.org/rfc/rfc9112.html)

The project is intentionally minimal but complete—it implements message
framing, request parsing, conditional requests, Range handling, caching
semantics, connection management, and a fully asynchronous runtime using
Tokio.

## Installation

```bash
cargo add goose-http
```

Or add it manually to your `Cargo.toml`:

```toml
[dependencies]
goose-http = "0.1"
```

## Getting Started

### Prerequisites

- Rust toolchain (1.75+ recommended)
- `cargo` for building and running

### First steps

1. **Create a project**
   ```bash
   cargo new hello-goose
   cd hello-goose
   ```

2. **Add Goose HTTP**
   ```bash
   cargo add goose-http
   ```

3. **Wire up a basic server** – drop this into `src/main.rs`:

   ```rust
   use goose_http::{
       router,
       request::Request,
       response::Response,
       common::StatusCode,
       Server,
   };

   fn handle_root(_req: Request) -> Response {
       let mut res = Response::new(StatusCode::OK);
       res.set_body_text_static("Hello from Goose HTTP!\n");
       res
   }

   #[tokio::main]
   async fn main() -> anyhow::Result<()> {
       let router = router().get("/", handle_root).build();
       let server = Server::builder().with_addr("127.0.0.1:8080").with_handler(router).build();
       println!("Listening on {}", server.addr());
       server.run().await?;
       Ok(())
   }
   ```

4. **Run it**
   ```bash
   cargo run
   ```

You now have a minimal HTTP/1.1 server that speaks the Goose HTTP stack.

### Example Harness

```bash
cargo run --bin compliance_harness -- --help
```

The compliance harness binds to `127.0.0.1:18080` by default and exposes sample
routes that exercise validators, range requests, and caching semantics. It is
used to drive the conformance suites under `scripts/compliance/`.

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
- `src/bin/compliance_harness.rs`: Compliance test harness used for automated
  suites.

## Logging & Diagnostics

Logging is powered by the [`tracing`](https://crates.io/crates/tracing) crate.
Call `goose_http::log::init()` once during application startup (the demo binary
does this automatically). Environment-based filtering is supported via
`RUST_LOG`.

## License

This project is released under the CC0-1.0 license. See `LICENSE` for details.
