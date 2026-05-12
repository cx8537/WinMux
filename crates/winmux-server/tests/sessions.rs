//! 세션 dispatcher의 통합 테스트.
//!
//! `winmux-server`의 in-memory registry가 와이어 위에서 `NewSession`/
//! `Attach`/`ListSessions`/`KillSession` 흐름과 어떻게 맞물리는지 검증한다.
//! PTY는 아직 등장하지 않으므로 첫 패널은 `alive: false` placeholder다.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::oneshot;
use tokio::time::timeout;
use winmux_ipc_client::{Client, connect_with_retry};
use winmux_protocol::{
    AttachTarget, ClientKind, ClientMessage, ErrorCode, KillSessionTarget, NewSessionRequest,
    PROTOCOL_VERSION, ServerMessage, UserIdentity,
};
use winmux_server::jobobj::JobObject;
use winmux_server::pipe;

fn fresh_job() -> Arc<JobObject> {
    Arc::new(JobObject::create_kill_on_close().expect("create job object"))
}

fn unique_username(tag: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("itest-{tag}-{}-{now_ns}", std::process::id())
}

fn empty_new_session_request(name: Option<&str>, detached: bool) -> NewSessionRequest {
    NewSessionRequest {
        name: name.map(str::to_owned),
        shell: None,
        cwd: None,
        env: BTreeMap::new(),
        detached,
    }
}

#[tokio::test]
async fn new_session_then_list_returns_one() -> Result<()> {
    let username = unique_username("newls");
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

    // NewSession (-d to keep this client unattached).
    let req_id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::NewSession {
            v: PROTOCOL_VERSION,
            id: req_id.clone(),
            request: empty_new_session_request(Some("work"), true),
        })
        .await?;
    match resp {
        ServerMessage::Ok { id, .. } => assert_eq!(id, req_id),
        other => panic!("expected Ok for detached NewSession, got {other:?}"),
    }

    // ListSessions should now contain exactly one entry.
    let list_id = client.next_message_id();
    let list_resp = client
        .request(&ClientMessage::ListSessions {
            v: PROTOCOL_VERSION,
            id: list_id,
        })
        .await?;
    match list_resp {
        ServerMessage::SessionList { sessions, .. } => {
            assert_eq!(sessions.len(), 1);
            let s = &sessions[0];
            assert_eq!(s.name, "work");
            assert_eq!(s.windows, 1);
            // detached로 만들었으므로 어태치된 클라이언트가 없어야 한다.
            assert_eq!(s.attached_clients, 0);
            assert!(!s.created_at.is_empty());
        }
        other => panic!("expected SessionList, got {other:?}"),
    }

    client.close().await?;
    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn new_session_auto_attaches_and_disconnect_decrements() -> Result<()> {
    let username = unique_username("autoattach");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    // 첫 클라이언트는 NewSession을 자동 어태치로 생성.
    let pipe1 = connect_with_retry(&pipe_name).await.context("connect")?;
    let mut client1 = Client::new(pipe1);
    client1
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;

    let create_id = client1.next_message_id();
    let resp = client1
        .request(&ClientMessage::NewSession {
            v: PROTOCOL_VERSION,
            id: create_id.clone(),
            request: empty_new_session_request(Some("auto"), false),
        })
        .await?;
    let attached_session = match resp {
        ServerMessage::Attached {
            id,
            session_id,
            windows,
            panes,
            initial_snapshots,
            ..
        } => {
            assert_eq!(id, create_id);
            assert_eq!(windows.len(), 1);
            assert_eq!(panes.len(), 1);
            assert!(panes[0].alive, "ConPTY-backed pane should be alive");
            // VirtualTerm 통합 이후로는 새 세션 어태치 응답에도 panes와
            // 1:1인 빈 화면 스냅샷이 들어온다 (`terminal::VirtualTerm::snapshot`).
            assert_eq!(initial_snapshots.len(), 1);
            assert_eq!(initial_snapshots[0].pane_id, panes[0].id);
            assert!(!initial_snapshots[0].bytes_base64.is_empty());
            session_id
        }
        other => panic!("expected Attached, got {other:?}"),
    };

    // 두 번째 클라이언트는 ListSessions로 attached_clients가 1임을 본다.
    let pipe2 = connect_with_retry(&pipe_name).await.context("connect2")?;
    let mut client2 = Client::new(pipe2);
    client2
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;
    let list_id = client2.next_message_id();
    let list_resp = client2
        .request(&ClientMessage::ListSessions {
            v: PROTOCOL_VERSION,
            id: list_id,
        })
        .await?;
    match list_resp {
        ServerMessage::SessionList { sessions, .. } => {
            let s = sessions
                .iter()
                .find(|s| s.id == attached_session)
                .expect("session must be listed");
            assert_eq!(s.attached_clients, 1);
        }
        other => panic!("expected SessionList, got {other:?}"),
    }

    // 첫 클라이언트가 Bye → 서버가 attached_clients를 -1로 내려야 한다.
    client1.close().await?;

    // 잠시 server가 정리할 시간을 준다(짧은 spin).
    for _ in 0..20 {
        let list_id = client2.next_message_id();
        let list_resp = client2
            .request(&ClientMessage::ListSessions {
                v: PROTOCOL_VERSION,
                id: list_id,
            })
            .await?;
        if let ServerMessage::SessionList { sessions, .. } = list_resp {
            let s = sessions
                .iter()
                .find(|s| s.id == attached_session)
                .expect("session still listed");
            if s.attached_clients == 0 {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let final_id = client2.next_message_id();
    let final_resp = client2
        .request(&ClientMessage::ListSessions {
            v: PROTOCOL_VERSION,
            id: final_id,
        })
        .await?;
    match final_resp {
        ServerMessage::SessionList { sessions, .. } => {
            let s = sessions
                .iter()
                .find(|s| s.id == attached_session)
                .expect("session still listed");
            assert_eq!(s.attached_clients, 0, "attached count must drop to 0");
        }
        other => panic!("expected SessionList, got {other:?}"),
    }

    client2.close().await?;
    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn kill_session_removes_it_and_unknown_target_errors() -> Result<()> {
    let username = unique_username("kill");
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

    // 만든다.
    let create_id = client.next_message_id();
    client
        .request(&ClientMessage::NewSession {
            v: PROTOCOL_VERSION,
            id: create_id,
            request: empty_new_session_request(Some("doomed"), true),
        })
        .await?;

    // 죽인다.
    let kill_id = client.next_message_id();
    let kill_resp = client
        .request(&ClientMessage::KillSession {
            v: PROTOCOL_VERSION,
            id: kill_id.clone(),
            session: KillSessionTarget::Name("doomed".to_owned()),
        })
        .await?;
    match kill_resp {
        ServerMessage::Ok { id, .. } => assert_eq!(id, kill_id),
        other => panic!("expected Ok, got {other:?}"),
    }

    // 더 이상 목록에 없다.
    let list_id = client.next_message_id();
    let list_resp = client
        .request(&ClientMessage::ListSessions {
            v: PROTOCOL_VERSION,
            id: list_id,
        })
        .await?;
    match list_resp {
        ServerMessage::SessionList { sessions, .. } => assert!(sessions.is_empty()),
        other => panic!("expected SessionList, got {other:?}"),
    }

    // 두 번째 kill은 SESSION_NOT_FOUND.
    let bad_id = client.next_message_id();
    let bad_resp = client
        .request(&ClientMessage::KillSession {
            v: PROTOCOL_VERSION,
            id: bad_id,
            session: KillSessionTarget::Name("doomed".to_owned()),
        })
        .await?;
    match bad_resp {
        ServerMessage::Error { payload, .. } => {
            assert_eq!(payload.code, ErrorCode::SessionNotFound);
            assert!(
                payload.recoverable,
                "kill miss is recoverable — connection stays open"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }

    client.close().await?;
    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn attach_to_existing_session_by_name() -> Result<()> {
    let username = unique_username("attach");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    // 첫 클라이언트가 detached로 세션을 만든다.
    let pipe1 = connect_with_retry(&pipe_name).await.context("connect")?;
    let mut client1 = Client::new(pipe1);
    client1
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;
    let create_id = client1.next_message_id();
    client1
        .request(&ClientMessage::NewSession {
            v: PROTOCOL_VERSION,
            id: create_id,
            request: empty_new_session_request(Some("shared"), true),
        })
        .await?;
    client1.close().await?;

    // 두 번째 클라이언트가 이름으로 Attach.
    let pipe2 = connect_with_retry(&pipe_name).await.context("connect2")?;
    let mut client2 = Client::new(pipe2);
    client2
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;
    let attach_id = client2.next_message_id();
    let attach_resp = client2
        .request(&ClientMessage::Attach {
            v: PROTOCOL_VERSION,
            id: attach_id.clone(),
            session: AttachTarget::Name("shared".to_owned()),
            client_size: winmux_protocol::PaneSize {
                rows: 30,
                cols: 100,
            },
        })
        .await?;
    match attach_resp {
        ServerMessage::Attached {
            id, windows, panes, ..
        } => {
            assert_eq!(id, attach_id);
            assert_eq!(windows.len(), 1);
            assert_eq!(panes.len(), 1);
        }
        other => panic!("expected Attached, got {other:?}"),
    }

    client2.close().await?;
    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}
