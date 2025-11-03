use std::{
    env,
    error::Error,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;

use goose_http::{
    Server, cache,
    common::{Method, StatusCode},
    headers::header_keys,
    range,
    request::{Request, RequestTarget},
    response::{Response, ResponseBody},
    routing::Handler,
};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 18080;
const HARNESS_ETAG: &str = "\"compliance-validator\"";
const HARNESS_IMMUTABLE_ETAG: &str = "\"compliance-immutable\"";
const LAST_MODIFIED_STAMP: &str = "Wed, 21 Oct 2015 07:28:00 GMT";

const INDEX_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/static/index.html"
));
const RANGE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/static/range.txt"
));
const STATIC_ASSET: &str = "Static asset served by Goose HTTP compliance harness.\n";
const CACHE_BODY: &str = "Cache validation payload from Goose HTTP.\n";
const IMMUTABLE_BODY: &str = "Immutable cache resource from Goose HTTP.\n";

#[derive(Debug)]
struct HarnessConfig {
    host: String,
    port: u16,
    shutdown_after: Option<Duration>,
}

#[derive(Debug, Default, Clone, Copy)]
struct HarnessService;

impl Handler for HarnessService {
    fn handle(&self, request: Request) -> Response {
        harness_handler(request)
    }
}

#[derive(Debug)]
enum ConfigError {
    MissingValue(&'static str),
    InvalidPort(String),
    InvalidDuration(String),
    InvalidAddr(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::MissingValue(flag) => write!(f, "missing value for {flag}"),
            ConfigError::InvalidPort(value) => write!(f, "invalid port: {value}"),
            ConfigError::InvalidDuration(value) => {
                write!(f, "invalid shutdown duration (seconds): {value}")
            }
            ConfigError::InvalidAddr(value) => {
                write!(f, "invalid addr (expected host:port): {value}")
            }
        }
    }
}

impl Error for ConfigError {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = parse_args(env::args())?;
    let addr = format!("{}:{}", config.host, config.port);

    let server = Server::builder()
        .with_addr(addr.clone())
        .with_handler(HarnessService)
        .build();

    println!("Goose HTTP compliance harness listening on {addr}");

    if let Some(duration) = config.shutdown_after {
        tokio::select! {
            res = server.run() => {
                res?;
            }
            _ = tokio::time::sleep(duration) => {
                println!(
                    "Compliance harness elapsed {} seconds; shutting down",
                    duration.as_secs()
                );
            }
        }
    } else {
        server.run().await?;
    }

    Ok(())
}

fn parse_args<I>(mut args: I) -> Result<HarnessConfig, ConfigError>
where
    I: Iterator<Item = String>,
{
    // Skip executable name.
    let _ = args.next();

    let mut host = DEFAULT_HOST.to_string();
    let mut port = DEFAULT_PORT;
    let mut shutdown_after = None;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--host" => {
                host = args.next().ok_or(ConfigError::MissingValue("--host"))?;
            }
            "--port" => {
                let value = args.next().ok_or(ConfigError::MissingValue("--port"))?;
                port = value.parse().map_err(|_| ConfigError::InvalidPort(value))?;
            }
            "--addr" => {
                let value = args.next().ok_or(ConfigError::MissingValue("--addr"))?;
                if let Some((h, p)) = value.rsplit_once(':') {
                    host = h.to_string();
                    port = p
                        .parse()
                        .map_err(|_| ConfigError::InvalidAddr(value.clone()))?;
                } else {
                    return Err(ConfigError::InvalidAddr(value));
                }
            }
            "--shutdown-after" => {
                let value = args
                    .next()
                    .ok_or(ConfigError::MissingValue("--shutdown-after"))?;
                let seconds: u64 = value
                    .parse()
                    .map_err(|_| ConfigError::InvalidDuration(value))?;
                shutdown_after = Some(Duration::from_secs(seconds));
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                eprintln!("unrecognised flag: {other}");
                print_usage();
                std::process::exit(2);
            }
        }
    }

    Ok(HarnessConfig {
        host,
        port,
        shutdown_after,
    })
}

fn print_usage() {
    eprintln!(
        "Usage: compliance-harness [--host HOST] [--port PORT] [--shutdown-after SECONDS]\n\
         Options:\n  --host HOST              Bind host (default {DEFAULT_HOST})\n  --port PORT              Bind port (default {DEFAULT_PORT})\n  --addr HOST:PORT         Convenience override for host and port\n  --shutdown-after SECONDS Exit automatically after the given seconds\n  -h, --help               Print this message"
    );
}

fn harness_handler(request: Request) -> Response {
    let method = request.method().clone();
    let target = request.target().clone();
    dispatch_request(method, target, request)
}

fn dispatch_request(method: Method, target: RequestTarget, request: Request) -> Response {
    match method {
        Method::Get => handle_get(target, request),
        Method::Head => handle_head(target, request),
        Method::Post => handle_post(target, request),
        Method::Options => serve_options(request),
        Method::Extension(_) => not_implemented_response(),
        _ => method_not_allowed(),
    }
}

fn handle_get(target: RequestTarget, request: Request) -> Response {
    match target {
        RequestTarget::Origin(path) => handle_get_route(path, request),
        RequestTarget::Asterisk => Response::new(StatusCode::BAD_REQUEST),
        RequestTarget::Absolute(_) | RequestTarget::Authority(_) => {
            Response::new(status_not_found())
        }
    }
}

fn handle_head(target: RequestTarget, request: Request) -> Response {
    match target {
        RequestTarget::Origin(path) => {
            let mut response = handle_get_route(path, request);
            normalize_head_response(&mut response);
            response
        }
        RequestTarget::Asterisk => {
            let mut response = method_not_allowed();
            normalize_head_response(&mut response);
            response
        }
        RequestTarget::Absolute(_) | RequestTarget::Authority(_) => {
            Response::new(status_not_found())
        }
    }
}

fn handle_post(target: RequestTarget, request: Request) -> Response {
    match target {
        RequestTarget::Origin(path) => match path.as_str() {
            "/" => serve_post_echo(request),
            _ => Response::new(status_not_found()),
        },
        RequestTarget::Asterisk => Response::new(StatusCode::BAD_REQUEST),
        RequestTarget::Absolute(_) | RequestTarget::Authority(_) => {
            Response::new(status_not_found())
        }
    }
}

fn handle_get_route(path: String, request: Request) -> Response {
    match path.as_str() {
        "/" => serve_index(request),
        "/__health" => serve_health(request),
        "/static/hello.txt" => serve_static_asset(request),
        "/json/time" => serve_time_json(request),
        "/cache/validator" => serve_cache_validator(request),
        "/cache/immutable" => serve_cache_immutable(request),
        "/vary/accept-language" => serve_vary_accept_language(request),
        "/range/demo" => serve_range_demo(request),
        _ => Response::new(status_not_found()),
    }
}

fn serve_index(_request: Request) -> Response {
    let mut response = Response::new(StatusCode::OK);
    response
        .headers_mut()
        .insert(header_keys::CONTENT_TYPE, "text/html; charset=utf-8");
    response
        .headers_mut()
        .insert(header_keys::CACHE_CONTROL, "no-store");
    cache::ensure_age_header(response.headers_mut());
    response.set_body_bytes(Bytes::from_static(INDEX_HTML.as_bytes()));
    response
}

fn serve_health(_request: Request) -> Response {
    Response::new(status_no_content())
}

fn serve_static_asset(_request: Request) -> Response {
    let mut response = Response::new(StatusCode::OK);
    response
        .headers_mut()
        .insert(header_keys::CONTENT_TYPE, "text/plain; charset=utf-8");
    response
        .headers_mut()
        .insert(header_keys::CACHE_CONTROL, "max-age=120");
    cache::ensure_age_header(response.headers_mut());
    response.set_body_bytes(Bytes::from_static(STATIC_ASSET.as_bytes()));
    response
}

fn serve_time_json(_request: Request) -> Response {
    let mut response = Response::new(StatusCode::OK);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let body = format!(
        "{{\"epoch_seconds\": {}.{:09}}}",
        now.as_secs(),
        now.subsec_nanos()
    );
    response
        .headers_mut()
        .insert(header_keys::CONTENT_TYPE, "application/json; charset=utf-8");
    response
        .headers_mut()
        .insert(header_keys::CACHE_CONTROL, "no-store");
    cache::ensure_age_header(response.headers_mut());
    response.set_body_bytes(body);
    response
}

fn serve_post_echo(request: Request) -> Response {
    let body = request.into_body_bytes();
    let mut response = Response::new(StatusCode::OK);
    response
        .headers_mut()
        .insert(header_keys::CONTENT_TYPE, "application/octet-stream");
    response
        .headers_mut()
        .insert(header_keys::CACHE_CONTROL, "no-store");
    cache::ensure_age_header(response.headers_mut());
    response.set_body_bytes(body);
    response
}

fn serve_cache_validator(request: Request) -> Response {
    let mut response = Response::new(StatusCode::OK);
    response
        .headers_mut()
        .insert(header_keys::CONTENT_TYPE, "text/plain; charset=utf-8");
    response
        .headers_mut()
        .insert(header_keys::CACHE_CONTROL, "max-age=60, must-revalidate");
    response
        .headers_mut()
        .insert(header_keys::ETAG, HARNESS_ETAG);
    response
        .headers_mut()
        .insert(header_keys::LAST_MODIFIED, LAST_MODIFIED_STAMP);

    if matches_if_none_match(&request, HARNESS_ETAG) {
        let mut not_modified = Response::new(StatusCode::NOT_MODIFIED);
        not_modified
            .headers_mut()
            .insert(header_keys::ETAG, HARNESS_ETAG);
        not_modified
            .headers_mut()
            .insert(header_keys::LAST_MODIFIED, LAST_MODIFIED_STAMP);
        not_modified
            .headers_mut()
            .insert(header_keys::CACHE_CONTROL, "max-age=60, must-revalidate");
        cache::ensure_age_header(not_modified.headers_mut());
        return not_modified;
    }

    cache::ensure_age_header(response.headers_mut());
    response.set_body_bytes(Bytes::from_static(CACHE_BODY.as_bytes()));
    response
}

fn serve_cache_immutable(_request: Request) -> Response {
    let mut response = Response::new(StatusCode::OK);
    response
        .headers_mut()
        .insert(header_keys::CONTENT_TYPE, "text/plain; charset=utf-8");
    response
        .headers_mut()
        .insert(header_keys::CACHE_CONTROL, "max-age=31536000, immutable");
    response
        .headers_mut()
        .insert(header_keys::ETAG, HARNESS_IMMUTABLE_ETAG);
    cache::ensure_age_header(response.headers_mut());
    response.set_body_bytes(Bytes::from_static(IMMUTABLE_BODY.as_bytes()));
    response
}

fn serve_vary_accept_language(request: Request) -> Response {
    let mut response = Response::new(StatusCode::OK);
    response
        .headers_mut()
        .insert(header_keys::CONTENT_TYPE, "text/plain; charset=utf-8");
    response
        .headers_mut()
        .insert(header_keys::CACHE_CONTROL, "max-age=30");
    response
        .headers_mut()
        .insert(header_keys::VARY, "Accept-Language");

    let preference = request
        .header(header_keys::ACCEPT_LANGUAGE)
        .and_then(parse_language_tag)
        .unwrap_or("en");

    let body = match preference {
        "fr" => "Bonjour du harnais de conformité Goose HTTP.\n",
        "es" => "Saludos desde el arnés de conformidad de Goose HTTP.\n",
        _ => "Hello from the Goose HTTP compliance harness.\n",
    };

    cache::ensure_age_header(response.headers_mut());
    response.set_body_bytes(Bytes::from_static(body.as_bytes()));
    response
}

fn serve_range_demo(request: Request) -> Response {
    if let Some(range_header) = request.header(header_keys::RANGE) {
        if let Ok(specs) = range::parse_range_header(range_header) {
            let ranges = range::compute_satisfiable_ranges(&specs, RANGE_BYTES.len() as u64);
            if ranges.is_empty() {
                let mut partial = Response::new(StatusCode::RANGE_NOT_SATISFIABLE);
                partial.headers_mut().insert(
                    header_keys::CONTENT_RANGE,
                    range::format_unsatisfied_range(RANGE_BYTES.len() as u64),
                );
                partial.set_body(ResponseBody::Empty);
                return partial;
            }

            let range = ranges[0];
            let slice = &RANGE_BYTES[range.start as usize..=range.end as usize];
            let mut partial = Response::new(StatusCode::PARTIAL_CONTENT);
            partial.headers_mut().insert(
                header_keys::CONTENT_RANGE,
                range::format_content_range(range, RANGE_BYTES.len() as u64),
            );
            partial
                .headers_mut()
                .insert(header_keys::ACCEPT_RANGES, "bytes");
            partial
                .headers_mut()
                .insert(header_keys::CONTENT_TYPE, "text/plain; charset=utf-8");
            partial.set_body(ResponseBody::Full(Bytes::copy_from_slice(slice)));
            return partial;
        }
    }

    let mut response = Response::new(StatusCode::OK);
    response
        .headers_mut()
        .insert(header_keys::CONTENT_TYPE, "text/plain; charset=utf-8");
    response
        .headers_mut()
        .insert(header_keys::ACCEPT_RANGES, "bytes");
    response
        .headers_mut()
        .insert(header_keys::ETAG, HARNESS_IMMUTABLE_ETAG);
    response
        .headers_mut()
        .insert(header_keys::LAST_MODIFIED, LAST_MODIFIED_STAMP);
    cache::ensure_age_header(response.headers_mut());
    response.set_body_bytes(Bytes::from_static(RANGE_BYTES));
    response
}

fn serve_options(_request: Request) -> Response {
    let mut response = Response::new(status_no_content());
    response
        .headers_mut()
        .insert(header_keys::ALLOW, "GET, HEAD, OPTIONS");
    response
        .headers_mut()
        .insert(header_keys::CACHE_CONTROL, "no-store");
    cache::ensure_age_header(response.headers_mut());
    response.set_body(ResponseBody::Empty);
    response
}

fn matches_if_none_match(request: &Request, etag: &str) -> bool {
    request
        .if_none_match()
        .map(|raw| {
            raw.split(',')
                .map(|token| token.trim())
                .any(|candidate| candidate == "*" || candidate == etag)
        })
        .unwrap_or(false)
}

fn parse_language_tag(header: &str) -> Option<&'static str> {
    header
        .split(',')
        .filter_map(|entry| entry.split(';').next())
        .map(|tag| tag.trim())
        .find_map(|tag| {
            if tag.is_empty() {
                None
            } else {
                match tag {
                    "fr" | "fr-FR" => Some("fr"),
                    "es" | "es-ES" | "es-MX" => Some("es"),
                    _ => Some("en"),
                }
            }
        })
}

fn method_not_allowed() -> Response {
    let mut response = Response::new(StatusCode::METHOD_NOT_ALLOWED);
    response
        .headers_mut()
        .insert(header_keys::ALLOW, "GET, HEAD, OPTIONS");
    response
        .headers_mut()
        .insert(header_keys::CACHE_CONTROL, "no-store");
    cache::ensure_age_header(response.headers_mut());
    response.set_body(ResponseBody::Empty);
    response
}

fn not_implemented_response() -> Response {
    let mut response = Response::new(StatusCode::NOT_IMPLEMENTED);
    response
        .headers_mut()
        .insert(header_keys::CACHE_CONTROL, "no-store");
    cache::ensure_age_header(response.headers_mut());
    response.set_body(ResponseBody::Empty);
    response
}

fn status_no_content() -> StatusCode {
    StatusCode::from_u16(204).expect("204 is a valid status code")
}

fn status_not_found() -> StatusCode {
    StatusCode::from_u16(404).expect("404 is a valid status code")
}

fn normalize_head_response(response: &mut Response) {
    let body_len = match response.body() {
        ResponseBody::Full(bytes) => Some(bytes.len()),
        _ => None,
    };

    if let Some(len) = body_len {
        if !response.headers().contains(header_keys::CONTENT_LENGTH) {
            response
                .headers_mut()
                .insert(header_keys::CONTENT_LENGTH, len.to_string());
        }
    }
    response.set_body(ResponseBody::Empty);
}
