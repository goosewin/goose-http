//! Logging utilities.
//!
//! Centralised logging helpers wrapping `tracing` so components can emit
//! structured diagnostics without depending on concrete subscribers.

use std::sync::Once;

static INIT: Once = Once::new();

/// Initialise a default tracing subscriber if none has been installed.
///
/// Subsequent calls are no-ops, allowing libraries, binaries, and tests to call
/// this function without worrying about double initialisation panics.
pub fn init() {
    INIT.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info".into()),
            )
            .with_target(false)
            .try_init();
    });
}

/// Emit an informational log line.
pub fn info(message: &str) {
    tracing::info!(message);
}

/// Emit a warning log line.
pub fn warn(message: &str) {
    tracing::warn!(message);
}

/// Emit an error log line.
pub fn error(message: &str) {
    tracing::error!(message);
}
