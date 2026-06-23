//! 제한 YAML loader (I.2.4).
//!
//! yaml-rust2 event API를 사용해 단일 document의 mapping/sequence/scalar만 허용합니다.
//! anchor, alias, custom tag, merge key, 다중 document는 위치(line/column)를 포함한
//! 고유 오류로 거부합니다. TaskStatus 필드 해석은 하지 않습니다.

use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::Marker;

/// 제한 YAML 값 트리.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YamlValue {
    Scalar(String),
    Sequence(Vec<YamlValue>),
    Mapping(Vec<(YamlValue, YamlValue)>),
}

/// loader 오류 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderErrorKind {
    Anchor,
    Alias,
    CustomTag,
    MergeKey,
    MultipleDocuments,
    EmptyDocument,
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
            LoaderErrorKind::MultipleDocuments => "multiple-documents",
            LoaderErrorKind::EmptyDocument => "empty-document",
            LoaderErrorKind::Parse => "parse",
        }
    }
}

/// 위치를 포함한 loader 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderError {
    pub kind: LoaderErrorKind,
    pub line: usize,
    pub col: usize,
    pub message: String,
}

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
    fn new() -> Self {
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

    /// 값을 현재 컨테이너에 넣습니다. merge key 발견 시 Err를 반환합니다.
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
                    if matches!(&value, YamlValue::Scalar(key) if key == "<<") {
                        return Err(LoaderErrorKind::MergeKey);
                    }
                    *pending_key = Some(value);
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
            self.set_error(kind, mark, "merge key는 허용되지 않습니다");
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

/// 제한 YAML 문자열을 값 트리로 로드합니다.
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
        load_restricted(input).expect_err("should reject")
    }

    #[test]
    fn accepts_mapping_sequence_scalar() {
        let value = load_restricted("tasks:\n  - id: I.1.1\n    deps: [a, b]\n").unwrap();
        let YamlValue::Mapping(pairs) = value else {
            panic!("expected mapping");
        };
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, YamlValue::Scalar("tasks".to_string()));
        let YamlValue::Sequence(items) = &pairs[0].1 else {
            panic!("expected sequence");
        };
        assert_eq!(items.len(), 1);
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
        assert!(e.line >= 1);
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
    fn rejects_alias_usage() {
        // yaml-rust2는 정의된 anchor+alias를 anchor 이벤트로 먼저 거부하고,
        // 미정의 alias는 parse 오류로 거부합니다. 어느 경우든 거부되어야 합니다.
        let defined = err("a: &x 1\nb: *x\n");
        assert!(matches!(defined.kind.code(), "anchor" | "alias"));
        let undefined = err("a: *missing\n");
        assert!(matches!(undefined.kind.code(), "alias" | "parse"));
    }

    #[test]
    fn rejects_empty_document() {
        let e = err("");
        assert_eq!(e.kind.code(), "empty-document");
    }
}
