//! Tracing/logging setup shared across the workspace.

use tracing_subscriber::{fmt, EnvFilter};

/// Initialize a `tracing` subscriber. `verbose` bumps the default filter
/// from `info` to `debug`. `RAI_LOG` overrides everything.
pub fn init(verbose: bool) {
    let default = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_env("RAI_LOG").unwrap_or_else(|_| EnvFilter::new(default));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
