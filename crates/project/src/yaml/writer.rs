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
//!
//! round-trip 보강(XVI.6.1 fuzz 결과): 제어·비출력·줄 구분 문자는 더블쿼트 안에서 `\uXXXX`로
//! 기록하고, 렌더링 길이가 implicit key 상한(1024자)을 넘는 키는 explicit key(`? `)로 기록합니다.
//!
//! 제한 강제: 파일로 기록하는 writer(매니페스트·잠금 파일·신뢰 목록)는
//! [`write_canonical_checked`]를 씁니다. 제한 loader(IV.1.2)가 거부하는 구조(중복 키·merge key·
//! 비스칼라 키·중첩 깊이 초과)와 크기(입력 상한·이벤트 선읽기 상한)를 기록 **전에** 거부하고,
//! 출력이 loader로 같은 값으로 다시 읽히는지 확인하므로 기록된 파일은 항상 다시 열립니다.

use std::collections::HashSet;

use crate::yaml::loader::{LoaderError, MAX_INPUT_BYTES, MAX_NESTING_DEPTH};
use crate::yaml::{load_restricted, YamlValue};

/// 제한 강제 writer 오류. 이 오류가 나면 아무 텍스트도 반환하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// 매핑 키가 스칼라가 아님(loader `non-scalar-key`).
    NonScalarKey {
        /// 문제 위치(`a.b[0]` 형식, 최상위는 빈 문자열).
        path: String,
    },
    /// merge key `<<`(loader `merge-key`).
    MergeKey {
        /// 문제 매핑 위치.
        path: String,
    },
    /// 같은 매핑 안의 중복 키(loader `duplicate-key`).
    DuplicateKey {
        /// 문제 매핑 위치.
        path: String,
        /// 중복된 키.
        key: String,
    },
    /// 컨테이너 중첩 깊이가 [`MAX_NESTING_DEPTH`]를 넘음(loader `nesting-too-deep`).
    NestingTooDeep {
        /// 문제 위치.
        path: String,
    },
    /// 출력이 [`MAX_INPUT_BYTES`]를 넘음(loader `input-too-large`).
    OutputTooLarge {
        /// 출력 bytes.
        bytes: usize,
    },
    /// 출력을 제한 loader가 다시 읽지 못함(예: 선읽기 상한을 넘는 긴 스칼라).
    NotReloadable(LoaderError),
    /// 다시 읽은 값이 기록하려던 값과 다름.
    RoundTripMismatch,
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::NonScalarKey { path } => {
                write!(f, "매핑 키는 스칼라만 기록할 수 있습니다: {path:?}")
            }
            WriteError::MergeKey { path } => {
                write!(f, "merge key(<<)는 기록할 수 없습니다: {path:?}")
            }
            WriteError::DuplicateKey { path, key } => {
                write!(f, "중복 매핑 키는 기록할 수 없습니다: {path:?}의 {key:?}")
            }
            WriteError::NestingTooDeep { path } => write!(
                f,
                "컨테이너 중첩 깊이가 상한 {MAX_NESTING_DEPTH}을 넘습니다: {path:?}"
            ),
            WriteError::OutputTooLarge { bytes } => write!(
                f,
                "출력이 {bytes} bytes로 상한 {MAX_INPUT_BYTES} bytes를 넘습니다"
            ),
            WriteError::NotReloadable(e) => write!(f, "출력을 다시 읽을 수 없습니다: {e}"),
            WriteError::RoundTripMismatch => {
                f.write_str("출력을 다시 읽은 값이 원래 값과 다릅니다")
            }
        }
    }
}

impl std::error::Error for WriteError {}

/// loader 제한을 강제하는 canonical writer.
///
/// 1. 구조 검사(반복 순회, 재귀 없음): 비스칼라 키·`<<` 키·중복 키·중첩 깊이.
/// 2. [`write_canonical`]로 렌더링.
/// 3. 출력 크기 상한 검사.
/// 4. 제한 loader로 다시 읽어 같은 값인지 확인(선읽기 상한 등 나머지 loader 제한 포함).
pub fn write_canonical_checked(value: &YamlValue) -> Result<String, WriteError> {
    check_structure(value)?;
    let text = write_canonical(value);
    if text.len() > MAX_INPUT_BYTES {
        return Err(WriteError::OutputTooLarge { bytes: text.len() });
    }
    let reloaded = load_restricted(&text).map_err(WriteError::NotReloadable)?;
    if reloaded != *value {
        return Err(WriteError::RoundTripMismatch);
    }
    Ok(text)
}

/// loader가 거부하는 구조를 찾습니다. 깊이 상한을 넘는 트리도 재귀 없이 순회합니다.
fn check_structure(root: &YamlValue) -> Result<(), WriteError> {
    // (값, 이 값이 컨테이너일 때의 중첩 깊이, 위치)
    let mut stack: Vec<(&YamlValue, usize, String)> = vec![(root, 1, String::new())];
    while let Some((value, depth, path)) = stack.pop() {
        match value {
            YamlValue::Scalar(_) => {}
            YamlValue::Sequence(items) => {
                if depth > MAX_NESTING_DEPTH {
                    return Err(WriteError::NestingTooDeep { path });
                }
                for (index, item) in items.iter().enumerate() {
                    stack.push((item, depth + 1, format!("{path}[{index}]")));
                }
            }
            YamlValue::Mapping(pairs) => {
                if depth > MAX_NESTING_DEPTH {
                    return Err(WriteError::NestingTooDeep { path });
                }
                let mut seen: HashSet<&str> = HashSet::with_capacity(pairs.len());
                for (key, child) in pairs {
                    let YamlValue::Scalar(key) = key else {
                        return Err(WriteError::NonScalarKey { path });
                    };
                    if key == "<<" {
                        return Err(WriteError::MergeKey { path });
                    }
                    if !seen.insert(key.as_str()) {
                        return Err(WriteError::DuplicateKey {
                            path,
                            key: key.clone(),
                        });
                    }
                    let child_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    stack.push((child, depth + 1, child_path));
                }
            }
        }
    }
    Ok(())
}

/// [`YamlValue`]를 canonical YAML 텍스트로 기록합니다(trailing newline 포함).
///
/// 렌더링만 수행하며 loader 제한을 검사하지 않습니다. 파일로 기록하는 경로는
/// [`write_canonical_checked`]를 사용합니다.
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

/// YAML implicit key 길이 상한(렌더링된 문자 수, YAML 1.2 §7.4).
const MAX_IMPLICIT_KEY_CHARS: usize = 1024;

fn write_mapping(out: &mut String, pairs: &[(YamlValue, YamlValue)], indent: usize) {
    for (key, value) in pairs {
        push_indent(out, indent);
        let rendered = render_key(key);
        if rendered.chars().count() > MAX_IMPLICIT_KEY_CHARS {
            // implicit key 상한을 넘는 키는 explicit key(`? `)로 기록합니다(XVI.6.1). implicit
            // key로 쓰면 제한 loader가 다시 읽지 못합니다.
            out.push_str("? ");
            out.push_str(&rendered);
            out.push('\n');
            push_indent(out, indent);
        } else {
            out.push_str(&rendered);
        }
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
                // 제어·비출력 문자와 줄 구분 문자는 `\uXXXX`로 기록합니다(XVI.6.1). 원문으로 두면
                // 제한 loader가 다시 읽지 못하거나(NUL 등) 값이 달라질 수 있습니다.
                c if needs_unicode_escape(c) => {
                    out.push_str(&format!("\\u{:04x}", u32::from(c)));
                }
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

/// 더블쿼트 안에서도 `\uXXXX` 이스케이프가 필요한 문자인지 판정합니다.
///
/// C0 제어 문자(전용 이스케이프가 있는 `\t`·`\n`·`\r` 제외), DEL, C1 제어 문자(NEL 포함),
/// U+2028/U+2029 줄 구분 문자, BOM(U+FEFF), 비문자 U+FFFE/U+FFFF. 모두 BMP 문자입니다.
fn needs_unicode_escape(c: char) -> bool {
    matches!(
        c,
        '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{2028}' | '\u{2029}' | '\u{feff}' | '\u{fffe}' | '\u{ffff}'
    )
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
    use crate::manifest::fixtures::tag_dependency;
    use crate::manifest::{Manifest, ProjectMeta, TargetProfile, CURRENT_SCHEMA_VERSION};
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
            dependencies: vec![tag_dependency(
                "design-system",
                1,
                "git@github.com:example/design-system.git",
                "v2.4.0",
            )],
        }
    }

    #[test]
    fn control_and_separator_chars_are_escaped_and_round_trip() {
        // XVI.6.1 fuzz 발견: 원문 NUL 등을 기록하면 제한 loader가 다시 읽지 못했음.
        let tricky = [
            "\u{0}",
            "a\u{1}b",
            "\u{1b}[0m",
            "del\u{7f}",
            "nel\u{85}",
            "c1\u{9f}",
            "ls\u{2028}ps\u{2029}",
            "bom\u{feff}",
            "\u{fffe}\u{ffff}",
            "tab\tnl\ncr\r",
        ];
        for s in tricky {
            let tree = YamlValue::Mapping(vec![(YamlValue::scalar(s), YamlValue::scalar(s))]);
            let text = write_canonical(&tree);
            assert!(
                text.chars().all(|c| c == '\n' || !needs_unicode_escape(c)),
                "원문 제어 문자 기록: {text:?}"
            );
            assert_eq!(load_restricted(&text).unwrap(), tree, "입력: {s:?}");
        }
        assert_eq!(
            write_canonical(&YamlValue::scalar("\u{0}")),
            "\"\\u0000\"\n"
        );
    }

    #[test]
    fn keys_over_implicit_limit_use_explicit_form_and_round_trip() {
        // XVI.6.1 fuzz 발견: 1024자를 넘는 키를 implicit key로 쓰면 다시 읽지 못했음.
        // 렌더링 길이 1024(plain)는 implicit, 1025는 explicit.
        let at_limit = format!("k{}", "a".repeat(MAX_IMPLICIT_KEY_CHARS - 1));
        let over_limit = format!("k{}", "a".repeat(MAX_IMPLICIT_KEY_CHARS));
        let escaped = "\u{0}".repeat(200); // 렌더링 시 6배로 늘어남.
        for key in [at_limit.as_str(), over_limit.as_str(), escaped.as_str()] {
            for value in [
                YamlValue::scalar("v"),
                YamlValue::Mapping(vec![(YamlValue::scalar("x"), YamlValue::scalar("1"))]),
                YamlValue::Sequence(vec![YamlValue::scalar("a"), YamlValue::scalar("b")]),
                YamlValue::Mapping(vec![]),
            ] {
                let tree = YamlValue::Mapping(vec![
                    (
                        YamlValue::scalar("outer"),
                        YamlValue::Mapping(vec![(YamlValue::scalar(key), value.clone())]),
                    ),
                    (YamlValue::scalar(key), value),
                ]);
                let text = write_canonical(&tree);
                assert_eq!(
                    load_restricted(&text).unwrap(),
                    tree,
                    "key len {}",
                    key.len()
                );
            }
        }
        let text = write_canonical(&YamlValue::Mapping(vec![(
            YamlValue::scalar(at_limit.as_str()),
            YamlValue::scalar("v"),
        )]));
        assert!(!text.starts_with("? "));
        let text = write_canonical(&YamlValue::Mapping(vec![(
            YamlValue::scalar(over_limit.as_str()),
            YamlValue::scalar("v"),
        )]));
        assert!(text.starts_with("? ") && text.ends_with("\n: v\n"));
    }

    fn map(pairs: Vec<(YamlValue, YamlValue)>) -> YamlValue {
        YamlValue::Mapping(pairs)
    }

    /// 컨테이너 `depth`개가 중첩된 트리(맨 안쪽은 스칼라).
    fn nested(depth: usize) -> YamlValue {
        let mut value = YamlValue::scalar("leaf");
        for i in 0..depth {
            value = if i % 2 == 0 {
                YamlValue::Sequence(vec![value])
            } else {
                map(vec![(YamlValue::scalar("k"), value)])
            };
        }
        value
    }

    /// 깊은 트리의 재귀 drop(stack overflow)을 피하려고 바깥부터 해체합니다.
    fn dismantle(mut value: YamlValue) {
        loop {
            value = match value {
                YamlValue::Sequence(mut items) if !items.is_empty() => items.swap_remove(0),
                YamlValue::Mapping(mut pairs) if !pairs.is_empty() => pairs.swap_remove(0).1,
                _ => break,
            };
        }
    }

    #[test]
    fn checked_writer_rejects_structures_the_loader_rejects() {
        let dup = map(vec![(
            YamlValue::scalar("deps"),
            map(vec![
                (YamlValue::scalar("a"), YamlValue::scalar("1")),
                (YamlValue::scalar("a"), YamlValue::scalar("2")),
            ]),
        )]);
        // 검사 없는 writer 출력은 loader가 실제로 거부함(기록하면 다시 열 수 없음).
        assert!(load_restricted(&write_canonical(&dup)).is_err());
        assert_eq!(
            write_canonical_checked(&dup),
            Err(WriteError::DuplicateKey {
                path: "deps".to_string(),
                key: "a".to_string()
            })
        );

        let merge = map(vec![(YamlValue::scalar("<<"), YamlValue::scalar("x"))]);
        assert!(load_restricted(&write_canonical(&merge)).is_err());
        assert_eq!(
            write_canonical_checked(&merge),
            Err(WriteError::MergeKey {
                path: String::new()
            })
        );

        let complex_key = YamlValue::Sequence(vec![map(vec![(
            YamlValue::Sequence(vec![]),
            YamlValue::scalar("x"),
        )])]);
        assert_eq!(
            write_canonical_checked(&complex_key),
            Err(WriteError::NonScalarKey {
                path: "[0]".to_string()
            })
        );
    }

    #[test]
    fn checked_writer_enforces_nesting_depth_limit() {
        let at_limit = nested(MAX_NESTING_DEPTH);
        let text = write_canonical_checked(&at_limit).unwrap();
        assert_eq!(load_restricted(&text).unwrap(), at_limit);

        let over = nested(MAX_NESTING_DEPTH + 1);
        assert!(load_restricted(&write_canonical(&over)).is_err());
        assert!(matches!(
            write_canonical_checked(&over),
            Err(WriteError::NestingTooDeep { .. })
        ));
        // 매우 깊은 트리도 검사 단계에서 stack overflow 없이 거부(렌더링 전).
        let very_deep = nested(100_000);
        assert!(matches!(
            write_canonical_checked(&very_deep),
            Err(WriteError::NestingTooDeep { .. })
        ));
        dismantle(very_deep);
    }

    #[test]
    fn checked_writer_enforces_size_limits() {
        use crate::yaml::loader::{LoaderErrorKind, MAX_EVENT_LOOKAHEAD_CHARS};
        // 이벤트 선읽기 상한을 넘는 스칼라: loader가 거부하므로 기록하지 않음.
        let long = map(vec![(
            YamlValue::scalar("k"),
            YamlValue::scalar("a".repeat(MAX_EVENT_LOOKAHEAD_CHARS + 1)),
        )]);
        assert!(load_restricted(&write_canonical(&long)).is_err());
        match write_canonical_checked(&long) {
            Err(WriteError::NotReloadable(e)) => {
                assert_eq!(e.kind, LoaderErrorKind::LookaheadTooLarge)
            }
            other => panic!("NotReloadable이어야 함: {other:?}"),
        }
        // 상한보다 짧은 스칼라는 기록·재로드됨.
        let fits = map(vec![(
            YamlValue::scalar("k"),
            YamlValue::scalar("a".repeat(MAX_EVENT_LOOKAHEAD_CHARS / 2)),
        )]);
        assert_eq!(
            load_restricted(&write_canonical_checked(&fits).unwrap()).unwrap(),
            fits
        );

        // 전체 출력 크기 상한.
        let item = "b".repeat(MAX_EVENT_LOOKAHEAD_CHARS / 2);
        let count = MAX_INPUT_BYTES / item.len() + 1;
        let big = YamlValue::Sequence(vec![YamlValue::scalar(item); count]);
        assert!(load_restricted(&write_canonical(&big)).is_err());
        assert!(matches!(
            write_canonical_checked(&big),
            Err(WriteError::OutputTooLarge { .. })
        ));
    }

    #[test]
    fn checked_writer_output_equals_unchecked_for_valid_trees() {
        for tree in [
            sample_tree(),
            full_manifest().to_yaml(),
            minimal_manifest().to_yaml(),
        ] {
            assert_eq!(
                write_canonical_checked(&tree).unwrap(),
                write_canonical(&tree)
            );
        }
    }
}
