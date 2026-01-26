# goose-http

[![Crates.io](https://img.shields.io/crates/v/goose-http.svg)](https://crates.io/crates/goose-http)

Spec-focused HTTP/1.1 server and compliance harness for Rust. Implements RFC
9110/9111/9112 semantics including caching and range handling.

## Scope

- Request parsing and message framing
- Conditional requests and cache semantics
- Range requests and multipart responses
- Async runtime built on Tokio

## Installation

```bash
cargo add goose-http
```

Or add it manually to your `Cargo.toml`:

```toml
[dependencies]
goose-http = "0.1"
```

## Quick Start

### Prerequisites

- Rust toolchain (1.75+ recommended)
- `cargo` for building and running

### First steps

1. Create a project
   ```bash
   cargo new hello-goose
   cd hello-goose
   ```
2. Add goose-http
   ```bash
   cargo add goose-http
   ```
3. Wire up a basic server in `src/main.rs`
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
       res.set_body_text_static("Hello from goose-http!\n");
       res
   }

   #[tokio::main]
   async fn main() -> anyhow::Result<()> {
       let router = router().get("/", handle_root).build();
       let server = Server::builder()
           .with_addr("127.0.0.1:8080")
           .with_handler(router)
           .build();
       println!("Listening on {}", server.addr());
       server.run().await?;
       Ok(())
   }
   ```
4. Run it
   ```bash
   cargo run
   ```

## Compliance Harness

```bash
cargo run --bin compliance_harness -- --help
```

The harness binds to `127.0.0.1:18080` by default and exposes routes that
exercise validators, range requests, and caching semantics. It drives the
conformance suites under `scripts/compliance/`.

## Testing

```bash
cargo test
```

Unit tests cover parsers, caching helpers, and range utilities. Integration
tests in `tests/http_flow.rs` spin up the server to verify `Expect:
100-continue`, pipelined responses, and multi-range responses end-to-end.

## Project Layout

- `src/conn/`: Connection state machine, keep-alive and pipelining logic,
  timeouts, conditional request evaluation.
- `src/parse/`: Request-line, header parsing, and body framing
  (Content-Length and chunked) following RFC 9112.
- `src/encode/`: Response serialization including chunked transfer-coding,
  trailers, and mandatory headers.
- `src/cache/`: Cache-Control parsing, freshness calculations, Age defaults.
- `src/range/`: Range header parsing and satisfiable range computation.
- `src/request/` and `src/response/`: Typed message representations with
  convenience builders.
- `src/server/`: Tokio accept loop, connection orchestration, configurable
  timeouts and structured logging.
- `src/log/`: Tracing-based logging facade with lazy initialization.
- `src/bin/compliance_harness.rs`: Compliance test harness for automated suites.

## Logging

Logging is powered by [`tracing`](https://crates.io/crates/tracing). Call
`goose_http::log::init()` once during application startup (the demo binary does
this automatically). Environment-based filtering is supported via `RUST_LOG`.

## License

MIT © Dan Goosewin
