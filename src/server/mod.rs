//! Tokio-powered listener orchestration for Goose HTTP.
//!
//! The [`Server`] owns the accept loop and spawns per-connection tasks that
//! drive the [`conn`](crate::conn) state machine. Concrete routing and
//! application logic hooks plug into [`ServerBuilder`] during configuration.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use thiserror::Error;
use tokio::{net::TcpListener, task, time};

use crate::{
    conn::{Connection, ConnectionConfig},
    log,
    routing::{DefaultRouter, Handler},
};

/// Top-level HTTP server handle.
pub struct Server {
    addr: String,
    handler: Arc<dyn Handler>,
    next_id: AtomicU64,
    config: ConnectionConfig,
}

impl Server {
    /// Create a builder for configuring a server instance.
    pub fn builder() -> ServerBuilder {
        ServerBuilder::default()
    }

    /// Returns the configured bind address.
    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// Start accepting connections and spawn per-connection tasks.
    pub async fn run(&self) -> Result<(), ServerError> {
        log::init();
        let listener = TcpListener::bind(&self.addr)
            .await
            .map_err(|error| ServerError::Bind {
                addr: self.addr.clone(),
                source: error,
            })?;

        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(pair) => pair,
                Err(error) => {
                    log::warn(&format!("accept failed: {error}"));
                    // Back off briefly on accept failures to avoid tight loop.
                    time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };

            if let Err(error) = stream.set_nodelay(true) {
                log::warn(&format!("failed to set TCP_NODELAY: {error}"));
            }

            let handler = Arc::clone(&self.handler);
            let connection_id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let config = self.config.clone();

            task::spawn(async move {
                let connection = Connection::new(connection_id, stream, handler, config);
                if let Err(error) = connection.run().await {
                    log::warn(&format!(
                        "connection {connection_id} closed with error: {error}"
                    ));
                }
            });
        }
    }
}

/// Builder for constructing a [`Server`] with custom options.
pub struct ServerBuilder {
    addr: String,
    handler: Arc<dyn Handler>,
    config: ConnectionConfig,
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self {
            addr: String::from("127.0.0.1:3000"),
            handler: Arc::new(DefaultRouter),
            config: ConnectionConfig::default(),
        }
    }
}

impl ServerBuilder {
    /// Override the bind address used by the server.
    pub fn with_addr(mut self, addr: impl Into<String>) -> Self {
        self.addr = addr.into();
        self
    }

    /// Provide a custom request handler implementation.
    pub fn with_handler<H>(mut self, handler: H) -> Self
    where
        H: Handler,
    {
        self.handler = Arc::new(handler);
        self
    }

    /// Override the timeout used when reading request headers.
    pub fn with_header_read_timeout(mut self, timeout: Duration) -> Self {
        self.config.header_read_timeout = timeout;
        self
    }

    /// Override the timeout applied when draining request bodies.
    pub fn with_body_read_timeout(mut self, timeout: Duration) -> Self {
        self.config.body_read_timeout = timeout;
        self
    }

    /// Override the idle timeout between pipelined requests.
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.config.idle_timeout = timeout;
        self
    }

    /// Provide a fully-specified connection configuration.
    pub fn with_connection_config(mut self, config: ConnectionConfig) -> Self {
        self.config = config;
        self
    }

    /// Finalise the builder into a [`Server`].
    pub fn build(self) -> Server {
        Server {
            addr: self.addr,
            handler: self.handler,
            next_id: AtomicU64::new(1),
            config: self.config,
        }
    }
}

/// Errors that can occur while running the server accept loop.
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("failed to bind {addr}: {source}")]
    Bind {
        addr: String,
        source: std::io::Error,
    },
}
