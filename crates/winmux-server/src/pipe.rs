//! Named Pipe 서버.
//!
//! 본 모듈은 다음을 책임진다:
//! - 명시 ACL을 단 한 번 빌드하고, 매 accept iteration마다 새 파이프
//!   인스턴스를 그 ACL로 만든다.
//! - 첫 인스턴스에만 `FILE_FLAG_FIRST_PIPE_INSTANCE` 적용
//!   (`docs/spec/01-ipc-protocol.md` § Pipe creation).
//! - accept된 클라이언트의 SID를 서버 자신과 비교 검증.
//! - 검증 성공 시 한 task를 띄워 Hello 핸드셰이크와 후속 메시지 수신을 맡긴다.
//! - shutdown 신호를 받으면 accept 루프를 빠져나간다.
//!
//! 본 모듈은 PTY 콘텐츠를 다루지 않으며, 따라서 어떤 로그도 그 내용을
//! 포함하지 않는다 (CLAUDE.md Rule 1).

pub mod handshake;
pub mod security;

pub use handshake::AuthenticatedClient;
pub use security::PipeAcl;

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufStream};
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use winmux_protocol::{
    ClientMessage, ErrorCode, MessageId, PROTOCOL_VERSION, ServerMessage, SessionId, UserIdentity,
    decode_line,
};

use crate::pipe::handshake::{perform_handshake, send_error_message, send_server_message};
use crate::pipe::security::verify_client_user;
use crate::session::{Registry, RegistryError, SharedRegistry};

/// 단일 파이프 인스턴스당 커널 in/out 버퍼 hint(바이트).
const PIPE_IN_BUF: u32 = 64 * 1024;
const PIPE_OUT_BUF: u32 = 64 * 1024;

/// 최대 동시 인스턴스 수. tokio의 ServerOptions는 254까지만 허용한다
/// (Win32의 `PIPE_UNLIMITED_INSTANCES=255`는 매직 값으로 별도 처리됨).
const PIPE_MAX_INSTANCES: usize = 254;

/// accept 루프 진입점.
///
/// 한 번에 하나씩 새 파이프 인스턴스를 만들고, 한 클라이언트를 받으면
/// 핸들러 task로 분리한 뒤 다음 인스턴스를 만든다. `shutdown_rx`가
/// 신호를 받으면 즉시 종료한다 (반쯤 만들어진 인스턴스는 Drop으로 정리됨).
pub async fn run(identity: UserIdentity, mut shutdown_rx: oneshot::Receiver<()>) -> Result<()> {
    let acl = PipeAcl::build_for_current_user().context("build pipe security descriptor")?;
    let pipe_name = identity.pipe_name();
    info!(pipe = %pipe_name, "ipc.listening");

    // 단일 server 프로세스 안에서 공유되는 세션 registry. std::sync::Mutex로
    // 충분 — critical section은 짧고 절대 `.await`을 잡고 있지 않는다.
    let registry: SharedRegistry = Arc::new(Mutex::new(Registry::new()));

    let mut first = true;
    loop {
        let server = make_server_instance(&pipe_name, &acl, first)
            .with_context(|| format!("create pipe instance for {pipe_name}"))?;
        first = false;
        debug!("ipc.accept.waiting");

        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                info!("ipc.shutdown.received");
                return Ok(());
            }
            connect_res = server.connect() => {
                connect_res.context("pipe connect")?;
                debug!("ipc.client.connected");
                if let Err(e) = verify_client_user(&server, acl.server_sid()) {
                    warn!(error = %e, "client.sid_check.rejected");
                    drop(server);
                    continue;
                }
                let username = identity.username.clone();
                let registry = registry.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(server, username, registry).await {
                        warn!(error = %e, "client.handler.failed");
                    }
                });
            }
        }
    }
}

/// 한 파이프 인스턴스를 만든다. 첫 인스턴스에는 `FIRST_PIPE_INSTANCE` 적용.
fn make_server_instance(name: &str, acl: &PipeAcl, first: bool) -> Result<NamedPipeServer> {
    let attrs = acl.as_security_attributes_ptr();
    // SAFETY: `attrs`는 `acl`이 살아있는 동안 유효한 SECURITY_ATTRIBUTES를 가리킨다.
    //         tokio는 본 호출 시점에 그 디스크립터를 커널에 전달한 뒤 더 이상
    //         참조하지 않는다.
    #[allow(unsafe_code)]
    let server = unsafe {
        ServerOptions::new()
            .access_inbound(true)
            .access_outbound(true)
            .pipe_mode(PipeMode::Byte)
            .first_pipe_instance(first)
            .max_instances(PIPE_MAX_INSTANCES)
            .in_buffer_size(PIPE_IN_BUF)
            .out_buffer_size(PIPE_OUT_BUF)
            .create_with_security_attributes_raw(name, attrs)
    }
    .context("CreateNamedPipeW (with custom security attributes)")?;
    Ok(server)
}

/// 디스패처의 다음 행동.
enum DispatchOutcome {
    /// 다음 메시지를 계속 읽는다.
    Continue,
    /// `Bye` 등으로 정상 종료. 루프를 빠져나간다.
    Disconnect,
}

/// `attached_clients` 카운트를 보장 감소시키는 RAII 가드.
///
/// 한 클라이언트 task가 attach한 세션을 추적해서, 명시적 `Detach`·`Bye`·
/// 비정상 disconnect 어느 경로로 끝나든 [`Registry::detach`]가 정확히
/// 한 번 호출되도록 한다.
struct AttachGuard {
    registry: SharedRegistry,
    session_id: SessionId,
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        let mut reg = lock_registry(&self.registry);
        reg.detach(&self.session_id);
    }
}

/// poison 가드. mutex가 panic 후에도 안전하게 데이터를 빌릴 수 있다.
fn lock_registry(reg: &SharedRegistry) -> std::sync::MutexGuard<'_, Registry> {
    reg.lock().unwrap_or_else(|e| e.into_inner())
}

/// 클라이언트 핸들러: 핸드셰이크 → 메시지 dispatch.
async fn handle_client(
    server: NamedPipeServer,
    username: String,
    registry: SharedRegistry,
) -> Result<()> {
    let mut stream = BufStream::new(server);
    let auth = perform_handshake(&mut stream, &username).await?;

    info!(
        client = ?auth.client_kind,
        client_pid = auth.client_pid,
        client_version = %auth.client_version,
        "client.session.start"
    );

    let mut attach: Option<AttachGuard> = None;

    let mut buf = Vec::with_capacity(512);
    loop {
        buf.clear();
        let n = stream.read_until(b'\n', &mut buf).await.context("read")?;
        if n == 0 {
            debug!("client.eof");
            break;
        }
        let body = match decode_line(&buf) {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "client.frame.invalid");
                send_error_message(
                    &mut stream,
                    None,
                    ErrorCode::ProtocolViolation,
                    &format!("frame error: {e}"),
                    true,
                )
                .await;
                continue;
            }
        };
        let msg = match serde_json::from_str::<ClientMessage>(body) {
            Ok(m) => m,
            Err(e) => {
                warn!(error = %e, "client.message.invalid");
                send_error_message(
                    &mut stream,
                    None,
                    ErrorCode::ProtocolViolation,
                    &format!("deserialize failed: {e}"),
                    true,
                )
                .await;
                continue;
            }
        };
        match dispatch_message(&mut stream, &registry, &mut attach, msg).await? {
            DispatchOutcome::Continue => {}
            DispatchOutcome::Disconnect => break,
        }
    }

    // attach Drop이 카운터를 -1 해준다.
    info!(client = ?auth.client_kind, "client.session.end");
    Ok(())
}

/// 핸드셰이크 이후 들어온 한 메시지를 처리한다.
///
/// `attach`는 현재 클라이언트가 어태치한 세션을 추적한다. 새 어태치 또는
/// 디태치 시점에 갱신되어 Drop이 카운터를 정리하게 한다.
async fn dispatch_message<S>(
    stream: &mut BufStream<S>,
    registry: &SharedRegistry,
    attach: &mut Option<AttachGuard>,
    msg: ClientMessage,
) -> Result<DispatchOutcome>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let type_name = message_type_name(&msg);
    debug!(message_type = %type_name, "client.message.received");

    match msg {
        ClientMessage::Hello { id, .. } => {
            send_error_message(
                stream,
                Some(id),
                ErrorCode::ProtocolViolation,
                "Hello already completed for this connection",
                false,
            )
            .await;
            Ok(DispatchOutcome::Disconnect)
        }
        ClientMessage::Ping { id, .. } => {
            let pong = ServerMessage::Pong {
                v: PROTOCOL_VERSION,
                id,
            };
            send_server_message(stream, &pong).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::Bye { .. } => {
            info!("client.bye");
            Ok(DispatchOutcome::Disconnect)
        }
        ClientMessage::ListSessions { id, .. } => {
            let sessions = lock_registry(registry).list_summaries();
            let resp = ServerMessage::SessionList {
                v: PROTOCOL_VERSION,
                id,
                sessions,
            };
            send_server_message(stream, &resp).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::NewSession { id, request, .. } => {
            handle_new_session(stream, registry, attach, id, request).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::Attach { id, session, .. } => {
            handle_attach(stream, registry, attach, id, session).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::Detach { id, .. } => {
            // attach Drop이 카운터를 -1 한다.
            *attach = None;
            let resp = ServerMessage::Ok {
                v: PROTOCOL_VERSION,
                id,
            };
            send_server_message(stream, &resp).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::KillSession { id, session, .. } => {
            let result = lock_registry(registry).kill_session(&session);
            match result {
                Ok(killed_id) => {
                    info!(session = %killed_id, "session.killed");
                    // 이 클라이언트가 그 세션에 어태치되어 있었다면 attach guard가
                    // 살아있는 동안 Drop은 detach를 호출하지만 세션은 이미 제거되어
                    // no-op. 명시적으로 우리 attach를 비워둔다.
                    if let Some(g) = attach.as_ref()
                        && g.session_id == killed_id
                    {
                        *attach = None;
                    }
                    let resp = ServerMessage::Ok {
                        v: PROTOCOL_VERSION,
                        id,
                    };
                    send_server_message(stream, &resp).await?;
                }
                Err(RegistryError::SessionNotFound(_)) => {
                    send_error_message(
                        stream,
                        Some(id),
                        ErrorCode::SessionNotFound,
                        "session not found",
                        true,
                    )
                    .await;
                }
                Err(e) => {
                    send_error_message(stream, Some(id), ErrorCode::Internal, &e.to_string(), true)
                        .await;
                }
            }
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::PtyInput { .. } => {
            debug!("client.pty_input.discarded");
            Ok(DispatchOutcome::Continue)
        }
        other => {
            let request_id = client_message_id(&other);
            warn!(message_type = %type_name, "client.message.unhandled");
            send_error_message(
                stream,
                request_id,
                ErrorCode::Internal,
                &format!("`{type_name}` is parsed but not yet dispatched"),
                true,
            )
            .await;
            Ok(DispatchOutcome::Continue)
        }
    }
}

/// `NewSession` 분기. 생성 성공 후 detached가 아니면 자동 어태치.
async fn handle_new_session<S>(
    stream: &mut BufStream<S>,
    registry: &SharedRegistry,
    attach: &mut Option<AttachGuard>,
    request_id: MessageId,
    request: winmux_protocol::NewSessionRequest,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let detached = request.detached;
    let create_result = lock_registry(registry).create_session(&request);
    let new_id = match create_result {
        Ok(id) => id,
        Err(e) => {
            warn!(error = %e, "session.new_session.failed");
            let code = match e {
                RegistryError::SessionNotFound(_) => ErrorCode::SessionNotFound,
                _ => ErrorCode::Internal,
            };
            send_error_message(stream, Some(request_id), code, &e.to_string(), true).await;
            return Ok(());
        }
    };
    info!(session = %new_id, detached, "session.created");

    if detached {
        let resp = ServerMessage::Ok {
            v: PROTOCOL_VERSION,
            id: request_id,
        };
        send_server_message(stream, &resp).await?;
        return Ok(());
    }

    // 자동 어태치.
    let attach_result =
        lock_registry(registry).attach(&winmux_protocol::AttachTarget::Id { id: new_id.clone() });
    match attach_result {
        Ok(a) => {
            *attach = Some(AttachGuard {
                registry: registry.clone(),
                session_id: a.session_id.clone(),
            });
            let resp = ServerMessage::Attached {
                v: PROTOCOL_VERSION,
                id: request_id,
                session_id: a.session_id,
                active_window: a.active_window,
                windows: a.windows,
                panes: a.panes,
                initial_snapshots: Vec::new(),
            };
            send_server_message(stream, &resp).await?;
        }
        Err(e) => {
            send_error_message(
                stream,
                Some(request_id),
                ErrorCode::Internal,
                &e.to_string(),
                true,
            )
            .await;
        }
    }
    Ok(())
}

/// `Attach` 분기.
async fn handle_attach<S>(
    stream: &mut BufStream<S>,
    registry: &SharedRegistry,
    attach: &mut Option<AttachGuard>,
    request_id: MessageId,
    target: winmux_protocol::AttachTarget,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let result = lock_registry(registry).attach(&target);
    match result {
        Ok(a) => {
            // 이전 어태치는 새 guard를 대입하기 전에 Drop된다.
            *attach = Some(AttachGuard {
                registry: registry.clone(),
                session_id: a.session_id.clone(),
            });
            let resp = ServerMessage::Attached {
                v: PROTOCOL_VERSION,
                id: request_id,
                session_id: a.session_id,
                active_window: a.active_window,
                windows: a.windows,
                panes: a.panes,
                initial_snapshots: Vec::new(),
            };
            send_server_message(stream, &resp).await?;
        }
        Err(RegistryError::SessionNotFound(_)) => {
            send_error_message(
                stream,
                Some(request_id),
                ErrorCode::SessionNotFound,
                "session not found",
                true,
            )
            .await;
        }
        Err(e) => {
            send_error_message(
                stream,
                Some(request_id),
                ErrorCode::Internal,
                &e.to_string(),
                true,
            )
            .await;
        }
    }
    Ok(())
}

/// 메시지의 `id` 필드를 추출한다. 스트리밍 메시지(`Bye`, `PtyInput`)는 `None`.
fn client_message_id(msg: &ClientMessage) -> Option<MessageId> {
    match msg {
        ClientMessage::Hello { id, .. }
        | ClientMessage::Ping { id, .. }
        | ClientMessage::ListSessions { id, .. }
        | ClientMessage::NewSession { id, .. }
        | ClientMessage::Attach { id, .. }
        | ClientMessage::Detach { id, .. }
        | ClientMessage::KillSession { id, .. }
        | ClientMessage::NewWindow { id, .. }
        | ClientMessage::SplitPane { id, .. }
        | ClientMessage::KillPane { id, .. }
        | ClientMessage::KillWindow { id, .. }
        | ClientMessage::Resize { id, .. }
        | ClientMessage::SelectPane { id, .. }
        | ClientMessage::SelectWindow { id, .. }
        | ClientMessage::Command { id, .. } => Some(id.clone()),
        ClientMessage::Bye { .. } | ClientMessage::PtyInput { .. } => None,
    }
}

fn message_type_name(msg: &ClientMessage) -> &'static str {
    match msg {
        ClientMessage::Hello { .. } => "Hello",
        ClientMessage::Ping { .. } => "Ping",
        ClientMessage::Bye { .. } => "Bye",
        ClientMessage::ListSessions { .. } => "ListSessions",
        ClientMessage::NewSession { .. } => "NewSession",
        ClientMessage::Attach { .. } => "Attach",
        ClientMessage::Detach { .. } => "Detach",
        ClientMessage::KillSession { .. } => "KillSession",
        ClientMessage::NewWindow { .. } => "NewWindow",
        ClientMessage::SplitPane { .. } => "SplitPane",
        ClientMessage::KillPane { .. } => "KillPane",
        ClientMessage::KillWindow { .. } => "KillWindow",
        ClientMessage::Resize { .. } => "Resize",
        ClientMessage::SelectPane { .. } => "SelectPane",
        ClientMessage::SelectWindow { .. } => "SelectWindow",
        ClientMessage::Command { .. } => "Command",
        ClientMessage::PtyInput { .. } => "PtyInput",
    }
}
