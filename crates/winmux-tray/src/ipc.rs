//! 트레이의 Named Pipe IPC 매니저.
//!
//! 한 트레이 프로세스가 한 server 인스턴스와 가지는 영속 연결. Tauri의
//! 명령 핸들러는 [`IpcHandle::request`]·[`IpcHandle::send`]로 server에
//! 요청을 보내고, server에서 오는 `PtyOutput`·`Event`·`ServerBye`는
//! [`tauri::AppHandle`]에 등록된 webview로 emit된다.
//!
//! 백그라운드에서 두 task가 도는데:
//! - **reader**: pipe → [`ServerMessage`] → 응답이면 pending oneshot으로,
//!   푸시 메시지이면 webview emit.
//! - **writer**: outgoing mpsc → pipe.
//!
//! 외부에서는 [`IpcHandle`]만 다룬다. `IpcHandle`은 `Clone`이며 내부적으로
//! `Arc`로 카운트되므로 백그라운드 task와 Tauri State에 동시에 보관해도 된다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex as TokioMutex, mpsc, oneshot};
use tokio::time::timeout;
use tracing::{debug, info, warn};
use winmux_ipc_client::{ConnectError, connect, connect_with_retry};
use winmux_protocol::{
    ClientKind, ClientMessage, MessageId, PROTOCOL_VERSION, PaneId, PaneSnapshot, PaneSummary,
    ServerMessage, SessionId, UserIdentity, WindowId, WindowSummary, decode_line, encode_line,
};

/// 일반 요청의 응답 타임아웃 (`docs/spec/01-ipc-protocol.md` § Timeouts).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// `Attach`·`NewSession`은 PTY 스폰까지 포함하므로 10초.
const ATTACH_TIMEOUT: Duration = Duration::from_secs(10);
/// outgoing 큐 깊이. control plane 권장값 — 코딩 컨벤션 64.
const OUTGOING_CAPACITY: usize = 64;

/// 매니저의 현재 상태. webview에 `server:status` 이벤트로 emit된다.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ServerStatus {
    /// 연결 시도 중. 아직 Hello 완료 전.
    Connecting,
    /// Hello/HelloAck 완료. 명령을 받을 준비가 됐다.
    Connected {
        /// 서버 빌드 버전 (`CARGO_PKG_VERSION`).
        server_version: String,
        /// 서버가 검증한 사용자 이름.
        user: String,
    },
    /// 연결이 끊겼다. `reason`은 사람이 읽을 수 있는 한 줄.
    Disconnected {
        /// 종료 사유.
        reason: String,
    },
}

/// `pty:output` 이벤트로 webview에 emit되는 페이로드.
#[derive(Clone, Debug, Serialize)]
pub struct PtyOutputPayload {
    /// 출력 출처 패널.
    pub pane_id: PaneId,
    /// base64 인코딩된 원시 바이트 (`docs/spec/01-ipc-protocol.md` § PtyOutput).
    pub bytes_base64: String,
}

/// Tauri 명령에서 webview로 돌려주는 어태치 결과.
#[derive(Clone, Debug, Serialize)]
pub struct AttachOutcome {
    /// 어태치된 세션.
    pub session_id: SessionId,
    /// 활성 윈도우.
    pub active_window: WindowId,
    /// 윈도우 요약.
    pub windows: Vec<WindowSummary>,
    /// 활성 윈도우의 패널들.
    pub panes: Vec<PaneSummary>,
    /// 패널별 초기 스냅샷. alacritty_terminal 통합 전까지는 빈 벡터.
    pub initial_snapshots: Vec<PaneSnapshot>,
}

/// 응답 매칭용 펜딩 테이블.
type Pending = HashMap<String, oneshot::Sender<ServerMessage>>;

/// IPC 매니저 공개 핸들.
///
/// 내부적으로 `Arc<Inner>`라 [`Clone`]은 저렴하다. Tauri State 등록과
/// 백그라운드 task가 동일 핸들을 공유한다.
#[derive(Clone)]
pub struct IpcHandle {
    inner: Arc<Inner>,
}

struct Inner {
    out_tx: mpsc::Sender<ClientMessage>,
    pending: TokioMutex<Pending>,
    next_id: TokioMutex<u64>,
    status: TokioMutex<ServerStatus>,
}

impl IpcHandle {
    /// 현재 IPC 상태를 복제해 반환한다.
    pub async fn status(&self) -> ServerStatus {
        self.inner.status.lock().await.clone()
    }

    /// 응답을 기다리는 요청을 보낸다. `make`는 발급된 [`MessageId`]를 받아
    /// 메시지를 만든다.
    pub async fn request<F>(&self, make: F) -> Result<ServerMessage>
    where
        F: FnOnce(MessageId) -> ClientMessage,
    {
        self.request_inner(make, REQUEST_TIMEOUT).await
    }

    /// `Attach`/`NewSession`처럼 PTY 스폰이 포함된 요청 (10초 타임아웃).
    pub async fn request_attach<F>(&self, make: F) -> Result<ServerMessage>
    where
        F: FnOnce(MessageId) -> ClientMessage,
    {
        self.request_inner(make, ATTACH_TIMEOUT).await
    }

    /// 응답이 없는 메시지를 fire-and-forget으로 보낸다 (예: `PtyInput`).
    pub async fn send(&self, msg: ClientMessage) -> Result<()> {
        self.inner
            .out_tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("outgoing queue closed: {e}"))
    }

    async fn next_message_id(&self) -> MessageId {
        let mut guard = self.inner.next_id.lock().await;
        let n = *guard;
        *guard = guard.wrapping_add(1);
        let body = format!("{n:020x}");
        MessageId::from_body(&body).unwrap_or_else(|_| MessageId::from_raw(format!("msg-{body}")))
    }

    async fn request_inner<F>(&self, make: F, wait: Duration) -> Result<ServerMessage>
    where
        F: FnOnce(MessageId) -> ClientMessage,
    {
        let id = self.next_message_id().await;
        let msg = make(id.clone());
        let key = id.as_str().to_owned();
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(key.clone(), tx);

        if let Err(e) = self.inner.out_tx.send(msg).await {
            self.inner.pending.lock().await.remove(&key);
            bail!("outgoing queue closed: {e}");
        }
        match timeout(wait, rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => {
                self.inner.pending.lock().await.remove(&key);
                bail!("response channel dropped")
            }
            Err(_) => {
                self.inner.pending.lock().await.remove(&key);
                bail!("request timed out after {wait:?}")
            }
        }
    }
}

/// 트레이 IPC 매니저를 시작한다. 백그라운드에서 connect → Hello → read/write
/// 루프를 돌리고, 즉시 [`IpcHandle`]을 반환한다.
///
/// 호출자(`winmux_tray::start_ipc`)는 반환된 핸들을 Tauri State에 등록한다.
/// 추가 호출은 다시 백그라운드 task를 띄우므로 호출은 한 번만 한다.
pub fn start(app: AppHandle, identity: UserIdentity, tray_version: &str) -> IpcHandle {
    let (out_tx, out_rx) = mpsc::channel::<ClientMessage>(OUTGOING_CAPACITY);
    let inner = Arc::new(Inner {
        out_tx,
        pending: TokioMutex::new(HashMap::new()),
        next_id: TokioMutex::new(1),
        status: TokioMutex::new(ServerStatus::Connecting),
    });
    let handle = IpcHandle {
        inner: inner.clone(),
    };

    let pipe_name = identity.pipe_name();
    let version = tray_version.to_owned();

    tauri::async_runtime::spawn(manager_main(app, inner, pipe_name, version, out_rx));

    handle
}

async fn manager_main(
    app: AppHandle,
    inner: Arc<Inner>,
    pipe_name: String,
    version: String,
    out_rx: mpsc::Receiver<ClientMessage>,
) {
    info!(pipe = %pipe_name, "tray.ipc.connecting");
    set_status(&app, &inner, ServerStatus::Connecting).await;

    let pipe = match ensure_server_running(&pipe_name).await {
        Ok(p) => p,
        Err(reason) => {
            warn!(reason = %reason, "tray.ipc.connect_failed");
            set_status(&app, &inner, ServerStatus::Disconnected { reason }).await;
            return;
        }
    };

    let (read_half, write_half) = tokio::io::split(pipe);
    let mut read = BufReader::new(read_half);
    let mut write = write_half;

    // Hello 송신
    let hello_body = format!("{:020x}", 0u64);
    let hello_id = MessageId::from_body(&hello_body)
        .unwrap_or_else(|_| MessageId::from_raw(format!("msg-{hello_body}")));
    let hello = ClientMessage::Hello {
        v: PROTOCOL_VERSION,
        id: hello_id,
        client: ClientKind::Tray,
        pid: std::process::id(),
        version,
    };
    if let Err(e) = write_message(&mut write, &hello).await {
        warn!(error = %e, "tray.ipc.hello_send_failed");
        set_status(
            &app,
            &inner,
            ServerStatus::Disconnected {
                reason: e.to_string(),
            },
        )
        .await;
        return;
    }

    // HelloAck 대기
    let ack = match timeout(REQUEST_TIMEOUT, read_message(&mut read)).await {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            warn!(error = %e, "tray.ipc.hello_recv_failed");
            set_status(
                &app,
                &inner,
                ServerStatus::Disconnected {
                    reason: e.to_string(),
                },
            )
            .await;
            return;
        }
        Err(_) => {
            warn!("tray.ipc.hello_timeout");
            set_status(
                &app,
                &inner,
                ServerStatus::Disconnected {
                    reason: "HelloAck timed out".to_owned(),
                },
            )
            .await;
            return;
        }
    };
    let (server_version, user) = match ack {
        ServerMessage::HelloAck {
            server_version,
            user,
            ..
        } => (server_version, user),
        ServerMessage::Error { payload, .. } => {
            warn!(
                code = payload.code.as_str(),
                message = %payload.message,
                "tray.ipc.hello_rejected"
            );
            set_status(
                &app,
                &inner,
                ServerStatus::Disconnected {
                    reason: payload.message,
                },
            )
            .await;
            return;
        }
        other => {
            // CLAUDE.md Rule 1: 메시지 페이로드 자체는 절대 로그에 남기지 않는다 —
            // 변종 이름만.
            warn!(
                message_type = server_message_type(&other),
                "tray.ipc.hello_unexpected"
            );
            set_status(
                &app,
                &inner,
                ServerStatus::Disconnected {
                    reason: "unexpected HelloAck".to_owned(),
                },
            )
            .await;
            return;
        }
    };

    info!(
        server_version = %server_version,
        user = %user,
        "tray.ipc.connected"
    );
    set_status(
        &app,
        &inner,
        ServerStatus::Connected {
            server_version,
            user,
        },
    )
    .await;

    // 본 루프: read/write task 두 개를 띄우고 read가 끝나면 write도 abort.
    let app_for_read = app.clone();
    let inner_for_read = inner.clone();
    let read_task = tokio::spawn(async move {
        loop {
            let msg = match read_message(&mut read).await {
                Ok(m) => m,
                Err(e) => {
                    debug!(error = %e, "tray.ipc.read_ended");
                    break;
                }
            };
            dispatch_server_message(&app_for_read, &inner_for_read, msg).await;
        }
    });

    let write_task = tokio::spawn(async move {
        let mut out_rx = out_rx;
        while let Some(msg) = out_rx.recv().await {
            if let Err(e) = write_message(&mut write, &msg).await {
                warn!(error = %e, "tray.ipc.write_failed");
                break;
            }
        }
    });

    let _ = read_task.await;
    write_task.abort();
    info!("tray.ipc.connection_ended");
    set_status(
        &app,
        &inner,
        ServerStatus::Disconnected {
            reason: "pipe closed".to_owned(),
        },
    )
    .await;
}

async fn set_status(app: &AppHandle, inner: &Inner, status: ServerStatus) {
    *inner.status.lock().await = status.clone();
    if let Err(e) = app.emit("server:status", status) {
        warn!(error = %e, "tray.ipc.emit_status_failed");
    }
}

async fn dispatch_server_message(app: &AppHandle, inner: &Inner, msg: ServerMessage) {
    match msg {
        ServerMessage::PtyOutput {
            pane_id,
            bytes_base64,
            ..
        } => {
            let payload = PtyOutputPayload {
                pane_id,
                bytes_base64,
            };
            if let Err(e) = app.emit("pty:output", payload) {
                warn!(error = %e, "tray.ipc.emit_pty_failed");
            }
        }
        // 푸시 이벤트들은 평면 wire 구조를 그대로 webview로 흘려보낸다.
        // ServerMessage variant의 직렬화 결과(`{"type":"PaneExited",...}`)가
        // 트레이 webview의 `EventPayload` 정의와 곧바로 정합한다.
        ref msg @ (ServerMessage::PaneExited { .. }
        | ServerMessage::WindowClosed { .. }
        | ServerMessage::SessionRenamed { .. }
        | ServerMessage::PaneTitleChanged { .. }
        | ServerMessage::AlertBell { .. }
        | ServerMessage::PaneCursorVisibility { .. }) => {
            if let Err(e) = app.emit("server:event", msg) {
                warn!(error = %e, "tray.ipc.emit_event_failed");
            }
        }
        ServerMessage::ServerBye { .. } => {
            if let Err(e) = app.emit("server:bye", ()) {
                warn!(error = %e, "tray.ipc.emit_bye_failed");
            }
        }
        other => {
            let id_opt = response_id(&other).cloned();
            if let Some(id) = id_opt {
                let mut guard = inner.pending.lock().await;
                if let Some(tx) = guard.remove(id.as_str()) {
                    let _ = tx.send(other);
                } else {
                    warn!(id = %id.as_str(), "tray.ipc.orphan_response");
                }
            } else {
                warn!("tray.ipc.unmatched_response");
            }
        }
    }
}

fn response_id(msg: &ServerMessage) -> Option<&MessageId> {
    match msg {
        ServerMessage::HelloAck { id, .. }
        | ServerMessage::Pong { id, .. }
        | ServerMessage::SessionList { id, .. }
        | ServerMessage::Attached { id, .. }
        | ServerMessage::CommandResult { id, .. }
        | ServerMessage::Ok { id, .. } => Some(id),
        ServerMessage::Error { payload, .. } => payload.id.as_ref(),
        _ => None,
    }
}

/// 로그에 남길 수 있는 변종 이름. 페이로드는 절대 포함하지 않는다
/// (CLAUDE.md Rule 1).
fn server_message_type(msg: &ServerMessage) -> &'static str {
    match msg {
        ServerMessage::HelloAck { .. } => "HelloAck",
        ServerMessage::Pong { .. } => "Pong",
        ServerMessage::ServerBye { .. } => "ServerBye",
        ServerMessage::SessionList { .. } => "SessionList",
        ServerMessage::Attached { .. } => "Attached",
        ServerMessage::CommandResult { .. } => "CommandResult",
        ServerMessage::Ok { .. } => "Ok",
        ServerMessage::PtyOutput { .. } => "PtyOutput",
        ServerMessage::PaneExited { .. } => "PaneExited",
        ServerMessage::WindowClosed { .. } => "WindowClosed",
        ServerMessage::SessionRenamed { .. } => "SessionRenamed",
        ServerMessage::PaneTitleChanged { .. } => "PaneTitleChanged",
        ServerMessage::AlertBell { .. } => "AlertBell",
        ServerMessage::PaneCursorVisibility { .. } => "PaneCursorVisibility",
        ServerMessage::Error { .. } => "Error",
    }
}

async fn write_message<W>(write: &mut W, msg: &ClientMessage) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let json = serde_json::to_vec(msg).map_err(|e| anyhow::anyhow!("serialize: {e}"))?;
    let line = encode_line(&json).map_err(|e| anyhow::anyhow!("encode: {e}"))?;
    write
        .write_all(&line)
        .await
        .map_err(|e| anyhow::anyhow!("write: {e}"))?;
    write
        .flush()
        .await
        .map_err(|e| anyhow::anyhow!("flush: {e}"))?;
    Ok(())
}

async fn read_message<R>(read: &mut R) -> Result<ServerMessage>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut buf = Vec::with_capacity(512);
    let n = read
        .read_until(b'\n', &mut buf)
        .await
        .map_err(|e| anyhow::anyhow!("read: {e}"))?;
    if n == 0 {
        bail!("pipe closed");
    }
    let body = decode_line(&buf).map_err(|e| anyhow::anyhow!("frame: {e}"))?;
    serde_json::from_str::<ServerMessage>(body).map_err(|e| anyhow::anyhow!("parse: {e}"))
}

/// spec § 00-overview "Tray.Server discovery"의 자동 기동 절차.
///
/// 1. 한 번 connect 시도. 떠 있으면 그대로 사용.
/// 2. `ERROR_FILE_NOT_FOUND`이면 같은 디렉터리의 `winmux-server.exe`를
///    `DETACHED_PROCESS | CREATE_NO_WINDOW`로 spawn.
/// 3. 100/300/1000/3000 ms 백오프로 재시도. 끝까지 안 잡히면 실패.
///
/// 실패 시 사람이 읽을 수 있는 사유 문자열을 돌려준다 — 호출자가 그대로
/// [`ServerStatus::Disconnected`]에 넣어 webview로 emit한다.
async fn ensure_server_running(
    pipe_name: &str,
) -> Result<tokio::net::windows::named_pipe::NamedPipeClient, String> {
    // 1) 빠른 단발 시도. 이미 떠 있으면 즉시 끝.
    match connect(pipe_name) {
        Ok(p) => return Ok(p),
        Err(ConnectError::NotRunning(_)) => {
            // 아래에서 spawn.
        }
        Err(e) => return Err(format!("connect failed: {e}")),
    }

    // 2) winmux-server.exe 위치 결정 + spawn.
    let server_exe = locate_server_exe().map_err(|e| format!("server is not running and {e}"))?;
    info!(path = %server_exe.display(), "tray.ipc.server_spawn");
    spawn_server_detached(&server_exe)
        .map_err(|e| format!("failed to spawn `{}`: {e}", server_exe.display()))?;

    // 3) 백오프로 재시도. 첫 시도는 즉시이므로 startup overlap에도 안전.
    connect_with_retry(pipe_name)
        .await
        .map_err(|e| format!("server failed to start: {e}"))
}

/// `winmux-app.exe`(=Tauri 호스트)와 같은 폴더의 `winmux-server.exe`를 찾는다.
/// cargo dev 빌드에서는 `target/debug/`에 둘 다 있고, 인스톨러는 같은 install
/// dir에 둔다.
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

/// `DETACHED_PROCESS | CREATE_NO_WINDOW`로 server를 띄운다. spec § Tray.Server
/// discovery 의 spawn flags.
#[cfg(windows)]
fn spawn_server_detached(server_exe: &Path) -> Result<(), std::io::Error> {
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
fn spawn_server_detached(_server_exe: &Path) -> Result<(), std::io::Error> {
    Err(std::io::Error::other(
        "winmux-server spawn is only supported on Windows",
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;
    use winmux_protocol::{ErrorCode, ErrorPayload};

    #[test]
    fn server_status_connecting_shape() {
        let json = serde_json::to_value(ServerStatus::Connecting).expect("ser");
        assert_eq!(json, serde_json::json!({ "state": "connecting" }));
    }

    #[test]
    fn server_status_connected_shape() {
        let json = serde_json::to_value(ServerStatus::Connected {
            server_version: "0.1.0".to_owned(),
            user: "alice".to_owned(),
        })
        .expect("ser");
        assert_eq!(
            json,
            serde_json::json!({
                "state": "connected",
                "server_version": "0.1.0",
                "user": "alice"
            })
        );
    }

    #[test]
    fn response_id_for_known_variants() {
        let id = MessageId::from_body("ABCD").expect("msg id");
        let pong = ServerMessage::Pong {
            v: PROTOCOL_VERSION,
            id: id.clone(),
        };
        assert_eq!(response_id(&pong).map(MessageId::as_str), Some(id.as_str()));

        let err = ServerMessage::Error {
            v: PROTOCOL_VERSION,
            payload: ErrorPayload {
                id: Some(id.clone()),
                code: ErrorCode::Internal,
                message: "x".to_owned(),
                recoverable: true,
            },
        };
        assert_eq!(response_id(&err).map(MessageId::as_str), Some(id.as_str()));
    }

    #[test]
    fn response_id_returns_none_for_pty_output() {
        let pid = PaneId::from_body("ABCD").expect("pane id");
        let msg = ServerMessage::PtyOutput {
            v: PROTOCOL_VERSION,
            pane_id: pid,
            bytes_base64: "Zg==".to_owned(),
        };
        assert!(response_id(&msg).is_none());
    }
}
