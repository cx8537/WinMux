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

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufStream};
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use winmux_protocol::{ClientMessage, decode_line};

use winmux_protocol::UserIdentity;

use crate::pipe::handshake::perform_handshake;
use crate::pipe::security::verify_client_user;

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
                tokio::spawn(async move {
                    if let Err(e) = handle_client(server, username).await {
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

/// 클라이언트 핸들러: 핸드셰이크 → 메시지 수신 (M0 skeleton).
///
/// M0 skeleton 단계에서는 HelloAck 이후 들어오는 메시지를 파싱만 하고
/// `Bye`까지 기다린다. 본격 dispatcher는 후속 작업에서 추가한다.
async fn handle_client(server: NamedPipeServer, username: String) -> Result<()> {
    let mut stream = BufStream::new(server);
    let auth = perform_handshake(&mut stream, &username).await?;

    info!(
        client = ?auth.client_kind,
        client_pid = auth.client_pid,
        client_version = %auth.client_version,
        "client.session.start"
    );

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
                continue;
            }
        };
        match serde_json::from_str::<ClientMessage>(body) {
            Ok(ClientMessage::Bye { .. }) => {
                info!("client.bye");
                break;
            }
            Ok(other) => {
                debug!(message_type = %message_type_name(&other), "client.message.received");
            }
            Err(e) => {
                warn!(error = %e, "client.message.invalid");
            }
        }
    }

    info!(client = ?auth.client_kind, "client.session.end");
    Ok(())
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
