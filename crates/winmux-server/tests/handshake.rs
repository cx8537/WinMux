//! 실제 Named Pipe 위에서 Hello/HelloAck 라운드트립 통합 테스트.
//!
//! 본 테스트는 서버 라이브러리(`winmux-server::pipe::run`)를 직접 띄우고,
//! Tokio의 `NamedPipeClient`로 연결하여 와이어 프로토콜이 끝에서 끝까지
//! 통하는지 검증한다. PTY는 등장하지 않는다 — 핸드셰이크와 와이어 검증만.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::sync::oneshot;
use tokio::time::timeout;
use winmux_protocol::{
    ClientKind, ClientMessage, MessageId, PROTOCOL_VERSION, ServerMessage, decode_line, encode_line,
};
use winmux_server::pipe;
use winmux_server::user::UserIdentity;

#[tokio::test]
async fn hello_helloack_roundtrip_over_real_pipe() -> Result<()> {
    // 테스트마다 고유한 user_sha8을 써서 동시 실행 시 인스턴스 충돌 회피.
    let username = format!("itest-roundtrip-{}", std::process::id());
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let server_task = tokio::spawn(async move { pipe::run(server_identity, shutdown_rx).await });

    let client = open_client_with_retry(&pipe_name).await?;
    let (reader, writer) = tokio::io::split(client);
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    let hello = ClientMessage::Hello {
        v: PROTOCOL_VERSION,
        id: MessageId::from_body("01HKTEST")?,
        client: ClientKind::Cli,
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    let json = serde_json::to_vec(&hello)?;
    let line = encode_line(&json)?;
    writer.write_all(&line).await?;
    writer.flush().await?;

    let mut buf = Vec::new();
    timeout(Duration::from_secs(5), reader.read_until(b'\n', &mut buf))
        .await
        .context("HelloAck timed out")??;
    let body = decode_line(&buf)?;
    let resp: ServerMessage = serde_json::from_str(body)?;
    match resp {
        ServerMessage::HelloAck {
            user,
            server_version,
            v,
            ..
        } => {
            assert_eq!(user, username);
            assert!(!server_version.is_empty());
            assert_eq!(v, PROTOCOL_VERSION);
        }
        other => panic!("expected HelloAck, got {other:?}"),
    }

    // 깨끗한 종료.
    drop(reader);
    drop(writer);
    let _ = shutdown_tx.send(());
    let server_result = timeout(Duration::from_secs(5), server_task)
        .await
        .context("server didn't shut down in time")??;
    server_result?;
    Ok(())
}

#[tokio::test]
async fn non_hello_first_message_is_rejected_over_real_pipe() -> Result<()> {
    let username = format!("itest-violation-{}", std::process::id());
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let server_task = tokio::spawn(async move { pipe::run(server_identity, shutdown_rx).await });

    let client = open_client_with_retry(&pipe_name).await?;
    let (reader, writer) = tokio::io::split(client);
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    // 첫 메시지로 Bye를 보낸다 — PROTOCOL_VIOLATION을 받아야 한다.
    let bye = ClientMessage::Bye {
        v: PROTOCOL_VERSION,
    };
    let json = serde_json::to_vec(&bye)?;
    let line = encode_line(&json)?;
    writer.write_all(&line).await?;
    writer.flush().await?;

    let mut buf = Vec::new();
    timeout(Duration::from_secs(5), reader.read_until(b'\n', &mut buf))
        .await
        .context("Error response timed out")??;
    let body = decode_line(&buf)?;
    let resp: ServerMessage = serde_json::from_str(body)?;
    match resp {
        ServerMessage::Error { payload, .. } => {
            assert_eq!(payload.code, winmux_protocol::ErrorCode::ProtocolViolation);
            assert!(!payload.recoverable);
        }
        other => panic!("expected Error, got {other:?}"),
    }

    drop(reader);
    drop(writer);
    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

/// 서버가 첫 인스턴스를 열기까지 시간이 걸릴 수 있으므로 짧은 백오프로 재시도.
async fn open_client_with_retry(pipe_name: &str) -> Result<NamedPipeClient> {
    let backoffs_ms = [25u64, 50, 100, 200, 400, 800, 1600];
    let mut last_err: Option<std::io::Error> = None;
    for ms in backoffs_ms {
        match ClientOptions::new().open(pipe_name) {
            Ok(c) => return Ok(c),
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
        }
    }
    Err(anyhow::anyhow!(
        "failed to open client pipe {pipe_name}: {:?}",
        last_err
    ))
}
