//! `send-keys`가 받는 tmux 스타일 키 토큰을 PTY로 보낼 바이트 시퀀스로
//! 변환한다.
//!
//! M0 PoC 단계의 매핑은 의도적으로 **최소 셋**으로만 한정한다. 풀 tmux
//! 호환 키 표는 `docs/spec/04-key-handling.md` § Key Tables를 따라 M1에서
//! 확장한다.
//!
//! 지원 토큰:
//! - `Enter` → `\r`
//! - `Tab`   → `\t`
//! - `Up`    → `\x1b[A`
//! - `Down`  → `\x1b[B`
//! - `Right` → `\x1b[C`
//! - `Left`  → `\x1b[D`
//! - `C-c`   → `\x03` (= ETX, SIGINT를 셸이 받음)
//!
//! 그 외 토큰은 **literal**로 취급되어 UTF-8 바이트가 그대로 PTY에 흘러간다.
//! 따라서 `winmux send-keys -t work:0 "echo hello" Enter`는 `echo hello\r`을 보낸다.
//!
//! 인식하지 못하는 special key 형태(예: `M-x`, `Pageup`)는 에러로 반환한다 —
//! literal로 그냥 흘리면 사용자가 의도와 다른 결과를 보게 된다.

use thiserror::Error;

/// 키 토큰 변환 실패 사유.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum KeyError {
    /// 알려진 형식이지만 본 PoC에서 아직 매핑하지 않은 special key.
    #[error("unsupported key name in M0 PoC: `{0}`")]
    Unsupported(String),
}

/// 한 토큰을 PTY 바이트 시퀀스로 변환한다.
///
/// 토큰이 "특수 이름처럼 보이지만 알 수 없는 형태"(예: `M-x`, `F1`, `C-?`)면
/// [`KeyError::Unsupported`]를 돌려준다. 평범한 텍스트는 그대로 UTF-8 바이트로
/// 흘러간다.
pub fn token_to_bytes(token: &str) -> Result<Vec<u8>, KeyError> {
    // 명시적 셋. 매우 작은 표 — `match`로도 충분하다.
    match token {
        "Enter" => Ok(b"\r".to_vec()),
        "Tab" => Ok(b"\t".to_vec()),
        "Up" => Ok(b"\x1b[A".to_vec()),
        "Down" => Ok(b"\x1b[B".to_vec()),
        "Right" => Ok(b"\x1b[C".to_vec()),
        "Left" => Ok(b"\x1b[D".to_vec()),
        "C-c" => Ok(vec![0x03]),
        _ => {
            // "C-x", "M-x", 1글자 식별자처럼 special key를 의도한 듯한 형태는
            // 거부한다 — literal로 흘리면 사용자 의도와 어긋날 가능성이 크다.
            // 보수적 휴리스틱: 2~12자 사이의 ASCII alnum/하이픈만으로 구성되고
            // 첫 글자가 대문자거나 (대문자+하이픈+...) 패턴이면 special.
            if looks_like_special_name(token) {
                Err(KeyError::Unsupported(token.to_owned()))
            } else {
                Ok(token.as_bytes().to_vec())
            }
        }
    }
}

/// 여러 토큰을 순서대로 변환해 하나의 바이트 벡터로 잇는다.
pub fn tokens_to_bytes(tokens: &[String]) -> Result<Vec<u8>, KeyError> {
    let mut out = Vec::new();
    for t in tokens {
        out.extend_from_slice(&token_to_bytes(t)?);
    }
    Ok(out)
}

/// "특수 키 이름 같이 생겼는데 모르겠음" 휴리스틱.
///
/// 일반 입력 문자열(`"echo hello"`, 한국어, 숫자, 공백 포함)은 false. `M-x`,
/// `Pageup`, `F1`, `C-x` 같은 형태는 true.
fn looks_like_special_name(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    // 공백·구두점이 들어가면 literal 텍스트.
    if token
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '/' | '`'))
    {
        return false;
    }
    // C-x / M-x 형태: 정확히 3자, 두 번째가 '-'.
    if token.len() == 3 {
        let bytes = token.as_bytes();
        if (bytes[0] == b'C' || bytes[0] == b'M') && bytes[1] == b'-' && bytes[2].is_ascii_graphic()
        {
            return true;
        }
    }
    // F1 ~ F12 같은 functional key.
    if let Some(rest) = token.strip_prefix('F')
        && let Ok(n) = rest.parse::<u8>()
        && (1..=24).contains(&n)
    {
        return true;
    }
    // 알파벳 한 단어인데 대문자로 시작 & 길이 ≥ 2이며 ASCII alpha만으로 구성:
    // Pageup / Home / End / Escape 같은 특수 키 이름.
    let bytes = token.as_bytes();
    if bytes[0].is_ascii_uppercase()
        && bytes.len() >= 2
        && bytes.iter().all(|b| b.is_ascii_alphabetic())
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn enter_maps_to_cr() {
        assert_eq!(token_to_bytes("Enter").unwrap(), b"\r");
    }

    #[test]
    fn tab_maps_to_tab() {
        assert_eq!(token_to_bytes("Tab").unwrap(), b"\t");
    }

    #[test]
    fn arrow_keys_map_to_csi_sequences() {
        assert_eq!(token_to_bytes("Up").unwrap(), b"\x1b[A");
        assert_eq!(token_to_bytes("Down").unwrap(), b"\x1b[B");
        assert_eq!(token_to_bytes("Right").unwrap(), b"\x1b[C");
        assert_eq!(token_to_bytes("Left").unwrap(), b"\x1b[D");
    }

    #[test]
    fn ctrl_c_maps_to_etx() {
        assert_eq!(token_to_bytes("C-c").unwrap(), vec![0x03]);
    }

    #[test]
    fn ordinary_text_passes_through_as_utf8() {
        assert_eq!(token_to_bytes("echo hello").unwrap(), b"echo hello");
        // CJK utf-8.
        assert_eq!(token_to_bytes("안녕").unwrap(), "안녕".as_bytes());
    }

    #[test]
    fn unsupported_special_names_return_error() {
        assert!(matches!(
            token_to_bytes("M-x"),
            Err(KeyError::Unsupported(_))
        ));
        assert!(matches!(
            token_to_bytes("F1"),
            Err(KeyError::Unsupported(_))
        ));
        assert!(matches!(
            token_to_bytes("Pageup"),
            Err(KeyError::Unsupported(_))
        ));
        assert!(matches!(
            token_to_bytes("C-x"),
            Err(KeyError::Unsupported(_))
        ));
    }

    #[test]
    fn tokens_to_bytes_concatenates_in_order() {
        let toks = vec!["echo hello".to_owned(), "Enter".to_owned()];
        assert_eq!(tokens_to_bytes(&toks).unwrap(), b"echo hello\r");
    }

    #[test]
    fn tokens_to_bytes_propagates_first_error() {
        let toks = vec!["echo".to_owned(), "F2".to_owned()];
        assert!(matches!(
            tokens_to_bytes(&toks),
            Err(KeyError::Unsupported(_))
        ));
    }
}
