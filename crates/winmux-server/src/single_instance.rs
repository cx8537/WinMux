//! Named Mutex 기반 단일-인스턴스 보장.
//!
//! 서버 인스턴스가 둘 이상 동시에 뜨면 ConPTY 핸들·파이프 이름 등이
//! 충돌한다. `Local\WinMux-Server-{user_sha8}` 뮤텍스를 잡지 못하면
//! 다른 인스턴스가 이미 운영 중이라는 신호로 보고, 호출자는 조용히
//! 종료한다 (`docs/spec/00-overview.md` § Server lifecycle 2).

#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::PCWSTR;

/// 뮤텍스 획득 결과.
pub enum Outcome {
    /// 이 프로세스가 뮤텍스를 잡았다. 가드를 살려두면 핸들이 유지된다.
    Acquired(MutexGuard),
    /// 다른 프로세스가 이미 잡고 있다.
    AlreadyRunning,
}

/// 뮤텍스 핸들의 RAII 래퍼.
pub struct MutexGuard {
    handle: HANDLE,
}

impl Drop for MutexGuard {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            // SAFETY: 정상 경로에서만 들어오는 유효한 핸들.
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

/// 이름이 붙은 뮤텍스 획득을 시도한다.
///
/// `name`은 Windows의 ASCII null-terminated wide 형식으로 자동 변환된다.
/// 권장 형식은 `Local\<...>` — 세션-격리된 네임스페이스.
pub fn acquire(name: &str) -> Result<Outcome> {
    let wide = to_null_terminated_wide(name);

    // SAFETY: `wide`는 본 함수 스택에서 살아있고 null로 종료된다. CreateMutexW는
    //         lpName이 PCWSTR(유효한 null-terminated wide 포인터)임을 요구한다.
    let handle = unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())) }
        .context("CreateMutexW failed")?;

    // SAFETY: `GetLastError`는 현재 스레드의 thread-local 마지막 Win32 에러를 읽는
    //         단순 조회 호출이다 — 직전의 CreateMutexW 결과를 본다.
    let last = unsafe { GetLastError() };

    if last == ERROR_ALREADY_EXISTS {
        // 핸들은 받았지만 우리가 첫 잡이 아니다 — 핸들을 닫고 신호만 반환한다.
        // SAFETY: 위 CreateMutexW가 돌려준 유효 핸들.
        unsafe {
            let _ = CloseHandle(handle);
        }
        return Ok(Outcome::AlreadyRunning);
    }

    Ok(Outcome::Acquired(MutexGuard { handle }))
}

fn to_null_terminated_wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = OsStr::new(s).encode_wide().collect();
    v.push(0);
    v
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn unique_name() -> String {
        // PID + 시각으로 테스트 간 충돌을 피한다. CI 병렬 실행 안전.
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!(r"Local\WinMux-Test-{}-{now}", std::process::id())
    }

    #[test]
    fn first_acquire_succeeds() {
        let name = unique_name();
        let r = acquire(&name).expect("acquire");
        assert!(matches!(r, Outcome::Acquired(_)));
    }

    #[test]
    fn second_acquire_reports_already_running() {
        let name = unique_name();
        let first = acquire(&name).expect("first");
        assert!(matches!(first, Outcome::Acquired(_)));
        let second = acquire(&name).expect("second");
        assert!(matches!(second, Outcome::AlreadyRunning));
        drop(first);
        // 첫 가드를 놓아주면 같은 이름을 다시 잡을 수 있다.
        let third = acquire(&name).expect("third");
        assert!(matches!(third, Outcome::Acquired(_)));
    }
}
