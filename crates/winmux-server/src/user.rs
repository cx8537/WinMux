//! 현재 사용자 식별 — 파이프·뮤텍스 이름에 들어가는 `user_sha8`.
//!
//! 사용자 이름은 `USERNAME` 환경변수에서 가져온다. Windows에서는
//! 로그인 세션이 항상 이 변수를 설정하므로, 없으면 비정상 환경으로
//! 간주하고 오류를 반환한다.
//!
//! `user_sha8`은 SHA-256(username)의 처음 8 hex 글자. 이 값에 보안이
//! 의존하지 않는다 — 실제 접근 제어는 파이프의 ACL이 담당한다. 단지
//! Named Pipe 이름에 들어갈 수 없는 글자(예: 공백, 백슬래시 등)가
//! 사용자명에 들어 있는 경우를 피하기 위한 결정론적 ID이다.
//!
//! 동일한 prefix가 단일-인스턴스 뮤텍스 이름에도 쓰이므로 두 식별자가
//! 항상 페어로 움직인다.

use anyhow::{Context, Result};

use crate::sha256;

/// 와이어용 사용자 식별자 묶음.
#[derive(Clone, Debug)]
pub struct UserIdentity {
    /// 현재 프로세스의 `USERNAME`.
    pub username: String,
    /// SHA-256(username) 처음 8 hex 글자.
    pub user_sha8: String,
}

impl UserIdentity {
    /// 현재 프로세스 환경에서 식별자를 추출한다.
    ///
    /// `USERNAME`이 비어 있거나 누락되면 `Err`.
    pub fn detect() -> Result<Self> {
        let username =
            std::env::var("USERNAME").context("USERNAME environment variable is not set")?;
        if username.is_empty() {
            anyhow::bail!("USERNAME environment variable is empty");
        }
        let hex = sha256::sha256_hex(username.as_bytes());
        // SHA-256 hex는 항상 64자이므로 .get(..8)은 절대 None이 아니다.
        let user_sha8 = hex
            .get(..8)
            .context("SHA-256 hex output shorter than 8 chars (unreachable)")?
            .to_owned();
        Ok(Self {
            username,
            user_sha8,
        })
    }

    /// 명시적 사용자명으로 식별자를 만든다. 테스트에서 결정론적인 값을
    /// 쓸 때 사용한다.
    #[must_use]
    pub fn for_username(username: &str) -> Self {
        let hex = sha256::sha256_hex(username.as_bytes());
        let user_sha8 = hex.get(..8).unwrap_or("00000000").to_owned();
        Self {
            username: username.to_owned(),
            user_sha8,
        }
    }

    /// Named Pipe 이름 (`\\.\pipe\winmux-{user_sha8}`).
    #[must_use]
    pub fn pipe_name(&self) -> String {
        format!(r"\\.\pipe\winmux-{}", self.user_sha8)
    }

    /// 단일-인스턴스 뮤텍스 이름 (`Local\WinMux-Server-{user_sha8}`).
    #[must_use]
    pub fn mutex_name(&self) -> String {
        format!(r"Local\WinMux-Server-{}", self.user_sha8)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn for_username_produces_eight_hex_prefix() {
        let id = UserIdentity::for_username("alice");
        assert_eq!(id.user_sha8.len(), 8);
        assert!(id.user_sha8.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn pipe_and_mutex_names_share_prefix() {
        let id = UserIdentity::for_username("bob");
        assert!(id.pipe_name().ends_with(&id.user_sha8));
        assert!(id.mutex_name().ends_with(&id.user_sha8));
        assert!(id.pipe_name().starts_with(r"\\.\pipe\winmux-"));
        assert!(id.mutex_name().starts_with(r"Local\WinMux-Server-"));
    }

    #[test]
    fn different_usernames_produce_different_prefixes() {
        let a = UserIdentity::for_username("alice");
        let b = UserIdentity::for_username("bob");
        assert_ne!(a.user_sha8, b.user_sha8);
    }
}
