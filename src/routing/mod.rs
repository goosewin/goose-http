//! Request routing abstractions.
//!
//! This module exposes traits and builders for plugging application logic into
//! the server. It now includes a fluent router builder focused on ergonomics.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use crate::{
    common::{Method, StatusCode},
    headers::header_keys,
    request::Request,
    response::Response,
};

/// Trait implemented by application handlers.
pub trait Handler: Send + Sync + 'static {
    /// Handle a request and produce a response. Async support will be added
    /// later using Tokio primitives.
    fn handle(&self, request: Request) -> Response;
}

impl<F> Handler for F
where
    F: Fn(Request) -> Response + Send + Sync + 'static,
{
    fn handle(&self, request: Request) -> Response {
        (self)(request)
    }
}

type SharedHandler = Arc<dyn Handler>;

/// Builder for constructing a [`Router`] instance with fluent method helpers.
pub struct RouterBuilder {
    routes: HashMap<String, RouteEntry>,
    auto_head: bool,
}

impl Default for RouterBuilder {
    fn default() -> Self {
        Self {
            routes: HashMap::new(),
            auto_head: true,
        }
    }
}

impl RouterBuilder {
    /// Create an empty router builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure whether HEAD requests should automatically reuse GET handlers.
    pub fn auto_head(mut self, enabled: bool) -> Self {
        self.auto_head = enabled;
        self
    }

    /// Register a handler for an arbitrary method token.
    pub fn route<H>(mut self, method: Method, path: impl Into<String>, handler: H) -> Self
    where
        H: Handler,
    {
        let shared: SharedHandler = Arc::new(handler);
        self.insert_route(method, path.into(), shared);
        self
    }

    /// Register a GET handler.
    pub fn get<H>(self, path: impl Into<String>, handler: H) -> Self
    where
        H: Handler,
    {
        self.route(Method::Get, path, handler)
    }

    /// Register a HEAD handler.
    pub fn head<H>(self, path: impl Into<String>, handler: H) -> Self
    where
        H: Handler,
    {
        self.route(Method::Head, path, handler)
    }

    /// Register a POST handler.
    pub fn post<H>(self, path: impl Into<String>, handler: H) -> Self
    where
        H: Handler,
    {
        self.route(Method::Post, path, handler)
    }

    /// Register a PUT handler.
    pub fn put<H>(self, path: impl Into<String>, handler: H) -> Self
    where
        H: Handler,
    {
        self.route(Method::Put, path, handler)
    }

    /// Register a DELETE handler.
    pub fn delete<H>(self, path: impl Into<String>, handler: H) -> Self
    where
        H: Handler,
    {
        self.route(Method::Delete, path, handler)
    }

    /// Register an OPTIONS handler.
    pub fn options<H>(self, path: impl Into<String>, handler: H) -> Self
    where
        H: Handler,
    {
        self.route(Method::Options, path, handler)
    }

    /// Register a TRACE handler.
    pub fn trace<H>(self, path: impl Into<String>, handler: H) -> Self
    where
        H: Handler,
    {
        self.route(Method::Trace, path, handler)
    }

    /// Register a PATCH handler.
    pub fn patch<H>(self, path: impl Into<String>, handler: H) -> Self
    where
        H: Handler,
    {
        self.route(Method::Patch, path, handler)
    }

    /// Finalise the builder into a [`Router`].
    pub fn build(self) -> Router {
        Router {
            routes: self.routes,
            auto_head: self.auto_head,
        }
    }

    fn insert_route(&mut self, method: Method, path: String, handler: SharedHandler) {
        let entry = self.routes.entry(path).or_insert_with(RouteEntry::default);
        entry.insert(method, handler);
    }
}

/// Construct a fresh [`RouterBuilder`] without needing to import the type.
pub fn router() -> RouterBuilder {
    RouterBuilder::default()
}

/// Router that dispatches requests to registered handlers by method and path.
pub struct Router {
    routes: HashMap<String, RouteEntry>,
    auto_head: bool,
}

impl Handler for Router {
    fn handle(&self, request: Request) -> Response {
        let method_token = request.method().as_str().to_owned();
        let target_key = target_key(&request);

        if let Some(route) = self.routes.get(&target_key) {
            match route.resolve(&method_token, self.auto_head) {
                RouteMatch::Matched {
                    handler,
                    head_fallback,
                } => {
                    let mut response = handler.handle(request);
                    if head_fallback {
                        response.strip_body_for_head();
                    }
                    response
                }
                RouteMatch::MethodNotAllowed { allow } => {
                    if matches!(request.method(), Method::Extension(_)) {
                        not_implemented()
                    } else {
                        method_not_allowed(allow)
                    }
                }
            }
        } else if matches!(request.method(), Method::Extension(_)) {
            not_implemented()
        } else {
            not_found()
        }
    }
}

/// Simple router that always returns a 501 placeholder.
pub struct DefaultRouter;

impl Handler for DefaultRouter {
    fn handle(&self, _request: Request) -> Response {
        Response::new(StatusCode::NOT_IMPLEMENTED)
    }
}

#[derive(Default)]
struct RouteEntry {
    handlers: HashMap<String, SharedHandler>,
    methods: BTreeSet<String>,
}

impl RouteEntry {
    fn insert(&mut self, method: Method, handler: SharedHandler) {
        let token = method.as_str().to_owned();
        self.handlers.insert(token.clone(), handler);
        self.methods.insert(token);
    }

    fn resolve(&self, method: &str, auto_head: bool) -> RouteMatch {
        if let Some(handler) = self.handlers.get(method) {
            return RouteMatch::matched(handler, false);
        }

        if auto_head && method.eq_ignore_ascii_case("HEAD") {
            if let Some(handler) = self.handlers.get("GET") {
                return RouteMatch::matched(handler, true);
            }
        }

        RouteMatch::method_not_allowed(self.allow_header(auto_head))
    }

    fn allow_header(&self, auto_head: bool) -> String {
        let mut allowed = self.methods.clone();
        if auto_head && allowed.contains("GET") {
            allowed.insert(String::from("HEAD"));
        }
        allowed.into_iter().collect::<Vec<_>>().join(", ")
    }
}

enum RouteMatch {
    Matched {
        handler: SharedHandler,
        head_fallback: bool,
    },
    MethodNotAllowed {
        allow: String,
    },
}

impl RouteMatch {
    fn matched(handler: &SharedHandler, head_fallback: bool) -> Self {
        RouteMatch::Matched {
            handler: Arc::clone(handler),
            head_fallback,
        }
    }

    fn method_not_allowed(allow: String) -> Self {
        RouteMatch::MethodNotAllowed { allow }
    }
}

fn target_key(request: &Request) -> String {
    match request.target() {
        crate::request::RequestTarget::Origin(path)
        | crate::request::RequestTarget::Absolute(path)
        | crate::request::RequestTarget::Authority(path) => path.clone(),
        crate::request::RequestTarget::Asterisk => String::from("*"),
    }
}

fn not_found() -> Response {
    Response::new(StatusCode::NOT_FOUND)
}

fn method_not_allowed(allow: String) -> Response {
    let mut response = Response::new(StatusCode::METHOD_NOT_ALLOWED);
    response.headers_mut().insert(header_keys::ALLOW, allow);
    response
}

fn not_implemented() -> Response {
    Response::new(StatusCode::NOT_IMPLEMENTED)
}
