//! Structured logging via `tracing`.
//!
//! Normal mode is intentionally quiet (the UI reports its own status lines).
//! `--debug` enables debug logs, `-v` enables info logs. `RUST_LOG` always
//! wins when set, for power users.

use std::sync::atomic::{AtomicBool, Ordering};

use tracing_subscriber::EnvFilter;

static DEBUG: AtomicBool = AtomicBool::new(false);

pub fn debug_enabled() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

pub fn init(debug: bool, verbose: bool) {
    DEBUG.store(debug, Ordering::Relaxed);

    let default_level = if debug {
        "get_svg=debug,info"
    } else if verbose {
        "get_svg=info"
    } else {
        "get_svg=warn"
    };

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal_inner())
        .try_init();
}

trait IsTerminalInner {
    fn is_terminal_inner(&self) -> bool;
}

impl IsTerminalInner for std::io::Stderr {
    fn is_terminal_inner(&self) -> bool {
        use std::io::IsTerminal;
        std::io::stderr().is_terminal()
    }
}
