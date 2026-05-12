//! `winmux-server` 바이너리 엔트리.
//!
//! 실제 로직은 [`winmux_server::run`]에 있다. 본 파일은 단지 종료 코드를
//! 매핑하고, 운영 중 치명적 오류를 한 줄 더 로그에 남기는 wrapper일 뿐이다.
//! 콘솔 출력 매크로(`println!`/`eprintln!`)는 lint로 차단되어 있다 —
//! 로그는 `tracing`만 사용한다 (CLAUDE.md Rule 12).

use std::process::ExitCode;

fn main() -> ExitCode {
    match winmux_server::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // 로깅이 초기화된 이후라면 이 줄이 파일/stderr에 남는다.
            // 초기화 전이라면 무음 종료다 (그 경우엔 어차피 메시지를
            // 띄울 안전한 경로가 없다).
            tracing::error!(error = ?err, "server.fatal");
            ExitCode::from(1)
        }
    }
}
