//! 서버 수명 동안 모든 자식 셸을 묶는 Job Object.
//!
//! Drop 시 Job 핸들이 닫히고 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`로 인해
//! 모든 자식 프로세스가 종료된다. server.exe가 panic으로 죽어도 OS가
//! 이 보장을 지킨다.
//!
//! M0 단계에서는 **서버 전체에 한 개**의 Job Object를 쓴다 (spec §
//! Job Object의 per-pane Job은 후속 단계에서 분리한다). 이렇게 두어도
//! "server 종료 시 모든 자식 정리"라는 핵심 안전망은 동일하다.

#![allow(unsafe_code)]

use std::ptr;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};
use windows::core::PCWSTR;

/// `KILL_ON_JOB_CLOSE`가 설정된 Job 핸들의 RAII 래퍼.
pub struct JobObject {
    handle: HANDLE,
}

impl JobObject {
    /// 익명 Job을 만들고 `KILL_ON_JOB_CLOSE`를 설정한다.
    pub fn create_kill_on_close() -> Result<Self> {
        // SAFETY: 두 lp* 인자는 nullable. 익명 Job을 만든다.
        let handle =
            unsafe { CreateJobObjectW(None, PCWSTR::null()) }.context("CreateJobObjectW failed")?;

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .context("JOBOBJECT_EXTENDED_LIMIT_INFORMATION size overflow")?;

        // SAFETY: handle 유효, &info는 size 바이트의 valid initialized 구조체.
        let res = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                ptr::from_ref::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>(&info).cast(),
                size,
            )
        };
        if let Err(e) = res {
            // SAFETY: 위에서 받은 유효 핸들. 실패 경로에서 닫는다.
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(anyhow::Error::from(e).context("SetInformationJobObject failed"));
        }
        Ok(Self { handle })
    }

    /// 지정한 PID의 프로세스를 이 Job에 부여한다. 실패 시 caller가 로그를 남긴다.
    pub fn assign_pid(&self, pid: u32) -> Result<()> {
        // SAFETY: PROCESS_SET_QUOTA | PROCESS_TERMINATE는 자기 사용자의 프로세스에
        //         대해 열 수 있는 최소 권한.
        let proc_handle = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) }
            .with_context(|| format!("OpenProcess({pid}) failed"))?;
        // SAFETY: 위 OpenProcess가 돌려준 유효 핸들과 이미 만든 Job 핸들.
        let res = unsafe { AssignProcessToJobObject(self.handle, proc_handle) };
        // 핸들은 Assign 호출이 성공했든 실패했든 즉시 닫는다 — Job은 PID로 추적한다.
        // SAFETY: 유효 proc handle.
        unsafe {
            let _ = CloseHandle(proc_handle);
        }
        res.context("AssignProcessToJobObject failed")
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            // SAFETY: Drop은 단 한 번. handle은 우리가 만들었다.
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

// SAFETY: HANDLE은 raw pointer이지만 OS 커널 객체의 식별자일 뿐이며,
//         이 구조체를 thread 사이로 옮겨도 그 식별자의 의미는 변하지 않는다.
//         내부적인 변경 없음 (assign_pid는 OS-level call).
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}
