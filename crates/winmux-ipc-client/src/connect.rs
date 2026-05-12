//! Named Pipe 연결 헬퍼.
//!
//! `docs/spec/00-overview.md` § Tray lifecycle 3의 백오프 패턴을 구현한다:
//! `100ms → 300ms → 1s → 3s` 후 포기.

use std::io;
use std::time::Duration;

use thiserror::Error;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tracing::debug;

/// `ERROR_FILE_NOT_FOUND` — Win32 `winerror.h`.
const ERROR_FILE_NOT_FOUND: i32 = 2;
/// `ERROR_PIPE_BUSY` — Win32 `winerror.h`. 사용 가능한 인스턴스가 없다.
const ERROR_PIPE_BUSY: i32 = 231;

/// 클라이언트 연결 실패 사유.
#[derive(Debug, Error)]
pub enum ConnectError {
    /// 파이프가 존재하지 않는다 — 서버가 실행 중이지 않다.
    #[error("server is not running (pipe not found): {0}")]
    NotRunning(String),
    /// 다른 IO 오류.
    #[error("pipe connect failed: {0}")]
    Io(#[from] io::Error),
    /// 정해진 백오프 끝에도 연결되지 않았다.
    #[error("connect retries exhausted for pipe `{pipe}`")]
    RetriesExhausted {
        /// 시도한 파이프 이름.
        pipe: String,
    },
}

/// 단일 시도 연결. 서버가 없으면 [`ConnectError::NotRunning`].
pub fn connect(pipe_name: &str) -> Result<NamedPipeClient, ConnectError> {
    match ClientOptions::new().open(pipe_name) {
        Ok(c) => Ok(c),
        Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => {
            Err(ConnectError::NotRunning(pipe_name.to_owned()))
        }
        Err(e) => Err(ConnectError::Io(e)),
    }
}

/// 백오프 재시도 연결. 시도 패턴: 즉시 → 100ms → 300ms → 1s → 3s 후 포기
/// (`docs/spec/00-overview.md` § Tray lifecycle 3). 총 5번 시도.
pub async fn connect_with_retry(pipe_name: &str) -> Result<NamedPipeClient, ConnectError> {
    let delays_ms = [0u64, 100, 300, 1000, 3000];
    let mut last_err: io::Error = io::Error::other("connect_with_retry: no attempts made");

    for ms in delays_ms {
        if ms > 0 {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        match ClientOptions::new().open(pipe_name) {
            Ok(c) => return Ok(c),
            Err(e) => {
                debug!(pipe = %pipe_name, error = %e, "ipc.connect.retry");
                last_err = e;
            }
        }
    }

    match last_err.raw_os_error() {
        Some(ERROR_FILE_NOT_FOUND) => Err(ConnectError::NotRunning(pipe_name.to_owned())),
        Some(ERROR_PIPE_BUSY) => Err(ConnectError::RetriesExhausted {
            pipe: pipe_name.to_owned(),
        }),
        _ => Err(ConnectError::Io(last_err)),
    }
}
