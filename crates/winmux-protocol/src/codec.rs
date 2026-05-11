//! JSON-Lines 프레이밍 보조 도구와 base64 헬퍼.
//!
//! 이 크레이트는 메시지를 직접 직렬화하지 않는다. 호출자(서버·클라이언트)가
//! 자신의 JSON 인코더로 UTF-8 바이트를 만든 뒤, [`encode_line`]으로 줄바꿈을
//! 붙이고, [`decode_line`]으로 들어온 한 줄에서 본문을 떼어내고 크기를
//! 검증한다.
//!
//! 의존성을 최소로 유지하기 위해 base64는 표준 라이브러리만으로 구현했다.
//! 알파벳은 RFC 4648 §4(표준 base64, 패딩 `=` 포함).

use std::fmt;

/// 한 메시지가 가질 수 있는 최대 바이트 수 (`docs/spec/01-ipc-protocol.md` § Framing).
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// JSON-Lines 프레이밍 오류.
#[derive(Debug)]
pub enum CodecError {
    /// 메시지가 16 MiB를 초과.
    TooLarge {
        /// 한도.
        limit: usize,
        /// 실제 크기.
        actual: usize,
    },
    /// 한 줄 안에 줄바꿈이 두 번 이상 들어 있음.
    EmbeddedNewline,
    /// 들어온 바이트가 유효한 UTF-8이 아님.
    NotUtf8,
    /// base64 알파벳에 없는 문자가 들어 있거나 길이가 4의 배수가 아님.
    InvalidBase64,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { limit, actual } => {
                write!(f, "message exceeds {limit} bytes (was {actual})")
            }
            Self::EmbeddedNewline => f.write_str("message body contains an embedded newline"),
            Self::NotUtf8 => f.write_str("message is not valid UTF-8"),
            Self::InvalidBase64 => f.write_str("invalid base64 payload"),
        }
    }
}

impl std::error::Error for CodecError {}

/// 한 메시지(UTF-8 JSON 바이트)를 와이어 형식 한 줄로 만든다.
///
/// 입력에 줄바꿈(`\n`)이 들어 있으면 [`CodecError::EmbeddedNewline`].
/// 크기 한도를 넘으면 [`CodecError::TooLarge`].
pub fn encode_line(body: &[u8]) -> Result<Vec<u8>, CodecError> {
    if body.len() >= MAX_MESSAGE_BYTES {
        return Err(CodecError::TooLarge {
            limit: MAX_MESSAGE_BYTES,
            actual: body.len(),
        });
    }
    if body.contains(&b'\n') {
        return Err(CodecError::EmbeddedNewline);
    }
    let mut out = Vec::with_capacity(body.len() + 1);
    out.extend_from_slice(body);
    out.push(b'\n');
    Ok(out)
}

/// 들어온 한 줄(끝에 `\n`이 있거나 없을 수 있음)에서 본문을 떼어낸다.
///
/// 결과는 UTF-8 문자열로 반환한다. 크기 한도 검사도 여기서 수행한다.
pub fn decode_line(line: &[u8]) -> Result<&str, CodecError> {
    let body = line.strip_suffix(b"\n").unwrap_or(line);
    if body.len() > MAX_MESSAGE_BYTES {
        return Err(CodecError::TooLarge {
            limit: MAX_MESSAGE_BYTES,
            actual: body.len(),
        });
    }
    std::str::from_utf8(body).map_err(|_| CodecError::NotUtf8)
}

// ---------------------------------------------------------------------------
// base64 (RFC 4648 §4)
// ---------------------------------------------------------------------------

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 원시 바이트를 표준 base64 문자열로 인코딩 (`+/`, 패딩 `=`).
#[must_use]
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in chunks.by_ref() {
        // chunk.len() == 3 임이 보장됨 — chunks_exact의 invariant.
        let b0 = chunk[0];
        let b1 = chunk[1];
        let b2 = chunk[2];
        let n: u32 = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPHABET[(n & 0x3F) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        0 => {}
        1 => {
            let b0 = rem[0];
            let n: u32 = u32::from(b0) << 16;
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b0 = rem[0];
            let b1 = rem[1];
            let n: u32 = (u32::from(b0) << 16) | (u32::from(b1) << 8);
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        // chunks_exact(3) 의 나머지는 0..=2 이므로 그 외 길이는 불가능하다.
        _ => unreachable!("chunks_exact(3) remainder is always 0..=2"),
    }
    out
}

/// 표준 base64 문자열을 원시 바이트로 디코딩.
///
/// 공백·줄바꿈은 허용하지 않는다 (와이어상에는 들어올 일이 없다).
pub fn base64_decode(text: &str) -> Result<Vec<u8>, CodecError> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(CodecError::InvalidBase64);
    }
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks_exact(4) {
        let c0 = chunk[0];
        let c1 = chunk[1];
        let c2 = chunk[2];
        let c3 = chunk[3];

        let v0 = decode_char(c0)?;
        let v1 = decode_char(c1)?;
        // 처음 두 글자는 항상 데이터 글자(패딩 불가).
        let v0 = v0.ok_or(CodecError::InvalidBase64)?;
        let v1 = v1.ok_or(CodecError::InvalidBase64)?;

        match (decode_char(c2)?, decode_char(c3)?) {
            (None, None) => {
                out.push((v0 << 2) | (v1 >> 4));
            }
            (Some(v2), None) => {
                out.push((v0 << 2) | (v1 >> 4));
                out.push(((v1 & 0x0F) << 4) | (v2 >> 2));
            }
            (Some(v2), Some(v3)) => {
                out.push((v0 << 2) | (v1 >> 4));
                out.push(((v1 & 0x0F) << 4) | (v2 >> 2));
                out.push(((v2 & 0x03) << 6) | v3);
            }
            // `=` 뒤에 데이터 글자가 오는 형태는 잘못된 입력이다.
            (None, Some(_)) => return Err(CodecError::InvalidBase64),
        }
    }
    Ok(out)
}

/// 한 base64 글자를 6비트 값으로 변환. 패딩(`=`)이면 `Ok(None)`.
fn decode_char(c: u8) -> Result<Option<u8>, CodecError> {
    match c {
        b'A'..=b'Z' => Ok(Some(c - b'A')),
        b'a'..=b'z' => Ok(Some(c - b'a' + 26)),
        b'0'..=b'9' => Ok(Some(c - b'0' + 52)),
        b'+' => Ok(Some(62)),
        b'/' => Ok(Some(63)),
        b'=' => Ok(None),
        _ => Err(CodecError::InvalidBase64),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn encode_line_appends_newline() {
        let out = encode_line(b"{\"v\":1}").expect("encode");
        assert_eq!(out, b"{\"v\":1}\n");
    }

    #[test]
    fn encode_line_rejects_embedded_newline() {
        let bad = encode_line(b"{\"v\":1}\nextra");
        assert!(matches!(bad, Err(CodecError::EmbeddedNewline)));
    }

    #[test]
    fn encode_line_rejects_oversize() {
        let huge = vec![b'a'; MAX_MESSAGE_BYTES + 1];
        let bad = encode_line(&huge);
        assert!(matches!(bad, Err(CodecError::TooLarge { .. })));
    }

    #[test]
    fn decode_line_strips_newline() {
        let s = decode_line(b"{\"v\":1}\n").expect("decode");
        assert_eq!(s, "{\"v\":1}");
    }

    #[test]
    fn decode_line_rejects_non_utf8() {
        let bad = decode_line(&[0xFF, 0xFE, 0xFD]);
        assert!(matches!(bad, Err(CodecError::NotUtf8)));
    }

    #[test]
    fn base64_known_vectors() {
        // RFC 4648 §10.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_decode_known_vectors() {
        assert_eq!(base64_decode("").expect("0"), b"");
        assert_eq!(base64_decode("Zg==").expect("1"), b"f");
        assert_eq!(base64_decode("Zm8=").expect("2"), b"fo");
        assert_eq!(base64_decode("Zm9v").expect("3"), b"foo");
        assert_eq!(base64_decode("Zm9vYg==").expect("4"), b"foob");
        assert_eq!(base64_decode("Zm9vYmE=").expect("5"), b"fooba");
        assert_eq!(base64_decode("Zm9vYmFy").expect("6"), b"foobar");
    }

    #[test]
    fn base64_roundtrip_random_bytes() {
        // 결정론적 의사 난수: 시드 고정으로 매 실행 같은 값.
        let mut buf = Vec::with_capacity(257);
        let mut state: u32 = 0x9E37_79B9;
        for _ in 0..257 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            buf.push((state >> 24) as u8);
        }
        let enc = base64_encode(&buf);
        let dec = base64_decode(&enc).expect("roundtrip");
        assert_eq!(dec, buf);
    }

    #[test]
    fn base64_decode_rejects_garbage() {
        assert!(matches!(
            base64_decode("####"),
            Err(CodecError::InvalidBase64)
        ));
        assert!(matches!(
            base64_decode("abc"),
            Err(CodecError::InvalidBase64)
        ));
        // `=` 뒤 데이터 글자.
        assert!(matches!(
            base64_decode("A=BC"),
            Err(CodecError::InvalidBase64)
        ));
    }
}
