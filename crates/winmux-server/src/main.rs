//! Entry point for the `winmux-server` background daemon.

use std::io::{self, Write};

fn main() {
    // Tracing init lands in a follow-up task; this scaffold just
    // identifies itself on stderr so the verification build can run
    // the binary and observe output.
    let _ = writeln!(
        io::stderr(),
        "{} {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    );
}
