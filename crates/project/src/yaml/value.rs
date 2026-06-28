//! 제한 YAML 값 트리 (IV.1).
//!
//! [`YamlValue`]는 제한 YAML(매핑/시퀀스/스칼라)의 메모리 표현입니다. 텍스트 → 값 트리 파싱은
//! IV.1.2의 제한 loader가, 값 트리 → canonical 텍스트 기록은 IV.1.5의 writer가 담당합니다.
//! 본 모듈은 값 트리 타입과 조회 헬퍼만 정의합니다.

/// 제한 YAML 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YamlValue {
    /// 스칼라(문자열로 보존, 암시적 타입 추론 없음 — IV.1.4).
    Scalar(String),
    /// 시퀀스.
    Sequence(Vec<YamlValue>),
    /// 매핑(키 순서 보존).
    Mapping(Vec<(YamlValue, YamlValue)>),
}

impl YamlValue {
    /// 스칼라를 만듭니다.
    pub fn scalar(text: impl Into<String>) -> YamlValue {
        YamlValue::Scalar(text.into())
    }

    /// 스칼라 문자열이면 반환합니다.
    pub fn as_scalar(&self) -> Option<&str> {
        match self {
            YamlValue::Scalar(s) => Some(s),
            _ => None,
        }
    }

    /// 시퀀스이면 슬라이스를 반환합니다.
    pub fn as_sequence(&self) -> Option<&[YamlValue]> {
        match self {
            YamlValue::Sequence(items) => Some(items),
            _ => None,
        }
    }

    /// 매핑이면 키-값 쌍을 반환합니다.
    pub fn as_mapping(&self) -> Option<&[(YamlValue, YamlValue)]> {
        match self {
            YamlValue::Mapping(pairs) => Some(pairs),
            _ => None,
        }
    }

    /// 매핑에서 스칼라 키로 값을 찾습니다(매핑이 아니거나 키가 없으면 `None`).
    pub fn get(&self, key: &str) -> Option<&YamlValue> {
        let YamlValue::Mapping(pairs) = self else {
            return None;
        };
        pairs
            .iter()
            .find(|(k, _)| k.as_scalar() == Some(key))
            .map(|(_, v)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_accessor() {
        assert_eq!(YamlValue::scalar("x").as_scalar(), Some("x"));
        assert_eq!(YamlValue::Sequence(vec![]).as_scalar(), None);
    }

    #[test]
    fn mapping_get_by_scalar_key() {
        let map = YamlValue::Mapping(vec![
            (YamlValue::scalar("a"), YamlValue::scalar("1")),
            (YamlValue::scalar("b"), YamlValue::Sequence(vec![])),
        ]);
        assert_eq!(map.get("a").and_then(YamlValue::as_scalar), Some("1"));
        assert!(map.get("b").unwrap().as_sequence().is_some());
        assert!(map.get("missing").is_none());
        // 매핑이 아니면 None.
        assert!(YamlValue::scalar("x").get("a").is_none());
    }
}
