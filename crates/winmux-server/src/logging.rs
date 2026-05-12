//! tracing 초기화.
//!
//! stderr와 `%APPDATA%\winmux\logs\server-YYYY-MM-DD.log`(일별 로테이션)
//! 두 곳으로 INFO 이상 이벤트를 보낸다. `WINMUX_LOG` 환경변수로 모듈별
//! 필터를 덮어쓸 수 있다 (`docs/nonfunctional/logging.md` § Levels).
//!
//! 파일 appender 설치에 실패해도 stderr만으로 init은 성공한다 — 서버는
//! 디스크 접근 실패 정도로 죽지 않는다.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// 로그 파일 파일명 prefix. 실제 파일은
/// `server.2026-05-11`처럼 날짜가 붙는다.
const LOG_PREFIX: &str = "server";

/// 프로세스 수명 동안 들고 있어야 하는 가드.
///
/// 비동기 writer는 백그라운드 스레드를 유지하며, 이 가드를 drop하면
/// 버퍼된 라인을 flush하고 종료한다. `main` 함수의 스코프와 같이 둔다.
pub struct LogGuard {
    _file: Option<WorkerGuard>,
}

/// stderr + 일별 파일 두 곳에 로그를 보낸다.
///
/// `WINMUX_LOG` 환경변수를 읽어 모듈별 필터를 적용한다.
/// 두 번 호출하면 두 번째 호출이 `Err`을 반환한다.
pub fn init() -> Result<LogGuard> {
    let env_filter = std::env::var("WINMUX_LOG")
        .ok()
        .and_then(|raw| EnvFilter::try_new(raw).ok())
        .unwrap_or_else(|| EnvFilter::new("info"));

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .with_target(true);

    let (file_layer_opt, file_guard, setup_err) = match install_file_appender() {
        Ok((writer, guard)) => {
            let layer = fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
                .with_target(true);
            (Some(layer), Some(guard), None)
        }
        Err(e) => (None, None, Some(e.to_string())),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .with(file_layer_opt)
        .try_init()
        .context("tracing subscriber already initialized")?;

    if let Some(err) = setup_err {
        tracing::warn!(error = %err, "logging.file_appender_disabled");
    }

    Ok(LogGuard { _file: file_guard })
}

fn install_file_appender() -> Result<(NonBlocking, WorkerGuard)> {
    let dir = log_dir().context("compute log directory")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("create_dir_all({})", dir.display()))?;
    let appender = RollingFileAppender::new(Rotation::DAILY, &dir, LOG_PREFIX);
    Ok(tracing_appender::non_blocking(appender))
}

fn log_dir() -> Result<PathBuf> {
    let appdata = std::env::var("APPDATA").context("APPDATA environment variable is not set")?;
    let mut path = PathBuf::from(appdata);
    path.push("winmux");
    path.push("logs");
    Ok(path)
}
