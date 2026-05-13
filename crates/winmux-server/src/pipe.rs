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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufStream};
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use tokio::sync::{Mutex as TokioMutex, broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use winmux_protocol::{
    AttachTarget, ClientMessage, CommandRequest, CommandResultPayload, ErrorCode, EventMessage,
    MessageId, NewSessionRequest, PROTOCOL_VERSION, PaneId, PaneSize, PaneSnapshot, ServerMessage,
    SessionId, UserIdentity, WindowId, codec, decode_line,
};

use crate::jobobj::JobObject;
use crate::keys::{KeyError, tokens_to_bytes};
use crate::pipe::handshake::{perform_handshake, send_error_message, send_server_message};
use crate::pipe::security::verify_client_user;
use crate::pty::{Pty, PtyEvent, SpawnRequest};
use crate::session::{
    PaneRuntime, Registry, RegistryError, SharedRegistry, VtermFeederHandle, new_pane_id,
    new_session_id, new_window_id,
};
use crate::terminal::VirtualTerm;

/// 단일 파이프 인스턴스당 커널 in/out 버퍼 hint(바이트).
const PIPE_IN_BUF: u32 = 64 * 1024;
const PIPE_OUT_BUF: u32 = 64 * 1024;

/// 최대 동시 인스턴스 수. tokio의 ServerOptions는 254까지만 허용한다
/// (Win32의 `PIPE_UNLIMITED_INSTANCES=255`는 매직 값으로 별도 처리됨).
const PIPE_MAX_INSTANCES: usize = 254;

/// 새 패널의 기본 셀 크기. 첫 Attach에서 클라이언트가 자기 크기로 `Resize`를
/// 보낼 때까지 임시로 쓴다.
const DEFAULT_PANE_ROWS: u16 = 40;
const DEFAULT_PANE_COLS: u16 = 120;

/// 한 클라이언트가 받을 수 있는 outgoing 메시지 큐. PtyOutput · push 이벤트 ·
/// 응답이 모두 이 채널을 통한다.
const CLIENT_OUTBOX_CAPACITY: usize = 256;

/// accept 루프 진입점.
///
/// 한 번에 하나씩 새 파이프 인스턴스를 만들고, 한 클라이언트를 받으면
/// 핸들러 task로 분리한 뒤 다음 인스턴스를 만든다. `shutdown_rx`가
/// 신호를 받으면 즉시 종료한다 (반쯤 만들어진 인스턴스는 Drop으로 정리됨).
pub async fn run(
    identity: UserIdentity,
    job: Arc<JobObject>,
    mut shutdown_rx: oneshot::Receiver<()>,
) -> Result<()> {
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
                let job = job.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(server, username, registry, job).await {
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

/// `attached_clients` 카운트와 broadcast forwarder들의 lifecycle을 함께 묶는
/// RAII 가드.
///
/// 한 클라이언트 task가 attach한 세션을 추적해서, 명시적 `Detach`·`Bye`·
/// 비정상 disconnect 어느 경로로 끝나든 (1) [`Registry::detach`]가 정확히
/// 한 번 호출되고 (2) 그 세션의 PTY broadcast forwarder task들이 abort된다.
struct AttachGuard {
    registry: SharedRegistry,
    session_id: SessionId,
    forwarders: Vec<JoinHandle<()>>,
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        let mut reg = lock_registry(&self.registry);
        reg.detach(&self.session_id);
        for h in &self.forwarders {
            h.abort();
        }
    }
}

/// poison 가드. mutex가 panic 후에도 안전하게 데이터를 빌릴 수 있다.
fn lock_registry(reg: &SharedRegistry) -> std::sync::MutexGuard<'_, Registry> {
    reg.lock().unwrap_or_else(|e| e.into_inner())
}

/// dispatcher와 broadcast forwarder들이 공유하는 상수 컨텍스트.
struct ClientContext {
    registry: SharedRegistry,
    job: Arc<JobObject>,
    forward_tx: mpsc::Sender<ServerMessage>,
}

/// 클라이언트 핸들러: 핸드셰이크 → 메시지 dispatch + PtyOutput/이벤트 forward.
async fn handle_client(
    server: NamedPipeServer,
    username: String,
    registry: SharedRegistry,
    job: Arc<JobObject>,
) -> Result<()> {
    let mut stream = BufStream::new(server);
    let auth = perform_handshake(&mut stream, &username).await?;

    info!(
        client = ?auth.client_kind,
        client_pid = auth.client_pid,
        client_version = %auth.client_version,
        "client.session.start"
    );

    let (forward_tx, mut forward_rx) = mpsc::channel::<ServerMessage>(CLIENT_OUTBOX_CAPACITY);
    let ctx = ClientContext {
        registry,
        job,
        forward_tx,
    };

    let mut attach: Option<AttachGuard> = None;
    let mut buf = Vec::with_capacity(512);

    loop {
        buf.clear();
        // `read_until` cancel-safety: 부분 누적 데이터는 buf에 남고 다음 iteration에서
        // 이어서 읽는다. `forward_rx.recv()` 분기는 stream을 빌리지 않으므로 충돌 없다.
        tokio::select! {
            biased;
            Some(out_msg) = forward_rx.recv() => {
                send_server_message(&mut stream, &out_msg).await?;
                continue;
            }
            read_res = stream.read_until(b'\n', &mut buf) => {
                let n = read_res.context("read")?;
                if n == 0 {
                    debug!("client.eof");
                    break;
                }
            }
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
        match dispatch_message(&mut stream, &ctx, &mut attach, msg).await? {
            DispatchOutcome::Continue => {}
            DispatchOutcome::Disconnect => break,
        }
    }

    // attach Drop이 detach 카운트와 forwarder 정리를 모두 처리한다.
    info!(client = ?auth.client_kind, "client.session.end");
    Ok(())
}

/// 핸드셰이크 이후 들어온 한 메시지를 처리한다.
async fn dispatch_message<S>(
    stream: &mut BufStream<S>,
    ctx: &ClientContext,
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
            let sessions = lock_registry(&ctx.registry).list_summaries();
            let resp = ServerMessage::SessionList {
                v: PROTOCOL_VERSION,
                id,
                sessions,
            };
            send_server_message(stream, &resp).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::NewSession { id, request, .. } => {
            handle_new_session(stream, ctx, attach, id, request).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::Attach { id, session, .. } => {
            handle_attach(stream, ctx, attach, id, session).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::Detach { id, .. } => {
            *attach = None;
            let resp = ServerMessage::Ok {
                v: PROTOCOL_VERSION,
                id,
            };
            send_server_message(stream, &resp).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::KillSession { id, session, .. } => {
            let result = lock_registry(&ctx.registry).kill_session(&session);
            match result {
                Ok(killed_id) => {
                    info!(session = %killed_id, "session.killed");
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
        ClientMessage::PtyInput {
            pane_id,
            bytes_base64,
            ..
        } => {
            handle_pty_input(&ctx.registry, &pane_id, &bytes_base64).await;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::Resize {
            id,
            pane_id,
            rows,
            cols,
            ..
        } => {
            handle_resize(stream, &ctx.registry, id, pane_id, rows, cols).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::KillPane { id, pane_id, .. } => {
            handle_kill_pane(stream, ctx, attach, id, pane_id).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::KillWindow { id, window_id, .. } => {
            handle_kill_window(stream, ctx, attach, id, window_id).await?;
            Ok(DispatchOutcome::Continue)
        }
        ClientMessage::Command { id, request, .. } => {
            handle_command(stream, ctx, attach, id, request).await?;
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

/// `NewSession` 분기. 실제 셸을 spawn해 PaneRuntime을 만든 뒤 메타데이터를
/// 등록한다. detached가 아니면 본 클라이언트를 그 세션에 자동 어태치한다.
async fn handle_new_session<S>(
    stream: &mut BufStream<S>,
    ctx: &ClientContext,
    attach: &mut Option<AttachGuard>,
    request_id: MessageId,
    request: NewSessionRequest,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let detached = request.detached;

    // 1) ID를 미리 만들어 spawn 호출 결과와 묶을 준비.
    let session_id = new_session_id();
    let window_id = new_window_id();
    let pane_id = new_pane_id();

    // 2) 셸·환경 결정.
    let shell = resolve_shell(&request);
    let cwd = request
        .cwd
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let env = build_env(&request, &session_id);

    // 3) Pty::spawn — 실패 시 메타데이터 등록조차 시도하지 않는다.
    let spawn_req = SpawnRequest {
        shell,
        args: Vec::new(),
        cwd,
        env,
        rows: DEFAULT_PANE_ROWS,
        cols: DEFAULT_PANE_COLS,
    };
    let pty = match Pty::spawn(spawn_req, Some(&ctx.job)) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            warn!(error = ?e, "session.spawn.failed");
            send_error_message(
                stream,
                Some(request_id),
                ErrorCode::Internal,
                &format!("spawn failed: {e}"),
                true,
            )
            .await;
            return Ok(());
        }
    };

    // 가상 터미널 + server-side feeder. feeder는 PTY broadcast를 구독해
    // vterm을 누적 갱신한다 — reattach 시 snapshot()이 정확하도록.
    let vterm = Arc::new(TokioMutex::new(VirtualTerm::new(
        DEFAULT_PANE_ROWS,
        DEFAULT_PANE_COLS,
    )));
    let feeder = spawn_vterm_feeder(pane_id.clone(), pty.subscribe(), vterm.clone());

    let runtime = PaneRuntime {
        pane_id: pane_id.clone(),
        window_id: window_id.clone(),
        session_id: session_id.clone(),
        pty,
        vterm,
        _vterm_feeder: VtermFeederHandle::new(feeder),
    };

    // 4) Registry 등록 — 실패 시 runtime drop → Pty drop → 자식 kill로 정리.
    let create_result = lock_registry(&ctx.registry).create_session_with_runtime(
        &request,
        PaneSize {
            rows: DEFAULT_PANE_ROWS,
            cols: DEFAULT_PANE_COLS,
        },
        runtime,
    );
    let created = match create_result {
        Ok((sid, _pid)) => sid,
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
    info!(session = %created, detached, "session.created");

    if detached {
        let resp = ServerMessage::Ok {
            v: PROTOCOL_VERSION,
            id: request_id,
        };
        send_server_message(stream, &resp).await?;
        return Ok(());
    }

    // 5) 자동 어태치 + broadcast forwarder.
    let attach_result = lock_registry(&ctx.registry).attach(&AttachTarget::Id {
        id: created.clone(),
    });
    match attach_result {
        Ok(a) => {
            let forwarders = spawn_forwarders(&ctx.registry, &a.session_id, &ctx.forward_tx);
            *attach = Some(AttachGuard {
                registry: ctx.registry.clone(),
                session_id: a.session_id.clone(),
                forwarders,
            });
            let initial_snapshots = collect_initial_snapshots(&ctx.registry, &a.session_id).await;
            let resp = ServerMessage::Attached {
                v: PROTOCOL_VERSION,
                id: request_id,
                session_id: a.session_id,
                active_window: a.active_window,
                windows: a.windows,
                panes: a.panes,
                initial_snapshots,
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
    ctx: &ClientContext,
    attach: &mut Option<AttachGuard>,
    request_id: MessageId,
    target: AttachTarget,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let result = lock_registry(&ctx.registry).attach(&target);
    match result {
        Ok(a) => {
            // 이전 attach guard를 먼저 drop해 그쪽 forwarder들이 abort되도록 한다.
            *attach = None;
            let forwarders = spawn_forwarders(&ctx.registry, &a.session_id, &ctx.forward_tx);
            *attach = Some(AttachGuard {
                registry: ctx.registry.clone(),
                session_id: a.session_id.clone(),
                forwarders,
            });
            let initial_snapshots = collect_initial_snapshots(&ctx.registry, &a.session_id).await;
            let resp = ServerMessage::Attached {
                v: PROTOCOL_VERSION,
                id: request_id,
                session_id: a.session_id,
                active_window: a.active_window,
                windows: a.windows,
                panes: a.panes,
                initial_snapshots,
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

/// `PtyInput` 분기 — 응답 없는 streaming. 알 수 없는 패널이면 조용히 무시.
async fn handle_pty_input(registry: &SharedRegistry, pane_id: &PaneId, bytes_base64: &str) {
    let bytes = match codec::base64_decode(bytes_base64) {
        Ok(b) => b,
        Err(e) => {
            debug!(error = %e, %pane_id, "client.pty_input.invalid_base64");
            return;
        }
    };
    let pty = lock_registry(registry).pty_for_pane(pane_id);
    if let Some(pty) = pty {
        if let Err(e) = pty.write_input(bytes).await {
            warn!(error = %e, %pane_id, "pty.input.write_failed");
        }
    } else {
        debug!(%pane_id, "pty_input.unknown_pane");
    }
}

/// `Resize` 분기 — master.resize + 메타 size 갱신.
async fn handle_resize<S>(
    stream: &mut BufStream<S>,
    registry: &SharedRegistry,
    request_id: MessageId,
    pane_id: PaneId,
    rows: u16,
    cols: u16,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let pty_and_vterm = {
        let reg = lock_registry(registry);
        reg.pty_for_pane(&pane_id)
            .map(|pty| (pty, reg.vterm_for_pane(&pane_id)))
    };
    let Some((pty, vterm)) = pty_and_vterm else {
        send_error_message(
            stream,
            Some(request_id),
            ErrorCode::PaneNotFound,
            "pane not found",
            true,
        )
        .await;
        return Ok(());
    };
    match pty.try_resize(rows, cols) {
        Ok(()) => {
            lock_registry(registry).update_pane_size(&pane_id, PaneSize { rows, cols });
            // ConPTY와 가상 터미널은 항상 같은 크기여야 한다 — 그렇지 않으면
            // 다음 snapshot이 ConPTY의 wrap과 어긋난다.
            if let Some(vt) = vterm {
                vt.lock().await.resize(rows, cols);
            }
            let resp = ServerMessage::Ok {
                v: PROTOCOL_VERSION,
                id: request_id,
            };
            send_server_message(stream, &resp).await?;
        }
        Err(e) => {
            warn!(error = %e, %pane_id, "pty.resize.failed");
            send_error_message(
                stream,
                Some(request_id),
                ErrorCode::Internal,
                &format!("resize failed: {e}"),
                true,
            )
            .await;
        }
    }
    Ok(())
}

/// 한 세션의 모든 패널에 대해 `PtyEvent → ServerMessage` 변환 task를 띄운다.
fn spawn_forwarders(
    registry: &SharedRegistry,
    session_id: &SessionId,
    forward_tx: &mpsc::Sender<ServerMessage>,
) -> Vec<JoinHandle<()>> {
    let ptys = lock_registry(registry).ptys_for_session(session_id);
    let mut handles = Vec::with_capacity(ptys.len());
    for (pane_id, pty) in ptys {
        let rx = pty.subscribe();
        let tx = forward_tx.clone();
        handles.push(tokio::spawn(async move {
            forward_pane_events(pane_id, rx, tx).await;
        }));
    }
    handles
}

/// `KillPane` 분기. 패널을 정리하고 cascade로 사라진 윈도우/세션이 있으면
/// attach guard도 함께 푼다.
async fn handle_kill_pane<S>(
    stream: &mut BufStream<S>,
    ctx: &ClientContext,
    attach: &mut Option<AttachGuard>,
    request_id: MessageId,
    pane_id: PaneId,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let outcome = lock_registry(&ctx.registry).kill_pane(&pane_id);
    match outcome {
        Ok(o) => {
            info!(pane = %pane_id, session_removed = o.session_removed, "pane.killed");
            // 현재 클라이언트가 그 세션에 어태치 중이고 세션이 사라졌다면
            // attach guard를 풀어 forwarder도 함께 abort되도록 한다.
            if o.session_removed
                && attach
                    .as_ref()
                    .is_some_and(|g| g.session_id == o.session_id)
            {
                *attach = None;
            }
            let resp = ServerMessage::Ok {
                v: PROTOCOL_VERSION,
                id: request_id,
            };
            send_server_message(stream, &resp).await?;
        }
        Err(RegistryError::PaneNotFound(_)) => {
            send_error_message(
                stream,
                Some(request_id),
                ErrorCode::PaneNotFound,
                "pane not found",
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

/// `KillWindow` 분기. 윈도우 안의 모든 패널을 정리한다.
async fn handle_kill_window<S>(
    stream: &mut BufStream<S>,
    ctx: &ClientContext,
    attach: &mut Option<AttachGuard>,
    request_id: MessageId,
    window_id: WindowId,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let outcome = lock_registry(&ctx.registry).kill_window(&window_id);
    match outcome {
        Ok(o) => {
            info!(window = %window_id, session_removed = o.session_removed, "window.killed");
            if o.session_removed
                && attach
                    .as_ref()
                    .is_some_and(|g| g.session_id == o.session_id)
            {
                *attach = None;
            }
            let resp = ServerMessage::Ok {
                v: PROTOCOL_VERSION,
                id: request_id,
            };
            send_server_message(stream, &resp).await?;
        }
        Err(RegistryError::WindowNotFound(_)) => {
            send_error_message(
                stream,
                Some(request_id),
                ErrorCode::WindowNotFound,
                "window not found",
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

/// `Command` 분기 (현재는 `send-keys`만 지원). 다른 tmux 명령은 미구현으로
/// `CommandResult { ok: false, ... }`를 돌려준다.
async fn handle_command<S>(
    stream: &mut BufStream<S>,
    ctx: &ClientContext,
    attach: &Option<AttachGuard>,
    request_id: MessageId,
    request: CommandRequest,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let tmux = request.tmux.as_str();
    match tmux {
        "send-keys" => handle_send_keys(stream, ctx, attach, request_id, &request.args).await,
        other => {
            let resp = ServerMessage::CommandResult {
                v: PROTOCOL_VERSION,
                id: request_id,
                result: CommandResultPayload {
                    ok: false,
                    stdout: None,
                    stderr: Some(format!("unsupported tmux command: {other}")),
                },
            };
            send_server_message(stream, &resp).await?;
            Ok(())
        }
    }
}

/// `send-keys` 분기: target 결정 → 키 변환 → PTY write → `CommandResult`.
async fn handle_send_keys<S>(
    stream: &mut BufStream<S>,
    ctx: &ClientContext,
    attach: &Option<AttachGuard>,
    request_id: MessageId,
    args: &[String],
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (target, key_tokens) = match parse_send_keys_args(args) {
        Ok(p) => p,
        Err(msg) => {
            let resp = ServerMessage::CommandResult {
                v: PROTOCOL_VERSION,
                id: request_id,
                result: CommandResultPayload {
                    ok: false,
                    stdout: None,
                    stderr: Some(msg),
                },
            };
            send_server_message(stream, &resp).await?;
            return Ok(());
        }
    };

    // 키 변환은 PTY를 잡기 전에 — 키 이름이 잘못된 경우에도 PTY는 건드리지
    // 않는다.
    let bytes = match tokens_to_bytes(&key_tokens) {
        Ok(b) => b,
        Err(KeyError::Unsupported(name)) => {
            let resp = ServerMessage::CommandResult {
                v: PROTOCOL_VERSION,
                id: request_id,
                result: CommandResultPayload {
                    ok: false,
                    stdout: None,
                    stderr: Some(format!(
                        "unsupported key in M0 PoC: `{name}` (see docs/spec/04-key-handling.md)"
                    )),
                },
            };
            send_server_message(stream, &resp).await?;
            return Ok(());
        }
    };

    // target → pane_id 해상도.
    let pane_id = match resolve_send_keys_target(&ctx.registry, attach, target.as_deref()) {
        Ok(p) => p,
        Err(msg) => {
            let resp = ServerMessage::CommandResult {
                v: PROTOCOL_VERSION,
                id: request_id,
                result: CommandResultPayload {
                    ok: false,
                    stdout: None,
                    stderr: Some(msg),
                },
            };
            send_server_message(stream, &resp).await?;
            return Ok(());
        }
    };

    let pty = lock_registry(&ctx.registry).pty_for_pane(&pane_id);
    let Some(pty) = pty else {
        let resp = ServerMessage::CommandResult {
            v: PROTOCOL_VERSION,
            id: request_id,
            result: CommandResultPayload {
                ok: false,
                stdout: None,
                stderr: Some("pane has no live PTY".to_owned()),
            },
        };
        send_server_message(stream, &resp).await?;
        return Ok(());
    };

    if let Err(e) = pty.write_input(bytes).await {
        warn!(error = %e, %pane_id, "send_keys.write_failed");
        let resp = ServerMessage::CommandResult {
            v: PROTOCOL_VERSION,
            id: request_id,
            result: CommandResultPayload {
                ok: false,
                stdout: None,
                stderr: Some(format!("write failed: {e}")),
            },
        };
        send_server_message(stream, &resp).await?;
        return Ok(());
    }

    let resp = ServerMessage::CommandResult {
        v: PROTOCOL_VERSION,
        id: request_id,
        result: CommandResultPayload {
            ok: true,
            stdout: None,
            stderr: None,
        },
    };
    send_server_message(stream, &resp).await?;
    Ok(())
}

/// `send-keys`의 args를 `(target, key_tokens)`로 가른다. `-t TARGET`이
/// 있으면 첫 두 토큰을 소비한다. 그 외 플래그(M0에서는 지원하지 않음)는
/// 에러로 본다 — 사용자가 별 의도로 넣은 입력을 키로 흘리지 않기 위해.
fn parse_send_keys_args(args: &[String]) -> Result<(Option<String>, Vec<String>), String> {
    let mut iter = args.iter().peekable();
    let mut target: Option<String> = None;
    while let Some(arg) = iter.peek() {
        if arg.as_str() == "-t" {
            iter.next();
            let value = iter
                .next()
                .ok_or_else(|| "send-keys: -t requires a target argument".to_owned())?;
            target = Some(value.clone());
        } else if arg.starts_with('-') && arg.len() > 1 {
            return Err(format!("send-keys: unknown flag `{arg}` in M0 PoC"));
        } else {
            break;
        }
    }
    let keys: Vec<String> = iter.cloned().collect();
    if keys.is_empty() {
        return Err("send-keys: no key tokens given".to_owned());
    }
    Ok((target, keys))
}

/// target 문자열을 PaneId로 해상한다. M0 PoC에서 지원하는 형태:
///
/// - `None` → 현재 attach 중인 세션의 active pane.
/// - `pane-XYZ` → 그대로.
/// - `ses-XYZ` → 그 세션의 active pane.
/// - 이름 (`work`, `work:0`, `work:0.0`) → `work` 부분만 보고 그 세션의
///   active pane. `:windowindex.paneindex`는 M0에서는 무시한다 (한 세션-한
///   윈도우-한 패널 가정 — 본 PoC 범위 내).
fn resolve_send_keys_target(
    registry: &SharedRegistry,
    attach: &Option<AttachGuard>,
    target: Option<&str>,
) -> Result<PaneId, String> {
    let reg = lock_registry(registry);
    match target {
        None => {
            let session_id = attach
                .as_ref()
                .map(|g| g.session_id.clone())
                .ok_or_else(|| "send-keys: no target and client is not attached".to_owned())?;
            reg.active_pane_of_session(&session_id)
                .ok_or_else(|| format!("send-keys: session has no active pane: {session_id}"))
        }
        Some(t) => {
            // pane-XYZ 형태인지 가장 먼저 확인.
            if t.starts_with("pane-") {
                let pid = PaneId::from_raw(t.to_owned());
                if reg.pty_for_pane(&pid).is_some() {
                    return Ok(pid);
                }
                return Err(format!("send-keys: pane not found: {t}"));
            }
            // ses-XYZ 형태.
            if t.starts_with("ses-") {
                let sid = SessionId::from_raw(t.to_owned());
                return reg
                    .active_pane_of_session(&sid)
                    .ok_or_else(|| format!("send-keys: session not found: {t}"));
            }
            // 이름. `name`, `name:0`, `name:0.0` 모두 `name`만 본다.
            let name = t.split(':').next().unwrap_or(t);
            let sid = reg
                .session_id_by_name(name)
                .ok_or_else(|| format!("send-keys: session not found: {name}"))?;
            reg.active_pane_of_session(&sid)
                .ok_or_else(|| format!("send-keys: session has no active pane: {name}"))
        }
    }
}

/// `pty.subscribe()`의 출력을 server-local `VirtualTerm`에 누적해서 reattach
/// 스냅샷이 정확하도록 만든다. 본 task는 `PaneRuntime`이 살아있는 동안만
/// 돌아야 하므로 [`VtermFeederHandle`]에 감싸 RAII로 abort된다.
///
/// CLAUDE.md Rule 1: 본 feeder는 받은 바이트 어떤 것도 로그하지 않는다.
fn spawn_vterm_feeder(
    pane_id: PaneId,
    mut rx: broadcast::Receiver<PtyEvent>,
    vterm: Arc<TokioMutex<VirtualTerm>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(PtyEvent::Output(bytes)) => {
                    let mut vt = vterm.lock().await;
                    vt.feed(&bytes);
                }
                Ok(PtyEvent::Exited { .. }) => break,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(dropped = n, %pane_id, "vterm.broadcast.lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// 어태치 응답에 들어갈 [`PaneSnapshot`] 벡터를 만든다. 각 vterm에서 짧게
/// lock을 잡아 `snapshot()`을 호출하므로 feeder와 직렬화된다.
async fn collect_initial_snapshots(
    registry: &SharedRegistry,
    session_id: &SessionId,
) -> Vec<PaneSnapshot> {
    let vterms = lock_registry(registry).vterms_for_session(session_id);
    let mut out = Vec::with_capacity(vterms.len());
    for (pane_id, vterm) in vterms {
        let bytes = vterm.lock().await.snapshot();
        out.push(PaneSnapshot {
            pane_id,
            bytes_base64: codec::base64_encode(&bytes),
        });
    }
    out
}

async fn forward_pane_events(
    pane_id: PaneId,
    mut rx: broadcast::Receiver<PtyEvent>,
    forward_tx: mpsc::Sender<ServerMessage>,
) {
    loop {
        match rx.recv().await {
            Ok(PtyEvent::Output(bytes)) => {
                let msg = ServerMessage::PtyOutput {
                    v: PROTOCOL_VERSION,
                    pane_id: pane_id.clone(),
                    bytes_base64: codec::base64_encode(&bytes),
                };
                if forward_tx.send(msg).await.is_err() {
                    break;
                }
            }
            Ok(PtyEvent::Exited { code }) => {
                let msg = ServerMessage::Event {
                    v: PROTOCOL_VERSION,
                    event: EventMessage::PaneExited {
                        v: PROTOCOL_VERSION,
                        pane_id: pane_id.clone(),
                        exit_code: code.unwrap_or(-1),
                    },
                };
                let _ = forward_tx.send(msg).await;
                break;
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!(dropped = n, %pane_id, "client.broadcast.lagged");
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

/// 셸 결정: 명시 → `COMSPEC` → `powershell.exe`. spec § Default shell의
/// 우선순위 1(winmux.toml) / 2(.tmux.conf)는 설정 모듈이 들어온 뒤 추가.
fn resolve_shell(req: &NewSessionRequest) -> PathBuf {
    if let Some(s) = &req.shell
        && !s.is_empty()
    {
        return PathBuf::from(s);
    }
    if let Ok(comspec) = std::env::var("COMSPEC")
        && !comspec.is_empty()
    {
        return PathBuf::from(comspec);
    }
    PathBuf::from("powershell.exe")
}

/// spec § Environment의 정적 `WINMUX_*` 변수 + spawn 시 추가 env.
fn build_env(req: &NewSessionRequest, session_id: &SessionId) -> BTreeMap<String, String> {
    let mut env = req.env.clone();
    env.insert(
        "WINMUX_VERSION".to_owned(),
        env!("CARGO_PKG_VERSION").to_owned(),
    );
    env.insert("WINMUX_SESSION_ID".to_owned(), session_id.to_string());
    env.insert("WINMUX_WINDOW_INDEX".to_owned(), "0".to_owned());
    env.insert("WINMUX_PANE_INDEX".to_owned(), "0".to_owned());
    env
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
