//! library cache entry content hash 문자열 (XV.3.1).
//!
//! 형식은 ADR-0004(V.1.5)의 `ContentHash` 문자열과 같은 `sha256:<64자리 소문자 16진수>`입니다.
//! hash 계산은 storage crate의 SHA-256 구현을 project가 연결합니다(이 crate는 core-types에만
//! 의존). 이 타입은 형식만 검증합니다.

use std::fmt;
use std::str::FromStr;

use crate::node::ValueError;

/// 알고리즘 접두사.
pub const SHA256_PREFIX: &str = "sha256:";

/// `sha256:<64 hex>` content hash 문자열.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHashText(String);

impl ContentHashText {
    /// 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 32바이트 digest로 만듭니다.
    pub fn from_sha256_digest(digest: [u8; 32]) -> ContentHashText {
        let mut text = String::with_capacity(SHA256_PREFIX.len() + 64);
        text.push_str(SHA256_PREFIX);
        for byte in digest {
            text.push_str(&format!("{byte:02x}"));
        }
        ContentHashText(text)
    }
}

impl FromStr for ContentHashText {
    type Err = ValueError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s
            .strip_prefix(SHA256_PREFIX)
            .ok_or(ValueError::InvalidContentHash)?;
        if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(ValueError::InvalidContentHash);
        }
        Ok(ContentHashText(s.to_string()))
    }
}

impl fmt::Display for ContentHashText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ContentHashText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHashText({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_is_sha256_lowercase_hex() {
        let ok = format!("sha256:{}", "0a".repeat(32));
        assert_eq!(ok.parse::<ContentHashText>().unwrap().as_str(), ok);
        assert_eq!(ContentHashText::from_sha256_digest([0x0a; 32]).as_str(), ok);
        for bad in [
            format!("sha256:{}", "0A".repeat(32)),
            format!("sha256:{}", "0a".repeat(31)),
            format!("sha1:{}", "0a".repeat(32)),
            "0a".repeat(32),
            "sha256:".to_string(),
        ] {
            assert_eq!(
                bad.parse::<ContentHashText>(),
                Err(ValueError::InvalidContentHash),
                "{bad}"
            );
        }
    }
}
