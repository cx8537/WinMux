//! `winmux` 단일-shot CLI 바이너리 엔트리.
//!
//! 모든 로직은 [`winmux_cli::run`]에 있다. 본 파일은 종료 코드를
//! 그대로 OS로 돌려보내는 wrapper일 뿐이다.

use std::process::ExitCode;

fn main() -> ExitCode {
    winmux_cli::run()
}
