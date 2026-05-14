//! v1 메시지 카탈로그.
//!
//! 와이어상 모든 메시지는 `{ "v": 1, "type": "<...>", ... }` 모양이며,
//! Rust 측에서는 두 방향(클라이언트→서버, 서버→클라이언트)으로 나뉜
//! enum으로 표현한다.
//!
//! 각 변종은 `#[serde(rename_all = "PascalCase")]`로 와이어상의
//! `"type"` 값을 자동 매핑한다. `Hello`, `HelloAck` 등 PascalCase
//! 식별자는 `docs/spec/01-ipc-protocol.md`와 일치한다.

use serde::{Deserialize, Serialize};

use crate::errors::ErrorPayload;
use crate::ids::{MessageId, PaneId, SessionId, WindowId};
use crate::types::{
    ClientKind, CommandRequest, CommandResultPayload, NewSessionRequest, PaneSize, PaneSummary,
    SelectDirection, SessionSummary, SplitDirection, WindowSummary,
};

/// `Attach`와 `KillSession`이 받는 세션 지정 방식.
///
/// 와이어는 두 모양 중 하나로 들어온다:
/// - `"work"` — 사람용 이름 문자열
/// - `{ "id": "ses-..." }` — 정확한 식별자
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttachTarget {
    /// 사용자가 정한 이름.
    Name(String),
    /// 정확한 세션 ID.
    Id {
        /// 세션 식별자.
        id: SessionId,
    },
}

/// `KillSession`의 타깃. 와이어 호환을 위해 `session` 필드 이름을 유지.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KillSessionTarget {
    /// 이름으로 지정.
    Name(String),
    /// ID로 지정.
    Id(SessionId),
}

/// 어태치 응답에 포함되는 패널별 초기 스냅샷.
///
/// `bytes_base64`는 가상 터미널 상태를 이스케이프 시퀀스로 직렬화한
/// 원시 바이트의 base64. 로그에 절대 찍지 않는다 (CLAUDE.md Rule 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneSnapshot {
    /// 대상 패널.
    pub pane_id: PaneId,
    /// base64로 인코딩된 초기 화면 바이트.
    pub bytes_base64: String,
}

/// 클라이언트 → 서버 메시지.
///
/// `#[serde(tag = "type")]`로 변종 이름이 `"type"` 필드와 매핑되며,
/// 모든 페이로드는 `deny_unknown_fields`로 미지 필드를 거부한다.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ClientMessage {
    /// 첫 메시지로 반드시 보내야 하는 자기소개.
    Hello {
        /// 프로토콜 버전.
        v: u32,
        /// 요청 상관용 ID.
        id: MessageId,
        /// 클라이언트 종류.
        client: ClientKind,
        /// 운영체제 PID.
        pid: u32,
        /// 클라이언트 자체 빌드 버전 (`CARGO_PKG_VERSION`).
        version: String,
    },

    /// 헬스체크. 응답은 [`ServerMessage::Pong`].
    Ping {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
    },

    /// 깨끗한 종료 신호. 서버는 응답하지 않는다.
    Bye {
        /// 프로토콜 버전.
        v: u32,
    },

    /// 모든 세션을 나열.
    ListSessions {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
    },

    /// 새 세션 생성. 응답은 [`ServerMessage::Attached`] 혹은 `Error`.
    NewSession {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 세부 요청 페이로드.
        #[serde(flatten)]
        request: NewSessionRequest,
    },

    /// 기존 세션에 어태치.
    Attach {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 어태치할 세션.
        session: AttachTarget,
        /// 클라이언트의 현재 화면 크기.
        client_size: PaneSize,
    },

    /// 디태치.
    Detach {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
    },

    /// 세션 종료.
    KillSession {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 종료할 세션.
        session: KillSessionTarget,
    },

    /// 새 윈도우 생성.
    NewWindow {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 윈도우를 생성할 세션.
        session: AttachTarget,
        /// 셸 별칭/경로. 미지정이면 세션 기본값.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
    },

    /// 패널 분할.
    SplitPane {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 분할 기준 패널.
        pane_id: PaneId,
        /// 분할 방향.
        direction: SplitDirection,
        /// 분할 비율(%). 1..=99 범위 권장. `None`이면 50%.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        percentage: Option<u8>,
    },

    /// 패널 종료.
    KillPane {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 종료할 패널.
        pane_id: PaneId,
    },

    /// 윈도우 종료.
    KillWindow {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 종료할 윈도우.
        window_id: WindowId,
    },

    /// 패널 크기 변경. 서버는 `ResizePseudoConsole`을 호출한다.
    Resize {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 대상 패널.
        pane_id: PaneId,
        /// 새 행 수.
        rows: u16,
        /// 새 열 수.
        cols: u16,
    },

    /// 활성 패널 변경.
    SelectPane {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 이동 방향.
        direction: SelectDirection,
    },

    /// 활성 윈도우 변경.
    SelectWindow {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 이동 방향.
        direction: SelectDirection,
    },

    /// tmux 등가 명령 실행. 서버가 내부 명령 enum으로 변환한다.
    Command {
        /// 프로토콜 버전.
        v: u32,
        /// 상관 ID.
        id: MessageId,
        /// 명령 페이로드.
        #[serde(flatten)]
        request: CommandRequest,
    },

    /// 스트리밍 키 입력. 응답 없음. `id`도 없다.
    PtyInput {
        /// 프로토콜 버전.
        v: u32,
        /// 대상 패널.
        pane_id: PaneId,
        /// base64로 인코딩된 원시 바이트.
        bytes_base64: String,
    },
}

impl ClientMessage {
    /// 이 메시지가 와이어로 알린 프로토콜 버전.
    #[must_use]
    pub fn protocol_version(&self) -> u32 {
        match self {
            Self::Hello { v, .. }
            | Self::Ping { v, .. }
            | Self::Bye { v }
            | Self::ListSessions { v, .. }
            | Self::NewSession { v, .. }
            | Self::Attach { v, .. }
            | Self::Detach { v, .. }
            | Self::KillSession { v, .. }
            | Self::NewWindow { v, .. }
            | Self::SplitPane { v, .. }
            | Self::KillPane { v, .. }
            | Self::KillWindow { v, .. }
            | Self::Resize { v, .. }
            | Self::SelectPane { v, .. }
            | Self::SelectWindow { v, .. }
            | Self::Command { v, .. }
            | Self::PtyInput { v, .. } => *v,
        }
    }
}

/// 서버 → 클라이언트 메시지.
///
/// 푸시 이벤트(PaneExited, WindowClosed, SessionRenamed,
/// PaneTitleChanged, AlertBell, PaneCursorVisibility)도 별도 wrapper
/// 없이 본 enum의 variant로 직접 직렬화된다. 이전에는 `Event` wrapper와
/// 내부 `EventMessage` enum의 이중 tag 구조였는데, 외부 `tag = "type"`과
/// 내부 `tag = "type"`이 같은 JSON 객체에 두 번 적혀 역직렬화가 실패하는
/// latent bug가 있었다(`serde_json` "duplicate field `type`"). 평면
/// 구조로 합치면 트레이의 `EventPayload`와도 그대로 정합한다.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ServerMessage {
    /// `Hello` 응답.
    HelloAck {
        /// 프로토콜 버전.
        v: u32,
        /// 원 요청의 `id`.
        id: MessageId,
        /// 서버 빌드 버전 (`CARGO_PKG_VERSION`).
        server_version: String,
        /// 검증된 사용자 이름.
        user: String,
    },

    /// `Ping`에 대한 응답.
    Pong {
        /// 프로토콜 버전.
        v: u32,
        /// 원 요청의 `id`.
        id: MessageId,
    },

    /// 서버 종료 직전의 작별 인사. 응답 필요 없음.
    ServerBye {
        /// 프로토콜 버전.
        v: u32,
    },

    /// `ListSessions` 응답.
    SessionList {
        /// 프로토콜 버전.
        v: u32,
        /// 원 요청의 `id`.
        id: MessageId,
        /// 세션 요약 배열.
        sessions: Vec<SessionSummary>,
    },

    /// `Attach`(또는 `NewSession`) 성공 응답.
    Attached {
        /// 프로토콜 버전.
        v: u32,
        /// 원 요청의 `id`.
        id: MessageId,
        /// 어태치된 세션.
        session_id: SessionId,
        /// 활성 윈도우.
        active_window: WindowId,
        /// 윈도우들의 요약 (현재는 활성 윈도우만이라도).
        #[serde(default)]
        windows: Vec<WindowSummary>,
        /// 활성 윈도우의 패널들.
        panes: Vec<PaneSummary>,
        /// 패널별 초기 화면 스냅샷.
        initial_snapshots: Vec<PaneSnapshot>,
    },

    /// `Command`의 결과.
    CommandResult {
        /// 프로토콜 버전.
        v: u32,
        /// 원 요청의 `id`.
        id: MessageId,
        /// 명령 결과 페이로드.
        #[serde(flatten)]
        result: CommandResultPayload,
    },

    /// 부수효과만 있는 명령에 대한 단순 성공 응답.
    Ok {
        /// 프로토콜 버전.
        v: u32,
        /// 원 요청의 `id`.
        id: MessageId,
    },

    /// 스트리밍 PTY 출력. 같은 세션에 어태치된 모든 클라이언트에 브로드캐스트.
    PtyOutput {
        /// 프로토콜 버전.
        v: u32,
        /// 출처 패널.
        pane_id: PaneId,
        /// base64로 인코딩된 원시 바이트.
        bytes_base64: String,
    },

    // ---- 푸시 이벤트들 (요청-응답이 아닌 서버 발신) -----------------------
    /// 셸이 종료되어 패널이 죽음.
    PaneExited {
        /// 프로토콜 버전.
        v: u32,
        /// 죽은 패널.
        pane_id: PaneId,
        /// 셸이 보고한 종료 코드.
        exit_code: i32,
    },

    /// 윈도우가 닫힘.
    WindowClosed {
        /// 프로토콜 버전.
        v: u32,
        /// 닫힌 윈도우.
        window_id: WindowId,
    },

    /// 세션 이름이 변경됨.
    SessionRenamed {
        /// 프로토콜 버전.
        v: u32,
        /// 대상 세션.
        session_id: SessionId,
        /// 새 이름.
        name: String,
    },

    /// 패널 타이틀이 변경됨(OSC 0/2).
    PaneTitleChanged {
        /// 프로토콜 버전.
        v: u32,
        /// 대상 패널.
        pane_id: PaneId,
        /// 새 타이틀.
        title: String,
    },

    /// 패널에서 BEL(`0x07`) 발생.
    AlertBell {
        /// 프로토콜 버전.
        v: u32,
        /// 발신 패널.
        pane_id: PaneId,
    },

    /// PTY 커서 가시성(DECTCEM)이 바뀌었다. `ESC[?25l`로 숨김 → `false`,
    /// `ESC[?25h`로 다시 켜짐 → `true`.
    ///
    /// 트레이는 본 이벤트를 받아 IME composition overlay의 앵커를 전환한다.
    /// 일반 셸은 PTY 커서가 화면상의 caret과 일치하므로 helper-textarea
    /// 좌표(=PTY 커서 셀)를 그대로 따라가지만, TUI 앱(claude, lazygit,
    /// htop, btop 등)은 `ESC[?25l`로 PTY 커서를 숨기고 자체 caret을 다른
    /// 셀에 그린다. 그 상태에서 helper-textarea 좌표를 따라가면 overlay가
    /// 의미 없는 위치에 뜨므로, 클라이언트는 가시성 = false일 때 패널 고정
    /// 좌표(예: bottom-left)로 anchor를 바꾼다.
    ///
    /// 초기 상태(가시 = true)는 암묵적 baseline이며 서버는 전이가 일어날
    /// 때만 이벤트를 발사한다. 같은 값이 반복돼도 스팸하지 않는다.
    PaneCursorVisibility {
        /// 프로토콜 버전.
        v: u32,
        /// 대상 패널.
        pane_id: PaneId,
        /// 새 가시성 상태. true = 커서 표시, false = 숨김.
        visible: bool,
    },

    /// 오류 응답.
    Error {
        /// 프로토콜 버전.
        v: u32,
        /// 오류 페이로드.
        #[serde(flatten)]
        payload: ErrorPayload,
    },
}

impl ServerMessage {
    /// 이 메시지가 와이어로 알린 프로토콜 버전.
    #[must_use]
    pub fn protocol_version(&self) -> u32 {
        match self {
            Self::HelloAck { v, .. }
            | Self::Pong { v, .. }
            | Self::ServerBye { v }
            | Self::SessionList { v, .. }
            | Self::Attached { v, .. }
            | Self::CommandResult { v, .. }
            | Self::Ok { v, .. }
            | Self::PtyOutput { v, .. }
            | Self::PaneExited { v, .. }
            | Self::WindowClosed { v, .. }
            | Self::SessionRenamed { v, .. }
            | Self::PaneTitleChanged { v, .. }
            | Self::AlertBell { v, .. }
            | Self::PaneCursorVisibility { v, .. }
            | Self::Error { v, .. } => *v,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::version::PROTOCOL_VERSION;

    fn msg_id() -> MessageId {
        MessageId::from_body("01HKJ4Z6PXA7G3M2F9XQ7VWERT").expect("msg id")
    }

    #[test]
    fn hello_roundtrips() {
        let hello = ClientMessage::Hello {
            v: PROTOCOL_VERSION,
            id: msg_id(),
            client: ClientKind::Tray,
            pid: 4242,
            version: "0.1.0".into(),
        };
        let json = serde_json::to_string(&hello).expect("ser");
        let back: ClientMessage = serde_json::from_str(&json).expect("de");
        assert_eq!(hello, back);
        assert!(json.contains("\"type\":\"Hello\""));
    }

    #[test]
    fn pty_input_carries_no_id() {
        let msg = ClientMessage::PtyInput {
            v: PROTOCOL_VERSION,
            pane_id: PaneId::from_body("ABCD").expect("pane id"),
            bytes_base64: "aGVsbG8=".into(),
        };
        let json = serde_json::to_string(&msg).expect("ser");
        assert!(
            !json.contains("\"id\""),
            "PtyInput must not carry an `id` field"
        );
        let back: ClientMessage = serde_json::from_str(&json).expect("de");
        assert_eq!(msg, back);
    }

    #[test]
    fn unknown_type_is_rejected() {
        let bad = r#"{"v":1,"type":"DefinitelyNotAType","id":"msg-X"}"#;
        let parsed: Result<ClientMessage, _> = serde_json::from_str(bad);
        assert!(parsed.is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let bad = r#"{"v":1,"type":"Bye","extra":1}"#;
        let parsed: Result<ClientMessage, _> = serde_json::from_str(bad);
        assert!(parsed.is_err(), "unknown top-level field must be rejected");
    }

    #[test]
    fn pane_cursor_visibility_event_roundtrips() {
        let pane_id = PaneId::from_body("PANE-CVZ").expect("pane id");
        let event = ServerMessage::PaneCursorVisibility {
            v: PROTOCOL_VERSION,
            pane_id: pane_id.clone(),
            visible: false,
        };
        let json = serde_json::to_string(&event).expect("ser");
        assert!(
            json.contains("\"type\":\"PaneCursorVisibility\""),
            "wire tag missing: {json}"
        );
        assert!(json.contains("\"visible\":false"));
        let back: ServerMessage = serde_json::from_str(&json).expect("de");
        assert_eq!(event, back);
        match back {
            ServerMessage::PaneCursorVisibility {
                pane_id: pid,
                visible,
                ..
            } => {
                assert_eq!(pid, pane_id);
                assert!(!visible);
            }
            other => panic!("expected PaneCursorVisibility, got {other:?}"),
        }
    }

    #[test]
    fn pane_exited_roundtrips() {
        // 평면 wire 구조 회귀 가드. 예전 `Event` wrapper + 내부 `EventMessage`
        // 조합이었을 때는 `"type"` 키가 두 번 등장해 역직렬화가 깨졌다.
        let pane_id = PaneId::from_body("PANE-EXIT").expect("pane id");
        let event = ServerMessage::PaneExited {
            v: PROTOCOL_VERSION,
            pane_id: pane_id.clone(),
            exit_code: 137,
        };
        let json = serde_json::to_string(&event).expect("ser");
        assert!(json.contains("\"type\":\"PaneExited\""));
        // `type`이 한 번만 등장하는지 보호한다.
        assert_eq!(json.matches("\"type\"").count(), 1);
        let back: ServerMessage = serde_json::from_str(&json).expect("de");
        assert_eq!(event, back);
    }

    #[test]
    fn attach_target_accepts_both_shapes() {
        let by_name: ClientMessage =
            serde_json::from_str(r#"{"v":1,"type":"Attach","id":"msg-A","session":"work","client_size":{"rows":40,"cols":120}}"#)
                .expect("name form");
        match by_name {
            ClientMessage::Attach {
                session: AttachTarget::Name(n),
                ..
            } => assert_eq!(n, "work"),
            other => panic!("expected Name, got {other:?}"),
        }

        let by_id: ClientMessage = serde_json::from_str(
            r#"{"v":1,"type":"Attach","id":"msg-B","session":{"id":"ses-X"},"client_size":{"rows":40,"cols":120}}"#,
        )
        .expect("id form");
        match by_id {
            ClientMessage::Attach {
                session: AttachTarget::Id { id },
                ..
            } => {
                assert_eq!(id.as_str(), "ses-X");
            }
            other => panic!("expected Id, got {other:?}"),
        }
    }
}
