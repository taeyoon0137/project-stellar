//! Text node 부분 편집. 조합이 확정된 한 번의 입력을 한 transaction으로 기록합니다.
//!
//! - [`ReplaceText`](`replace-text`): Unicode scalar 범위의 본문 교체입니다. 빈 범위는 삽입, 빈 문자열은
//!   삭제입니다. ADR-0079 결정 1의 "ReplaceTextRange"가 이 종류입니다(같은 의미의 종류를 따로 만들지 않음).
//!   style run·paragraph 갱신은 VI.7.5 helper [`edit_text`]가 정합니다.
//! - [`SetTextRangeStyle`](`set-text-range-style`, XII.10.32): 범위의 문자 style을 한 style로 바꾸거나
//!   지웁니다(기본 style). 본문은 그대로입니다.
//!
//! 입력 종류와 caret 정보는 history grouping key(VII.4.3)로 넘깁니다. 조합(IME) 중 문자열은 transient이고
//! 확정한 문자열만 command가 됩니다. 두 command의 역은 원래 레코드를 되돌리는 [`RestoreNodeRecords`]입니다.
use crate::{
    codec::{CommandCodecError, Reader, Writer},
    command::CommandError,
    convert::RestoreNodeRecords,
    edit::EditContext,
};
use stellar_core_types::ids::StableId;
use stellar_core_types::text::UserText;
use stellar_document::{
    model::DocumentModel,
    records::NodePayload,
    text::{
        edit_text, normalize_style_runs, scalar_len, StyleRun, TextIndex, TextPayload, TextRange,
    },
};

/// UTF-8 바이트 위치와 무관한 Unicode scalar 반열린 범위.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceText {
    /// 편집할 Text node.
    pub node: StableId,
    /// 교체 범위. 빈 범위는 삽입입니다.
    pub range: TextRange,
    /// 확정된 본문. 빈 문자열은 삭제입니다.
    pub replacement: String,
}

impl ReplaceText {
    /// scalar 범위가 역전된 요청을 거부합니다.
    pub fn new(node: StableId, start: u32, end: u32, replacement: String) -> Option<Self> {
        Some(Self {
            node,
            range: TextRange::new(start, end)?,
            replacement,
        })
    }

    fn source<'a>(&self, model: &'a DocumentModel) -> Result<&'a TextPayload, CommandError> {
        match model.payload(self.node) {
            Some(NodePayload::Text(text)) => Ok(text),
            None => Err(CommandError::UnknownNode { id: self.node }),
            _ => Err(CommandError::InvalidRequest {
                reason: "Text 본문은 Text node에서만 편집할 수 있습니다",
            }),
        }
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let text = self.source(model)?;
        if self.range.start > self.range.end || text.slice(self.range).is_none() {
            return Err(CommandError::InvalidRequest {
                reason: "Text 교체 범위가 본문 밖입니다",
            });
        }
        Ok(())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<RestoreNodeRecords, CommandError> {
        self.validate(model)?;
        // 붙여넣기와 삭제는 여러 문단·스타일을 합칠 수 있어 역변환에는 정확한 이전 레코드가 필요합니다.
        let original = model
            .record(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        Ok(RestoreNodeRecords {
            records: vec![original],
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        self.validate(ctx.model())?;
        let original = self.source(ctx.model())?;
        if original.slice(self.range) == Some(self.replacement.as_str()) {
            return Ok(());
        }
        // 현재 저장 문서는 숫자 스타일 ID를 보유합니다. 별도 스타일 정의를 추측하지 않습니다.
        let new = edit_text(original, self.range, &self.replacement, &[]).map_err(|_| {
            CommandError::InvalidRequest {
                reason: "Text 본문 편집이 유효하지 않습니다",
            }
        })?;
        ctx.set_payload(self.node, NodePayload::Text(new))?;
        ctx.changes().structure(self.node);
        Ok(())
    }
}

/// Text node의 scalar 범위에 문자 style 하나를 적용하거나(`Some`) 지웁니다(`None`, 기본 style).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetTextRangeStyle {
    /// 편집할 Text node.
    pub node: StableId,
    /// 비어 있지 않은 scalar 범위.
    pub range: TextRange,
    /// 적용할 문서 문자 style 번호(`DocumentTextStyles::runs` key). `None`이면 범위를 기본 style로 둡니다.
    pub style: Option<u32>,
}

impl SetTextRangeStyle {
    fn source<'a>(&self, model: &'a DocumentModel) -> Result<&'a TextPayload, CommandError> {
        match model.payload(self.node) {
            Some(NodePayload::Text(text)) => Ok(text),
            None => Err(CommandError::UnknownNode { id: self.node }),
            _ => Err(CommandError::InvalidRequest {
                reason: "Text style은 Text node에서만 바꿀 수 있습니다",
            }),
        }
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let text = self.source(model)?;
        if self.range.is_empty() || self.range.end.0 > scalar_len(text.content()) {
            return Err(CommandError::InvalidRequest {
                reason: "Text style 범위가 비었거나 본문 밖입니다",
            });
        }
        let runs = &model.definition().text_styles.runs;
        // style 표가 없는 구형 문서는 숫자 style 번호만 보유하므로 표 검사를 하지 않습니다.
        if let Some(style) = self.style {
            if !runs.is_empty() && !runs.contains_key(&style) {
                return Err(CommandError::InvalidRequest {
                    reason: "문서 style 표에 없는 문자 style입니다",
                });
            }
        }
        Ok(())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<RestoreNodeRecords, CommandError> {
        let original = model
            .record(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        Ok(RestoreNodeRecords {
            records: vec![original],
        })
    }

    /// 범위 밖 부분만 남기도록 기존 run을 자르고, 새 style run을 더해 canonical 형식으로 정규화합니다.
    fn restyled(&self, text: &TextPayload) -> Result<TextPayload, CommandError> {
        let (a, b) = (self.range.start.0, self.range.end.0);
        let mut runs: Vec<StyleRun> = Vec::with_capacity(text.style_runs().len() + 2);
        let piece = |start: u32, end: u32, style: u32| StyleRun {
            range: TextRange {
                start: TextIndex(start),
                end: TextIndex(end),
            },
            style,
        };
        for run in text.style_runs() {
            let (s, e) = (run.range.start.0, run.range.end.0);
            if e <= a || s >= b {
                runs.push(*run);
                continue;
            }
            if s < a {
                runs.push(piece(s, a, run.style));
            }
            if e > b {
                runs.push(piece(b, e, run.style));
            }
        }
        if let Some(style) = self.style {
            runs.push(piece(a, b, style));
        }
        runs.sort_by_key(|run| (run.range.start.0, run.range.end.0));
        let runs = normalize_style_runs(&runs, &[]);
        TextPayload::new(
            UserText::new(text.content()),
            text.paragraphs().to_vec(),
            runs,
        )
        .map_err(|_| CommandError::InvalidRequest {
            reason: "Text style 편집 결과가 유효하지 않습니다",
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        self.validate(ctx.model())?;
        let text = self.source(ctx.model())?;
        let next = self.restyled(text)?;
        if &next == text {
            return Ok(());
        }
        ctx.set_payload(self.node, NodePayload::Text(next))?;
        ctx.changes().structure(self.node);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        out.u32(self.range.start.0);
        out.u32(self.range.end.0);
        match self.style {
            None => out.u8(0),
            Some(style) => {
                out.u8(1);
                out.u32(style);
            }
        }
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<SetTextRangeStyle, CommandCodecError> {
        let node = input.id()?;
        let range =
            TextRange::new(input.u32()?, input.u32()?).ok_or(CommandCodecError::Malformed {
                reason: "Text style 범위 역전",
            })?;
        let style = if input.bool()? {
            Some(input.u32()?)
        } else {
            None
        };
        Ok(SetTextRangeStyle { node, range, style })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_fixtures::{definition, frame, sid};
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use crate::{
        codec::{decode_command, encode_command},
        command::Command,
    };
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::records::NodeRecord;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_document::text::Paragraph;

    fn run(start: u32, end: u32, style: u32) -> StyleRun {
        StyleRun {
            range: TextRange::new(start, end).unwrap(),
            style,
        }
    }

    /// root(10) 아래 Text(20) "안녕 세상"(scalar 5), run [0,2)=1.
    fn state() -> DocumentState {
        let mut model = DocumentModel::new(definition());
        model.insert_node(frame(10, None, 0x10)).unwrap();
        let text = TextPayload::new(
            UserText::new("안녕 세상"),
            vec![Paragraph { style: 0 }],
            vec![run(0, 2, 1)],
        )
        .unwrap();
        model
            .insert_node(NodeRecord {
                id: sid(20),
                parent: Some(sid(10)),
                order_key: OrderKey::from_bytes(vec![0x10]).unwrap(),
                visible: true,
                locked: false,
                display_name: "text".into(),
                payload: NodePayload::Text(text),
            })
            .unwrap();
        DocumentState::new(model)
    }

    fn text(state: &DocumentState) -> TextPayload {
        match state.model().payload(sid(20)) {
            Some(NodePayload::Text(text)) => text.clone(),
            other => panic!("{other:?}"),
        }
    }

    /// apply → 확인 → inverse로 원래 문서 → codec 왕복.
    fn fixture(command: Command, check: impl FnOnce(&TextPayload)) {
        let mut state = state();
        let before = model_snapshot_json(state.model()).unwrap();
        let outcome = state
            .execute(&CommandRequest::at(&state, command.clone()))
            .unwrap();
        check(&text(&state));
        state
            .execute(&CommandRequest::at(&state, outcome.inverse))
            .unwrap();
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
        let bytes = encode_command(&command);
        assert_eq!(decode_command(&bytes).unwrap(), command);
        for cut in 0..bytes.len() {
            assert!(decode_command(&bytes[..cut]).is_err());
        }
    }

    fn replace(start: u32, end: u32, text: &str) -> Command {
        Command::ReplaceText(ReplaceText::new(sid(20), start, end, text.into()).unwrap())
    }

    fn style(start: u32, end: u32, style: Option<u32>) -> Command {
        Command::SetTextRangeStyle(SetTextRangeStyle {
            node: sid(20),
            range: TextRange::new(start, end).unwrap(),
            style,
        })
    }

    #[test]
    fn insert_delete_and_replace_fixtures() {
        // 삽입: run [0,2) 끝에 붙인 문자는 왼쪽 style을 상속합니다.
        fixture(replace(2, 2, "하"), |t| {
            assert_eq!(t.content(), "안녕하 세상");
            assert_eq!(t.style_runs(), &[run(0, 3, 1)]);
        });
        // 삭제: run 안 문자를 지우면 run이 줄어듭니다.
        fixture(replace(0, 1, ""), |t| {
            assert_eq!(t.content(), "녕 세상");
            assert_eq!(t.style_runs(), &[run(0, 1, 1)]);
        });
        // 교체: 줄바꿈이 들어가면 paragraph가 늘어납니다.
        fixture(replace(2, 3, "\n"), |t| {
            assert_eq!(t.content(), "안녕\n세상");
            assert_eq!(t.paragraphs().len(), 2);
        });
        // 한글 조합 확정: 조합 중(ㅎ→하→한)은 command가 아니고, 확정한 "한"만 scalar 하나로 들어갑니다.
        fixture(replace(5, 5, "한"), |t| {
            assert_eq!(t.content(), "안녕 세상한");
            assert_eq!(scalar_len(t.content()), 6);
        });
        // 조합형 자모(NFD)도 정규화하지 않고 scalar 그대로 보존합니다.
        fixture(replace(5, 5, "\u{1112}\u{1161}\u{11AB}"), |t| {
            assert_eq!(t.content(), "안녕 세상\u{1112}\u{1161}\u{11AB}");
        });
    }

    #[test]
    fn range_style_fixtures() {
        // 범위 style: 기존 run 위 일부를 다른 style로.
        fixture(style(1, 4, Some(2)), |t| {
            assert_eq!(t.content(), "안녕 세상");
            assert_eq!(t.style_runs(), &[run(0, 1, 1), run(1, 4, 2)]);
        });
        // 지우기: run 가운데를 기본 style로 두면 두 조각이 남습니다.
        fixture(style(1, 2, None), |t| {
            assert_eq!(t.style_runs(), &[run(0, 1, 1)]);
        });
        // 같은 style로 이어 붙이면 하나로 합쳐집니다.
        fixture(style(2, 5, Some(1)), |t| {
            assert_eq!(t.style_runs(), &[run(0, 5, 1)]);
        });
        let mut state = state();
        let before = model_snapshot_json(state.model()).unwrap();
        // 빈 범위·범위 밖·Text 아닌 node는 거부합니다.
        for command in [
            style(2, 2, Some(1)),
            style(0, 9, Some(1)),
            Command::SetTextRangeStyle(SetTextRangeStyle {
                node: sid(10),
                range: TextRange::new(0, 1).unwrap(),
                style: None,
            }),
        ] {
            assert!(state.execute(&CommandRequest::at(&state, command)).is_err());
        }
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
        // style 표가 있는 문서는 표에 없는 번호를 거부합니다.
        let mut model = DocumentModel::new({
            let mut d = definition();
            d.text_styles = stellar_document::text_styles::DocumentTextStyles::initial(sid(90));
            d
        });
        model.insert_node(frame(10, None, 0x10)).unwrap();
        let mut record = state.model().record(sid(20)).unwrap();
        record.payload = NodePayload::Text(
            TextPayload::new(UserText::new("ab"), vec![Paragraph { style: 0 }], vec![]).unwrap(),
        );
        model.insert_node(record).unwrap();
        let mut styled = DocumentState::new(model);
        assert!(styled
            .execute(&CommandRequest::at(&styled, style(0, 1, Some(7))))
            .is_err());
        assert!(styled
            .execute(&CommandRequest::at(&styled, style(0, 1, Some(0))))
            .is_ok());
    }

    #[test]
    fn text_command_canonical_bytes_keep_scalar_indices_and_utf8() {
        let node = StableId::new_v7(7, 0, 1);
        let edit = Command::ReplaceText(ReplaceText::new(node, 1, 4, "한🎨".into()).unwrap());
        let bytes = encode_command(&edit);
        assert_eq!(decode_command(&bytes).unwrap(), edit);
        assert_eq!(&bytes[..2], &0x0050u16.to_le_bytes());
        for cut in 0..bytes.len() {
            assert!(decode_command(&bytes[..cut]).is_err());
        }
    }
}
