//! Hello/HelloAck 핸드셰이크.
//!
//! 연결 직후 클라이언트는 반드시 `Hello`를 첫 메시지로 보내야 한다
//! (`docs/spec/01-ipc-protocol.md` § State Machine). 그렇지 않으면
//! `Error { code: PROTOCOL_VIOLATION, recoverable: false }`을 보내고
//! 연결을 끊는다. 클라이언트의 `v`가 호환 범위 밖이면 `VERSION_MISMATCH`로
//! 끊는다.
//!
//! 성공하면 [`AuthenticatedClient`]를 돌려준다. 후속 메시지 dispatch는
//! 후속 작업에서 다룬다.
//!
//! 본 모듈은 어떤 PTY 콘텐츠도 로그에 남기지 않는다 (CLAUDE.md Rule 1).

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufStream};
use tracing::{debug, info};
use winmux_protocol::{
    ClientKind, ClientMessage, ErrorCode, ErrorPayload, MessageId, PROTOCOL_VERSION, ServerMessage,
    decode_line, encode_line, is_compatible, version,
};

/// 핸드셰이크가 성공한 직후 클라이언트의 식별 정보.
#[derive(Clone, Debug)]
pub struct AuthenticatedClient {
    /// 클라이언트가 자기소개한 종류.
    pub client_kind: ClientKind,
    /// 클라이언트의 OS PID.
    pub client_pid: u32,
    /// 클라이언트가 자기소개한 빌드 버전.
    pub client_version: String,
    /// 검증된 사용자 이름.
    pub username: String,
}

/// 단일 연결에 대한 핸드셰이크 수행.
///
/// `stream`은 한 클라이언트의 양방향 byte-mode 스트림. 함수가 정상 종료하면
/// 스트림은 `Active` 상태로 진입한 것이며, `Err`이면 핸드셰이크가 실패했고
/// 응답이 시도되었다(또는 EOF로 끊겼다).
pub async fn perform_handshake<S>(
    stream: &mut BufStream<S>,
    username: &str,
) -> Result<AuthenticatedClient>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let server_version = env!("CARGO_PKG_VERSION").to_owned();

    let line = match read_one_line(stream).await? {
        Some(b) => b,
        None => anyhow::bail!("client closed before Hello"),
    };

    let body = match decode_line(&line) {
        Ok(b) => b,
        Err(e) => {
            send_error(
                stream,
                None,
                ErrorCode::ProtocolViolation,
                &format!("invalid first frame: {e}"),
            )
            .await;
            anyhow::bail!("framing error on first message: {e}");
        }
    };

    let msg: ClientMessage = match serde_json::from_str(body) {
        Ok(m) => m,
        Err(e) => {
            send_error(
                stream,
                None,
                ErrorCode::ProtocolViolation,
                &format!("invalid Hello payload: {e}"),
            )
            .await;
            anyhow::bail!("deserialize Hello: {e}");
        }
    };

    let (v, id, client_kind, client_pid, client_version) = match msg {
        ClientMessage::Hello {
            v,
            id,
            client,
            pid,
            version: cver,
        } => (v, id, client, pid, cver),
        other => {
            let type_name = variant_name(&other);
            send_error(
                stream,
                None,
                ErrorCode::ProtocolViolation,
                "Hello required before any other message",
            )
            .await;
            anyhow::bail!("client sent {type_name} in Greeting state");
        }
    };

    if !is_compatible(v) {
        send_error(
            stream,
            Some(id),
            ErrorCode::VersionMismatch,
            &format!(
                "client protocol v{v}, server accepts v{}..=v{}",
                version::MIN_COMPATIBLE_VERSION,
                version::MAX_COMPATIBLE_VERSION,
            ),
        )
        .await;
        anyhow::bail!("client protocol version mismatch v{v}");
    }

    let ack = ServerMessage::HelloAck {
        v: PROTOCOL_VERSION,
        id,
        server_version: server_version.clone(),
        user: username.to_owned(),
    };
    send_server_message(stream, &ack).await?;

    info!(
        client = ?client_kind,
        client_pid,
        client_version = %client_version,
        "client.authenticated"
    );
    debug!(server_version = %server_version, "client.handshake.ack_sent");

    Ok(AuthenticatedClient {
        client_kind,
        client_pid,
        client_version,
        username: username.to_owned(),
    })
}

async fn read_one_line<S>(stream: &mut BufStream<S>) -> Result<Option<Vec<u8>>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = Vec::with_capacity(512);
    let n = stream
        .read_until(b'\n', &mut buf)
        .await
        .context("read line")?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(buf))
}

/// 한 [`ServerMessage`]를 JSON Lines 한 줄로 보낸다. 같은 crate 안의
/// 다른 모듈(예: `pipe`의 dispatcher)이 재사용할 수 있도록 `pub(crate)`.
pub(crate) async fn send_server_message<S>(
    stream: &mut BufStream<S>,
    msg: &ServerMessage,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let json = serde_json::to_vec(msg).context("serialize ServerMessage")?;
    let line = encode_line(&json).context("encode line")?;
    stream.write_all(&line).await.context("write line")?;
    stream.flush().await.context("flush line")?;
    Ok(())
}

/// Best-effort `Error` 메시지 송신. 클라이언트가 이미 끊었으면 결과 무시.
///
/// `recoverable = false`면 호출자는 곧 연결을 끊어야 한다.
pub(crate) async fn send_error_message<S>(
    stream: &mut BufStream<S>,
    id: Option<MessageId>,
    code: ErrorCode,
    message: &str,
    recoverable: bool,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let err = ServerMessage::Error {
        v: PROTOCOL_VERSION,
        payload: ErrorPayload {
            id,
            code,
            message: message.to_owned(),
            recoverable,
        },
    };
    let _ = send_server_message(stream, &err).await;
}

/// `recoverable = false`로 Error를 보내는 핸드셰이크 전용 단축형.
async fn send_error<S>(
    stream: &mut BufStream<S>,
    id: Option<MessageId>,
    code: ErrorCode,
    message: &str,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    send_error_message(stream, id, code, message, false).await;
}

fn variant_name(msg: &ClientMessage) -> &'static str {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    #[tokio::test]
    async fn rejects_non_hello_first_message() {
        // 클라이언트는 `Bye`를 가장 먼저 보낸다 — PROTOCOL_VIOLATION 기대.
        let (server_io, mut client_io) = duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            let mut stream = BufStream::new(server_io);
            let r = perform_handshake(&mut stream, "tester").await;
            assert!(r.is_err());
        });

        // Bye 메시지 전송.
        let bye = ClientMessage::Bye {
            v: PROTOCOL_VERSION,
        };
        let json = serde_json::to_vec(&bye).expect("ser");
        let line = encode_line(&json).expect("enc");
        client_io.write_all(&line).await.expect("write");
        client_io.flush().await.expect("flush");

        // Error 응답 한 줄을 받는다.
        let mut buf = Vec::new();
        let _ = tokio::io::AsyncBufReadExt::read_until(
            &mut tokio::io::BufReader::new(&mut client_io),
            b'\n',
            &mut buf,
        )
        .await
        .expect("read");
        assert!(!buf.is_empty(), "server must respond with Error");
        let body = decode_line(&buf).expect("decode");
        let resp: ServerMessage = serde_json::from_str(body).expect("parse");
        match resp {
            ServerMessage::Error { payload, .. } => {
                assert_eq!(payload.code, ErrorCode::ProtocolViolation);
                assert!(!payload.recoverable);
            }
            other => panic!("expected Error, got {other:?}"),
        }

        // 서버 측 종료 대기.
        server_task.await.expect("join");
    }

    #[tokio::test]
    async fn rejects_incompatible_version() {
        let (server_io, mut client_io) = duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            let mut stream = BufStream::new(server_io);
            let r = perform_handshake(&mut stream, "tester").await;
            assert!(r.is_err());
        });

        let hello = ClientMessage::Hello {
            v: PROTOCOL_VERSION + 100,
            id: MessageId::from_body("01HK").expect("id"),
            client: ClientKind::Cli,
            pid: 1,
            version: "0.0.0".into(),
        };
        let json = serde_json::to_vec(&hello).expect("ser");
        let line = encode_line(&json).expect("enc");
        client_io.write_all(&line).await.expect("write");
        client_io.flush().await.expect("flush");

        let mut all = Vec::new();
        client_io.read_to_end(&mut all).await.expect("read_to_end");
        let body = decode_line(&all).expect("decode");
        let resp: ServerMessage = serde_json::from_str(body).expect("parse");
        match resp {
            ServerMessage::Error { payload, .. } => {
                assert_eq!(payload.code, ErrorCode::VersionMismatch);
                assert!(!payload.recoverable);
            }
            other => panic!("expected Error, got {other:?}"),
        }

        server_task.await.expect("join");
    }

    #[tokio::test]
    async fn accepts_valid_hello_and_sends_ack() {
        let (server_io, mut client_io) = duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            let mut stream = BufStream::new(server_io);
            perform_handshake(&mut stream, "tester")
                .await
                .expect("handshake ok")
        });

        let hello = ClientMessage::Hello {
            v: PROTOCOL_VERSION,
            id: MessageId::from_body("01HKABCDE").expect("id"),
            client: ClientKind::Tray,
            pid: 42,
            version: "0.1.0".into(),
        };
        let json = serde_json::to_vec(&hello).expect("ser");
        let line = encode_line(&json).expect("enc");
        client_io.write_all(&line).await.expect("write");
        client_io.flush().await.expect("flush");

        let mut buf = Vec::new();
        let _ = tokio::io::AsyncBufReadExt::read_until(
            &mut tokio::io::BufReader::new(&mut client_io),
            b'\n',
            &mut buf,
        )
        .await
        .expect("read");
        let body = decode_line(&buf).expect("decode");
        let resp: ServerMessage = serde_json::from_str(body).expect("parse");
        match resp {
            ServerMessage::HelloAck {
                user,
                server_version,
                ..
            } => {
                assert_eq!(user, "tester");
                assert!(!server_version.is_empty());
            }
            other => panic!("expected HelloAck, got {other:?}"),
        }

        let auth = server_task.await.expect("join");
        assert_eq!(auth.username, "tester");
        assert_eq!(auth.client_kind, ClientKind::Tray);
        assert_eq!(auth.client_pid, 42);
    }
}
