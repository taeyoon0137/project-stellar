//! canonical YAML writer (IV.1.5).
//!
//! [`YamlValue`] 트리를 결정론적 canonical 텍스트로 기록합니다. 고정 규칙:
//! - **키 순서**: [`YamlValue::Mapping`]의 저장 순서를 그대로 유지(재정렬하지 않음).
//! - **들여쓰기**: 중첩 한 단계마다 공백 2칸.
//! - **줄바꿈**: LF(`\n`)만 사용.
//! - **마지막 newline**: 항상 trailing newline으로 끝납니다.
//! - **주석 보존 없음**: 입력 모델에 주석이 없으므로 출력에도 없습니다.
//!
//! 스칼라 인용 정책: round-trip·interop 안전을 위해 **안전한 plain 식별자**(영문 시작,
//! 영문/숫자/`_`/`-`로 구성, 불리언·널 키워드 아님)만 무인용으로 출력하고, 그 외(숫자처럼 보임,
//! 특수문자, 공백, 빈 문자열, 키워드)는 이스케이프한 더블쿼트로 출력합니다. 이로써 제한
//! loader(IV.1.2)로 다시 읽으면 동일한 [`YamlValue`]가 됩니다.

use crate::yaml::YamlValue;

/// [`YamlValue`]를 canonical YAML 텍스트로 기록합니다(trailing newline 포함).
pub fn write_canonical(value: &YamlValue) -> String {
    let mut out = String::new();
    match value {
        YamlValue::Scalar(s) => {
            out.push_str(&render_scalar(s));
            out.push('\n');
        }
        YamlValue::Mapping(pairs) if pairs.is_empty() => out.push_str("{}\n"),
        YamlValue::Sequence(items) if items.is_empty() => out.push_str("[]\n"),
        YamlValue::Mapping(pairs) => write_mapping(&mut out, pairs, 0),
        YamlValue::Sequence(items) => write_sequence(&mut out, items, 0),
    }
    out
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push(' ');
    }
}

fn write_mapping(out: &mut String, pairs: &[(YamlValue, YamlValue)], indent: usize) {
    for (key, value) in pairs {
        push_indent(out, indent);
        out.push_str(&render_key(key));
        out.push(':');
        write_child(out, value, indent);
    }
}

fn write_sequence(out: &mut String, items: &[YamlValue], indent: usize) {
    for item in items {
        push_indent(out, indent);
        out.push('-');
        write_child(out, item, indent);
    }
}

/// 매핑 값 또는 시퀀스 항목(부모 마커 뒤)을 기록합니다.
fn write_child(out: &mut String, value: &YamlValue, indent: usize) {
    match value {
        YamlValue::Scalar(s) => {
            out.push(' ');
            out.push_str(&render_scalar(s));
            out.push('\n');
        }
        YamlValue::Mapping(pairs) if pairs.is_empty() => out.push_str(" {}\n"),
        YamlValue::Sequence(items) if items.is_empty() => out.push_str(" []\n"),
        YamlValue::Mapping(pairs) => {
            out.push('\n');
            write_mapping(out, pairs, indent + 2);
        }
        YamlValue::Sequence(items) => {
            out.push('\n');
            write_sequence(out, items, indent + 2);
        }
    }
}

/// 매핑 키를 렌더링합니다(키도 스칼라이며 동일한 인용 정책 적용).
fn render_key(key: &YamlValue) -> String {
    match key {
        YamlValue::Scalar(s) => render_scalar(s),
        // loader가 비스칼라 키를 거부하므로 정상 트리에서는 도달하지 않습니다.
        _ => render_scalar("?"),
    }
}

/// 스칼라를 plain 또는 더블쿼트로 렌더링합니다.
fn render_scalar(s: &str) -> String {
    if is_safe_plain(s) {
        s.to_string()
    } else {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

/// 무인용 plain으로 안전한지 판정합니다(영문 시작, 영문/숫자/`_`/`-`, 키워드 아님).
fn is_safe_plain(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false; // 빈 문자열
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return false;
    }
    !is_confusable_keyword(s)
}

/// YAML 1.1/1.2에서 불리언·널로 읽힐 수 있는 키워드인지(대소문자 무시) 판정합니다.
fn is_confusable_keyword(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        Dependency, Manifest, ProjectMeta, TargetProfile, CURRENT_SCHEMA_VERSION,
    };
    use crate::yaml::load_restricted;

    fn sample_tree() -> YamlValue {
        YamlValue::Mapping(vec![
            (YamlValue::scalar("name"), YamlValue::scalar("web")),
            (
                YamlValue::scalar("nested"),
                YamlValue::Mapping(vec![
                    (YamlValue::scalar("a"), YamlValue::scalar("apple")),
                    (YamlValue::scalar("b"), YamlValue::scalar("box")),
                ]),
            ),
            (
                YamlValue::scalar("list"),
                YamlValue::Sequence(vec![YamlValue::scalar("one"), YamlValue::scalar("two")]),
            ),
            (
                YamlValue::scalar("empty_seq"),
                YamlValue::Sequence(Vec::new()),
            ),
            (
                YamlValue::scalar("empty_map"),
                YamlValue::Mapping(Vec::new()),
            ),
        ])
    }

    #[test]
    fn canonical_format_golden() {
        let expected = "name: web\n\
                        nested:\n  a: apple\n  b: box\n\
                        list:\n  - one\n  - two\n\
                        empty_seq: []\n\
                        empty_map: {}\n";
        assert_eq!(write_canonical(&sample_tree()), expected);
    }

    #[test]
    fn uses_lf_and_trailing_newline_only() {
        let text = write_canonical(&sample_tree());
        assert!(text.ends_with('\n'));
        assert!(!text.contains('\r'));
        // 정확히 하나의 trailing newline.
        assert!(!text.ends_with("\n\n"));
    }

    #[test]
    fn deterministic_100_writes_byte_identical() {
        let manifest = full_manifest();
        let tree = manifest.to_yaml();
        let first = write_canonical(&tree);
        for _ in 0..100 {
            assert_eq!(write_canonical(&tree), first);
        }
    }

    #[test]
    fn confusable_and_special_scalars_are_quoted() {
        // 숫자처럼 보임·키워드·특수문자·빈 문자열·공백은 더블쿼트.
        for s in [
            "1", "010", "true", "yes", "on", "26.0", "v2.4.0", "a b", "", "x: y", "#c",
        ] {
            let tree = YamlValue::Mapping(vec![(YamlValue::scalar("k"), YamlValue::scalar(s))]);
            let text = write_canonical(&tree);
            assert!(text.contains('"'), "스칼라 {s:?}는 인용되어야 함: {text:?}");
        }
        // 안전한 식별자는 무인용.
        let tree = YamlValue::Mapping(vec![(
            YamlValue::scalar("k"),
            YamlValue::scalar("design-system"),
        )]);
        assert_eq!(write_canonical(&tree), "k: design-system\n");
    }

    #[test]
    fn diverse_scalars_round_trip_as_value_and_item() {
        // 까다로운 스칼라가 매핑 값·시퀀스 항목·키 위치에서 모두 정확히 round-trip 하는지.
        let scalars = [
            "",
            " ",
            "  leading",
            "trailing  ",
            "a b c",
            "x: y",
            "key:value",
            "ends_with:",
            "#comment-like",
            "has#hash",
            "back\\slash",
            "quote\"inside",
            "line1\nline2",
            "tab\there",
            "carriage\rreturn",
            "true",
            "False",
            "yes",
            "on",
            "null",
            "~",
            "1",
            "010",
            "26.0",
            "-dash-start",
            "[bracket]",
            "{brace}",
            "한글값",
            "emoji😀",
            "<<",
            "*alias-like",
            "&anchor-like",
            "!tag-like",
        ];
        for s in scalars {
            // 매핑 값.
            let as_value = YamlValue::Mapping(vec![(YamlValue::scalar("k"), YamlValue::scalar(s))]);
            let text = write_canonical(&as_value);
            assert_eq!(
                load_restricted(&text).unwrap_or_else(|e| panic!("값 {s:?} 재로드 실패: {e}")),
                as_value,
                "값 round-trip 불일치: {s:?} → {text:?}"
            );

            // 시퀀스 항목.
            let as_item = YamlValue::Sequence(vec![YamlValue::scalar(s)]);
            let text = write_canonical(&as_item);
            assert_eq!(
                load_restricted(&text).unwrap_or_else(|e| panic!("항목 {s:?} 재로드 실패: {e}")),
                as_item,
                "항목 round-trip 불일치: {s:?} → {text:?}"
            );

            // 매핑 키(`<<`는 loader가 merge-key로 거부하므로 제외).
            if s != "<<" {
                let as_key =
                    YamlValue::Mapping(vec![(YamlValue::scalar(s), YamlValue::scalar("v"))]);
                let text = write_canonical(&as_key);
                assert_eq!(
                    load_restricted(&text).unwrap_or_else(|e| panic!("키 {s:?} 재로드 실패: {e}")),
                    as_key,
                    "키 round-trip 불일치: {s:?} → {text:?}"
                );
            }
        }
    }

    #[test]
    fn round_trips_through_loader() {
        // write → load_restricted → 동일 트리.
        for tree in [
            sample_tree(),
            full_manifest().to_yaml(),
            minimal_manifest().to_yaml(),
        ] {
            let text = write_canonical(&tree);
            let reloaded = load_restricted(&text)
                .unwrap_or_else(|e| panic!("재로드 실패: {e}\n--- 출력 ---\n{text}"));
            assert_eq!(reloaded, tree, "round-trip 불일치\n--- 출력 ---\n{text}");
        }
    }

    #[test]
    fn manifest_text_round_trips() {
        // 모델 → 텍스트 → 모델 전체 경로.
        for manifest in [minimal_manifest(), full_manifest()] {
            let text = write_canonical(&manifest.to_yaml());
            let value = load_restricted(&text).unwrap();
            let restored = Manifest::from_yaml(&value).unwrap();
            assert_eq!(restored, manifest);
        }
    }

    fn minimal_manifest() -> Manifest {
        Manifest {
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectMeta {
                id: "0195c530-3acd-7bc1-8414-c213f61cb965".to_string(),
                name: "Project Stellar".to_string(),
            },
            target_profiles: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    fn full_manifest() -> Manifest {
        Manifest {
            schema_version: 1,
            project: ProjectMeta {
                id: "0195c530-3acd-7bc1-8414-c213f61cb965".to_string(),
                name: "Project Stellar".to_string(),
            },
            target_profiles: vec![
                TargetProfile {
                    id: "apple-desktop".to_string(),
                    platform: "apple".to_string(),
                    minimum_os: Some("26.0".to_string()),
                    baseline: None,
                },
                TargetProfile {
                    id: "web-modern".to_string(),
                    platform: "web".to_string(),
                    minimum_os: None,
                    baseline: Some("modern".to_string()),
                },
            ],
            dependencies: vec![Dependency {
                name: "design-system".to_string(),
                source: "git".to_string(),
                repository: "git@github.com:example/design-system.git".to_string(),
                version: "v2.4.0".to_string(),
            }],
        }
    }
}
