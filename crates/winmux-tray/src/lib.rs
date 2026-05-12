//! WinMux 트레이/GUI 글루.
//!
//! 본 crate는 트레이 프로세스의 Rust 측을 담당한다: Named Pipe 클라이언트
//! 부트스트랩, 향후 Tauri 명령 핸들러 등록. Tauri 런타임 자체는 `src-tauri`
//! 에서 실행한다(`docs/spec/00-overview.md` § Build Layout).
//!
//! M0 단계에서 [`run`]은:
//! 1. stderr `tracing` subscriber를 한 번 설치한다 (이미 설치돼 있으면 무시).
//! 2. `UserIdentity`로 파이프 이름을 만든다.
//! 3. 한 번 `connect` + `Hello`를 시도해 server 상태를 INFO/WARN 로그로
//!    남긴다. 실패해도 GUI는 계속 뜬다 — 사용자가 트레이에서 server를
//!    띄우거나 자동 기동될 수 있도록.

use anyhow::Result;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use winmux_ipc_client::{Client, ConnectError, connect};
use winmux_protocol::{ClientKind, UserIdentity};

/// 트레이 부트스트랩.
///
/// Tauri builder가 시작되기 전에 호출된다. 본 함수는 빠르게 반환해야 하며
/// (~수십 ms), GUI 진입을 지연시키지 않는다.
pub fn run() -> Result<()> {
    install_tracing();

    let identity = UserIdentity::detect()?;
    let pipe_name = identity.pipe_name();
    info!(
        pipe = %pipe_name,
        version = env!("CARGO_PKG_VERSION"),
        "tray.starting"
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    runtime.block_on(async {
        match connect(&pipe_name) {
            Ok(pipe) => {
                let mut client = Client::new(pipe);
                match client
                    .hello(ClientKind::Tray, env!("CARGO_PKG_VERSION"))
                    .await
                {
                    Ok(hello) => {
                        info!(
                            server_version = %hello.server_version,
                            user = %hello.user,
                            "tray.server_connected"
                        );
                        let _ = client.close().await;
                    }
                    Err(e) => warn!(error = %e, "tray.hello_failed"),
                }
            }
            Err(ConnectError::NotRunning(_)) => {
                // 서버가 아직 안 떴다 — M0에서는 자동 기동을 하지 않는다.
                info!("tray.server_not_running");
            }
            Err(e) => warn!(error = %e, "tray.connect_failed"),
        }
    });

    Ok(())
}

fn install_tracing() {
    let filter = std::env::var("WINMUX_LOG")
        .ok()
        .and_then(|v| EnvFilter::try_new(v).ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    // try_init은 이미 설치된 subscriber가 있으면 Err을 돌려준다 — 무시.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .try_init();
}
