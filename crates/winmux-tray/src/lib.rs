//! WinMux 트레이/GUI 글루.
//!
//! 본 crate는 트레이 프로세스의 Rust 측을 담당한다:
//! 1. tracing 설치 ([`install_tracing`]) — Tauri 빌더 시작 전에 한 번.
//! 2. IPC 매니저 시작 ([`start_ipc`]) — 백그라운드에서 server에 연결하고,
//!    `PtyOutput`·`Event`를 webview로 emit하며, Tauri 명령에서 호출할 수
//!    있는 [`IpcHandle`]을 돌려준다.
//! 3. Tauri 명령 묶음 ([`commands`]) — `src-tauri`가 `generate_handler!`로
//!    등록한다.
//!
//! Tauri 런타임 자체는 `src-tauri`에서 실행한다. 본 crate는 PTY나 GUI
//! 의존성을 갖지 않으며, Win32 `unsafe`도 쓰지 않는다
//! (`docs/spec/00-overview.md` § Build Layout).

pub mod commands;
pub mod ipc;

use anyhow::{Context, Result};
use tauri::AppHandle;
use tracing_subscriber::EnvFilter;
use winmux_protocol::UserIdentity;

pub use crate::ipc::{AttachOutcome, IpcHandle, PtyOutputPayload, ServerStatus};

/// stderr `tracing` subscriber를 설치한다. 이미 설치돼 있으면 무시한다.
///
/// `WINMUX_LOG` 환경 변수로 필터를 덮어쓸 수 있다 (`RUST_LOG`와 같은 문법).
/// Tauri 런타임이 시작되기 전에 호출해도 안전하며, 두 번 호출되어도
/// 부작용이 없다.
pub fn install_tracing() {
    let filter = std::env::var("WINMUX_LOG")
        .ok()
        .and_then(|v| EnvFilter::try_new(v).ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    // try_init은 이미 설치된 subscriber가 있으면 Err을 돌려준다 — 무시.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .try_init();
}

/// 트레이 IPC 매니저를 시작하고 [`IpcHandle`]을 반환한다.
///
/// 반환 후 백그라운드 task가 connect → Hello → read/write 루프를 자동으로
/// 돈다 (`docs/spec/00-overview.md` § Tray lifecycle 3). 호출자는 반환된
/// 핸들을 `app.manage(handle)`로 Tauri State에 등록해 명령 핸들러에서
/// 꺼내 쓰면 된다.
///
/// # Errors
/// `USERNAME` 환경 변수가 비어 있거나 누락되면 [`anyhow::Error`]로 보고한다.
pub fn start_ipc(app: AppHandle) -> Result<IpcHandle> {
    let identity = UserIdentity::detect().context("detect current user")?;
    Ok(ipc::start(app, identity, env!("CARGO_PKG_VERSION")))
}
