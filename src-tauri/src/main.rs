// Disable the console window for release builds. Dev keeps it so
// `tracing`/`println` from inside the Tauri runtime stay visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! winmux-app — the Tauri host that wraps the WinMux tray process.

use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    if let Err(err) = winmux_tray::run() {
        let _ = writeln!(io::stderr(), "winmux-tray initialization failed: {err:#}");
        return ExitCode::from(1);
    }

    let context = tauri::generate_context!();
    match tauri::Builder::default().run(context) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(io::stderr(), "tauri runtime failed: {err}");
            ExitCode::from(1)
        }
    }
}
