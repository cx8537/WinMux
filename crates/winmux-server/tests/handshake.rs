//! 실제 Named Pipe 위에서 양쪽(server + ipc-client)을 함께 검증한다.
//!
//! 본 테스트는 `winmux-server`의 `pipe::run`을 직접 띄우고, 같은 코드 베이스의
//! `winmux-ipc-client::Client`로 핸드셰이크를 수행한다. 그래서 server측
//! 핸드셰이크 로직과 client측 와이어 어댑터가 동시에 회귀 검증된다.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::oneshot;
use tokio::time::timeout;
use winmux_ipc_client::{Client, connect_with_retry};
use winmux_protocol::{
    ClientKind, ClientMessage, ErrorCode, PROTOCOL_VERSION, ServerMessage, UserIdentity,
};
use winmux_server::pipe;

/// 새 통합 시나리오마다 PID + 시각 prefix로 user_sha8을 만들어
/// 동시 실행 시 파이프 이름이 충돌하지 않게 한다.
fn unique_username(tag: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("itest-{tag}-{}-{now_ns}", std::process::id())
}

#[tokio::test]
async fn hello_helloack_roundtrip_over_real_pipe() -> Result<()> {
    let username = unique_username("roundtrip");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let server_task = tokio::spawn(async move { pipe::run(server_identity, shutdown_rx).await });

    let pipe = connect_with_retry(&pipe_name)
        .await
        .context("ipc-client connect")?;
    let mut client = Client::new(pipe);
    let ack = client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await
        .context("hello")?;
    assert_eq!(ack.user, username);
    assert_eq!(ack.protocol_version, PROTOCOL_VERSION);
    assert!(!ack.server_version.is_empty());

    client.close().await?;
    let _ = shutdown_tx.send(());
    let server_result = timeout(Duration::from_secs(5), server_task)
        .await
        .context("server didn't shut down in time")??;
    server_result?;
    Ok(())
}

#[tokio::test]
async fn non_hello_first_message_is_rejected_over_real_pipe() -> Result<()> {
    let username = unique_username("violation");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let server_task = tokio::spawn(async move { pipe::run(server_identity, shutdown_rx).await });

    let pipe = connect_with_retry(&pipe_name)
        .await
        .context("ipc-client connect")?;
    let mut client = Client::new(pipe);

    // 첫 메시지로 Bye를 보낸다 — server는 `Hello required first`로 거절해야 한다.
    client
        .send(&ClientMessage::Bye {
            v: PROTOCOL_VERSION,
        })
        .await?;
    let resp = timeout(Duration::from_secs(5), client.recv())
        .await
        .context("Error response timed out")??;
    match resp {
        ServerMessage::Error { payload, .. } => {
            assert_eq!(payload.code, ErrorCode::ProtocolViolation);
            assert!(!payload.recoverable);
        }
        other => panic!("expected Error, got {other:?}"),
    }

    drop(client);
    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn ping_returns_pong_with_same_id() -> Result<()> {
    let username = unique_username("ping");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let server_task = tokio::spawn(async move { pipe::run(server_identity, shutdown_rx).await });

    let pipe = connect_with_retry(&pipe_name).await.context("connect")?;
    let mut client = Client::new(pipe);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;

    let id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::Ping {
            v: PROTOCOL_VERSION,
            id: id.clone(),
        })
        .await?;
    match resp {
        ServerMessage::Pong { id: rid, v, .. } => {
            assert_eq!(rid, id, "Pong must correlate via the same id");
            assert_eq!(v, PROTOCOL_VERSION);
        }
        other => panic!("expected Pong, got {other:?}"),
    }

    client.close().await?;
    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn list_sessions_returns_empty_in_m0() -> Result<()> {
    let username = unique_username("listsessions");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let server_task = tokio::spawn(async move { pipe::run(server_identity, shutdown_rx).await });

    let pipe = connect_with_retry(&pipe_name).await.context("connect")?;
    let mut client = Client::new(pipe);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;

    let id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::ListSessions {
            v: PROTOCOL_VERSION,
            id: id.clone(),
        })
        .await?;
    match resp {
        ServerMessage::SessionList {
            id: rid,
            sessions,
            v,
            ..
        } => {
            assert_eq!(rid, id);
            assert_eq!(v, PROTOCOL_VERSION);
            assert!(
                sessions.is_empty(),
                "M0 dispatcher returns empty SessionList"
            );
        }
        other => panic!("expected SessionList, got {other:?}"),
    }

    client.close().await?;
    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}
