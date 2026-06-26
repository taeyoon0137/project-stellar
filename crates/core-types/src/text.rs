//! 문자열 값과 정규화 경계 (III.3.4).
//!
//! 모든 문자열 값은 UTF-8(`String`)입니다. Unicode NFC 정규화는 **선택적으로**, 그리고
//! 타입으로 분리된 위치에서만 적용합니다:
//! - [`UserText`]: 사용자 텍스트(레이어 이름, 텍스트 내용 등). **정규화하지 않고 그대로 보존**합니다.
//! - [`NormalizedText`]: 식별자·파일 경로. 생성 시 주입된 [`Normalizer`]로 정규화합니다.
//!
//! core-types는 std 전용이라 NFC 테이블을 포함하지 않습니다. 실제 NFC 알고리즘은 상위
//! 계층에서 [`Normalizer`]로 주입합니다(정규화를 적용할 위치만 여기서 분리·고정).

/// 정규화기. 식별자·경로에 적용할 Unicode 정규화(NFC)의 주입 지점입니다.
pub trait Normalizer {
    /// 입력 문자열을 정규화해 반환합니다.
    fn normalize(&self, input: &str) -> String;
}

/// 정규화하지 않는 placeholder 정규화기. 실제 NFC 정규화기로 대체될 자리입니다.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityNormalizer;

impl Normalizer for IdentityNormalizer {
    fn normalize(&self, input: &str) -> String {
        input.to_string()
    }
}

/// 사용자 텍스트. 입력 UTF-8을 정규화 없이 그대로 보존합니다.
///
/// 구조적으로 정규화기를 받지 않으므로 사용자 텍스트는 절대 정규화되지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserText(String);

impl UserText {
    /// 입력을 그대로 보존하는 사용자 텍스트를 만듭니다.
    pub fn new(text: &str) -> UserText {
        UserText(text.to_string())
    }

    /// 보존된 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 내부 문자열로 변환합니다.
    pub fn into_string(self) -> String {
        self.0
    }
}

/// 정규화된 텍스트(식별자·파일 경로). 생성 시 주입된 정규화기로 정규화합니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NormalizedText(String);

impl NormalizedText {
    /// 주입된 정규화기로 입력을 정규화해 만듭니다.
    pub fn normalized<N: Normalizer>(raw: &str, normalizer: &N) -> NormalizedText {
        NormalizedText(normalizer.normalize(raw))
    }

    /// 정규화된 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 정규화기: "e + combining acute"(U+0065 U+0301)를 NFC 합성형 "é"(U+00E9)로 매핑.
    /// 실제 NFC의 한 사례를 흉내냅니다(std 전용 환경에서 정규화 적용 자체를 검증하기 위함).
    struct TestNfc;
    impl Normalizer for TestNfc {
        fn normalize(&self, input: &str) -> String {
            input.replace("e\u{0301}", "\u{00E9}")
        }
    }

    #[test]
    fn user_text_is_preserved_exactly() {
        // 분해형 시퀀스(e + combining acute)도 정규화 없이 그대로 보존.
        let raw = "e\u{0301} layer";
        let user = UserText::new(raw);
        assert_eq!(user.as_str(), raw);
        assert_eq!(user.as_str().chars().count(), raw.chars().count());
        assert_eq!(user.into_string(), raw);
    }

    #[test]
    fn normalized_text_applies_normalizer() {
        let raw = "e\u{0301}"; // 2 code points (분해형)
        let normalized = NormalizedText::normalized(raw, &TestNfc);
        assert_eq!(normalized.as_str(), "\u{00E9}"); // 1 code point (합성형)
        assert_eq!(normalized.as_str().chars().count(), 1);
    }

    #[test]
    fn separation_user_preserved_path_normalized() {
        let raw = "cafe\u{0301}"; // "café"의 분해형
                                  // 같은 입력이라도 사용자 텍스트는 보존, 경로/식별자는 정규화 → 결과가 다름.
        let user = UserText::new(raw);
        let path = NormalizedText::normalized(raw, &TestNfc);
        assert_eq!(user.as_str(), raw);
        assert_eq!(path.as_str(), "caf\u{00E9}");
        assert_ne!(user.as_str(), path.as_str());
    }

    #[test]
    fn identity_normalizer_is_noop() {
        let raw = "e\u{0301}";
        assert_eq!(
            NormalizedText::normalized(raw, &IdentityNormalizer).as_str(),
            raw
        );
    }
}
