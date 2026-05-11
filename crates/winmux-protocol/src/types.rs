//! 메시지 페이로드에서 재사용하는 공통 타입.
//!
//! 시간은 RFC3339 문자열 그대로 보관한다 (`docs/spec/01-ipc-protocol.md`).
//! 검증·파싱은 소비 측(주로 `winmux-server`)에서 수행한다. 이렇게 두면
//! `winmux-protocol`이 `time`·`chrono` 같은 시간 크레이트를 끌어들이지
//! 않아도 된다 (CLAUDE.md § Critical Boundaries).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::{PaneId, SessionId, WindowId};

/// 클라이언트 종류. `Hello`에서 자기소개에 쓴다.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// `winmux-tray` (Tauri 호스트).
    Tray,
    /// `winmux` CLI 단발성 클라이언트.
    Cli,
}

/// 패널 크기 (셀 단위).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneSize {
    /// 행(row) 수.
    pub rows: u16,
    /// 열(col) 수.
    pub cols: u16,
}

/// 윈도우를 둘로 가르는 방향.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    /// 가로 분할(위·아래).
    Horizontal,
    /// 세로 분할(왼쪽·오른쪽).
    Vertical,
}

/// `SelectPane` / `SelectWindow`에서 이동 방향.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SelectDirection {
    /// 왼쪽으로 이동.
    Left,
    /// 오른쪽으로 이동.
    Right,
    /// 위로 이동.
    Up,
    /// 아래로 이동.
    Down,
    /// 다음 형제로 이동.
    Next,
    /// 이전 형제로 이동.
    Previous,
}

/// `SessionList` 응답 항목.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSummary {
    /// 세션 ID.
    pub id: SessionId,
    /// 사용자가 부여한 이름.
    pub name: String,
    /// 세션이 생성된 시각(RFC3339).
    pub created_at: String,
    /// 윈도우 개수.
    pub windows: u16,
    /// 현재 어태치되어 있는 클라이언트 수.
    pub attached_clients: u16,
}

/// 윈도우 요약 (어태치 응답에 포함).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowSummary {
    /// 윈도우 ID.
    pub id: WindowId,
    /// 세션 내 인덱스(0부터).
    pub index: u8,
    /// 사용자가 지정한 이름. 없으면 `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 현재 활성 패널.
    pub active_pane: PaneId,
}

/// 패널 요약 (어태치 응답에 포함).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneSummary {
    /// 패널 ID.
    pub id: PaneId,
    /// 패널의 셀 크기.
    pub size: PaneSize,
    /// 현재 윈도우 안에서의 인덱스(0부터).
    pub index: u8,
    /// OSC 0/2로 보고된 타이틀(있으면).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 셸이 살아있는지 여부.
    pub alive: bool,
}

/// `NewSession` 요청 페이로드.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewSessionRequest {
    /// 세션 이름. 미지정이면 서버가 `untitled-N`을 부여.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 명시적 셸 경로 또는 `pwsh`/`cmd` 같은 별칭.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    /// 첫 패널의 현재 작업 디렉터리.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// 셸에 적용할 추가 환경 변수(사용자 환경에 덧붙임).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// `true`면 세션만 만들고 이 클라이언트는 자동 어태치하지 않음.
    #[serde(default)]
    pub detached: bool,
}

/// `Command` 메시지의 페이로드.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    /// tmux 등가 명령 이름 (예: `"send-keys"`, `"rename-session"`).
    pub tmux: String,
    /// 명령 인자. 위치 인자와 플래그를 그대로 받는다.
    #[serde(default)]
    pub args: Vec<String>,
}

/// `CommandResult` 응답 페이로드.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandResultPayload {
    /// 명령이 성공했는지.
    pub ok: bool,
    /// 표준 출력. 없으면 `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    /// 표준 오류. 없으면 `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn pane_size_roundtrips() {
        let size = PaneSize {
            rows: 40,
            cols: 120,
        };
        let json = serde_json::to_string(&size).expect("ser");
        let back: PaneSize = serde_json::from_str(&json).expect("de");
        assert_eq!(size, back);
    }

    #[test]
    fn split_direction_uses_lowercase_strings() {
        let json = serde_json::to_string(&SplitDirection::Horizontal).expect("ser");
        assert_eq!(json, "\"horizontal\"");
    }

    #[test]
    fn unknown_fields_rejected_on_pane_size() {
        let json = r#"{"rows":40,"cols":120,"extra":1}"#;
        let parsed: Result<PaneSize, _> = serde_json::from_str(json);
        assert!(parsed.is_err(), "unknown field must be rejected");
    }
}
