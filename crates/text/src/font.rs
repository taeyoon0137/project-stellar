//! 공통 font face record. 정의와 형식 테스트는 `stellar_core_types::font`에 있습니다.
//! 아래 helper는 이 crate의 다른 테스트가 같은 값을 만들 때 씁니다.
pub use stellar_core_types::font::*;

#[cfg(test)]
pub(crate) mod tests {
    use stellar_core_types::value::FiniteF32;

    use super::*;

    /// 테스트용 axis tag.
    pub(crate) fn tag(s: &str) -> OpenTypeTag {
        OpenTypeTag::parse(s).expect("4글자 ASCII tag")
    }

    /// 테스트용 axis 설정.
    pub(crate) fn axis(s: &str, value: f32) -> FontVariation {
        FontVariation {
            tag: tag(s),
            value: FiniteF32::new(value).expect("유한 값"),
        }
    }

    /// 모든 byte가 `fill`인 테스트용 file hash.
    pub(crate) fn hash(fill: u8) -> FontFileHash {
        FontFileHash::from_sha256_digest([fill; 32])
    }
}
