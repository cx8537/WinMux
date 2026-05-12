//! `winmux-server` 라이브러리 파사드.
//!
//! 바이너리 [`run`]은 단 한 번 호출되고 그 안에서 Tokio 런타임을 만들어
//! [`run_async`]를 돌린다. 통합 테스트는 [`run_async`] 또는 하위 모듈
//! ([`pipe`], [`user`] 등)을 직접 호출해서 외부 프로세스를 띄우지 않고도
//! 검증한다.
//!
//! 모듈 책임 분리는 `docs/spec/00-overview.md` § Three Processes 의
//! "server" 컬럼과 1:1 대응한다.

pub mod logging;
pub mod pipe;
pub mod sha256;
pub mod single_instance;
pub mod user;

use anyhow::{Context, Result};
use tokio::sync::oneshot;
use tracing::{info, warn};

/// 동기 entry. 로깅 init과 Tokio 런타임 빌드까지 책임진다.
///
/// 두 번째 호출은 로깅이 이미 초기화되어 있어 `Err`을 반환할 수 있다.
pub fn run() -> Result<()> {
    let _log_guard = logging::init().context("initialize logging")?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .context("build tokio runtime")?;

    runtime.block_on(run_async())
}

/// 비동기 서버 본체. 통합 테스트가 직접 호출할 수 있도록 `pub`.
///
/// `USERNAME` 미설정 같은 환경 문제는 `Err`로 올라온다. 다른 인스턴스가
/// 이미 운영 중이라면 `Ok(())`로 조용히 반환한다(`docs/spec/00-overview.md` §
/// Server lifecycle 2).
pub async fn run_async() -> Result<()> {
    let identity = user::UserIdentity::detect().context("detect current user")?;

    let mutex_name = identity.mutex_name();
    let _instance_guard = match single_instance::acquire(&mutex_name)? {
        single_instance::Outcome::Acquired(g) => g,
        single_instance::Outcome::AlreadyRunning => {
            info!(mutex = %mutex_name, "single_instance.already_running");
            return Ok(());
        }
    };

    info!(
        version = env!("CARGO_PKG_VERSION"),
        username = %identity.username,
        user_sha8 = %identity.user_sha8,
        "server.starting"
    );

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let signal_task = tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                info!("server.ctrl_c.received");
                let _ = shutdown_tx.send(());
            }
            Err(e) => {
                warn!(error = %e, "server.ctrl_c.listener.failed");
            }
        }
    });

    let pipe_result = pipe::run(identity, shutdown_rx).await;
    signal_task.abort();
    // signal_task가 abort 후에는 JoinError(Cancelled)만 돌아오므로 결과 무시.
    let _ = signal_task.await;

    info!("server.shutdown.complete");
    pipe_result
}
