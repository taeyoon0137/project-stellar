//! Text node 부분 편집. 조합이 확정된 한 번의 입력을 한 transaction으로 기록합니다.
use crate::{command::CommandError, convert::RestoreNodeRecords, edit::EditContext};
use stellar_core_types::ids::StableId;
use stellar_document::{
    model::DocumentModel,
    records::NodePayload,
    text::{edit_text, TextPayload, TextRange},
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        codec::{decode_command, encode_command},
        command::Command,
    };

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
