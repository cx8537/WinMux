//! winmux-cli의 write 명령 핸들러 통합 테스트.
//!
//! In-process로 `winmux_server::pipe::run`을 띄우고, cli의 lower-level
//! 핸들러(`new_session_with` / `kill_session_with`)를 직접 호출해
//! NewSession / KillSession 와이어 흐름을 검증한다. 본 테스트는
//! `winmux.exe`/`winmux-server.exe`를 외부 프로세스로 띄우지 않는다 —
//! 서버 자동 기동 자체는 별도의 수동 검증 대상이다 (sandbox에서 detached
//! spawn을 돌리기 어렵기 때문).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::oneshot;
use tokio::time::timeout;
use winmux_cli::args::NewSessionArgs;
use winmux_cli::{
    KillFailure, NewSessionResult, kill_pane_with, kill_session_with, kill_window_with,
    new_session_with, send_keys_with,
};
use winmux_ipc_client::{Client, connect_with_retry};
use winmux_protocol::{
    AttachTarget, ClientKind, ClientMessage, PROTOCOL_VERSION, PaneSize, ServerMessage,
    UserIdentity, codec,
};
use winmux_server::jobobj::JobObject;
use winmux_server::pipe;

fn unique_username(tag: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("itest-cli-{tag}-{}-{now_ns}", std::process::id())
}

fn fresh_job() -> Arc<JobObject> {
    Arc::new(JobObject::create_kill_on_close().expect("create job object"))
}

fn empty_new_session_args(name: Option<&str>, detached: bool) -> NewSessionArgs {
    NewSessionArgs {
        session: name.map(str::to_owned),
        detached,
        cwd: None,
        shell: None,
        shell_argv: Vec::new(),
    }
}

#[tokio::test]
async fn new_session_attached_returns_session_id_and_alive_pane() -> Result<()> {
    let username = unique_username("new-attach");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    let pipe = connect_with_retry(&pipe_name).await.context("connect")?;
    let args = empty_new_session_args(Some("work"), false);
    let result = new_session_with(pipe, &args).await?;
    match result {
        NewSessionResult::Attached {
            session_id,
            name,
            panes,
            ..
        } => {
            assert_eq!(name.as_deref(), Some("work"));
            assert!(session_id.as_str().starts_with("ses-"));
            assert_eq!(panes.len(), 1);
            assert!(panes[0].alive, "ConPTY-backed pane must be alive");
        }
        other => panic!("expected Attached, got {other:?}"),
    }

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn new_session_detached_returns_detached_only() -> Result<()> {
    let username = unique_username("new-detach");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    let pipe = connect_with_retry(&pipe_name).await.context("connect")?;
    let args = empty_new_session_args(Some("scratch"), true);
    let result = new_session_with(pipe, &args).await?;
    match result {
        NewSessionResult::Detached { name } => assert_eq!(name.as_deref(), Some("scratch")),
        other => panic!("expected Detached, got {other:?}"),
    }

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn kill_session_succeeds_for_existing_name() -> Result<()> {
    let username = unique_username("kill-ok");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    // 만든다 (detached로 — 클라이언트가 첫 attach 슬롯을 점유하지 않도록).
    let p1 = connect_with_retry(&pipe_name).await?;
    new_session_with(p1, &empty_new_session_args(Some("doomed"), true)).await?;

    // 같은 이름으로 kill.
    let p2 = connect_with_retry(&pipe_name).await?;
    let killed = kill_session_with(p2, "doomed")
        .await
        .map_err(|e| anyhow::anyhow!("kill failed: {e:?}"))?;
    assert_eq!(killed, "doomed");

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn attach_returns_non_empty_snapshot_after_pty_output() -> Result<()> {
    // 시나리오: 세션 생성(detached) → 잠시 기다려 셸 prompt가 vterm에
    // 누적되도록 한 뒤 → 다른 connection으로 Attach → initial_snapshots에
    // 셸이 그린 화면이 base64로 들어있는지 확인.
    //
    // 셸의 정확한 prompt 문자열은 환경마다 다르므로(언어/cwd) 본 테스트는
    // "stream의 길이가 빈 화면 baseline보다 크고, base64 해독이 가능하다"는
    // 보수적 조건만 검증한다. 빈 화면 baseline = ESC[2J ESC[H ESC[0m +
    // (rows 번 줄 시작 이동) + ESC[0m + cursor 위치. spawn 직후 어떤 셸이든
    // banner나 prompt를 출력하므로 그보다 길어야 한다.
    let username = unique_username("attach-snap");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    // (1) 세션을 detached로 만든다. detached 응답은 session_id를 안 돌려주므로
    // 그 다음 단계에서 이름으로 attach한다.
    let p1 = connect_with_retry(&pipe_name).await?;
    let _ = new_session_with(p1, &empty_new_session_args(Some("with-shell"), true)).await?;

    // (2) 셸이 prompt를 출력할 때까지 기다린다. 너무 짧으면 vterm이 아직
    // 비어 있을 수 있고, 너무 길면 테스트가 느려진다. 200ms이면 PowerShell이
    // banner를 그리기에 충분하다 (관찰 기반의 PoC heuristic).
    tokio::time::sleep(Duration::from_millis(400)).await;

    // (3) Attach를 직접 보낸다 — cli에는 `attach_with` 헬퍼가 아직 없다.
    let p2 = connect_with_retry(&pipe_name).await?;
    let mut client = Client::new(p2);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;
    let id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::Attach {
            v: PROTOCOL_VERSION,
            id,
            session: AttachTarget::Name("with-shell".to_owned()),
            client_size: PaneSize { rows: 24, cols: 80 },
        })
        .await?;

    match resp {
        ServerMessage::Attached {
            initial_snapshots, ..
        } => {
            assert_eq!(initial_snapshots.len(), 1, "expected single pane snapshot");
            let snap = &initial_snapshots[0];
            let bytes = codec::base64_decode(&snap.bytes_base64).context("decode snapshot")?;
            // baseline = ESC[2J(4) + ESC[H(3) + ESC[0m(4) + per-line "ESC[L;1H"(>=6) ×40
            //   + trailing ESC[0m(4) + cursor pos(>=6). 빈 80x40 화면이면 약 250바이트.
            // 셸이 뭐든 한 줄이라도 prompt를 그렸다면 글리프가 추가되어 더 길다.
            // 250보다 안전한 하한으로 200을 두고, 셸 출력이 있으면 그것보다 커진다.
            assert!(
                bytes.len() > 200,
                "snapshot is suspiciously short ({} bytes) — vterm feeder may not be running",
                bytes.len()
            );
            // 항상 ESC[2J로 시작해야 한다.
            assert!(bytes.starts_with(b"\x1b[2J\x1b[H\x1b[0m"));
        }
        other => panic!("expected Attached, got {other:?}"),
    }
    let _ = client.close().await;

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn kill_session_reports_not_found_for_missing_target() -> Result<()> {
    let username = unique_username("kill-miss");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    let pipe = connect_with_retry(&pipe_name).await?;
    let result = kill_session_with(pipe, "nope").await;
    match result {
        Err(KillFailure::NotFound(t)) => assert_eq!(t, "nope"),
        other => panic!("expected NotFound, got {other:?}"),
    }

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn kill_pane_removes_session_when_last_pane() -> Result<()> {
    // M0 PoC에서 한 세션은 한 윈도우·한 패널만 가지므로 그 패널을 죽이면
    // 세션 자체가 사라진다 — 이후 ListSessions에 등장하지 않아야 한다.
    let username = unique_username("kill-pane");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    // 세션 만들고 attached pane_id를 캡쳐.
    let p1 = connect_with_retry(&pipe_name).await?;
    let result = new_session_with(p1, &empty_new_session_args(Some("victim"), false)).await?;
    let pane_id = match result {
        NewSessionResult::Attached { panes, .. } => panes[0].id.clone(),
        other => panic!("expected Attached, got {other:?}"),
    };

    // kill-pane.
    let p2 = connect_with_retry(&pipe_name).await?;
    kill_pane_with(p2, &pane_id)
        .await
        .map_err(|e| anyhow::anyhow!("kill_pane: {e:?}"))?;

    // 세션이 사라졌는지 ListSessions로 검증.
    let p3 = connect_with_retry(&pipe_name).await?;
    let mut client = Client::new(p3);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;
    let id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::ListSessions {
            v: PROTOCOL_VERSION,
            id,
        })
        .await?;
    match resp {
        ServerMessage::SessionList { sessions, .. } => {
            assert!(
                !sessions.iter().any(|s| s.name == "victim"),
                "session should be gone after killing its sole pane"
            );
        }
        other => panic!("expected SessionList, got {other:?}"),
    }
    let _ = client.close().await;

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn kill_pane_unknown_target_returns_not_found() -> Result<()> {
    let username = unique_username("kill-pane-miss");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    let p = connect_with_retry(&pipe_name).await?;
    let bogus = winmux_protocol::PaneId::from_raw("pane-DOESNOTEXIST".to_owned());
    let r = kill_pane_with(p, &bogus).await;
    match r {
        Err(KillFailure::NotFound(t)) => assert!(t.starts_with("pane-DOESNOTEXIST")),
        other => panic!("expected NotFound, got {other:?}"),
    }

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn kill_window_removes_session_when_last_window() -> Result<()> {
    let username = unique_username("kill-window");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    // 세션을 만들고 Attached에서 active_window를 가져온다.
    let p1 = connect_with_retry(&pipe_name).await?;
    let mut client1 = Client::new(p1);
    client1
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;
    let id = client1.next_message_id();
    let resp = client1
        .request(&ClientMessage::NewSession {
            v: PROTOCOL_VERSION,
            id,
            request: winmux_protocol::NewSessionRequest {
                name: Some("winvictim".to_owned()),
                shell: None,
                cwd: None,
                env: Default::default(),
                detached: false,
            },
        })
        .await?;
    let window_id = match resp {
        ServerMessage::Attached { active_window, .. } => active_window,
        other => panic!("expected Attached, got {other:?}"),
    };
    let _ = client1.close().await;

    // kill-window.
    let p2 = connect_with_retry(&pipe_name).await?;
    kill_window_with(p2, &window_id)
        .await
        .map_err(|e| anyhow::anyhow!("kill_window: {e:?}"))?;

    // 세션이 사라졌는지 확인.
    let p3 = connect_with_retry(&pipe_name).await?;
    let mut client3 = Client::new(p3);
    client3
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;
    let id = client3.next_message_id();
    let resp = client3
        .request(&ClientMessage::ListSessions {
            v: PROTOCOL_VERSION,
            id,
        })
        .await?;
    match resp {
        ServerMessage::SessionList { sessions, .. } => {
            assert!(!sessions.iter().any(|s| s.name == "winvictim"));
        }
        other => panic!("expected SessionList, got {other:?}"),
    }
    let _ = client3.close().await;

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn send_keys_writes_bytes_to_pty_and_appears_in_snapshot() -> Result<()> {
    // 시나리오: detached session 생성 → send-keys로 "echo wm_marker" + Enter →
    // 잠시 기다림 → attach → initial_snapshots에 "wm_marker"가 들어있어야 한다.
    let username = unique_username("send-keys");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    let p1 = connect_with_retry(&pipe_name).await?;
    let _ = new_session_with(p1, &empty_new_session_args(Some("typed"), true)).await?;

    // 셸이 prompt를 그릴 시간을 짧게 준다.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // send-keys.
    let p2 = connect_with_retry(&pipe_name).await?;
    send_keys_with(
        p2,
        Some("typed"),
        &["echo wm_marker".to_owned(), "Enter".to_owned()],
    )
    .await
    .map_err(|e| anyhow::anyhow!("send_keys: {e}"))?;

    // echo가 ConPTY에서 돌아와 vterm에 누적되도록 잠시 대기.
    tokio::time::sleep(Duration::from_millis(700)).await;

    // attach → snapshot 확인.
    let p3 = connect_with_retry(&pipe_name).await?;
    let mut client = Client::new(p3);
    client
        .hello(ClientKind::Cli, env!("CARGO_PKG_VERSION"))
        .await?;
    let id = client.next_message_id();
    let resp = client
        .request(&ClientMessage::Attach {
            v: PROTOCOL_VERSION,
            id,
            session: AttachTarget::Name("typed".to_owned()),
            client_size: PaneSize { rows: 24, cols: 80 },
        })
        .await?;
    let snap_bytes = match resp {
        ServerMessage::Attached {
            initial_snapshots, ..
        } => {
            assert_eq!(initial_snapshots.len(), 1);
            codec::base64_decode(&initial_snapshots[0].bytes_base64).context("decode")?
        }
        other => panic!("expected Attached, got {other:?}"),
    };
    assert!(
        snap_bytes
            .windows(b"wm_marker".len())
            .any(|w| w == b"wm_marker"),
        "expected echo output `wm_marker` in snapshot"
    );
    let _ = client.close().await;

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}

#[tokio::test]
async fn send_keys_rejects_unsupported_key_name() -> Result<()> {
    let username = unique_username("send-keys-bad");
    let identity = UserIdentity::for_username(&username);
    let pipe_name = identity.pipe_name();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server_identity = identity.clone();
    let job = fresh_job();
    let server_task =
        tokio::spawn(async move { pipe::run(server_identity, job, shutdown_rx).await });

    let p1 = connect_with_retry(&pipe_name).await?;
    let _ = new_session_with(p1, &empty_new_session_args(Some("bad"), true)).await?;

    let p2 = connect_with_retry(&pipe_name).await?;
    let r = send_keys_with(p2, Some("bad"), &["F1".to_owned()]).await;
    match r {
        Err(msg) => assert!(msg.contains("unsupported key"), "got: {msg}"),
        Ok(()) => panic!("expected error for unsupported key"),
    }

    let _ = shutdown_tx.send(());
    let _ = timeout(Duration::from_secs(5), server_task).await?;
    Ok(())
}
