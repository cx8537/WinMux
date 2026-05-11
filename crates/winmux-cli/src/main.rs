//! Entry point for the `winmux` single-shot CLI client.

use std::io::{self, Write};

fn main() {
    // Argument parsing and subcommand dispatch arrive in a follow-up
    // task; this scaffold just identifies itself on stderr.
    let _ = writeln!(
        io::stderr(),
        "{} {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    );
}
