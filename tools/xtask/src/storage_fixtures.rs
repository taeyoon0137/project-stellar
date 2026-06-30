//! 저장 fixture 스키마 validator (V.1.1).
//!
//! `docs/fixtures/storage/canonical-primitives.yaml`의 구조를 검사합니다: 최상위 `primitives`
//! 시퀀스, 각 항목의 `name`/`type`/`value`/`bytes` 필드, type 허용 집합, bytes 16진수 형식.
//! golden bytes의 **값 정확성**은 검사하지 않습니다(그건 encoder, V.1.2/V.1.4 소관).

use crate::yaml::restricted_loader::{load_restricted, YamlValue};

/// 저장 fixture 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageFixtureError {
    pub code: &'static str,
    pub message: String,
}

impl StorageFixtureError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        StorageFixtureError {
            code,
            message: message.into(),
        }
    }
}

/// 허용 type 집합.
const ALLOWED_TYPES: [&str; 12] = [
    "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "bool", "f32", "f64", "string",
];

const ENTRY_FIELDS: [&str; 4] = ["name", "type", "value", "bytes"];

/// canonical-primitives fixture의 스키마를 검사합니다(빈 Vec = 통과).
pub fn check_canonical_primitives(content: &str) -> Vec<StorageFixtureError> {
    let value = match load_restricted(content) {
        Ok(v) => v,
        Err(e) => {
            return vec![StorageFixtureError::new(
                "parse",
                format!("YAML 파싱 실패: {}", e.message),
            )]
        }
    };

    let YamlValue::Mapping(top) = value else {
        return vec![StorageFixtureError::new(
            "top-level",
            "최상위는 mapping이어야 합니다",
        )];
    };

    let Some(primitives) = mapping_get(&top, "primitives") else {
        return vec![StorageFixtureError::new(
            "missing-primitives",
            "최상위에 primitives 키가 없습니다",
        )];
    };

    let YamlValue::Sequence(items) = primitives else {
        return vec![StorageFixtureError::new(
            "primitives-type",
            "primitives는 sequence여야 합니다",
        )];
    };

    let mut errors = Vec::new();
    if items.is_empty() {
        errors.push(StorageFixtureError::new(
            "empty",
            "primitives가 비어 있습니다",
        ));
    }

    let mut seen_names = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let YamlValue::Mapping(entry) = item else {
            errors.push(StorageFixtureError::new(
                "entry-type",
                format!("primitives[{index}]는 mapping이어야 합니다"),
            ));
            continue;
        };

        // 알 수 없는 키 거부.
        for (key, _) in entry {
            if let YamlValue::Scalar(name) = key {
                if !ENTRY_FIELDS.contains(&name.as_str()) {
                    errors.push(StorageFixtureError::new(
                        "unknown-field",
                        format!("primitives[{index}]에 알 수 없는 키: {name}"),
                    ));
                }
            }
        }

        let name = require_scalar(entry, "name", index, &mut errors);
        let ty = require_scalar(entry, "type", index, &mut errors);
        let _value = require_scalar(entry, "value", index, &mut errors);
        let bytes = require_scalar(entry, "bytes", index, &mut errors);

        if let Some(name) = name {
            if seen_names.contains(&name) {
                errors.push(StorageFixtureError::new(
                    "duplicate-name",
                    format!("중복 name: {name}"),
                ));
            } else {
                seen_names.push(name);
            }
        }

        if let Some(ty) = ty {
            if !ALLOWED_TYPES.contains(&ty) {
                errors.push(StorageFixtureError::new(
                    "bad-type",
                    format!("primitives[{index}] type이 허용 집합 밖: {ty}"),
                ));
            }
        }

        if let Some(bytes) = bytes {
            if let Some(reason) = invalid_hex(bytes) {
                errors.push(StorageFixtureError::new(
                    "bad-bytes",
                    format!("primitives[{index}] bytes {reason}: {bytes:?}"),
                ));
            }
        }
    }

    errors
}

/// mapping에서 scalar 키로 값을 찾습니다.
fn mapping_get<'a>(pairs: &'a [(YamlValue, YamlValue)], key: &str) -> Option<&'a YamlValue> {
    pairs
        .iter()
        .find(|(k, _)| matches!(k, YamlValue::Scalar(s) if s == key))
        .map(|(_, v)| v)
}

/// 필수 scalar 필드를 가져옵니다(없거나 scalar가 아니면 오류 기록 후 None).
fn require_scalar<'a>(
    entry: &'a [(YamlValue, YamlValue)],
    key: &str,
    index: usize,
    errors: &mut Vec<StorageFixtureError>,
) -> Option<&'a str> {
    match mapping_get(entry, key) {
        Some(YamlValue::Scalar(s)) => Some(s.as_str()),
        Some(_) => {
            errors.push(StorageFixtureError::new(
                "field-type",
                format!("primitives[{index}].{key}는 scalar여야 합니다"),
            ));
            None
        }
        None => {
            errors.push(StorageFixtureError::new(
                "missing-field",
                format!("primitives[{index}]에 {key} 필드가 없습니다"),
            ));
            None
        }
    }
}

/// 소문자 16진수·짝수 길이가 아니면 사유를 반환합니다(정상이면 None).
fn invalid_hex(s: &str) -> Option<&'static str> {
    if !s.len().is_multiple_of(2) {
        return Some("길이가 홀수");
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Some("소문자 16진수가 아님");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(content: &str) -> Vec<&'static str> {
        check_canonical_primitives(content)
            .iter()
            .map(|e| e.code)
            .collect()
    }

    #[test]
    fn valid_fixture_passes() {
        let content = "primitives:\n  - name: u32_one\n    type: u32\n    value: \"1\"\n    bytes: \"01000000\"\n";
        assert!(check_canonical_primitives(content).is_empty());
    }

    #[test]
    fn the_real_fixture_passes() {
        let content = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/fixtures/storage/canonical-primitives.yaml"
        ));
        let errors = check_canonical_primitives(content);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn rejects_missing_primitives() {
        assert!(codes("other: 1\n").contains(&"missing-primitives"));
    }

    #[test]
    fn rejects_bad_type() {
        let content =
            "primitives:\n  - name: x\n    type: u128\n    value: \"1\"\n    bytes: \"00\"\n";
        assert!(codes(content).contains(&"bad-type"));
    }

    #[test]
    fn rejects_bad_bytes() {
        let odd = "primitives:\n  - name: x\n    type: u8\n    value: \"1\"\n    bytes: \"0\"\n";
        assert!(codes(odd).contains(&"bad-bytes"));
        let upper = "primitives:\n  - name: x\n    type: u8\n    value: \"1\"\n    bytes: \"FF\"\n";
        assert!(codes(upper).contains(&"bad-bytes"));
    }

    #[test]
    fn rejects_missing_field_and_duplicate_name() {
        let missing = "primitives:\n  - name: x\n    type: u8\n    bytes: \"00\"\n";
        assert!(codes(missing).contains(&"missing-field"));
        let dup = "primitives:\n  - name: x\n    type: u8\n    value: \"0\"\n    bytes: \"00\"\n  - name: x\n    type: u8\n    value: \"1\"\n    bytes: \"01\"\n";
        assert!(codes(dup).contains(&"duplicate-name"));
    }
}
