//! `winmux-cli` 라이브러리 파사드.
//!
//! 바이너리 [`run`]은 인자 파싱과 명령 dispatch만 담당한다. M0 단계에서는
//! 대부분의 명령이 "아직 구현되지 않음" 메시지와 함께 [`exit::GENERAL_ERROR`]로
//! 종료한다 — IPC 호출과 출력 포매팅은 후속 작업에서 단계적으로 채운다.
//!
//! 본 라이브러리는 `println!`/`eprintln!` 매크로를 쓰지 않는다 (lint
//! 차단). 표준 출력에는 `writeln!(io::stdout(), ...)`, 에러 메시지는
//! `writeln!(io::stderr(), ...)`를 직접 호출한다.

pub mod args;

use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;

use crate::args::{Cli, Command};

/// 표준 exit 코드 (`docs/spec/06-cli.md` § Exit Codes).
pub mod exit {
    /// 성공.
    pub const SUCCESS: u8 = 0;
    /// 일반 오류(메시지를 stderr에 남긴다).
    pub const GENERAL_ERROR: u8 = 1;
    /// 서버가 실행 중이지 않고 자동 기동도 실패.
    pub const NO_SERVER: u8 = 2;
    /// 타깃(세션·윈도우·패널)을 찾지 못함.
    pub const TARGET_NOT_FOUND: u8 = 3;
    /// 프로토콜 버전 불일치.
    pub const PROTOCOL_MISMATCH: u8 = 4;
    /// SID 불일치 등 권한 거부.
    pub const PERMISSION_DENIED: u8 = 5;
    /// 인자 형식 오류(`EX_USAGE`와 일치).
    pub const USAGE_ERROR: u8 = 64;
}

/// 동기 entry. 인자 파싱 후 명령별 핸들러로 분기.
#[must_use]
pub fn run() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(err) => return handle_clap_error(&err),
    };
    dispatch(&cli)
}

/// clap이 만든 오류를 적절한 출력과 exit code로 매핑.
fn handle_clap_error(err: &clap::Error) -> ExitCode {
    // clap의 print는 자체적으로 stdout/stderr를 올바르게 선택한다.
    let _ = err.print();
    match err.kind() {
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
            ExitCode::SUCCESS
        }
        _ => ExitCode::from(exit::USAGE_ERROR),
    }
}

/// 명령별 분기.
fn dispatch(cli: &Cli) -> ExitCode {
    match &cli.command {
        Command::Version => handle_version(&cli.global),
        Command::Ls(args) => handle_ls(args, &cli.global),
        other => unimplemented_stub(command_name(other)),
    }
}

fn handle_version(global: &args::GlobalFlags) -> ExitCode {
    let cli_version = env!("CARGO_PKG_VERSION");
    let proto = winmux_protocol::PROTOCOL_VERSION;
    let server_info = probe_server_version();
    let mut stdout = io::stdout();

    if global.json {
        let server_value = server_info.as_deref().map_or(serde_json::Value::Null, |s| {
            serde_json::Value::String(s.to_owned())
        });
        let value = serde_json::json!({
            "winmux": cli_version,
            "server": server_value,
            "protocol": proto,
        });
        let _ = writeln!(stdout, "{value}");
    } else {
        let _ = writeln!(stdout, "winmux {cli_version}");
        match &server_info {
            Some(s) => {
                let _ = writeln!(stdout, "server {s}");
            }
            None => {
                let _ = writeln!(stdout, "server unknown (not running)");
            }
        }
        let _ = writeln!(stdout, "protocol v{proto}");
    }
    ExitCode::SUCCESS
}

/// 서버에 한 번 connect + Hello 시도. 실패는 silent (read-only 명령이라
/// 서버 자동 기동도 하지 않는다 — spec § Server Auto-Start).
fn probe_server_version() -> Option<String> {
    let identity = winmux_protocol::UserIdentity::detect().ok()?;
    let pipe_name = identity.pipe_name();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .ok()?;

    runtime.block_on(async move {
        let pipe = winmux_ipc_client::connect(&pipe_name).ok()?;
        let mut client = winmux_ipc_client::Client::new(pipe);
        let hello = client
            .hello(winmux_protocol::ClientKind::Cli, env!("CARGO_PKG_VERSION"))
            .await
            .ok()?;
        let _ = client.close().await;
        Some(hello.server_version)
    })
}

/// `ls`: 서버에 `ListSessions`를 보내고 결과를 출력한다.
fn handle_ls(_args: &args::LsArgs, global: &args::GlobalFlags) -> ExitCode {
    let identity = match winmux_protocol::UserIdentity::detect() {
        Ok(id) => id,
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: {e}");
            return ExitCode::from(exit::GENERAL_ERROR);
        }
    };
    let pipe_name = identity.pipe_name();

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: failed to start runtime: {e}");
            return ExitCode::from(exit::GENERAL_ERROR);
        }
    };

    // `ClientOptions::open`은 tokio reactor에 등록을 시도하므로 runtime 컨텍스트
    // 안에서 호출해야 한다 — 그래서 connect도 block_on 안으로 옮긴다.
    let connect_outcome = runtime.block_on(async { winmux_ipc_client::connect(&pipe_name) });

    let pipe = match connect_outcome {
        Ok(p) => p,
        Err(winmux_ipc_client::ConnectError::NotRunning(_)) => {
            // spec § ls Exit codes: 2 — server not running (read-only이라 auto-start 안 함).
            let _ = writeln!(io::stderr(), "winmux: server is not running");
            return ExitCode::from(exit::NO_SERVER);
        }
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: connect failed: {e}");
            return ExitCode::from(exit::GENERAL_ERROR);
        }
    };

    let result: anyhow::Result<Vec<winmux_protocol::SessionSummary>> = runtime.block_on(async {
        let mut client = winmux_ipc_client::Client::new(pipe);
        client
            .hello(winmux_protocol::ClientKind::Cli, env!("CARGO_PKG_VERSION"))
            .await?;
        let id = client.next_message_id();
        let resp = client
            .request(&winmux_protocol::ClientMessage::ListSessions {
                v: winmux_protocol::PROTOCOL_VERSION,
                id,
            })
            .await?;
        let sessions = match resp {
            winmux_protocol::ServerMessage::SessionList { sessions, .. } => sessions,
            winmux_protocol::ServerMessage::Error { payload, .. } => {
                anyhow::bail!(
                    "server error: {} ({})",
                    payload.message,
                    payload.code.as_str()
                );
            }
            other => anyhow::bail!("unexpected response: {other:?}"),
        };
        let _ = client.close().await;
        Ok(sessions)
    });

    match result {
        Ok(sessions) => {
            print_session_list(&sessions, global);
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: {e}");
            ExitCode::from(exit::GENERAL_ERROR)
        }
    }
}

/// `ls`의 사람 친화 또는 JSON 출력.
fn print_session_list(sessions: &[winmux_protocol::SessionSummary], global: &args::GlobalFlags) {
    let mut stdout = io::stdout();
    if global.json {
        let array: Vec<serde_json::Value> = sessions
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id.as_str(),
                    "name": s.name,
                    "windows": s.windows,
                    "attached": s.attached_clients > 0,
                })
            })
            .collect();
        let _ = writeln!(stdout, "{}", serde_json::Value::Array(array));
    } else if sessions.is_empty() {
        let _ = writeln!(stdout, "(no sessions)");
    } else {
        for s in sessions {
            let attached = if s.attached_clients > 0 {
                "  (attached)"
            } else {
                ""
            };
            let plural = if s.windows == 1 { "window" } else { "windows" };
            let _ = writeln!(stdout, "{}\t{} {}{}", s.name, s.windows, plural, attached);
        }
    }
}

fn unimplemented_stub(name: &str) -> ExitCode {
    let mut stderr = io::stderr();
    let _ = writeln!(
        stderr,
        "winmux: `{name}` is parsed but not yet wired to the server in this build"
    );
    ExitCode::from(exit::GENERAL_ERROR)
}

fn command_name(c: &Command) -> &'static str {
    match c {
        Command::Ls(_) => "ls",
        Command::NewSession(_) => "new-session",
        Command::Attach(_) => "attach",
        Command::Detach => "detach",
        Command::KillSession(_) => "kill-session",
        Command::KillWindow(_) => "kill-window",
        Command::KillPane(_) => "kill-pane",
        Command::SendKeys(_) => "send-keys",
        Command::ListWindows(_) => "list-windows",
        Command::ListPanes(_) => "list-panes",
        Command::SplitWindow(_) => "split-window",
        Command::SelectPane(_) => "select-pane",
        Command::ResizePane(_) => "resize-pane",
        Command::CapturePane(_) => "capture-pane",
        Command::DisplayMessage(_) => "display-message",
        Command::SourceFile(_) => "source-file",
        Command::ShowOptions(_) => "show-options",
        Command::BindKey(_) => "bind-key",
        Command::UnbindKey(_) => "unbind-key",
        Command::KillServer => "kill-server",
        Command::StartServer => "start-server",
        Command::Version => "version",
    }
}
