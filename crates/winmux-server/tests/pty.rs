//! ConPTY 통합 통합 테스트.
//!
//! 본 테스트는 실제 Windows 셸을 spawn한다(`cmd.exe`). 셸이 PATH에 없으면
//! Windows가 아닌 환경이거나 매우 비정상적인 상황이므로 실패해도 무방하다.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};
use winmux_ipc_client::{Client, connect_with_retry};
use winmux_protocol::{
    ClientKind, ClientMessage, NewSessionRequest, PROTOCOL_VERSION, ServerMessage, UserIdentity,
    codec,
};
use winmux_server::jobobj::JobObject;
use winmux_server::pipe;

fn unique_username(tag: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("itest-{tag}-{}-{now_ns}", std::process::id())
}

fn fresh_job() -> Arc<JobObject> {
    Arc::new(JobObject::create_kill_on_close().expect("create job object"))
}

/// `cmd.exe`를 실행해 `echo hi` + Enter를 보낸 뒤 PtyOutput에서 그 문자열을
/// 받을 수 있는지 검증한다. ConPTY end-to-end가 동작한다는 핵심 회귀 검증.
#[tokio::test]
async fn new_session_spawns_cmd_and_echoes_input() -> Result<()> {
    let username = unique_username("pty-echo");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    let pipe = connect_with_retry(&pipe_name).await.context("connect")?;
    let mut client = Client::new(pipe);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;

    // 자동 어태치되는 NewSession.
    let req_id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::NewSession {
            v: PROTOCOL_VERSION,
            id: req_id.clone(),
            request: NewSessionRequest {
                name: Some("pty-test".to_owned()),
                shell: Some(std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_owned())),
                cwd: None,
                env: BTreeMap::new(),
                detached: false,
            },
        })
        .await?;
    let attached_pane_id = match resp {
        ServerMessage::Attached { panes, .. } => {
            assert_eq!(panes.len(), 1);
            assert!(panes[0].alive, "real ConPTY pane must be alive");
            panes[0].id.clone()
        }
        other => panic!("expected Attached, got {other:?}"),
    };

    // cmd.exe가 prompt를 그리도록 잠시 대기 후 입력.
    // (입력 자체가 echo되므로 prompt 없이도 검증 가능하지만, 약간의 안정성용.)
    sleep(Duration::from_millis(200)).await;

    let payload = b"echo winmux-marker\r\n";
    client
        .send(&ClientMessage::PtyInput {
            v: PROTOCOL_VERSION,
            pane_id: attached_pane_id.clone(),
            bytes_base64: codec::base64_encode(payload),
        })
        .await?;

    // 마커가 PtyOutput에 등장할 때까지 메시지 수신.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut accumulated = String::new();
    let found = loop {
        if std::time::Instant::now() >= deadline {
            break false;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let msg = match timeout(remaining, client.recv()).await {
            Ok(Ok(m)) => m,
            Ok(Err(e)) => {
                return Err(e.context("recv during pty marker wait"));
            }
            Err(_) => break false,
        };
        if let ServerMessage::PtyOutput { bytes_base64, .. } = msg
            && let Ok(bytes) = codec::base64_decode(&bytes_base64)
        {
            accumulated.push_str(&String::from_utf8_lossy(&bytes));
            if accumulated.contains("winmux-marker") {
                break true;
            }
        }
    };

    assert!(
        found,
        "expected `winmux-marker` to appear in PtyOutput within 10s; got: {:?}",
        accumulated
    );

    // 깨끗한 종료. 세션을 죽이면 PaneRuntime drop → Pty drop → 자식 kill.
    let kill_id = client.next_message_id();
    let kill_resp = client
        .request(&ClientMessage::KillSession {
            v: PROTOCOL_VERSION,
            id: kill_id,
            session: winmux_protocol::KillSessionTarget::Name("pty-test".to_owned()),
        })
        .await?;
    assert!(matches!(kill_resp, ServerMessage::Ok { .. }));

    client.close().await?;
    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}
