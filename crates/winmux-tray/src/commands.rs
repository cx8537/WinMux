//! Tauri 명령 핸들러.
//!
//! 모든 핸들러는 `winmux_` 프리픽스로 시작하고 [`tauri::State<IpcHandle>`]을
//! 통해 IPC 매니저에 접근한다. `src-tauri`는 `tauri::generate_handler!`로
//! 본 모듈의 함수들을 등록한다.
//!
//! 와이어 모양은 `docs/spec/01-ipc-protocol.md` 카탈로그를 그대로 따른다.
//! 본 모듈은 응답을 webview가 다루기 쉬운 모양으로 풀어주는 얇은 어댑터
//! 역할만 한다.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::State;
use winmux_protocol::{
    AttachTarget, ClientMessage, ErrorPayload, KillSessionTarget, NewSessionRequest,
    PROTOCOL_VERSION, PaneId, PaneSize, ServerMessage, SessionId, SessionSummary,
};

use crate::ipc::{AttachOutcome, IpcHandle, ServerStatus};

/// Tauri 명령에서 webview로 돌려보내는 오류.
///
/// `code`는 서버가 알린 `ErrorCode`의 와이어 문자열 (예: `"SESSION_NOT_FOUND"`)
/// 이거나, 본 어댑터 내부에서 실패한 경우 `None`이다.
#[derive(Debug, Serialize)]
pub struct CommandError {
    /// 사람이 읽을 수 있는 메시지.
    pub message: String,
    /// 서버가 알린 오류 코드 (있을 때).
    pub code: Option<String>,
    /// 연결을 계속 써도 되는지.
    pub recoverable: bool,
}

impl From<anyhow::Error> for CommandError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            message: format!("{e:#}"),
            code: None,
            recoverable: true,
        }
    }
}

fn error_from_server(payload: ErrorPayload) -> CommandError {
    CommandError {
        message: payload.message,
        code: Some(payload.code.as_str().to_owned()),
        recoverable: payload.recoverable,
    }
}

fn unexpected(msg: ServerMessage) -> CommandError {
    CommandError {
        message: format!("unexpected server response: {msg:?}"),
        code: None,
        recoverable: true,
    }
}

/// 와이어상 `AttachTarget`의 webview 친화 표현.
///
/// JSON: `{ "kind": "name", "name": "work" }` 또는
///       `{ "kind": "id",   "id":   "ses-..." }`.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionRef {
    /// 이름으로 지정.
    Name {
        /// 세션 이름.
        name: String,
    },
    /// ID로 지정.
    Id {
        /// 세션 ID.
        id: SessionId,
    },
}

impl SessionRef {
    fn into_attach_target(self) -> AttachTarget {
        match self {
            Self::Name { name } => AttachTarget::Name(name),
            Self::Id { id } => AttachTarget::Id { id },
        }
    }

    fn into_kill_target(self) -> KillSessionTarget {
        match self {
            Self::Name { name } => KillSessionTarget::Name(name),
            Self::Id { id } => KillSessionTarget::Id(id),
        }
    }
}

/// `NewSession` 요청 페이로드 (webview → command).
#[derive(Debug, Deserialize)]
pub struct NewSessionArgs {
    /// 세션 이름. 미지정이면 서버가 `untitled-N`을 부여.
    #[serde(default)]
    pub name: Option<String>,
    /// 명시 셸 경로 또는 별칭.
    #[serde(default)]
    pub shell: Option<String>,
    /// 첫 패널의 작업 디렉터리.
    #[serde(default)]
    pub cwd: Option<String>,
    /// 추가 환경 변수.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// `true`면 만든 뒤 자동 어태치하지 않음.
    #[serde(default)]
    pub detached: bool,
}

/// `Attach` 요청 페이로드.
#[derive(Debug, Deserialize)]
pub struct AttachArgs {
    /// 대상 세션.
    pub session: SessionRef,
    /// 클라이언트 현재 행 수.
    pub rows: u16,
    /// 클라이언트 현재 열 수.
    pub cols: u16,
}

/// 현재 IPC 상태를 조회한다.
#[tauri::command]
pub async fn winmux_server_status(
    handle: State<'_, IpcHandle>,
) -> Result<ServerStatus, CommandError> {
    Ok(handle.status().await)
}

/// `Ping` — 헬스 체크.
#[tauri::command]
pub async fn winmux_ping(handle: State<'_, IpcHandle>) -> Result<(), CommandError> {
    let resp = handle
        .request(|id| ClientMessage::Ping {
            v: PROTOCOL_VERSION,
            id,
        })
        .await?;
    match resp {
        ServerMessage::Pong { .. } => Ok(()),
        ServerMessage::Error { payload, .. } => Err(error_from_server(payload)),
        other => Err(unexpected(other)),
    }
}

/// `ListSessions` — 현재 세션 요약 배열을 반환한다.
#[tauri::command]
pub async fn winmux_list_sessions(
    handle: State<'_, IpcHandle>,
) -> Result<Vec<SessionSummary>, CommandError> {
    let resp = handle
        .request(|id| ClientMessage::ListSessions {
            v: PROTOCOL_VERSION,
            id,
        })
        .await?;
    match resp {
        ServerMessage::SessionList { sessions, .. } => Ok(sessions),
        ServerMessage::Error { payload, .. } => Err(error_from_server(payload)),
        other => Err(unexpected(other)),
    }
}

/// `NewSession`. detached이면 `None`, 아니면 자동 어태치 결과를 반환한다.
#[tauri::command]
pub async fn winmux_new_session(
    args: NewSessionArgs,
    handle: State<'_, IpcHandle>,
) -> Result<Option<AttachOutcome>, CommandError> {
    let request = NewSessionRequest {
        name: args.name,
        shell: args.shell,
        cwd: args.cwd,
        env: args.env,
        detached: args.detached,
    };
    let resp = handle
        .request_attach(|id| ClientMessage::NewSession {
            v: PROTOCOL_VERSION,
            id,
            request,
        })
        .await?;
    match resp {
        ServerMessage::Attached {
            session_id,
            active_window,
            windows,
            panes,
            initial_snapshots,
            ..
        } => Ok(Some(AttachOutcome {
            session_id,
            active_window,
            windows,
            panes,
            initial_snapshots,
        })),
        ServerMessage::Ok { .. } => Ok(None),
        ServerMessage::Error { payload, .. } => Err(error_from_server(payload)),
        other => Err(unexpected(other)),
    }
}

/// `Attach` — 기존 세션에 어태치한다.
#[tauri::command]
pub async fn winmux_attach(
    args: AttachArgs,
    handle: State<'_, IpcHandle>,
) -> Result<AttachOutcome, CommandError> {
    let target = args.session.into_attach_target();
    let resp = handle
        .request_attach(|id| ClientMessage::Attach {
            v: PROTOCOL_VERSION,
            id,
            session: target,
            client_size: PaneSize {
                rows: args.rows,
                cols: args.cols,
            },
        })
        .await?;
    match resp {
        ServerMessage::Attached {
            session_id,
            active_window,
            windows,
            panes,
            initial_snapshots,
            ..
        } => Ok(AttachOutcome {
            session_id,
            active_window,
            windows,
            panes,
            initial_snapshots,
        }),
        ServerMessage::Error { payload, .. } => Err(error_from_server(payload)),
        other => Err(unexpected(other)),
    }
}

/// `Detach` — 현재 어태치 해제.
#[tauri::command]
pub async fn winmux_detach(handle: State<'_, IpcHandle>) -> Result<(), CommandError> {
    let resp = handle
        .request(|id| ClientMessage::Detach {
            v: PROTOCOL_VERSION,
            id,
        })
        .await?;
    match resp {
        ServerMessage::Ok { .. } => Ok(()),
        ServerMessage::Error { payload, .. } => Err(error_from_server(payload)),
        other => Err(unexpected(other)),
    }
}

/// `KillSession`.
#[tauri::command]
pub async fn winmux_kill_session(
    session: SessionRef,
    handle: State<'_, IpcHandle>,
) -> Result<(), CommandError> {
    let target = session.into_kill_target();
    let resp = handle
        .request(|id| ClientMessage::KillSession {
            v: PROTOCOL_VERSION,
            id,
            session: target,
        })
        .await?;
    match resp {
        ServerMessage::Ok { .. } => Ok(()),
        ServerMessage::Error { payload, .. } => Err(error_from_server(payload)),
        other => Err(unexpected(other)),
    }
}

/// `PtyInput` — 응답을 기다리지 않는 fire-and-forget 키 입력.
#[tauri::command]
pub async fn winmux_pty_input(
    pane_id: PaneId,
    bytes_base64: String,
    handle: State<'_, IpcHandle>,
) -> Result<(), CommandError> {
    handle
        .send(ClientMessage::PtyInput {
            v: PROTOCOL_VERSION,
            pane_id,
            bytes_base64,
        })
        .await?;
    Ok(())
}

/// `Resize` — 서버가 `ResizePseudoConsole`을 호출하도록 요청한다.
#[tauri::command]
pub async fn winmux_resize(
    pane_id: PaneId,
    rows: u16,
    cols: u16,
    handle: State<'_, IpcHandle>,
) -> Result<(), CommandError> {
    let resp = handle
        .request(|id| ClientMessage::Resize {
            v: PROTOCOL_VERSION,
            id,
            pane_id,
            rows,
            cols,
        })
        .await?;
    match resp {
        ServerMessage::Ok { .. } => Ok(()),
        ServerMessage::Error { payload, .. } => Err(error_from_server(payload)),
        other => Err(unexpected(other)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;
    use winmux_protocol::ErrorCode;

    #[test]
    fn command_error_carries_code_when_from_server() {
        let payload = ErrorPayload {
            id: None,
            code: ErrorCode::SessionNotFound,
            message: "missing".to_owned(),
            recoverable: true,
        };
        let err = error_from_server(payload);
        assert_eq!(err.code.as_deref(), Some("SESSION_NOT_FOUND"));
        assert!(err.recoverable);
    }

    #[test]
    fn command_error_from_anyhow_has_no_code() {
        let err: CommandError = anyhow::anyhow!("oops").into();
        assert!(err.code.is_none());
        assert!(err.message.contains("oops"));
    }

    #[test]
    fn session_ref_deserializes_name_form() {
        let val: SessionRef =
            serde_json::from_str(r#"{"kind":"name","name":"work"}"#).expect("name");
        match val {
            SessionRef::Name { name } => assert_eq!(name, "work"),
            other => panic!("expected Name, got {other:?}"),
        }
    }

    #[test]
    fn session_ref_deserializes_id_form() {
        let val: SessionRef = serde_json::from_str(r#"{"kind":"id","id":"ses-ABCD"}"#).expect("id");
        match val {
            SessionRef::Id { id } => assert_eq!(id.as_str(), "ses-ABCD"),
            other => panic!("expected Id, got {other:?}"),
        }
    }
}
