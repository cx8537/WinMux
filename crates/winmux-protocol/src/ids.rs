//! 프리픽스가 붙은 ID 뉴타입.
//!
//! 와이어 포맷에서 ID는 `<prefix>-<ULID>` 형태의 문자열로 다닌다
//! (`docs/conventions/naming.md` § IDs). 이 크레이트는 ID를 *생성*하지
//! 않는다 — ULID 생성은 서버 측의 책임이다. 여기서는 단지 모양을
//! 검증하고, 잘못된 ID를 잘못된 위치에 넣지 못하도록 타입으로 구분한다.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// ID 파싱 실패 사유.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdError {
    /// 프리픽스가 기대한 것과 다르다.
    #[error("expected prefix `{expected}-`, got `{actual}`")]
    WrongPrefix {
        /// 기대한 프리픽스.
        expected: &'static str,
        /// 실제로 받은 문자열의 앞부분(혹은 전체).
        actual: String,
    },

    /// 프리픽스 뒤 본문이 비어 있다.
    #[error("identifier body is empty after prefix `{0}-`")]
    EmptyBody(&'static str),
}

/// 한 ID 뉴타입을 정의한다.
///
/// - 와이어 표현은 그대로 `String` (`"<prefix>-<body>"`).
/// - `Display`는 와이어 표현을 그대로 찍는다.
/// - `FromStr`은 프리픽스를 검증한다.
/// - `Serialize`/`Deserialize`는 평범한 문자열 직렬화.
macro_rules! define_prefixed_id {
    ($name:ident, $prefix:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Eq, Hash, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// 와이어용 프리픽스.
            pub const PREFIX: &'static str = $prefix;

            /// 이미 프리픽스가 붙은 문자열을 검증 없이 받는다.
            ///
            /// 호출자가 invariant를 보장할 때만 사용한다. 보통은
            /// `FromStr` 또는 [`Self::from_body`]를 통해 만든다.
            #[must_use]
            pub fn from_raw(raw: String) -> Self {
                Self(raw)
            }

            /// 본문(ULID 등)에 프리픽스를 붙여 새 ID를 만든다.
            ///
            /// 빈 본문은 거절한다.
            pub fn from_body(body: &str) -> Result<Self, IdError> {
                if body.is_empty() {
                    return Err(IdError::EmptyBody($prefix));
                }
                Ok(Self(format!("{}-{}", $prefix, body)))
            }

            /// 와이어 표현 그대로 노출.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// 프리픽스를 제외한 본문(ULID 부분).
            #[must_use]
            pub fn body(&self) -> &str {
                // `FromStr` / `from_body` 경로로만 만들어지면 항상
                // `<prefix>-`로 시작한다. 검증되지 않은 `from_raw`로
                // 들어온 경우엔 그대로 돌려준다.
                self.0
                    .strip_prefix(concat!($prefix, "-"))
                    .unwrap_or(&self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let body =
                    s.strip_prefix(concat!($prefix, "-"))
                        .ok_or_else(|| IdError::WrongPrefix {
                            expected: $prefix,
                            actual: s.to_owned(),
                        })?;
                if body.is_empty() {
                    return Err(IdError::EmptyBody($prefix));
                }
                Ok(Self(s.to_owned()))
            }
        }
    };
}

define_prefixed_id!(SessionId, "ses", "세션 식별자 (`ses-<ULID>`).");
define_prefixed_id!(WindowId, "win", "윈도우 식별자 (`win-<ULID>`).");
define_prefixed_id!(PaneId, "pane", "패널 식별자 (`pane-<ULID>`).");
define_prefixed_id!(ClientId, "cli", "클라이언트 식별자 (`cli-<ULID>`).");
define_prefixed_id!(
    MessageId,
    "msg",
    "요청-응답 상관용 메시지 식별자 (`msg-<ULID>`)."
);

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn from_body_attaches_prefix() {
        let id = SessionId::from_body("01HKJ4Z6PXA7G3M2F9XQ7VWERT").expect("from_body");
        assert_eq!(id.as_str(), "ses-01HKJ4Z6PXA7G3M2F9XQ7VWERT");
        assert_eq!(id.body(), "01HKJ4Z6PXA7G3M2F9XQ7VWERT");
    }

    #[test]
    fn from_str_validates_prefix() {
        let parsed: PaneId = "pane-01HKJ4Z6PXA7G3M2F9XQ7VWERT".parse().expect("FromStr");
        assert_eq!(parsed.body(), "01HKJ4Z6PXA7G3M2F9XQ7VWERT");

        let wrong: Result<PaneId, _> = "ses-deadbeef".parse();
        assert!(matches!(wrong, Err(IdError::WrongPrefix { .. })));
    }

    #[test]
    fn empty_body_is_rejected() {
        let parsed: Result<WindowId, _> = "win-".parse();
        assert!(matches!(parsed, Err(IdError::EmptyBody("win"))));
        let built = WindowId::from_body("");
        assert!(matches!(built, Err(IdError::EmptyBody("win"))));
    }

    #[test]
    fn serde_roundtrips_wire_string() {
        let id = ClientId::from_body("ABCD").expect("from_body");
        let json = serde_json::to_string(&id).expect("ser");
        assert_eq!(json, "\"cli-ABCD\"");
        let back: ClientId = serde_json::from_str(&json).expect("de");
        assert_eq!(back.as_str(), "cli-ABCD");
    }
}
