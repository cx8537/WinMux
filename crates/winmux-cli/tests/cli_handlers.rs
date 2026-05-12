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
use winmux_cli::{KillFailure, NewSessionResult, kill_session_with, new_session_with};
use winmux_ipc_client::connect_with_retry;
use winmux_protocol::UserIdentity;
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
