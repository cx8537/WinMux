//! 프로토콜 버전 상수.
//!
//! 모든 메시지는 최상위 `v` 필드로 버전을 싣는다. 호환되지 않는
//! 변경(필드 이름·의미 변경, 새로운 필수 필드)에서만 PROTOCOL_VERSION을
//! 올린다. 옵션 필드나 새 메시지 타입 추가는 버전을 올리지 않는다.

/// 현재 와이어 프로토콜 버전.
pub const PROTOCOL_VERSION: u32 = 1;

/// 서버가 받아들일 수 있는 가장 낮은 클라이언트 프로토콜 버전.
/// 현재는 단일 버전이므로 PROTOCOL_VERSION과 동일하다.
pub const MIN_COMPATIBLE_VERSION: u32 = 1;

/// 서버가 받아들일 수 있는 가장 높은 클라이언트 프로토콜 버전.
pub const MAX_COMPATIBLE_VERSION: u32 = 1;

/// 호환 검사. 클라이언트가 보고한 버전이 서버가 처리 가능한 범위인지.
#[must_use]
pub const fn is_compatible(client_version: u32) -> bool {
    client_version >= MIN_COMPATIBLE_VERSION && client_version <= MAX_COMPATIBLE_VERSION
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn current_version_is_compatible() {
        assert!(is_compatible(PROTOCOL_VERSION));
    }

    #[test]
    fn future_versions_are_rejected() {
        assert!(!is_compatible(MAX_COMPATIBLE_VERSION + 1));
    }

    #[test]
    fn ancient_versions_are_rejected() {
        assert!(!is_compatible(0));
    }
}
