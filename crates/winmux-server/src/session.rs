//! 메타데이터 전용 세션 모델과 in-memory registry.
//!
//! M0 단계에서는 PTY/스크롤백 같은 런타임 자원은 등장하지 않는다 — 본
//! 모듈은 단지 spec § Session/Window/Pane(`docs/spec/03-session-model.md`)을
//! 따라 식별자와 카운트를 보관하고, `ListSessions`·`Attach`·`KillSession`
//! 같은 dispatcher 동작을 지탱한다.
//!
//! `pty: Pty`, `vterm: VirtualTerm`, `scrollback: Scrollback` 같은
//! spec 필드들은 ConPTY 통합 단계에서 [`PaneState`]에 합쳐진다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use ulid::Ulid;
use winmux_protocol::{
    AttachTarget, KillSessionTarget, NewSessionRequest, PaneId, PaneSize, PaneSummary, SessionId,
    SessionSummary, WindowId, WindowSummary,
};

use crate::pty::Pty;

/// 패널 한 개 분량의 메타데이터(ConPTY 미통합 stub 포함).
#[derive(Clone, Debug)]
pub struct PaneState {
    /// 와이어 식별자.
    pub id: PaneId,
    /// 윈도우 내 인덱스(0부터).
    pub index: u8,
    /// 현재 셀 크기.
    pub size: PaneSize,
    /// OSC 0/2 등으로 받은 타이틀.
    pub title: Option<String>,
    /// 셸이 살아있는지. ConPTY 통합 전에는 항상 `false`.
    pub alive: bool,
}

impl PaneState {
    fn summary(&self) -> PaneSummary {
        PaneSummary {
            id: self.id.clone(),
            size: self.size,
            index: self.index,
            title: self.title.clone(),
            alive: self.alive,
        }
    }
}

/// 윈도우(=탭) 한 개 분량의 메타데이터.
#[derive(Clone, Debug)]
pub struct WindowState {
    /// 와이어 식별자.
    pub id: WindowId,
    /// 세션 내 인덱스(0부터). 윈도우 종료 시 재사용하지 않는다 — spec § Index.
    pub index: u8,
    /// 사용자 지정 이름. `None`이면 활성 패널의 타이틀이 표시명이 된다.
    pub name: Option<String>,
    /// 활성 패널.
    pub active_pane: PaneId,
    /// 패널 목록. 빈 윈도우는 만들지 않는다 — 항상 최소 1개.
    pub panes: Vec<PaneState>,
}

impl WindowState {
    fn summary(&self) -> WindowSummary {
        WindowSummary {
            id: self.id.clone(),
            index: self.index,
            name: self.name.clone(),
            active_pane: self.active_pane.clone(),
        }
    }
}

/// 세션 한 개 분량의 메타데이터.
#[derive(Clone, Debug)]
pub struct SessionState {
    /// 와이어 식별자.
    pub id: SessionId,
    /// 사용자가 부여한 이름. 비어 있으면 registry가 `untitled-N`을 부여.
    pub name: String,
    /// 생성 시각(RFC 3339).
    pub created_at: String,
    /// 윈도우 목록(최소 1개).
    pub windows: Vec<WindowState>,
    /// 활성 윈도우.
    pub active_window: WindowId,
    /// 현재 어태치된 클라이언트 수.
    pub attached_clients: u16,
}

impl SessionState {
    fn summary(&self) -> SessionSummary {
        SessionSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            created_at: self.created_at.clone(),
            // 윈도우 개수는 u16에 충분히 들어간다 (spec 한도 32).
            windows: u16::try_from(self.windows.len()).unwrap_or(u16::MAX),
            attached_clients: self.attached_clients,
        }
    }
}

/// Registry 동작 실패 사유.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// 이름 또는 ID로 해당 세션을 찾지 못함.
    #[error("session not found: {0}")]
    SessionNotFound(String),
    /// 이미 같은 이름의 세션이 있음.
    #[error("session name `{0}` already exists")]
    NameTaken(String),
}

/// `Attach` 응답에 필요한 정보를 묶은 결과.
#[derive(Clone, Debug)]
pub struct AttachResult {
    /// 어태치된 세션.
    pub session_id: SessionId,
    /// 활성 윈도우.
    pub active_window: WindowId,
    /// 윈도우 요약.
    pub windows: Vec<WindowSummary>,
    /// 활성 윈도우의 패널 요약.
    pub panes: Vec<PaneSummary>,
}

/// 한 패널의 런타임 자원. 메타데이터([`PaneState`])와 동일한 lifecycle.
pub struct PaneRuntime {
    /// 와이어 식별자.
    pub pane_id: PaneId,
    /// 소속 윈도우.
    pub window_id: WindowId,
    /// 소속 세션.
    pub session_id: SessionId,
    /// 셸의 PTY. `Arc`로 broadcast subscriber와 writer가 공유한다.
    pub pty: Arc<Pty>,
}

/// 세션 in-memory 저장소. 단일 server 프로세스 안에서 공유된다.
pub struct Registry {
    sessions: HashMap<SessionId, SessionState>,
    name_index: HashMap<String, SessionId>,
    untitled_counter: u32,
    pane_runtimes: HashMap<PaneId, PaneRuntime>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// 빈 registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            name_index: HashMap::new(),
            untitled_counter: 0,
            pane_runtimes: HashMap::new(),
        }
    }

    /// 새 세션을 만든다. M0 단계의 첫 윈도우/패널은 placeholder다
    /// (alive = false) — ConPTY 통합 시 실제 셸로 대체된다.
    pub fn create_session(&mut self, req: &NewSessionRequest) -> Result<SessionId, RegistryError> {
        let name = match req.name.clone() {
            Some(n) if !n.is_empty() => {
                if self.name_index.contains_key(&n) {
                    return Err(RegistryError::NameTaken(n));
                }
                n
            }
            _ => self.next_untitled_name(),
        };

        let pane_id = new_pane_id();
        let pane = PaneState {
            id: pane_id.clone(),
            index: 0,
            size: PaneSize {
                rows: 40,
                cols: 120,
            },
            title: None,
            alive: false,
        };

        let window_id = new_window_id();
        let window = WindowState {
            id: window_id.clone(),
            index: 0,
            name: None,
            active_pane: pane_id,
            panes: vec![pane],
        };

        let session_id = new_session_id();
        let session = SessionState {
            id: session_id.clone(),
            name: name.clone(),
            created_at: now_rfc3339(),
            windows: vec![window],
            active_window: window_id,
            attached_clients: 0,
        };

        self.sessions.insert(session_id.clone(), session);
        self.name_index.insert(name, session_id.clone());
        Ok(session_id)
    }

    /// 세션을 종료한다. 등록되어 있던 모든 [`PaneRuntime`]은 함께 drop되어
    /// `Pty` Drop으로 자식이 정리된다.
    pub fn kill_session(&mut self, target: &KillSessionTarget) -> Result<SessionId, RegistryError> {
        let id = self
            .resolve_kill_target(target)
            .ok_or_else(|| RegistryError::SessionNotFound(target_kill_display(target)))?;
        let session = self
            .sessions
            .remove(&id)
            .ok_or_else(|| RegistryError::SessionNotFound(id.to_string()))?;
        self.name_index.remove(&session.name);
        // 메타데이터 제거 후 runtime도 제거 → Pty Drop → 자식 kill.
        for window in &session.windows {
            for pane in &window.panes {
                self.pane_runtimes.remove(&pane.id);
            }
        }
        Ok(id)
    }

    /// 사전에 만들어진 [`PaneRuntime`]을 첫 패널로 가진 세션을 등록한다.
    ///
    /// 호출자는 [`new_session_id`]·[`new_window_id`]·[`new_pane_id`]로 미리 ID를
    /// 만들고 [`Pty::spawn`]을 끝낸 뒤 본 함수를 호출한다. 등록이 실패하면
    /// 호출자가 받은 `runtime`(즉 PTY)을 그대로 drop해서 자식을 정리한다.
    pub fn create_session_with_runtime(
        &mut self,
        req: &NewSessionRequest,
        size: PaneSize,
        runtime: PaneRuntime,
    ) -> Result<(SessionId, PaneId), RegistryError> {
        let name = match req.name.clone() {
            Some(n) if !n.is_empty() => {
                if self.name_index.contains_key(&n) {
                    return Err(RegistryError::NameTaken(n));
                }
                n
            }
            _ => self.next_untitled_name(),
        };

        let session_id = runtime.session_id.clone();
        let window_id = runtime.window_id.clone();
        let pane_id = runtime.pane_id.clone();

        let pane = PaneState {
            id: pane_id.clone(),
            index: 0,
            size,
            title: None,
            // 실제 셸이 PTY로 살아있으므로 alive.
            alive: true,
        };

        let window = WindowState {
            id: window_id.clone(),
            index: 0,
            name: None,
            active_pane: pane_id.clone(),
            panes: vec![pane],
        };

        let session = SessionState {
            id: session_id.clone(),
            name: name.clone(),
            created_at: now_rfc3339(),
            windows: vec![window],
            active_window: window_id,
            attached_clients: 0,
        };

        self.sessions.insert(session_id.clone(), session);
        self.name_index.insert(name, session_id.clone());
        self.pane_runtimes.insert(pane_id.clone(), runtime);
        Ok((session_id, pane_id))
    }

    /// 한 패널의 PTY 핸들을 [`Arc`]로 빌려준다(있으면).
    #[must_use]
    pub fn pty_for_pane(&self, pane_id: &PaneId) -> Option<Arc<Pty>> {
        self.pane_runtimes.get(pane_id).map(|r| r.pty.clone())
    }

    /// 한 세션의 모든 (패널-ID, PTY) 쌍을 모은다 — broadcast forwarder 등록에 쓴다.
    #[must_use]
    pub fn ptys_for_session(&self, session_id: &SessionId) -> Vec<(PaneId, Arc<Pty>)> {
        let Some(session) = self.sessions.get(session_id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for window in &session.windows {
            for pane in &window.panes {
                if let Some(rt) = self.pane_runtimes.get(&pane.id) {
                    out.push((pane.id.clone(), rt.pty.clone()));
                }
            }
        }
        out
    }

    /// 한 패널의 메타 size를 갱신한다(`Resize` 메시지 dispatch 시).
    pub fn update_pane_size(&mut self, pane_id: &PaneId, size: PaneSize) -> bool {
        for session in self.sessions.values_mut() {
            for window in &mut session.windows {
                if let Some(pane) = window.panes.iter_mut().find(|p| &p.id == pane_id) {
                    pane.size = size;
                    return true;
                }
            }
        }
        false
    }

    /// 모든 세션을 와이어용 요약으로 변환.
    #[must_use]
    pub fn list_summaries(&self) -> Vec<SessionSummary> {
        let mut out: Vec<SessionSummary> =
            self.sessions.values().map(SessionState::summary).collect();
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        out
    }

    /// 한 세션에 어태치 — `attached_clients`를 +1 하고 Attach 응답 정보를 만든다.
    pub fn attach(&mut self, target: &AttachTarget) -> Result<AttachResult, RegistryError> {
        let id = self
            .resolve_attach_target(target)
            .ok_or_else(|| RegistryError::SessionNotFound(target_attach_display(target)))?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| RegistryError::SessionNotFound(id.to_string()))?;
        session.attached_clients = session.attached_clients.saturating_add(1);
        let active_window_id = session.active_window.clone();
        let windows: Vec<WindowSummary> =
            session.windows.iter().map(WindowState::summary).collect();
        let panes: Vec<PaneSummary> = session
            .windows
            .iter()
            .find(|w| w.id == active_window_id)
            .map(|w| w.panes.iter().map(PaneState::summary).collect())
            .unwrap_or_default();
        Ok(AttachResult {
            session_id: id,
            active_window: active_window_id,
            windows,
            panes,
        })
    }

    /// `attached_clients`를 -1 (포화 감소).
    pub fn detach(&mut self, id: &SessionId) {
        if let Some(session) = self.sessions.get_mut(id) {
            session.attached_clients = session.attached_clients.saturating_sub(1);
        }
    }

    fn resolve_attach_target(&self, target: &AttachTarget) -> Option<SessionId> {
        match target {
            AttachTarget::Name(name) => self.name_index.get(name).cloned(),
            AttachTarget::Id { id } => {
                if self.sessions.contains_key(id) {
                    Some(id.clone())
                } else {
                    None
                }
            }
        }
    }

    fn resolve_kill_target(&self, target: &KillSessionTarget) -> Option<SessionId> {
        match target {
            KillSessionTarget::Name(name) => self.name_index.get(name).cloned(),
            KillSessionTarget::Id(id) => {
                if self.sessions.contains_key(id) {
                    Some(id.clone())
                } else {
                    None
                }
            }
        }
    }

    fn next_untitled_name(&mut self) -> String {
        loop {
            self.untitled_counter = self.untitled_counter.wrapping_add(1);
            let candidate = format!("untitled-{}", self.untitled_counter);
            if !self.name_index.contains_key(&candidate) {
                return candidate;
            }
        }
    }
}

/// `Arc<Mutex<Registry>>`를 짧게 부르는 별칭.
pub type SharedRegistry = Arc<Mutex<Registry>>;

/// 새 [`SessionId`]를 ULID로 만든다.
#[must_use]
pub fn new_session_id() -> SessionId {
    let body = Ulid::new().to_string();
    SessionId::from_body(&body).unwrap_or_else(|_| SessionId::from_raw(format!("ses-{body}")))
}

/// 새 [`WindowId`]를 ULID로 만든다.
#[must_use]
pub fn new_window_id() -> WindowId {
    let body = Ulid::new().to_string();
    WindowId::from_body(&body).unwrap_or_else(|_| WindowId::from_raw(format!("win-{body}")))
}

/// 새 [`PaneId`]를 ULID로 만든다.
#[must_use]
pub fn new_pane_id() -> PaneId {
    let body = Ulid::new().to_string();
    PaneId::from_body(&body).unwrap_or_else(|_| PaneId::from_raw(format!("pane-{body}")))
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

fn target_attach_display(target: &AttachTarget) -> String {
    match target {
        AttachTarget::Name(n) => n.clone(),
        AttachTarget::Id { id } => id.to_string(),
    }
}

fn target_kill_display(target: &KillSessionTarget) -> String {
    match target {
        KillSessionTarget::Name(n) => n.clone(),
        KillSessionTarget::Id(id) => id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn empty_request() -> NewSessionRequest {
        NewSessionRequest {
            name: None,
            shell: None,
            cwd: None,
            env: Default::default(),
            detached: false,
        }
    }

    #[test]
    fn create_session_assigns_untitled_when_no_name() {
        let mut r = Registry::new();
        let id = r.create_session(&empty_request()).expect("create");
        let summary = r
            .list_summaries()
            .into_iter()
            .find(|s| s.id == id)
            .expect("listed");
        assert!(summary.name.starts_with("untitled-"));
        assert_eq!(summary.windows, 1);
        assert_eq!(summary.attached_clients, 0);
    }

    #[test]
    fn create_session_rejects_duplicate_name() {
        let mut r = Registry::new();
        let mut req = empty_request();
        req.name = Some("work".to_owned());
        r.create_session(&req).expect("first");
        let dup = r.create_session(&req);
        assert!(matches!(dup, Err(RegistryError::NameTaken(_))));
    }

    #[test]
    fn list_summaries_orders_by_created_at() {
        let mut r = Registry::new();
        for _ in 0..3 {
            r.create_session(&empty_request()).expect("create");
        }
        let summaries = r.list_summaries();
        assert_eq!(summaries.len(), 3);
        for pair in summaries.windows(2) {
            assert!(pair[0].created_at <= pair[1].created_at);
        }
    }

    #[test]
    fn attach_increments_and_detach_decrements_counter() {
        let mut r = Registry::new();
        let id = r.create_session(&empty_request()).expect("create");
        let attach = r
            .attach(&AttachTarget::Id { id: id.clone() })
            .expect("attach");
        assert_eq!(attach.session_id, id);
        assert_eq!(r.sessions[&id].attached_clients, 1);
        r.detach(&id);
        assert_eq!(r.sessions[&id].attached_clients, 0);
        // 추가 detach는 포화 — 0 유지.
        r.detach(&id);
        assert_eq!(r.sessions[&id].attached_clients, 0);
    }

    #[test]
    fn attach_by_name_resolves() {
        let mut r = Registry::new();
        let mut req = empty_request();
        req.name = Some("docs".to_owned());
        let id = r.create_session(&req).expect("create");
        let attach = r
            .attach(&AttachTarget::Name("docs".to_owned()))
            .expect("attach by name");
        assert_eq!(attach.session_id, id);
    }

    #[test]
    fn kill_removes_session_and_frees_name() {
        let mut r = Registry::new();
        let mut req = empty_request();
        req.name = Some("build".to_owned());
        let id = r.create_session(&req).expect("create");
        let killed = r
            .kill_session(&KillSessionTarget::Name("build".to_owned()))
            .expect("kill");
        assert_eq!(killed, id);
        assert!(r.list_summaries().is_empty());
        // 같은 이름 재사용 가능.
        r.create_session(&req).expect("recreate");
    }

    #[test]
    fn kill_unknown_target_returns_error() {
        let mut r = Registry::new();
        let res = r.kill_session(&KillSessionTarget::Name("nope".to_owned()));
        assert!(matches!(res, Err(RegistryError::SessionNotFound(_))));
    }
}
