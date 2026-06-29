//! 제한 YAML loader (IV.1.2).
//!
//! `yaml-rust2` event API를 감싸 단일 document의 매핑/시퀀스/스칼라만 허용합니다. anchor,
//! alias, custom tag(core schema·verbatim 포함), merge key(`<<`), 비스칼라(복합) 매핑 키,
//! 중복 매핑 키, 다중 document를 위치(line/column)를 포함한 고유 오류로 거부합니다. 스칼라는
//! 암시적 타입 추론 없이 문자열로 보존합니다(IV.1.4).
//!
//! 중복 키는 같은 매핑 안에서만 판정하며, 마지막 값으로 덮어쓰지 않고 거부합니다(IV.1.3).

use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::Marker;

use crate::yaml::YamlValue;

/// loader 오류 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderErrorKind {
    /// anchor 정의.
    Anchor,
    /// alias 참조.
    Alias,
    /// custom tag.
    CustomTag,
    /// merge key(`<<`).
    MergeKey,
    /// 비스칼라(복합) 매핑 키.
    NonScalarKey,
    /// 중복 매핑 키.
    DuplicateKey,
    /// 다중 document.
    MultipleDocuments,
    /// 빈 document.
    EmptyDocument,
    /// 하위 파서 오류.
    Parse,
}

impl LoaderErrorKind {
    /// 고유 오류 코드 문자열.
    pub fn code(self) -> &'static str {
        match self {
            LoaderErrorKind::Anchor => "anchor",
            LoaderErrorKind::Alias => "alias",
            LoaderErrorKind::CustomTag => "custom-tag",
            LoaderErrorKind::MergeKey => "merge-key",
            LoaderErrorKind::NonScalarKey => "non-scalar-key",
            LoaderErrorKind::DuplicateKey => "duplicate-key",
            LoaderErrorKind::MultipleDocuments => "multiple-documents",
            LoaderErrorKind::EmptyDocument => "empty-document",
            LoaderErrorKind::Parse => "parse",
        }
    }
}

/// 위치를 포함한 loader 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderError {
    /// 오류 종류.
    pub kind: LoaderErrorKind,
    /// 1-기반 줄.
    pub line: usize,
    /// 1-기반 열.
    pub col: usize,
    /// 사람이 읽는 메시지.
    pub message: String,
}

impl std::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {}:{} {}",
            self.kind.code(),
            self.line,
            self.col,
            self.message
        )
    }
}

impl std::error::Error for LoaderError {}

enum Container {
    Sequence(Vec<YamlValue>),
    Mapping {
        pairs: Vec<(YamlValue, YamlValue)>,
        pending_key: Option<YamlValue>,
    },
}

struct RestrictedReceiver {
    stack: Vec<Container>,
    root: Option<YamlValue>,
    document_count: usize,
    error: Option<LoaderError>,
}

impl RestrictedReceiver {
    fn new() -> RestrictedReceiver {
        RestrictedReceiver {
            stack: Vec::new(),
            root: None,
            document_count: 0,
            error: None,
        }
    }

    fn set_error(&mut self, kind: LoaderErrorKind, mark: Marker, message: &str) {
        if self.error.is_none() {
            self.error = Some(LoaderError {
                kind,
                line: mark.line(),
                col: mark.col(),
                message: message.to_string(),
            });
        }
    }

    /// 값을 현재 컨테이너에 넣습니다. merge key 발견 시 `Err`.
    fn push_value(&mut self, value: YamlValue) -> Result<(), LoaderErrorKind> {
        match self.stack.last_mut() {
            None => {
                if self.root.is_none() {
                    self.root = Some(value);
                }
                Ok(())
            }
            Some(Container::Sequence(items)) => {
                items.push(value);
                Ok(())
            }
            Some(Container::Mapping { pairs, pending_key }) => {
                if pending_key.is_none() {
                    // 키 위치: 스칼라만 허용. `<<`는 merge key로 거부, 시퀀스·매핑 복합 키도 거부.
                    // 같은 매핑 안의 중복 키는 거부합니다(마지막 값 덮어쓰기 금지).
                    match &value {
                        YamlValue::Scalar(key) if key == "<<" => {
                            return Err(LoaderErrorKind::MergeKey)
                        }
                        YamlValue::Scalar(_) => {
                            if pairs.iter().any(|(existing, _)| existing == &value) {
                                return Err(LoaderErrorKind::DuplicateKey);
                            }
                            *pending_key = Some(value);
                        }
                        _ => return Err(LoaderErrorKind::NonScalarKey),
                    }
                } else {
                    let key = pending_key.take().expect("pending key present");
                    pairs.push((key, value));
                }
                Ok(())
            }
        }
    }

    fn finish_value(&mut self, value: YamlValue, mark: Marker) {
        if let Err(kind) = self.push_value(value) {
            let message = match kind {
                LoaderErrorKind::MergeKey => "merge key(<<)는 허용되지 않습니다",
                LoaderErrorKind::NonScalarKey => "매핑 키는 스칼라만 허용됩니다",
                LoaderErrorKind::DuplicateKey => "중복 매핑 키는 허용되지 않습니다",
                _ => "허용되지 않는 구조입니다",
            };
            self.set_error(kind, mark, message);
        }
    }
}

impl MarkedEventReceiver for RestrictedReceiver {
    fn on_event(&mut self, event: Event, mark: Marker) {
        if self.error.is_some() {
            return;
        }
        match event {
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {}
            Event::DocumentStart => {
                self.document_count += 1;
                if self.document_count > 1 {
                    self.set_error(
                        LoaderErrorKind::MultipleDocuments,
                        mark,
                        "다중 document는 허용되지 않습니다",
                    );
                }
            }
            Event::Alias(_) => {
                self.set_error(LoaderErrorKind::Alias, mark, "alias는 허용되지 않습니다");
            }
            Event::Scalar(value, _style, anchor_id, tag) => {
                if anchor_id != 0 {
                    self.set_error(LoaderErrorKind::Anchor, mark, "anchor는 허용되지 않습니다");
                } else if tag.is_some() {
                    self.set_error(
                        LoaderErrorKind::CustomTag,
                        mark,
                        "custom tag는 허용되지 않습니다",
                    );
                } else {
                    self.finish_value(YamlValue::Scalar(value), mark);
                }
            }
            Event::SequenceStart(anchor_id, tag) => {
                if anchor_id != 0 {
                    self.set_error(LoaderErrorKind::Anchor, mark, "anchor는 허용되지 않습니다");
                } else if tag.is_some() {
                    self.set_error(
                        LoaderErrorKind::CustomTag,
                        mark,
                        "custom tag는 허용되지 않습니다",
                    );
                } else {
                    self.stack.push(Container::Sequence(Vec::new()));
                }
            }
            Event::SequenceEnd => {
                if let Some(Container::Sequence(items)) = self.stack.pop() {
                    self.finish_value(YamlValue::Sequence(items), mark);
                }
            }
            Event::MappingStart(anchor_id, tag) => {
                if anchor_id != 0 {
                    self.set_error(LoaderErrorKind::Anchor, mark, "anchor는 허용되지 않습니다");
                } else if tag.is_some() {
                    self.set_error(
                        LoaderErrorKind::CustomTag,
                        mark,
                        "custom tag는 허용되지 않습니다",
                    );
                } else {
                    self.stack.push(Container::Mapping {
                        pairs: Vec::new(),
                        pending_key: None,
                    });
                }
            }
            Event::MappingEnd => {
                if let Some(Container::Mapping { pairs, .. }) = self.stack.pop() {
                    self.finish_value(YamlValue::Mapping(pairs), mark);
                }
            }
        }
    }
}

/// 제한 YAML 문자열을 [`YamlValue`] 트리로 로드합니다.
pub fn load_restricted(input: &str) -> Result<YamlValue, LoaderError> {
    let mut receiver = RestrictedReceiver::new();
    let mut parser = Parser::new_from_str(input);
    if let Err(scan_error) = parser.load(&mut receiver, true) {
        if receiver.error.is_none() {
            let mark = scan_error.marker();
            return Err(LoaderError {
                kind: LoaderErrorKind::Parse,
                line: mark.line(),
                col: mark.col(),
                message: scan_error.info().to_string(),
            });
        }
    }
    if let Some(error) = receiver.error {
        return Err(error);
    }
    receiver.root.ok_or(LoaderError {
        kind: LoaderErrorKind::EmptyDocument,
        line: 0,
        col: 0,
        message: "빈 document는 허용되지 않습니다".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(input: &str) -> LoaderError {
        load_restricted(input).expect_err("거부되어야 함")
    }

    #[test]
    fn accepts_mapping_sequence_scalar() {
        let value = load_restricted("project:\n  id: x\n  tags: [a, b]\n").unwrap();
        let project = value.get("project").unwrap();
        assert_eq!(project.get("id").and_then(YamlValue::as_scalar), Some("x"));
        let tags = project.get("tags").unwrap().as_sequence().unwrap();
        assert_eq!(tags.len(), 2);
    }

    #[test]
    fn rejects_anchor() {
        let e = err("a: &x 1\n");
        assert_eq!(e.kind.code(), "anchor");
        assert!(e.line >= 1 && e.col >= 1);
    }

    #[test]
    fn rejects_custom_tag() {
        let e = err("a: !custom 1\n");
        assert_eq!(e.kind.code(), "custom-tag");
    }

    #[test]
    fn rejects_anchor_on_mapping_and_sequence() {
        assert_eq!(err("a: &anc\n  b: 1\n").kind.code(), "anchor");
        assert_eq!(err("a: &anc [1, 2]\n").kind.code(), "anchor");
    }

    #[test]
    fn rejects_merge_key() {
        let e = err("base: {a: 1}\nmerged:\n  <<: {b: 2}\n");
        assert_eq!(e.kind.code(), "merge-key");
    }

    #[test]
    fn rejects_multiple_documents() {
        let e = err("a: 1\n---\nb: 2\n");
        assert_eq!(e.kind.code(), "multiple-documents");
    }

    #[test]
    fn rejects_alias() {
        let defined = err("a: &x 1\nb: *x\n");
        assert!(matches!(defined.kind.code(), "anchor" | "alias"));
        let undefined = err("a: *missing\n");
        assert!(matches!(undefined.kind.code(), "alias" | "parse"));
    }

    #[test]
    fn rejects_empty_document() {
        assert_eq!(err("").kind.code(), "empty-document");
        assert_eq!(err("# 주석만\n").kind.code(), "empty-document");
    }

    #[test]
    fn rejects_core_schema_and_verbatim_tags() {
        // 적대적 검토(IV.1.2): core schema·verbatim·flow 항목 tag도 모두 거부.
        for input in [
            "a: !!str 123\n",
            "num: !!int 42\n",
            "value: !!null\n",
            "items: !!seq\n  - a\n  - b\n",
            "config: !!map\n  key: value\n",
            "a: !<tag:yaml.org,2002:str> hello\n",
            "items: [!tag a, b]\n",
        ] {
            assert_eq!(err(input).kind.code(), "custom-tag", "입력: {input:?}");
        }
    }

    #[test]
    fn rejects_non_scalar_mapping_key() {
        // 복합(비스칼라) 매핑 키는 거부(제한 매니페스트는 스칼라 키만).
        assert_eq!(err("? [a, b]\n: value\n").kind.code(), "non-scalar-key");
        assert_eq!(err("? {x: 1}\n: value\n").kind.code(), "non-scalar-key");
    }

    #[test]
    fn allows_merge_token_as_value_or_item() {
        // `<<`가 키가 아니라 값·항목으로 등장하면 문자열로 허용.
        let v = load_restricted("key: <<\n").unwrap();
        assert_eq!(v.get("key").and_then(YamlValue::as_scalar), Some("<<"));
        let v = load_restricted("list:\n  - <<\n  - value\n").unwrap();
        let items = v.get("list").unwrap().as_sequence().unwrap();
        assert_eq!(items[0].as_scalar(), Some("<<"));
        // flow 매핑의 `<<` 키는 거부.
        assert_eq!(err("{a: 1, <<: 2}\n").kind.code(), "merge-key");
    }

    #[test]
    fn preserves_null_and_empty_scalars_as_strings() {
        // 암시적 null 타입 누수 없음: ~/null/빈값/따옴표 모두 문자열 보존.
        let v = load_restricted("a: ~\nb: null\nc:\nd: \"123\"\n").unwrap();
        assert_eq!(v.get("a").and_then(YamlValue::as_scalar), Some("~"));
        assert_eq!(v.get("b").and_then(YamlValue::as_scalar), Some("null"));
        assert_eq!(v.get("c").and_then(YamlValue::as_scalar), Some(""));
        assert_eq!(v.get("d").and_then(YamlValue::as_scalar), Some("123"));
    }

    #[test]
    fn accepts_single_doc_with_marker_and_directives() {
        // 선행 `---`, `%YAML`/`%TAG` directive가 있어도 document가 하나면 허용.
        assert!(load_restricted("---\na: 1\n").is_ok());
        assert!(load_restricted("%YAML 1.2\n---\na: 1\n").is_ok());
        assert!(load_restricted("%TAG ! tag:example.com,2000:\n---\na: 1\n").is_ok());
    }

    #[test]
    fn rejects_document_end_marker_multidoc() {
        // `...` 뒤에 새 document가 오면 다중 document로 거부.
        assert_eq!(err("a: 1\n...\nb: 2\n").kind.code(), "multiple-documents");
    }

    #[test]
    fn rejects_top_level_duplicate_key() {
        // 마지막 값 덮어쓰기 금지: 중복 키는 거부.
        let e = err("a: 1\na: 2\n");
        assert_eq!(e.kind.code(), "duplicate-key");
        // 두 번째 키 위치를 가리킴.
        assert_eq!(e.line, 2);
    }

    #[test]
    fn rejects_nested_duplicate_key() {
        let e = err("m:\n  x: 1\n  x: 2\n");
        assert_eq!(e.kind.code(), "duplicate-key");
        assert_eq!(e.line, 3);
    }

    #[test]
    fn rejects_flow_duplicate_key() {
        assert_eq!(err("{a: 1, a: 2}\n").kind.code(), "duplicate-key");
    }

    #[test]
    fn rejects_quoted_and_plain_same_key() {
        // 따옴표 여부와 무관하게 같은 스칼라 키면 중복.
        assert_eq!(err("a: 1\n\"a\": 2\n").kind.code(), "duplicate-key");
    }

    #[test]
    fn allows_same_key_in_different_mappings() {
        // 서로 다른 매핑(스코프)의 동일 키는 중복이 아님.
        let v = load_restricted("a:\n  x: 1\nb:\n  x: 2\n").unwrap();
        assert_eq!(
            v.get("a").unwrap().get("x").and_then(YamlValue::as_scalar),
            Some("1")
        );
        assert_eq!(
            v.get("b").unwrap().get("x").and_then(YamlValue::as_scalar),
            Some("2")
        );
        // 시퀀스 안의 두 매핑이 같은 키를 가져도 각자 독립.
        assert!(load_restricted("- k: 1\n- k: 2\n").is_ok());
    }

    #[test]
    fn scalars_preserved_as_strings() {
        // 암시적 타입 추론 없음: yes/123/2020-01-01 모두 문자열(IV.1.4 대비).
        let value = load_restricted("flag: yes\nnum: 123\ndate: 2020-01-01\n").unwrap();
        assert_eq!(
            value.get("flag").and_then(YamlValue::as_scalar),
            Some("yes")
        );
        assert_eq!(value.get("num").and_then(YamlValue::as_scalar), Some("123"));
        assert_eq!(
            value.get("date").and_then(YamlValue::as_scalar),
            Some("2020-01-01")
        );
    }
}
