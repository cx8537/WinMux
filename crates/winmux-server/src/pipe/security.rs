//! Named Pipe 보안 디스크립터·SID 검증.
//!
//! 두 가지 책임:
//! 1. [`PipeAcl::build_for_current_user`] — 현재 프로세스 토큰에서 SID를
//!    뽑아, **그 SID에만** `GENERIC_READ | GENERIC_WRITE`을 허용하는
//!    명시적 DACL을 가진 절대-형식 `SECURITY_DESCRIPTOR`를 만든다.
//!    `Administrators`/`SYSTEM`/`Everyone` 모두에 권한 없음
//!    (`docs/spec/01-ipc-protocol.md` § Pipe creation).
//! 2. [`verify_client_user`] — accept된 클라이언트의 SID가 서버 사용자와
//!    일치하는지 검증한다. ACL이 이미 막아주지만 정책상 한 번 더 확인한다.
//!
//! 이 모듈은 unsafe Win32 호출이 집중된다. 모든 unsafe 블록에 `SAFETY:`
//! 주석이 붙고, 모든 핸들은 RAII 가드로 닫힌다.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::mem;
use std::os::windows::io::AsRawHandle;

use anyhow::{Context, Result};
use tokio::net::windows::named_pipe::NamedPipeServer;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    ACL, ACL_REVISION, AddAccessAllowedAce, CopySid, EqualSid, GetLengthSid, GetTokenInformation,
    InitializeAcl, InitializeSecurityDescriptor, IsValidSid, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// `SECURITY_DESCRIPTOR_REVISION` 값(SDK 정의: 1). windows-rs 0.58에서는
/// 상수가 노출되지 않으므로 직접 정의한다.
const SECURITY_DESCRIPTOR_REVISION_VALUE: u32 = 1;

/// Named Pipe ACL이 부여하는 권한.
///
/// 파이프 본 데이터 plane의 read/write만 필요하다. ChangeNotify·Synchronize
/// 등의 부가 권한은 부여하지 않는다.
const GENERIC_READ_RIGHTS: u32 = 0x8000_0000;
const GENERIC_WRITE_RIGHTS: u32 = 0x4000_0000;

/// Drop 시 `CloseHandle`을 호출하는 RAII 가드.
struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(h: HANDLE) -> Self {
        Self(h)
    }

    fn as_raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: 생성 경로는 항상 OpenProcess/OpenProcessToken이 돌려준
            //         유효한 핸들이고, Drop은 그 핸들을 정확히 한 번 닫는다.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// 자기 자신을 참조하는 `SECURITY_DESCRIPTOR`와 그 부속 버퍼들을 함께 소유.
///
/// `as_security_attributes_ptr`가 돌려준 raw 포인터는 `&PipeAcl`이 살아있는
/// 동안만 유효하다. 구조체를 이동(Move)해도 내부 Box·Vec의 heap 할당이
/// 이동하지 않으므로 SD 내부 포인터(Dacl, Sid)는 그대로 유효하다.
pub struct PipeAcl {
    // Drop 순서: 위에서 아래로. SA가 SD를, SD가 ACL과 SID를 가리키므로
    // SA가 가장 먼저 drop되어야 다른 쪽에서 dangling을 만들지 않는다.
    sa: Box<SECURITY_ATTRIBUTES>,
    // `sd`/`acl_buf`는 코드에서 직접 read하지 않지만, `sa.lpSecurityDescriptor`와
    // `sd` 내부의 Dacl 포인터가 각각 이들을 가리킨다. 살아있어야 한다.
    #[allow(dead_code)]
    sd: Box<SECURITY_DESCRIPTOR>,
    /// `u32` 단위로 정렬된 ACL 저장소. ACL은 DWORD 정렬 요구사항이 있다.
    #[allow(dead_code)]
    acl_buf: Vec<u32>,
    /// `u32` 단위로 정렬된 SID 사본. EqualSid 비교 시 ptr로 그대로 쓴다.
    sid_buf: Vec<u32>,
}

// SAFETY: 내부 raw 포인터는 모두 Box/Vec heap 메모리를 가리키며, 외부에서
//         이 구조체를 thread 사이로 옮기더라도 그 heap 위치는 변하지 않는다.
//         읽기·쓰기 동시 접근은 외부 동기화로 막는다(현재 acl 빌드는 1회뿐).
unsafe impl Send for PipeAcl {}
unsafe impl Sync for PipeAcl {}

impl PipeAcl {
    /// 현재 프로세스 토큰의 사용자 SID로 ACL을 빌드한다.
    pub fn build_for_current_user() -> Result<Self> {
        let sid_buf = current_user_sid_buf().context("read current user SID")?;
        let sid_ptr: PSID = PSID(sid_buf.as_ptr() as *mut c_void);

        // SAFETY: sid_buf는 SID 구조체의 바이트를 그대로 담고 있고, sid_ptr가
        //         가리키는 그 메모리는 본 함수가 sid_buf를 소유하는 한 유효하다.
        let valid = unsafe { IsValidSid(sid_ptr) };
        if !valid.as_bool() {
            anyhow::bail!("IsValidSid returned false for current user SID");
        }
        // SAFETY: sid_ptr는 유효한 SID를 가리킨다.
        let sid_len = unsafe { GetLengthSid(sid_ptr) };

        // ACL 크기: sizeof(ACL) + sizeof(ACCESS_ALLOWED_ACE) - sizeof(DWORD) + sid_len.
        // 마지막 DWORD를 빼는 이유는 ACCESS_ALLOWED_ACE.SidStart가 ACE 끝에
        // overlap된 SID 첫 DWORD이기 때문(Win32 컨벤션).
        let header = mem::size_of::<ACL>() as u32;
        let ace_fixed = (mem::size_of::<u32>() * 2) as u32; // ACE_HEADER + AccessMask
        let acl_size: u32 = header
            .checked_add(ace_fixed)
            .and_then(|n| n.checked_add(sid_len))
            .context("ACL size overflow")?;

        let acl_words = (acl_size as usize).div_ceil(4);
        let mut acl_buf: Vec<u32> = vec![0u32; acl_words];

        // SAFETY: acl_buf는 acl_size 바이트 이상이고 4-byte 정렬 보장.
        unsafe { InitializeAcl(acl_buf.as_mut_ptr().cast::<ACL>(), acl_size, ACL_REVISION) }
            .context("InitializeAcl failed")?;

        // SAFETY: 위 InitializeAcl로 초기화된 ACL, 유효한 sid_ptr.
        unsafe {
            AddAccessAllowedAce(
                acl_buf.as_mut_ptr().cast::<ACL>(),
                ACL_REVISION,
                GENERIC_READ_RIGHTS | GENERIC_WRITE_RIGHTS,
                sid_ptr,
            )
        }
        .context("AddAccessAllowedAce failed")?;

        // SECURITY_DESCRIPTOR를 절대 형식으로 초기화하고 DACL을 설정.
        let mut sd: Box<SECURITY_DESCRIPTOR> = Box::default();
        let sd_ptr =
            PSECURITY_DESCRIPTOR(std::ptr::from_mut::<SECURITY_DESCRIPTOR>(&mut *sd).cast());
        // SAFETY: sd는 freshly zeroed. SECURITY_DESCRIPTOR_REVISION은 1.
        unsafe { InitializeSecurityDescriptor(sd_ptr, SECURITY_DESCRIPTOR_REVISION_VALUE) }
            .context("InitializeSecurityDescriptor failed")?;
        // SAFETY: sd는 init 완료, acl_buf는 우리가 살려두는 ACL을 가리킨다.
        unsafe {
            SetSecurityDescriptorDacl(sd_ptr, true, Some(acl_buf.as_ptr().cast::<ACL>()), false)
        }
        .context("SetSecurityDescriptorDacl failed")?;

        let sa = Box::new(SECURITY_ATTRIBUTES {
            nLength: u32::try_from(mem::size_of::<SECURITY_ATTRIBUTES>())
                .context("SECURITY_ATTRIBUTES size overflow")?,
            lpSecurityDescriptor: std::ptr::from_mut::<SECURITY_DESCRIPTOR>(&mut *sd).cast(),
            bInheritHandle: false.into(),
        });

        Ok(Self {
            sa,
            sd,
            acl_buf,
            sid_buf,
        })
    }

    /// `SECURITY_ATTRIBUTES`를 가리키는 raw 포인터.
    ///
    /// `&PipeAcl`이 살아있는 동안만 유효하다. Tokio의
    /// `ServerOptions::create_with_security_attributes_raw`에 전달한다.
    pub fn as_security_attributes_ptr(&self) -> *mut c_void {
        // sa는 Box 안에 있고 heap 위치는 stable. `&*self.sa`가 가리키는
        // 메모리는 self가 drop되지 않는 한 변하지 않는다.
        std::ptr::from_ref::<SECURITY_ATTRIBUTES>(&*self.sa).cast::<c_void>() as *mut c_void
    }

    /// 서버 자신의 SID 포인터(EqualSid에 그대로 넘길 수 있다).
    pub fn server_sid(&self) -> PSID {
        PSID(self.sid_buf.as_ptr() as *mut c_void)
    }
}

/// 현재 프로세스 토큰에서 사용자 SID 사본을 만들어 돌려준다.
fn current_user_sid_buf() -> Result<Vec<u32>> {
    // SAFETY: GetCurrentProcess는 닫지 않아도 되는 pseudo-handle(-1)을 돌려준다.
    let proc: HANDLE = unsafe { GetCurrentProcess() };
    sid_buf_from_process(proc)
}

/// 임의의 프로세스 핸들에서 SID 사본을 만든다. 토큰 핸들은 본 함수에서
/// 닫는다(`OwnedHandle`).
fn sid_buf_from_process(process: HANDLE) -> Result<Vec<u32>> {
    let mut token = HANDLE::default();
    // SAFETY: process는 유효한 프로세스 핸들, token은 본 스택의 변수.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
        .context("OpenProcessToken failed")?;
    let token_guard = OwnedHandle::new(token);
    sid_buf_from_token(token_guard.as_raw())
}

fn sid_buf_from_token(token: HANDLE) -> Result<Vec<u32>> {
    let mut cb: u32 = 0;
    // 첫 호출은 크기만 알아온다. ERROR_INSUFFICIENT_BUFFER로 실패하는 것이 정상.
    // SAFETY: 모든 포인터/옵션이 올바른 형식.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut cb) };
    if cb == 0 {
        anyhow::bail!("GetTokenInformation probe returned size 0");
    }

    let words = (cb as usize).div_ceil(4);
    let mut buf: Vec<u32> = vec![0u32; words];
    // SAFETY: buf는 cb 바이트 이상, u32 정렬 보장.
    unsafe { GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), cb, &mut cb) }
        .context("GetTokenInformation(TokenUser) failed")?;

    // 버퍼 시작에 TOKEN_USER가 놓여 있고, 그 안의 Sid 포인터는 같은 버퍼 안의
    // 어느 위치를 가리킨다 — 사본을 만들어 자기-소유 형태로 바꾼다.
    let tu_ptr: *const TOKEN_USER = buf.as_ptr().cast();
    // SAFETY: GetTokenInformation 성공 후 buf 시작의 TOKEN_USER는 valid initialized.
    let sid_in_buf: PSID = unsafe { (*tu_ptr).User.Sid };
    if sid_in_buf.0.is_null() {
        anyhow::bail!("TOKEN_USER has null SID");
    }
    // SAFETY: sid_in_buf는 유효한 SID 포인터.
    let len = unsafe { GetLengthSid(sid_in_buf) };
    if len == 0 {
        anyhow::bail!("GetLengthSid returned 0");
    }
    let out_words = (len as usize).div_ceil(4);
    let mut out: Vec<u32> = vec![0u32; out_words];
    // SAFETY: out은 len 바이트 이상, sid_in_buf는 유효.
    unsafe { CopySid(len, PSID(out.as_mut_ptr().cast()), sid_in_buf) }.context("CopySid failed")?;
    Ok(out)
}

/// accept된 클라이언트의 SID가 서버의 SID와 일치하는지 확인한다.
///
/// ACL이 이미 다른 사용자 접근을 차단하지만, 정책상 한 번 더 확인한다
/// (`docs/spec/01-ipc-protocol.md` § Client connection flow).
pub fn verify_client_user(server: &NamedPipeServer, expected_sid: PSID) -> Result<()> {
    let raw = server.as_raw_handle();
    let server_handle = HANDLE(raw);

    let mut pid: u32 = 0;
    // SAFETY: server_handle은 tokio가 소유한 살아있는 Named Pipe 핸들.
    //         out 파라미터 `pid`는 본 스택의 변수.
    unsafe { GetNamedPipeClientProcessId(server_handle, &mut pid) }
        .context("GetNamedPipeClientProcessId failed")?;

    // SAFETY: PROCESS_QUERY_LIMITED_INFORMATION은 어떤 사용자라도 자신의
    //         프로세스에 대해 열 수 있는 최소 권한. 결과 핸들은 OwnedHandle에서 닫는다.
    let proc_handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .with_context(|| format!("OpenProcess(client pid={pid})"))?;
    let proc_guard = OwnedHandle::new(proc_handle);

    let client_sid_buf = sid_buf_from_process(proc_guard.as_raw())?;
    let client_sid_ptr = PSID(client_sid_buf.as_ptr() as *mut c_void);

    // SAFETY: 두 PSID 모두 유효한 SID를 가리킨다 (서버 측은 PipeAcl이 보관,
    //         클라이언트 측은 client_sid_buf가 보관). EqualSid는 windows-rs에서
    //         BOOL→Result 자동 wrapper에 의해 `Ok(())`가 곧 "동일", `Err`가
    //         "다르거나 호출 실패"를 의미한다.
    if unsafe { EqualSid(client_sid_ptr, expected_sid) }.is_err() {
        anyhow::bail!("client SID does not match server SID (pid={pid})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn current_user_sid_is_nonempty() {
        let buf = current_user_sid_buf().expect("sid");
        assert!(!buf.is_empty());
    }

    #[test]
    fn pipe_acl_builds_for_current_user() {
        let acl = PipeAcl::build_for_current_user().expect("build");
        let ptr = acl.as_security_attributes_ptr();
        assert!(!ptr.is_null());
        // 서버 SID는 우리 자신 — EqualSid가 Ok(())여야 한다(같은 SID).
        // SAFETY: PipeAcl이 살아있는 동안 server_sid()는 유효.
        let equal = unsafe { EqualSid(acl.server_sid(), acl.server_sid()) };
        assert!(equal.is_ok(), "self EqualSid should succeed");
    }
}
