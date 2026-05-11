//! 프로토콜 수준 오류와 와이어 `Error` 페이로드.
//!
//! [`ProtocolError`]는 디코더가 내부적으로 반환하는 Rust enum이고,
//! [`ErrorPayload`]는 그 결과를 클라이언트에 돌려보낼 때의 JSON 모양이다.
//! 코드 문자열은 `docs/spec/01-ipc-protocol.md` § Errors 표와 1:1 매칭.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ids::MessageId;

/// 와이어에 실리는 오류 코드.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[non_exhaustive]
pub enum ErrorCode {
    /// 클라이언트와 서버의 프로토콜 버전이 호환되지 않음.
    VersionMismatch,
    /// 메시지 순서나 형식이 잘못됨 (예: Hello 전 다른 메시지).
    ProtocolViolation,
    /// 이 프로토콜 버전이 모르는 `type` 값.
    UnknownMessageType,
    /// 참조한 세션이 없음.
    SessionNotFound,
    /// 참조한 패널이 없음.
    PaneNotFound,
    /// 클라이언트 SID가 서버 사용자와 일치하지 않음.
    PermissionDenied,
    /// 한 메시지가 16 MiB를 초과.
    TooLarge,
    /// 요청이 시간 안에 응답을 받지 못함.
    Timeout,
    /// 서버 내부 버그. 로그에 자세한 내용.
    Internal,
}

impl ErrorCode {
    /// 와이어에 쓰이는 문자열 표현.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VersionMismatch => "VERSION_MISMATCH",
            Self::ProtocolViolation => "PROTOCOL_VIOLATION",
            Self::UnknownMessageType => "UNKNOWN_MESSAGE_TYPE",
            Self::SessionNotFound => "SESSION_NOT_FOUND",
            Self::PaneNotFound => "PANE_NOT_FOUND",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::TooLarge => "TOO_LARGE",
            Self::Timeout => "TIMEOUT",
            Self::Internal => "INTERNAL",
        }
    }
}

/// 와이어용 오류 페이로드.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorPayload {
    /// 원인이 된 요청의 `id`. 자발적으로 끊을 때는 `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<MessageId>,
    /// 분류 코드.
    pub code: ErrorCode,
    /// 사람이 읽을 수 있는 설명.
    pub message: String,
    /// `false`면 서버가 곧 연결을 끊는다.
    pub recoverable: bool,
}

/// 프로토콜 디코더 내부에서 쓰는 Rust 측 오류 enum.
///
/// 와이어 표현이 아니다. 디코더가 이 값을 만들어 호출자에게 돌려주면,
/// 서버는 적절한 [`ErrorPayload`]로 변환해 클라이언트에 전송한다.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProtocolError {
    /// 알 수 없는 `type` 필드.
    #[error("unknown message type `{0}`")]
    UnknownType(String),

    /// `Hello` 이전에 다른 메시지가 들어옴 등 상태 머신 위반.
    #[error("protocol violation: {0}")]
    Violation(String),

    /// 클라이언트 버전이 받아들일 수 없는 범위.
    #[error("version mismatch: client v{client}, server accepts v{accepted_min}..=v{accepted_max}")]
    VersionMismatch {
        /// 클라이언트가 알린 버전.
        client: u32,
        /// 서버가 받아들이는 최소 버전.
        accepted_min: u32,
        /// 서버가 받아들이는 최대 버전.
        accepted_max: u32,
    },

    /// 메시지가 16 MiB를 초과.
    #[error("message exceeds {limit} bytes (was {actual})")]
    TooLarge {
        /// 한도.
        limit: usize,
        /// 실제 크기.
        actual: usize,
    },

    /// serde 등에서 올라온 역직렬화 실패.
    #[error("deserialize failed: {0}")]
    Deserialize(String),
}

impl ProtocolError {
    /// 이 오류를 와이어용 페이로드로 변환한다.
    #[must_use]
    pub fn to_payload(&self, request_id: Option<MessageId>) -> ErrorPayload {
        let (code, recoverable) = match self {
            Self::UnknownType(_) => (ErrorCode::UnknownMessageType, true),
            Self::Violation(_) => (ErrorCode::ProtocolViolation, false),
            Self::VersionMismatch { .. } => (ErrorCode::VersionMismatch, false),
            Self::TooLarge { .. } => (ErrorCode::TooLarge, false),
            Self::Deserialize(_) => (ErrorCode::ProtocolViolation, false),
        };
        ErrorPayload {
            id: request_id,
            code,
            message: self.to_string(),
            recoverable,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn error_code_serializes_as_screaming_snake() {
        let json = serde_json::to_string(&ErrorCode::ProtocolViolation).expect("ser");
        assert_eq!(json, "\"PROTOCOL_VIOLATION\"");
    }

    #[test]
    fn error_code_as_str_matches_wire() {
        assert_eq!(ErrorCode::TooLarge.as_str(), "TOO_LARGE");
        assert_eq!(ErrorCode::VersionMismatch.as_str(), "VERSION_MISMATCH");
    }

    #[test]
    fn protocol_error_maps_to_payload() {
        let err = ProtocolError::VersionMismatch {
            client: 2,
            accepted_min: 1,
            accepted_max: 1,
        };
        let payload = err.to_payload(None);
        assert_eq!(payload.code, ErrorCode::VersionMismatch);
        assert!(!payload.recoverable);
    }
}
