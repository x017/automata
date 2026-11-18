//! Engine logging, built on the `tracing` ecosystem.
//!
//! The subscriber is installed at most once per process; subsequent calls are
//! no-ops. If the host process (a test harness, the future Python runtime)
//! has already installed a global subscriber, that subscriber is kept.
//!
//! Library code must emit diagnostics exclusively through `tracing` macros,
//! never through `println!`/`eprintln!`.

use std::sync::Once;

use tracing_subscriber::EnvFilter;

use crate::config::DEFAULT_LOG_FILTER;

static INIT: Once = Once::new();

/// Installs the global `tracing` subscriber.
///
/// The `RUST_LOG` environment variable takes precedence over `filter`. If
/// `filter` itself is not a valid directive, the subscriber falls back to
/// [`DEFAULT_LOG_FILTER`] and a warning is printed to stderr. This function
/// never fails: logging must not prevent the engine from starting.
pub fn init(filter: &str) {
    INIT.call_once(|| {
        let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::try_new(filter).unwrap_or_else(|error| {
                eprintln!(
                    "automata: invalid log filter `{filter}` ({error}); \
                     falling back to `{DEFAULT_LOG_FILTER}`"
                );
                EnvFilter::new(DEFAULT_LOG_FILTER)
            })
        });

        // `try_init` fails only when a global subscriber already exists; in
        // that case the existing subscriber wins and we carry on.
        let _ = tracing_subscriber::fmt().with_env_filter(env_filter).try_init();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent() {
        init("info");
        init("debug");
        init("not a valid directive at all ::::");
    }
}
