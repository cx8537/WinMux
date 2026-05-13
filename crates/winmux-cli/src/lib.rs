//! `winmux-cli` 라이브러리 파사드.
//!
//! 바이너리 [`run`]은 인자 파싱과 명령 dispatch만 담당한다. 본 모듈에서는
//! 단일 연결 + 단일 요청으로 끝나는 write 명령들의 핵심 로직을 호스팅한다.
//! 통합 테스트가 in-process server에 connect한 뒤 [`new_session_with`]·
//! [`kill_session_with`]를 직접 호출해 와이어 동작을 검증한다.
//!
//! 본 라이브러리는 `println!`/`eprintln!` 매크로를 쓰지 않는다 (lint
//! 차단). 표준 출력에는 `writeln!(io::stdout(), ...)`, 에러 메시지는
//! `writeln!(io::stderr(), ...)`를 직접 호출한다.

pub mod args;

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

use clap::Parser;
use tokio::net::windows::named_pipe::NamedPipeClient;
use winmux_ipc_client::{Client, ConnectError, connect, connect_with_retry};
use winmux_protocol::{
    ClientKind, ClientMessage, CommandRequest, ErrorCode, KillSessionTarget, NewSessionRequest,
    PROTOCOL_VERSION, PaneId, PaneSummary, ServerMessage, SessionId, SessionSummary, UserIdentity,
    WindowId,
};

use crate::args::{
    Cli, Command, GlobalFlags, KillSessionArgs, LsArgs, NewSessionArgs, SendKeysArgs,
    TargetOnlyArgs,
};

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
        Command::NewSession(args) => handle_new_session(args, &cli.global),
        Command::KillSession(args) => handle_kill_session(args, &cli.global),
        Command::KillPane(args) => handle_kill_pane(args, &cli.global),
        Command::KillWindow(args) => handle_kill_window(args, &cli.global),
        Command::SendKeys(args) => handle_send_keys(args, &cli.global),
        other => unimplemented_stub(command_name(other)),
    }
}

fn handle_version(global: &GlobalFlags) -> ExitCode {
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
    let identity = UserIdentity::detect().ok()?;
    let pipe_name = identity.pipe_name();

    let runtime = make_runtime().ok()?;

    runtime.block_on(async move {
        let pipe = connect(&pipe_name).ok()?;
        let mut client = Client::new(pipe);
        let hello = client
            .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
            .await
            .ok()?;
        let _ = client.close().await;
        Some(hello.server_version)
    })
}

/// 단발성 명령용 current-thread tokio 런타임.
fn make_runtime() -> Result<tokio::runtime::Runtime, ExitCode> {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| {
            let _ = writeln!(io::stderr(), "winmux: failed to start runtime: {e}");
            ExitCode::from(exit::GENERAL_ERROR)
        })
}

/// `ls`: 서버에 `ListSessions`를 보내고 결과를 출력한다.
fn handle_ls(_args: &LsArgs, global: &GlobalFlags) -> ExitCode {
    let identity = match UserIdentity::detect() {
        Ok(id) => id,
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: {e}");
            return ExitCode::from(exit::GENERAL_ERROR);
        }
    };
    let pipe_name = identity.pipe_name();

    let runtime = match make_runtime() {
        Ok(r) => r,
        Err(code) => return code,
    };

    // `ClientOptions::open`은 tokio reactor에 등록을 시도하므로 runtime 컨텍스트
    // 안에서 호출해야 한다 — 그래서 connect도 block_on 안으로 옮긴다.
    let connect_outcome = runtime.block_on(async { connect(&pipe_name) });

    let pipe = match connect_outcome {
        Ok(p) => p,
        Err(ConnectError::NotRunning(_)) => {
            // spec § ls Exit codes: 2 — server not running (read-only이라 auto-start 안 함).
            let _ = writeln!(io::stderr(), "winmux: server is not running");
            return ExitCode::from(exit::NO_SERVER);
        }
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: connect failed: {e}");
            return ExitCode::from(exit::GENERAL_ERROR);
        }
    };

    let result: anyhow::Result<Vec<SessionSummary>> = runtime.block_on(async {
        let mut client = Client::new(pipe);
        client
            .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
            .await?;
        let id = client.next_message_id();
        let resp = client
            .request(&ClientMessage::ListSessions {
                v: PROTOCOL_VERSION,
                id,
            })
            .await?;
        let sessions = match resp {
            ServerMessage::SessionList { sessions, .. } => sessions,
            ServerMessage::Error { payload, .. } => {
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
fn print_session_list(sessions: &[SessionSummary], global: &GlobalFlags) {
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

// ---------------------------------------------------------------------------
// new-session
// ---------------------------------------------------------------------------

/// `new-session`. spec § Server Auto-Start에 따라 write 명령이므로 서버가
/// 없으면 자동 기동을 시도한다.
fn handle_new_session(args: &NewSessionArgs, global: &GlobalFlags) -> ExitCode {
    if args.shell_argv.len() > 1 {
        // 본 단계에서는 셸 인자 배열을 와이어로 옮기지 못한다 — 메시지 페이로드의
        // `shell`은 한 단어다. 인자가 있으면 첫 토큰만 셸로 쓰고 나머지는 경고.
        let _ = writeln!(
            io::stderr(),
            "winmux: warning: extra `--` arguments are not yet wired; only the shell binary is honored"
        );
    }

    let identity = match UserIdentity::detect() {
        Ok(id) => id,
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: {e}");
            return ExitCode::from(exit::GENERAL_ERROR);
        }
    };
    let pipe_name = identity.pipe_name();

    let runtime = match make_runtime() {
        Ok(r) => r,
        Err(code) => return code,
    };

    let outcome = runtime.block_on(async {
        let pipe = ensure_server_running(&pipe_name)
            .await
            .map_err(EnsureFailure::into_tuple)?;
        new_session_with(pipe, args)
            .await
            .map_err(|e| (exit::GENERAL_ERROR, format!("{e:#}")))
    });

    match outcome {
        Ok(result) => {
            print_new_session(&result, args, global);
            ExitCode::SUCCESS
        }
        Err((code, message)) => emit_error(&message, code, global),
    }
}

/// `new-session` 핸들러의 lower-level 진입점. 통합 테스트가 직접 호출한다.
///
/// 호출자가 이미 connect까지 한 [`NamedPipeClient`]를 넘기면 Hello +
/// `NewSession` 요청 + 응답 매칭을 끝낸다.
pub async fn new_session_with(
    pipe: NamedPipeClient,
    args: &NewSessionArgs,
) -> anyhow::Result<NewSessionResult> {
    let mut client = Client::new(pipe);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;

    // shell 결정: `--shell` → `-- shell_argv[0]` → 미지정(서버 기본값).
    let shell = match args.shell.as_ref() {
        Some(s) => Some(s.clone()),
        None => args.shell_argv.first().cloned(),
    };

    let request = NewSessionRequest {
        name: args.session.clone(),
        shell,
        cwd: args
            .cwd
            .as_ref()
            .map(|p| p.display().to_string())
            .filter(|s| !s.is_empty()),
        env: BTreeMap::new(),
        detached: args.detached,
    };
    let id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::NewSession {
            v: PROTOCOL_VERSION,
            id,
            request,
        })
        .await?;

    let result = match resp {
        ServerMessage::Attached {
            session_id,
            active_window,
            panes,
            ..
        } => NewSessionResult::Attached {
            session_id,
            name: args.session.clone(),
            active_window,
            panes,
        },
        ServerMessage::Ok { .. } => NewSessionResult::Detached {
            name: args.session.clone(),
        },
        ServerMessage::Error { payload, .. } => {
            anyhow::bail!(
                "server error: {} ({})",
                payload.message,
                payload.code.as_str()
            );
        }
        other => anyhow::bail!("unexpected response: {other:?}"),
    };
    let _ = client.close().await;
    Ok(result)
}

/// `new-session` 결과 (와이어 응답을 cli 친화 모양으로 정리).
#[derive(Debug)]
pub enum NewSessionResult {
    /// 클라이언트가 자동 어태치된 경우.
    Attached {
        /// 생성된 세션.
        session_id: SessionId,
        /// 사용자가 지정한 이름(미지정이면 `None`).
        name: Option<String>,
        /// 활성 윈도우.
        active_window: WindowId,
        /// 활성 윈도우의 패널들.
        panes: Vec<PaneSummary>,
    },
    /// `-d` 옵션으로 detached 생성된 경우. 서버는 ID를 반환하지 않는다.
    Detached {
        /// 사용자가 지정한 이름.
        name: Option<String>,
    },
}

fn print_new_session(result: &NewSessionResult, _args: &NewSessionArgs, global: &GlobalFlags) {
    let mut stdout = io::stdout();
    if global.json {
        let v = match result {
            NewSessionResult::Attached {
                session_id,
                name,
                active_window,
                panes,
            } => {
                let panes_json: Vec<serde_json::Value> = panes
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "id": p.id.as_str(),
                            "rows": p.size.rows,
                            "cols": p.size.cols,
                            "alive": p.alive,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "session_id": session_id.as_str(),
                    "name": name,
                    "detached": false,
                    "active_window": active_window.as_str(),
                    "panes": panes_json,
                })
            }
            NewSessionResult::Detached { name } => {
                serde_json::json!({
                    "name": name,
                    "detached": true,
                })
            }
        };
        let _ = writeln!(stdout, "{v}");
        return;
    }
    if global.quiet {
        return;
    }
    match result {
        NewSessionResult::Attached {
            session_id,
            name,
            panes,
            ..
        } => {
            let display_name = name.as_deref().unwrap_or("(unnamed)");
            let _ = writeln!(
                stdout,
                "created session '{display_name}' ({})",
                session_id.as_str()
            );
            if let Some(p) = panes.first() {
                let _ = writeln!(
                    stdout,
                    "  pane {} ({}x{})",
                    p.id.as_str(),
                    p.size.rows,
                    p.size.cols
                );
            }
        }
        NewSessionResult::Detached { name } => {
            let display_name = name.as_deref().unwrap_or("(unnamed)");
            let _ = writeln!(stdout, "created session '{display_name}' (detached)");
        }
    }
}

// ---------------------------------------------------------------------------
// kill-session
// ---------------------------------------------------------------------------

/// `kill-session`. write 명령이므로 서버 자동 기동을 시도한다.
fn handle_kill_session(args: &KillSessionArgs, global: &GlobalFlags) -> ExitCode {
    let identity = match UserIdentity::detect() {
        Ok(id) => id,
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: {e}");
            return ExitCode::from(exit::GENERAL_ERROR);
        }
    };
    let pipe_name = identity.pipe_name();

    let runtime = match make_runtime() {
        Ok(r) => r,
        Err(code) => return code,
    };

    let outcome = runtime.block_on(async {
        let pipe = ensure_server_running(&pipe_name)
            .await
            .map_err(EnsureFailure::into_tuple)?;
        kill_session_with(pipe, &args.target)
            .await
            .map_err(|e| match e {
                KillFailure::NotFound(t) => {
                    (exit::TARGET_NOT_FOUND, format!("session not found: '{t}'"))
                }
                KillFailure::Server(msg) => (exit::GENERAL_ERROR, msg),
            })
    });

    match outcome {
        Ok(target) => {
            let mut stdout = io::stdout();
            if global.json {
                let v = serde_json::json!({ "killed": target });
                let _ = writeln!(stdout, "{v}");
            } else if !global.quiet {
                let _ = writeln!(stdout, "killed session '{target}'");
            }
            ExitCode::SUCCESS
        }
        Err((code, message)) => emit_error(&message, code, global),
    }
}

/// `kill-session` 핸들러의 lower-level 진입점.
pub async fn kill_session_with(pipe: NamedPipeClient, target: &str) -> Result<String, KillFailure> {
    let mut client = Client::new(pipe);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|e| KillFailure::Server(format!("hello: {e}")))?;

    let session = parse_kill_target(target);
    let id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::KillSession {
            v: PROTOCOL_VERSION,
            id,
            session,
        })
        .await
        .map_err(|e| KillFailure::Server(format!("request: {e}")))?;

    let result = match resp {
        ServerMessage::Ok { .. } => Ok(target.to_owned()),
        ServerMessage::Error { payload, .. } => {
            if payload.code == ErrorCode::SessionNotFound {
                Err(KillFailure::NotFound(target.to_owned()))
            } else {
                Err(KillFailure::Server(format!(
                    "{} ({})",
                    payload.message,
                    payload.code.as_str()
                )))
            }
        }
        other => Err(KillFailure::Server(format!(
            "unexpected response: {other:?}"
        ))),
    };
    let _ = client.close().await;
    result
}

/// `kill-session` 측 실패 분류.
#[derive(Debug)]
pub enum KillFailure {
    /// 타깃 세션이 없다 — exit code 3.
    NotFound(String),
    /// 그 외 서버/와이어 오류 — exit code 1.
    Server(String),
}

// ---------------------------------------------------------------------------
// kill-pane / kill-window
// ---------------------------------------------------------------------------

/// `kill-pane`. 타깃 인자는 PaneId(`pane-XYZ`)만 지원한다 — `session:0.0` 같은
/// 인덱스 타깃은 M0 PoC에서 한 패널만 다루므로 의미가 없다.
fn handle_kill_pane(args: &TargetOnlyArgs, global: &GlobalFlags) -> ExitCode {
    let identity = match UserIdentity::detect() {
        Ok(id) => id,
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: {e}");
            return ExitCode::from(exit::GENERAL_ERROR);
        }
    };
    let pipe_name = identity.pipe_name();
    let pane_id = match parse_pane_target(&args.target) {
        Ok(p) => p,
        Err(msg) => return emit_error(&msg, exit::USAGE_ERROR, global),
    };

    let runtime = match make_runtime() {
        Ok(r) => r,
        Err(code) => return code,
    };

    let outcome = runtime.block_on(async {
        let pipe = ensure_server_running(&pipe_name)
            .await
            .map_err(EnsureFailure::into_tuple)?;
        kill_pane_with(pipe, &pane_id).await.map_err(|e| match e {
            KillFailure::NotFound(t) => (exit::TARGET_NOT_FOUND, format!("pane not found: '{t}'")),
            KillFailure::Server(msg) => (exit::GENERAL_ERROR, msg),
        })
    });

    match outcome {
        Ok(()) => {
            let mut stdout = io::stdout();
            if global.json {
                let v = serde_json::json!({ "killed_pane": pane_id.as_str() });
                let _ = writeln!(stdout, "{v}");
            } else if !global.quiet {
                let _ = writeln!(stdout, "killed pane {}", pane_id.as_str());
            }
            ExitCode::SUCCESS
        }
        Err((code, message)) => emit_error(&message, code, global),
    }
}

/// `kill-pane` lower-level. 통합 테스트가 직접 호출한다.
pub async fn kill_pane_with(pipe: NamedPipeClient, pane_id: &PaneId) -> Result<(), KillFailure> {
    let mut client = Client::new(pipe);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|e| KillFailure::Server(format!("hello: {e}")))?;
    let id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::KillPane {
            v: PROTOCOL_VERSION,
            id,
            pane_id: pane_id.clone(),
        })
        .await
        .map_err(|e| KillFailure::Server(format!("request: {e}")))?;
    let result = match resp {
        ServerMessage::Ok { .. } => Ok(()),
        ServerMessage::Error { payload, .. } => {
            if payload.code == ErrorCode::PaneNotFound {
                Err(KillFailure::NotFound(pane_id.as_str().to_owned()))
            } else {
                Err(KillFailure::Server(format!(
                    "{} ({})",
                    payload.message,
                    payload.code.as_str()
                )))
            }
        }
        other => Err(KillFailure::Server(format!(
            "unexpected response: {other:?}"
        ))),
    };
    let _ = client.close().await;
    result
}

/// `kill-window`. 타깃은 WindowId(`win-XYZ`)만 지원.
fn handle_kill_window(args: &TargetOnlyArgs, global: &GlobalFlags) -> ExitCode {
    let identity = match UserIdentity::detect() {
        Ok(id) => id,
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: {e}");
            return ExitCode::from(exit::GENERAL_ERROR);
        }
    };
    let pipe_name = identity.pipe_name();
    let window_id = match parse_window_target(&args.target) {
        Ok(w) => w,
        Err(msg) => return emit_error(&msg, exit::USAGE_ERROR, global),
    };

    let runtime = match make_runtime() {
        Ok(r) => r,
        Err(code) => return code,
    };

    let outcome = runtime.block_on(async {
        let pipe = ensure_server_running(&pipe_name)
            .await
            .map_err(EnsureFailure::into_tuple)?;
        kill_window_with(pipe, &window_id)
            .await
            .map_err(|e| match e {
                KillFailure::NotFound(t) => {
                    (exit::TARGET_NOT_FOUND, format!("window not found: '{t}'"))
                }
                KillFailure::Server(msg) => (exit::GENERAL_ERROR, msg),
            })
    });

    match outcome {
        Ok(()) => {
            let mut stdout = io::stdout();
            if global.json {
                let v = serde_json::json!({ "killed_window": window_id.as_str() });
                let _ = writeln!(stdout, "{v}");
            } else if !global.quiet {
                let _ = writeln!(stdout, "killed window {}", window_id.as_str());
            }
            ExitCode::SUCCESS
        }
        Err((code, message)) => emit_error(&message, code, global),
    }
}

/// `kill-window` lower-level.
pub async fn kill_window_with(
    pipe: NamedPipeClient,
    window_id: &WindowId,
) -> Result<(), KillFailure> {
    let mut client = Client::new(pipe);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|e| KillFailure::Server(format!("hello: {e}")))?;
    let id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::KillWindow {
            v: PROTOCOL_VERSION,
            id,
            window_id: window_id.clone(),
        })
        .await
        .map_err(|e| KillFailure::Server(format!("request: {e}")))?;
    let result = match resp {
        ServerMessage::Ok { .. } => Ok(()),
        ServerMessage::Error { payload, .. } => {
            if payload.code == ErrorCode::WindowNotFound {
                Err(KillFailure::NotFound(window_id.as_str().to_owned()))
            } else {
                Err(KillFailure::Server(format!(
                    "{} ({})",
                    payload.message,
                    payload.code.as_str()
                )))
            }
        }
        other => Err(KillFailure::Server(format!(
            "unexpected response: {other:?}"
        ))),
    };
    let _ = client.close().await;
    result
}

fn parse_pane_target(s: &str) -> Result<PaneId, String> {
    if !s.starts_with(PaneId::PREFIX) {
        return Err(format!(
            "kill-pane target must be a pane id (`pane-...`), got `{s}`"
        ));
    }
    PaneId::from_str(s).map_err(|e| format!("invalid pane id `{s}`: {e}"))
}

fn parse_window_target(s: &str) -> Result<WindowId, String> {
    if !s.starts_with(WindowId::PREFIX) {
        return Err(format!(
            "kill-window target must be a window id (`win-...`), got `{s}`"
        ));
    }
    WindowId::from_str(s).map_err(|e| format!("invalid window id `{s}`: {e}"))
}

// ---------------------------------------------------------------------------
// send-keys
// ---------------------------------------------------------------------------

/// `send-keys`. `Command { tmux: "send-keys" }`로 매핑한다.
fn handle_send_keys(args: &SendKeysArgs, global: &GlobalFlags) -> ExitCode {
    let identity = match UserIdentity::detect() {
        Ok(id) => id,
        Err(e) => {
            let _ = writeln!(io::stderr(), "winmux: {e}");
            return ExitCode::from(exit::GENERAL_ERROR);
        }
    };
    let pipe_name = identity.pipe_name();

    let runtime = match make_runtime() {
        Ok(r) => r,
        Err(code) => return code,
    };

    let outcome = runtime.block_on(async {
        let pipe = ensure_server_running(&pipe_name)
            .await
            .map_err(EnsureFailure::into_tuple)?;
        send_keys_with(pipe, args.target.as_deref(), &args.keys)
            .await
            .map_err(|e| (exit::GENERAL_ERROR, e))
    });

    match outcome {
        Ok(()) => {
            if !global.quiet && global.json {
                let _ = writeln!(io::stdout(), "{}", serde_json::json!({ "ok": true }));
            }
            ExitCode::SUCCESS
        }
        Err((code, message)) => emit_error(&message, code, global),
    }
}

/// `send-keys` lower-level. 통합 테스트가 직접 호출한다.
///
/// 와이어 args는 `[-t TARGET]` 다음에 키 토큰들이 오는 형식이다 — server 측
/// 파서가 그 형태를 기대한다 (`pipe::parse_send_keys_args`).
pub async fn send_keys_with(
    pipe: NamedPipeClient,
    target: Option<&str>,
    keys: &[String],
) -> Result<(), String> {
    let mut client = Client::new(pipe);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|e| format!("hello: {e}"))?;

    let mut args: Vec<String> = Vec::new();
    if let Some(t) = target {
        args.push("-t".to_owned());
        args.push(t.to_owned());
    }
    args.extend(keys.iter().cloned());

    let id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::Command {
            v: PROTOCOL_VERSION,
            id,
            request: CommandRequest {
                tmux: "send-keys".to_owned(),
                args,
            },
        })
        .await
        .map_err(|e| format!("request: {e}"))?;
    let result = match resp {
        ServerMessage::CommandResult { result, .. } => {
            if result.ok {
                Ok(())
            } else {
                Err(result
                    .stderr
                    .unwrap_or_else(|| "send-keys failed".to_owned()))
            }
        }
        ServerMessage::Error { payload, .. } => {
            Err(format!("{} ({})", payload.message, payload.code.as_str()))
        }
        other => Err(format!("unexpected response: {other:?}")),
    };
    let _ = client.close().await;
    result
}

fn parse_kill_target(s: &str) -> KillSessionTarget {
    // `ses-`로 시작하면 ID 모양으로 가정 — 본문이 비었거나 잘못된 형태면
    // `from_str`이 거절하므로 자연스럽게 Name으로 fallback된다.
    if s.starts_with(SessionId::PREFIX)
        && let Ok(id) = SessionId::from_str(s)
    {
        return KillSessionTarget::Id(id);
    }
    KillSessionTarget::Name(s.to_owned())
}

// ---------------------------------------------------------------------------
// 공통: 에러 출력, 서버 자동 기동
// ---------------------------------------------------------------------------

/// human / JSON 두 가지 모드에 맞는 에러 출력 + exit code 변환.
fn emit_error(message: &str, code: u8, global: &GlobalFlags) -> ExitCode {
    if global.json {
        let value = serde_json::json!({
            "error": {
                "code": exit_code_label(code),
                "message": message,
            }
        });
        // spec § Output Format: `--json`일 때 에러도 stdout으로.
        let _ = writeln!(io::stdout(), "{value}");
    } else {
        let _ = writeln!(io::stderr(), "winmux: {message}");
    }
    ExitCode::from(code)
}

fn exit_code_label(code: u8) -> &'static str {
    match code {
        exit::NO_SERVER => "NO_SERVER",
        exit::TARGET_NOT_FOUND => "SESSION_NOT_FOUND",
        exit::PROTOCOL_MISMATCH => "PROTOCOL_MISMATCH",
        exit::PERMISSION_DENIED => "PERMISSION_DENIED",
        exit::USAGE_ERROR => "USAGE_ERROR",
        _ => "ERROR",
    }
}

/// 서버 자동 기동 흐름의 통합 실패. exit code와 message를 함께 가진다.
struct EnsureFailure {
    code: u8,
    message: String,
}

impl EnsureFailure {
    fn into_tuple(self) -> (u8, String) {
        (self.code, self.message)
    }
}

/// spec § Server Auto-Start 절차의 구현.
///
/// 1. 한 번 connect 시도. 떠 있으면 그대로 사용.
/// 2. `ERROR_FILE_NOT_FOUND`이면 `winmux-server.exe`를 같은 디렉터리에서 spawn.
/// 3. `connect_with_retry`(100/300/1000/3000 ms)로 다시 시도.
async fn ensure_server_running(pipe_name: &str) -> Result<NamedPipeClient, EnsureFailure> {
    match connect(pipe_name) {
        Ok(p) => return Ok(p),
        Err(ConnectError::NotRunning(_)) => {
            // fall through to spawn + retry.
        }
        Err(e) => {
            return Err(EnsureFailure {
                code: exit::GENERAL_ERROR,
                message: format!("connect failed: {e}"),
            });
        }
    }

    let server_exe = match locate_server_exe() {
        Ok(p) => p,
        Err(why) => {
            return Err(EnsureFailure {
                code: exit::NO_SERVER,
                message: format!("server is not running and {why}"),
            });
        }
    };
    if let Err(e) = spawn_server_detached(&server_exe) {
        return Err(EnsureFailure {
            code: exit::NO_SERVER,
            message: format!("failed to spawn `{}`: {e}", server_exe.display()),
        });
    }

    match connect_with_retry(pipe_name).await {
        Ok(p) => Ok(p),
        Err(e) => Err(EnsureFailure {
            code: exit::NO_SERVER,
            message: format!("server failed to start: {e}"),
        }),
    }
}

/// `winmux.exe`와 같은 폴더의 `winmux-server.exe`를 찾는다.
fn locate_server_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let parent = exe
        .parent()
        .ok_or_else(|| "current_exe has no parent directory".to_owned())?;
    let server = parent.join("winmux-server.exe");
    if !server.exists() {
        return Err(format!("`{}` not found", server.display()));
    }
    Ok(server)
}

/// `DETACHED_PROCESS | CREATE_NO_WINDOW`로 server를 띄운다.
/// spec § Server Auto-Start 2.
#[cfg(windows)]
fn spawn_server_detached(server_exe: &Path) -> Result<(), io::Error> {
    use std::os::windows::process::CommandExt;
    // Win32 process creation flags. winbase.h.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    std::process::Command::new(server_exe)
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_server_detached(_server_exe: &Path) -> Result<(), io::Error> {
    Err(io::Error::other(
        "server auto-start is only supported on Windows",
    ))
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn parse_kill_target_id_form() {
        let t = parse_kill_target("ses-01HKJ4Z6PXA7G3M2F9XQ7VWERT");
        match t {
            KillSessionTarget::Id(id) => assert_eq!(id.as_str(), "ses-01HKJ4Z6PXA7G3M2F9XQ7VWERT"),
            other => panic!("expected Id, got {other:?}"),
        }
    }

    #[test]
    fn parse_kill_target_name_form() {
        let t = parse_kill_target("work");
        match t {
            KillSessionTarget::Name(n) => assert_eq!(n, "work"),
            other => panic!("expected Name, got {other:?}"),
        }
    }

    #[test]
    fn parse_kill_target_ses_prefix_without_body_falls_back_to_name() {
        // `ses-`로 시작하지만 본문이 없거나 prefix만 있는 형태는 ID 파싱이
        // 실패하므로 Name으로 처리되어야 한다.
        let t = parse_kill_target("ses-");
        match t {
            KillSessionTarget::Name(n) => assert_eq!(n, "ses-"),
            other => panic!("expected Name fallback, got {other:?}"),
        }
    }

    #[test]
    fn exit_code_labels_cover_known_codes() {
        assert_eq!(exit_code_label(exit::NO_SERVER), "NO_SERVER");
        assert_eq!(exit_code_label(exit::TARGET_NOT_FOUND), "SESSION_NOT_FOUND");
        assert_eq!(exit_code_label(exit::USAGE_ERROR), "USAGE_ERROR");
        assert_eq!(exit_code_label(exit::GENERAL_ERROR), "ERROR");
    }
}
