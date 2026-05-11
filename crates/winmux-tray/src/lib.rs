//! WinMux tray/GUI glue.
//!
//! This crate hosts the Rust side of the tray process (Named Pipe
//! client, Tauri command handlers). The Tauri runtime itself is
//! depended on and launched from `src-tauri`; see
//! `docs/spec/00-overview.md` § Build Layout.

/// Initialize the tray runtime.
///
/// Currently a no-op stub. The full implementation will wire up
/// tracing, connect to the server's Named Pipe, and register Tauri
/// commands.
pub fn run() -> anyhow::Result<()> {
    Ok(())
}
