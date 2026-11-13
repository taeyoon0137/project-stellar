//! 에셋 media type (XIII.5.1).
//!
//! `type/subtype` essence만 보관합니다(파라미터 없음). 이름 문자 규칙은 RFC 6838
//! `restricted-name`을 따르고, media type은 대소문자를 구분하지 않으므로 ASCII 소문자로
//! 정규화한 형태를 canonical 값으로 삼습니다. 어떤 media type을 import 허용할지는 정책
//! 문제이며 이 타입은 형식만 검증합니다(허용 목록은 XIII.5.7).

use std::fmt;

/// `type`·`subtype` 각각의 최대 길이(RFC 6838 `restricted-name`: 1 + 126자).
pub const MAX_MEDIA_TYPE_PART_LEN: usize = 127;

/// media type 형식 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaTypeError {
    /// `type/subtype` 형태가 아닙니다(`/`가 없거나 둘 이상).
    MissingSeparator,
    /// `type` 또는 `subtype`이 비어 있습니다.
    EmptyPart,
    /// `type` 또는 `subtype`이 최대 길이를 초과합니다.
    PartTooLong {
        /// 실제 길이(byte).
        len: usize,
    },
    /// 첫 글자가 영문자·숫자가 아닙니다.
    InvalidFirstCharacter {
        /// 위반 문자.
        found: char,
    },
    /// `restricted-name-chars`에 없는 문자입니다(공백·`;` 파라미터 등).
    InvalidCharacter {
        /// 위반 문자.
        found: char,
    },
}

impl fmt::Display for MediaTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaTypeError::MissingSeparator => {
                f.write_str("media type은 'type/subtype' 형태여야 합니다")
            }
            MediaTypeError::EmptyPart => f.write_str("media type의 type·subtype이 비어 있습니다"),
            MediaTypeError::PartTooLong { len } => write!(
                f,
                "media type의 type·subtype 길이가 {MAX_MEDIA_TYPE_PART_LEN}자를 초과합니다: {len}"
            ),
            MediaTypeError::InvalidFirstCharacter { found } => {
                write!(
                    f,
                    "media type 이름은 영문자·숫자로 시작해야 합니다: {found:?}"
                )
            }
            MediaTypeError::InvalidCharacter { found } => {
                write!(f, "media type에 허용되지 않는 문자입니다: {found:?}")
            }
        }
    }
}

impl std::error::Error for MediaTypeError {}

/// 검증된 media type(`type/subtype`, ASCII 소문자 canonical).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaType(String);

impl MediaType {
    /// media type 문자열을 검증하고 ASCII 소문자로 정규화합니다.
    ///
    /// 예: `"Image/PNG"` → `image/png`. 파라미터(`; charset=...`)와 공백은 거부합니다.
    pub fn new(text: &str) -> Result<MediaType, MediaTypeError> {
        let (top, sub) = text
            .split_once('/')
            .ok_or(MediaTypeError::MissingSeparator)?;
        if sub.contains('/') {
            return Err(MediaTypeError::MissingSeparator);
        }
        validate_part(top)?;
        validate_part(sub)?;
        Ok(MediaType(text.to_ascii_lowercase()))
    }

    /// canonical 문자열(`type/subtype`, 소문자).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 최상위 type(예: `image`).
    pub fn top_level(&self) -> &str {
        self.split().0
    }

    /// subtype(예: `png`, `svg+xml`).
    pub fn subtype(&self) -> &str {
        self.split().1
    }

    fn split(&self) -> (&str, &str) {
        // 생성 시 `/`가 정확히 하나임을 검증했습니다.
        self.0.split_once('/').unwrap_or((self.0.as_str(), ""))
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// RFC 6838 `restricted-name` 검사.
fn validate_part(part: &str) -> Result<(), MediaTypeError> {
    let mut chars = part.chars();
    let first = chars.next().ok_or(MediaTypeError::EmptyPart)?;
    if part.len() > MAX_MEDIA_TYPE_PART_LEN {
        return Err(MediaTypeError::PartTooLong { len: part.len() });
    }
    if !first.is_ascii_alphanumeric() {
        return Err(MediaTypeError::InvalidFirstCharacter { found: first });
    }
    for c in chars {
        let allowed = c.is_ascii_alphanumeric()
            || matches!(c, '!' | '#' | '$' | '&' | '-' | '^' | '_' | '.' | '+');
        if !allowed {
            return Err(MediaTypeError::InvalidCharacter { found: c });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_media_types_are_accepted() {
        for text in [
            "image/png",
            "image/svg+xml",
            "font/woff2",
            "application/octet-stream",
            "application/vnd.stellar.asset-v1+json",
            "x-custom/a!#$&^_.+-z",
        ] {
            let media = MediaType::new(text).unwrap();
            assert_eq!(media.as_str(), text);
            assert_eq!(media.to_string(), text);
        }
    }

    #[test]
    fn case_is_normalized_to_lowercase() {
        let media = MediaType::new("Image/SVG+XML").unwrap();
        assert_eq!(media.as_str(), "image/svg+xml");
        assert_eq!(media, MediaType::new("image/svg+xml").unwrap());
        assert_eq!(media.top_level(), "image");
        assert_eq!(media.subtype(), "svg+xml");
    }

    #[test]
    fn separator_errors() {
        for text in ["", "imagepng", "image/png/extra", "a//b"] {
            assert_eq!(
                MediaType::new(text).unwrap_err(),
                MediaTypeError::MissingSeparator,
                "{text:?}"
            );
        }
    }

    #[test]
    fn empty_parts_are_rejected() {
        for text in ["/png", "image/", "/"] {
            assert_eq!(
                MediaType::new(text).unwrap_err(),
                MediaTypeError::EmptyPart,
                "{text:?}"
            );
        }
    }

    #[test]
    fn invalid_characters_are_rejected() {
        assert_eq!(
            MediaType::new("text/plain; charset=utf-8").unwrap_err(),
            MediaTypeError::InvalidCharacter { found: ';' }
        );
        assert_eq!(
            MediaType::new("image/ png").unwrap_err(),
            MediaTypeError::InvalidFirstCharacter { found: ' ' }
        );
        assert_eq!(
            MediaType::new("image/pn g").unwrap_err(),
            MediaTypeError::InvalidCharacter { found: ' ' }
        );
        assert_eq!(
            MediaType::new("+image/png").unwrap_err(),
            MediaTypeError::InvalidFirstCharacter { found: '+' }
        );
        assert_eq!(
            MediaType::new("image/pngé").unwrap_err(),
            MediaTypeError::InvalidCharacter { found: 'é' }
        );
    }

    #[test]
    fn part_length_boundary() {
        let max = "a".repeat(MAX_MEDIA_TYPE_PART_LEN);
        assert!(MediaType::new(&format!("{max}/{max}")).is_ok());
        let over = "a".repeat(MAX_MEDIA_TYPE_PART_LEN + 1);
        assert_eq!(
            MediaType::new(&format!("{over}/png")).unwrap_err(),
            MediaTypeError::PartTooLong { len: 128 }
        );
        assert_eq!(
            MediaType::new(&format!("image/{over}")).unwrap_err(),
            MediaTypeError::PartTooLong { len: 128 }
        );
    }
}
