//! 제한 YAML loader (IV.1.2).
//!
//! `yaml-rust2` event API를 감싸 단일 document의 매핑/시퀀스/스칼라만 허용합니다. anchor,
//! alias, custom tag(core schema·verbatim 포함), merge key(`<<`), 비스칼라(복합) 매핑 키,
//! 중복 매핑 키, 다중 document를 위치(line/column)를 포함한 고유 오류로 거부합니다. 스칼라는
//! 암시적 타입 추론 없이 문자열로 보존합니다(IV.1.4).
//!
//! 중복 키는 같은 매핑 안에서만 판정하며, 마지막 값으로 덮어쓰지 않고 거부합니다(IV.1.3).
//!
//! 적대적 입력 방어(XVI.6.1 fuzz 결과, 기술 아키텍처 37.2 깊이·개수 제한):
//! - 입력 크기 상한 [`MAX_INPUT_BYTES`]를 넘으면 파싱 전에 거부합니다.
//! - YAML 1.2 `c-printable`이 아닌 문자(NUL 등 제어 문자)를 거부합니다. 하위 scanner는 NUL을
//!   입력 끝으로 취급해 뒤 내용을 조용히 버리므로, 거부하지 않으면 Git이 보는 내용과 Stellar가
//!   읽는 내용이 달라집니다.
//! - 하위 parser의 재귀 `load` 대신 event를 하나씩 당겨 처리합니다(깊은 block 중첩에서 stack
//!   overflow 없음). 컨테이너 중첩은 [`MAX_NESTING_DEPTH`]까지만 허용합니다.
//! - 이벤트 하나를 만들기 위한 scanner 선읽기를 [`MAX_EVENT_LOOKAHEAD_CHARS`]로 제한합니다. 하위
//!   scanner는 key가 될 수 있는 위치(예: 문서 최상위)의 flow collection을 끝까지 token(약 128
//!   bytes/개)으로 버퍼링한 뒤 첫 이벤트를 내므로, 이 상한이 없으면 입력 1 byte당 수백 bytes를
//!   할당합니다.
//! - 중복 키 판정은 매핑마다 키 hash 집합을 써서 선형 시간에 수행합니다(키 수 제곱 시간 없음).
//!   hash가 겹칠 때만 실제 키를 비교합니다.

use std::cell::Cell;
use std::collections::hash_map::RandomState;
use std::collections::HashSet;
use std::hash::BuildHasher;
use std::rc::Rc;

use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::Marker;

use crate::yaml::YamlValue;

/// 입력 크기 상한(bytes). 매니페스트·잠금 파일은 작은 설정 파일이므로 넉넉한 상한입니다.
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;

/// 컨테이너(매핑·시퀀스) 중첩 깊이 상한.
pub const MAX_NESTING_DEPTH: usize = 64;

/// 이벤트 하나를 만들기 위해 scanner가 미리 읽을 수 있는 문자 수 상한.
///
/// 단일 스칼라, 버퍼링되는 하나의 flow collection, 연속된 주석·공백이 이 길이를 넘으면
/// 거부합니다.
pub const MAX_EVENT_LOOKAHEAD_CHARS: usize = 32 * 1024;

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
    /// 입력 크기 상한 초과.
    InputTooLarge,
    /// `c-printable`이 아닌 문자(제어 문자 등).
    NonPrintable,
    /// 컨테이너 중첩 깊이 상한 초과.
    NestingTooDeep,
    /// 이벤트 하나에 필요한 선읽기 상한 초과(과도하게 긴 스칼라·flow collection·주석).
    LookaheadTooLarge,
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
            LoaderErrorKind::InputTooLarge => "input-too-large",
            LoaderErrorKind::NonPrintable => "non-printable",
            LoaderErrorKind::NestingTooDeep => "nesting-too-deep",
            LoaderErrorKind::LookaheadTooLarge => "lookahead-too-large",
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
        /// 이미 나온 키의 hash(중복 키 선형 시간 판정).
        key_hashes: HashSet<u64>,
    },
}

struct RestrictedReceiver {
    stack: Vec<Container>,
    root: Option<YamlValue>,
    document_count: usize,
    error: Option<LoaderError>,
    /// 키 hash용 무작위 seed hasher(입력이 hash 충돌을 고를 수 없음).
    hasher: RandomState,
}

impl RestrictedReceiver {
    fn new() -> RestrictedReceiver {
        RestrictedReceiver {
            stack: Vec::new(),
            root: None,
            document_count: 0,
            error: None,
            hasher: RandomState::new(),
        }
    }

    /// 컨테이너를 엽니다. 중첩 깊이 상한을 넘으면 오류를 기록합니다.
    fn open_container(&mut self, container: Container, mark: Marker) {
        if self.stack.len() >= MAX_NESTING_DEPTH {
            self.set_error(
                LoaderErrorKind::NestingTooDeep,
                mark,
                "컨테이너 중첩 깊이 상한을 넘었습니다",
            );
        } else {
            self.stack.push(container);
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
        let hasher = &self.hasher;
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
            Some(Container::Mapping {
                pairs,
                pending_key,
                key_hashes,
            }) => {
                if pending_key.is_none() {
                    // 키 위치: 스칼라만 허용. `<<`는 merge key로 거부, 시퀀스·매핑 복합 키도 거부.
                    // 같은 매핑 안의 중복 키는 거부합니다(마지막 값 덮어쓰기 금지).
                    match &value {
                        YamlValue::Scalar(key) if key == "<<" => {
                            return Err(LoaderErrorKind::MergeKey)
                        }
                        YamlValue::Scalar(key) => {
                            // 새 hash면 중복이 아님. hash가 겹칠 때만 실제 키를 비교합니다.
                            if !key_hashes.insert(hasher.hash_one(key.as_str()))
                                && pairs.iter().any(|(existing, _)| existing == &value)
                            {
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
                    self.open_container(Container::Sequence(Vec::new()), mark);
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
                    self.open_container(
                        Container::Mapping {
                            pairs: Vec::new(),
                            pending_key: None,
                            key_hashes: HashSet::new(),
                        },
                        mark,
                    );
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

/// scanner 선읽기 계측 상태(loader와 문자 iterator가 공유).
struct Lookahead {
    /// 지금까지 scanner에 건넨 문자 수.
    consumed: Cell<usize>,
    /// 마지막 이벤트를 받은 시점의 `consumed`.
    base: Cell<usize>,
    /// 상한을 넘어 입력 끝을 돌려주었는지.
    exceeded: Cell<bool>,
}

/// 이벤트 사이 선읽기가 [`MAX_EVENT_LOOKAHEAD_CHARS`]를 넘으면 입력 끝(`None`)을 돌려주는 문자
/// iterator. 그 뒤 loader는 결과와 무관하게 [`LoaderErrorKind::LookaheadTooLarge`]로 거부합니다.
struct BudgetedChars<'a> {
    chars: std::str::Chars<'a>,
    lookahead: Rc<Lookahead>,
}

impl Iterator for BudgetedChars<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        let lookahead = &self.lookahead;
        if lookahead.exceeded.get() {
            return None;
        }
        if lookahead.consumed.get() - lookahead.base.get() >= MAX_EVENT_LOOKAHEAD_CHARS {
            lookahead.exceeded.set(true);
            return None;
        }
        let c = self.chars.next()?;
        lookahead.consumed.set(lookahead.consumed.get() + 1);
        Some(c)
    }
}

/// YAML 1.2 `c-printable` 문자인지 판정합니다.
fn is_c_printable(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\r'
            | '\u{20}'..='\u{7e}'
            | '\u{85}'
            | '\u{a0}'..='\u{d7ff}'
            | '\u{e000}'..='\u{fffd}'
            | '\u{10000}'..='\u{10ffff}'
    )
}

/// 첫 `c-printable`이 아닌 문자의 (1-기반 줄, 1-기반 열, 문자)를 찾습니다.
fn find_non_printable(input: &str) -> Option<(usize, usize, char)> {
    let (mut line, mut col) = (1, 1);
    for c in input.chars() {
        if !is_c_printable(c) {
            return Some((line, col, c));
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    None
}

/// 문자 index의 (1-기반 줄, 1-기반 열).
fn char_position(input: &str, char_index: usize) -> (usize, usize) {
    let (mut line, mut col) = (1, 1);
    for c in input.chars().take(char_index) {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// 제한 YAML 문자열을 [`YamlValue`] 트리로 로드합니다.
///
/// 크기 상한·문자 집합을 먼저 검사한 뒤, 하위 parser에서 이벤트를 하나씩 당겨 처리합니다.
pub fn load_restricted(input: &str) -> Result<YamlValue, LoaderError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(LoaderError {
            kind: LoaderErrorKind::InputTooLarge,
            line: 0,
            col: 0,
            message: format!(
                "입력이 {} bytes로 상한 {MAX_INPUT_BYTES} bytes를 넘습니다",
                input.len()
            ),
        });
    }
    if let Some((line, col, c)) = find_non_printable(input) {
        return Err(LoaderError {
            kind: LoaderErrorKind::NonPrintable,
            line,
            col,
            message: format!("허용되지 않는 문자 U+{:04X}", u32::from(c)),
        });
    }

    let lookahead = Rc::new(Lookahead {
        consumed: Cell::new(0),
        base: Cell::new(0),
        exceeded: Cell::new(false),
    });
    let mut parser = Parser::new(BudgetedChars {
        chars: input.chars(),
        lookahead: Rc::clone(&lookahead),
    });
    let mut receiver = RestrictedReceiver::new();
    loop {
        let next = parser.next_token();
        // 선읽기 상한을 넘었으면 scanner가 본 입력 끝은 가짜이므로 결과와 무관하게 거부합니다.
        if lookahead.exceeded.get() {
            let (line, col) = char_position(input, lookahead.base.get());
            return Err(LoaderError {
                kind: LoaderErrorKind::LookaheadTooLarge,
                line,
                col,
                message: format!(
                    "이벤트 하나에 {MAX_EVENT_LOOKAHEAD_CHARS}자를 넘는 선읽기가 필요합니다\
                     (너무 긴 스칼라·flow collection·주석)"
                ),
            });
        }
        lookahead.base.set(lookahead.consumed.get());
        match next {
            Ok((event, mark)) => {
                let end = matches!(event, Event::StreamEnd);
                receiver.on_event(event, mark);
                // 첫 오류에서 멈춥니다(이후 입력은 결과에 영향 없음).
                if end || receiver.error.is_some() {
                    break;
                }
            }
            Err(scan_error) => {
                let mark = scan_error.marker();
                return Err(LoaderError {
                    kind: LoaderErrorKind::Parse,
                    line: mark.line(),
                    col: mark.col(),
                    message: scan_error.info().to_string(),
                });
            }
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

    // --- XVI.6.1: fuzz가 찾은 적대적 입력 방어 ---

    #[test]
    fn rejects_input_over_size_limit() {
        let at_limit = format!("a: {}", "x".repeat(MAX_INPUT_BYTES - 3));
        // 크기 상한 자체는 통과(선읽기 상한에서 별도로 거부될 수 있음).
        assert_ne!(
            load_restricted(&at_limit).map_err(|e| e.kind),
            Err(LoaderErrorKind::InputTooLarge)
        );
        let over = "#".repeat(MAX_INPUT_BYTES + 1);
        assert_eq!(err(&over).kind.code(), "input-too-large");
    }

    #[test]
    fn raw_nul_is_rejected_instead_of_truncating() {
        // 하위 scanner는 NUL을 입력 끝으로 보아 뒤 내용을 버렸음({a: b}로 성공).
        let e = err("a: b\n\u{0}\nc: [unclosed\n");
        assert_eq!(e.kind.code(), "non-printable");
        assert_eq!((e.line, e.col), (2, 1));
        assert_eq!(err("a: b\u{0}c: d\n").kind.code(), "non-printable");
    }

    #[test]
    fn rejects_non_printable_but_allows_printable_specials() {
        for bad in [
            "a: \u{1}\n",
            "a: \"\u{7f}\"\n",
            "a: \u{9f}\n",
            "a: \u{fffe}\n",
        ] {
            assert_eq!(err(bad).kind.code(), "non-printable", "입력: {bad:?}");
        }
        // TAB, NEL, NBSP, BOM, 보조 평면 문자는 c-printable.
        for ok in [
            "a: x\ty\n",
            "a: x\u{85}y\n",
            "a: x\u{a0}y\n",
            "\u{feff}a: b\n",
            "a: 😀\n",
        ] {
            assert!(load_restricted(ok).is_ok(), "입력: {ok:?}");
        }
        // 이스케이프로 표현한 제어 문자는 값으로 허용.
        let v = load_restricted("a: \"x\\0y\\x01\"\n").unwrap();
        assert_eq!(
            v.get("a").and_then(YamlValue::as_scalar),
            Some("x\u{0}y\u{1}")
        );
    }

    #[test]
    fn deep_block_nesting_is_rejected_without_stack_overflow() {
        // 재귀 load였다면 2 MiB stack thread에서 stack overflow로 abort.
        let deep = format!("{}x\n", "- ".repeat(100_000));
        assert_eq!(err(&deep).kind.code(), "nesting-too-deep");
        let mut mapping = String::new();
        for i in 0..200 {
            mapping.push_str(&" ".repeat(i));
            mapping.push_str("k:\n");
        }
        assert_eq!(err(&mapping).kind.code(), "nesting-too-deep");
    }

    #[test]
    fn nesting_limit_is_exact() {
        let nested = |depth: usize| format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        assert!(load_restricted(&nested(MAX_NESTING_DEPTH)).is_ok());
        assert_eq!(
            err(&nested(MAX_NESTING_DEPTH + 1)).kind.code(),
            "nesting-too-deep"
        );
    }

    #[test]
    fn many_keys_are_checked_in_linear_time() {
        // 제곱 시간 중복 검사였다면 debug 빌드에서 수십 초.
        let many: String = (0..50_000).map(|i| format!("k{i}: v\n")).collect();
        let started = std::time::Instant::now();
        let v = load_restricted(&many).unwrap();
        assert_eq!(v.as_mapping().map(<[_]>::len), Some(50_000));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        // 많은 키 뒤의 중복도 여전히 거부.
        let dup = format!("{many}k123: again\n");
        let e = err(&dup);
        assert_eq!(e.kind.code(), "duplicate-key");
        assert_eq!(e.line, 50_001);
    }

    #[test]
    fn lookahead_limit_bounds_long_scalars_and_flow_collections() {
        let long_scalar = format!("a: {}\n", "x".repeat(MAX_EVENT_LOOKAHEAD_CHARS + 1));
        let e = err(&long_scalar);
        assert_eq!(e.kind.code(), "lookahead-too-large");
        assert_eq!(e.line, 1);
        // key가 될 수 있는 위치(문서 최상위)의 flow collection은 scanner가 전체를 버퍼링함.
        let big_flow = format!("[{}x]\n", "x,".repeat(MAX_EVENT_LOOKAHEAD_CHARS));
        assert_eq!(err(&big_flow).kind.code(), "lookahead-too-large");
        let comments = format!("{}a: b\n", "# 주석\n".repeat(MAX_EVENT_LOOKAHEAD_CHARS / 4));
        assert_eq!(err(&comments).kind.code(), "lookahead-too-large");
        // 상한 안의 스칼라·flow collection은 허용(잘림 없이 전체 값).
        let ok_scalar = format!("a: {}\n", "x".repeat(MAX_EVENT_LOOKAHEAD_CHARS / 2));
        let v = load_restricted(&ok_scalar).unwrap();
        assert_eq!(
            v.get("a").and_then(YamlValue::as_scalar).map(str::len),
            Some(MAX_EVENT_LOOKAHEAD_CHARS / 2)
        );
        let ok_flow = format!("[{}x]\n", "x,".repeat(1_000));
        let v = load_restricted(&ok_flow).unwrap();
        assert_eq!(v.as_sequence().map(<[_]>::len), Some(1_001));
        // 작은 이벤트가 많은 긴 block 문서는 상한과 무관.
        let long_block: String = "- item\n".repeat(MAX_EVENT_LOOKAHEAD_CHARS);
        assert!(load_restricted(&long_block).is_ok());
    }
}
