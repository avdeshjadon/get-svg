//! Interactive terminal UI entry point.

mod app;
mod keymap;
mod render;
mod theme;

use std::io::IsTerminal;

use crate::error::{Error, Result};

/// Launch the interactive interface. Requires a TTY.
pub fn run() -> Result<i32> {
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Err(Error::Other(
            "Interactive mode requires a terminal (stdout is not a TTY).\n\n\
Use a non-interactive command instead, for example:\n  \
get-svg search github\n  \
get-svg search github --format json\n  \
get-svg --help"
                .to_string(),
        ));
    }
    app::run_interactive()
}
