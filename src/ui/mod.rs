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
            "Interactive search requires a terminal (stdout is not a TTY).\n\n\
Type a keyword (e.g. Amazon) and press Enter to search and download SVGs.\n\n\
For scripting, use commands like:\n  \
get-svg search amazon\n  \
get-svg search amazon --zip out.zip\n  \
get-svg --help"
                .to_string(),
        ));
    }
    app::run_interactive()
}
