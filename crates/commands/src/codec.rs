//! command canonical 직렬화 (VII.1.2).
//!
//! storage에 의존하지 않는 canonical 규칙(V.1: 고정 폭 little-endian, `u32` 길이 프리픽스,
//! trailing bytes 거부)을 따릅니다.
//!
//! command 하나의 형식:
//!
//! | 필드 | 형식 |
//! |---|---|
//! | kind | `u16` LE ([`CommandKind`] numeric) |
//! | payload 길이 | `u32` LE |
//! | payload | 종류별 canonical bytes |
//!
//! 디코딩은 registry에 없는 kind를 [`CommandCodecError::UnknownKind`]로 거부합니다 — payload를
//! 해석하지 않고 거부하므로 알 수 없는 command가 실행 경로에 닿지 않습니다.

use std::fmt;

use stellar_core_types::ids::StableId;

/// command codec 버전(ADR-0079). 새 command 종류나 종류별 payload 형식이 생기면 올립니다.
///
/// - 1: VII.1.2 ~ `replace-text`(0x0050)까지.
/// - 2: ADR-0079가 더한 종류(Page·View, 외부 snapshot 반영, Path point, Text 범위 style, 인스턴스 교체).
///
/// 각 종류가 처음 들어간 버전은 [`CommandKind::codec_version`]입니다. history journal header가 이 값을
/// 기록하므로 옛 reader는 모르는 종류를 담은 journal의 복원만 포기하고 문서는 건드리지 않습니다.
pub const COMMAND_CODEC_VERSION: u32 = 2;

use crate::command::Command;
use crate::convert::{ConvertFragmentToFrame, ConvertShapeToPath, RestoreNodeRecords};
use crate::duplicate::DuplicateSubtree;
use crate::kind::CommandKind;
use crate::modifier_ops::{InsertModifier, MoveModifier, RemoveModifier, SetModifierProperty};
use crate::node_ops::{DeleteSubtree, InsertNode, MoveNode};
use crate::page_view::{AddPage, AddView, RemovePage, RemoveView, RenamePage, RenameView};
use crate::property::SetProperty;

/// command 직렬화 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandCodecError {
    /// registry에 없는 command kind.
    UnknownKind {
        /// 발견된 값.
        value: u16,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for CommandCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandCodecError::UnknownKind { value } => {
                write!(f, "알 수 없는 command kind: {value:#06x}")
            }
            CommandCodecError::Malformed { reason } => {
                write!(f, "command binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for CommandCodecError {}

/// canonical byte writer.
#[derive(Debug, Default)]
pub(crate) struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub(crate) fn new() -> Writer {
        Writer { buf: Vec::new() }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    /// 길이 프리픽스 없이 bytes를 그대로 덧붙입니다(자체 framing을 가진 값).
    pub(crate) fn raw(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub(crate) fn bool(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// f32 LE(bit 그대로). 값은 finite 타입에서 오므로 유한합니다.
    pub(crate) fn f32(&mut self, value: f32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn id(&mut self, id: StableId) {
        self.buf.extend_from_slice(&id.to_bytes());
    }

    /// 선택적 id: flag `u8`(0/1) + (있으면) 16바이트.
    pub(crate) fn opt_id(&mut self, id: Option<StableId>) {
        match id {
            None => self.u8(0),
            Some(id) => {
                self.u8(1);
                self.id(id);
            }
        }
    }

    /// `u32` 길이 프리픽스 + bytes.
    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.u32(len_u32(bytes.len()));
        self.buf.extend_from_slice(bytes);
    }

    pub(crate) fn str(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }

    /// 목록 길이 프리픽스.
    pub(crate) fn len(&mut self, len: usize) {
        self.u32(len_u32(len));
    }
}

/// 길이를 `u32`로 줄입니다. command payload는 메모리 안 값이라 4 GiB를 넘지 않습니다
/// (넘으면 포화시켜 디코딩이 길이 불일치로 거부하게 합니다).
fn len_u32(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// canonical byte reader.
#[derive(Debug)]
pub(crate) struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(input: &'a [u8]) -> Reader<'a> {
        Reader { input, pos: 0 }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], CommandCodecError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.input.len())
            .ok_or(CommandCodecError::Malformed {
                reason: "입력 부족",
            })?;
        let slice = &self.input[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, CommandCodecError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn bool(&mut self) -> Result<bool, CommandCodecError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(CommandCodecError::Malformed {
                reason: "bool 값은 0 또는 1",
            }),
        }
    }

    pub(crate) fn u16(&mut self) -> Result<u16, CommandCodecError> {
        let mut raw = [0u8; 2];
        raw.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(raw))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, CommandCodecError> {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(raw))
    }

    /// f32 LE. NaN·±Infinity는 거부합니다.
    pub(crate) fn f32(&mut self) -> Result<f32, CommandCodecError> {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(self.take(4)?);
        let value = f32::from_le_bytes(raw);
        if value.is_finite() {
            Ok(value)
        } else {
            Err(CommandCodecError::Malformed {
                reason: "유한하지 않은 f32",
            })
        }
    }

    pub(crate) fn u64(&mut self) -> Result<u64, CommandCodecError> {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(raw))
    }

    pub(crate) fn id(&mut self) -> Result<StableId, CommandCodecError> {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(self.take(16)?);
        Ok(StableId::from_bytes(raw))
    }

    pub(crate) fn opt_id(&mut self) -> Result<Option<StableId>, CommandCodecError> {
        if self.bool()? {
            Ok(Some(self.id()?))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn bytes(&mut self) -> Result<&'a [u8], CommandCodecError> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    pub(crate) fn string(&mut self) -> Result<String, CommandCodecError> {
        let bytes = self.bytes()?;
        String::from_utf8(bytes.to_vec()).map_err(|_| CommandCodecError::Malformed {
            reason: "UTF-8이 아닌 문자열",
        })
    }

    /// 목록 길이. 남은 입력보다 큰 값은(원소마다 최소 1바이트) 거부해 거대 할당을 막습니다.
    pub(crate) fn len(&mut self) -> Result<usize, CommandCodecError> {
        let len = self.u32()? as usize;
        if len > self.remaining() {
            return Err(CommandCodecError::Malformed {
                reason: "목록 길이가 남은 입력보다 큽니다",
            });
        }
        Ok(len)
    }

    /// 아직 읽지 않은 입력(소비하지 않음).
    pub(crate) fn rest(&self) -> &'a [u8] {
        &self.input[self.pos..]
    }

    pub(crate) fn remaining(&self) -> usize {
        self.input.len() - self.pos
    }

    pub(crate) fn expect_end(&self) -> Result<(), CommandCodecError> {
        if self.pos == self.input.len() {
            Ok(())
        } else {
            Err(CommandCodecError::Malformed {
                reason: "trailing bytes",
            })
        }
    }
}

/// command를 canonical bytes로 인코딩합니다.
pub fn encode_command(command: &Command) -> Vec<u8> {
    let mut out = Writer::new();
    encode_command_into(command, &mut out);
    out.into_bytes()
}

/// command를 writer에 덧붙입니다.
pub(crate) fn encode_command_into(command: &Command, out: &mut Writer) {
    let mut payload = Writer::new();
    encode_payload(command, &mut payload);
    out.u16(command.kind().as_u16());
    out.bytes(&payload.into_bytes());
}

fn encode_payload(command: &Command, out: &mut Writer) {
    match command {
        Command::InsertNode(command) => command.encode_into(out),
        Command::DeleteSubtree(command) => command.encode_into(out),
        Command::MoveNode(command) => command.encode_into(out),
        Command::SetProperty(command) => command.encode_into(out),
        Command::InsertModifier(command) => command.encode_into(out),
        Command::RemoveModifier(command) => command.encode_into(out),
        Command::MoveModifier(command) => command.encode_into(out),
        Command::SetModifierProperty(command) => command.encode_into(out),
        Command::ConvertFragmentToFrame(command) => command.encode_into(out),
        Command::ConvertShapeToPath(command) => command.encode_into(out),
        Command::RestoreNodeRecords(command) => command.encode_into(out),
        Command::ReplaceText(edit) => {
            out.id(edit.node);
            out.u32(edit.range.start.0);
            out.u32(edit.range.end.0);
            out.str(&edit.replacement);
        }
        Command::DuplicateSubtree(command) => command.encode_into(out),
        Command::AddPage(command) => command.encode_into(out),
        Command::RemovePage(command) => command.encode_into(out),
        Command::RenamePage(command) => command.encode_into(out),
        Command::AddView(command) => command.encode_into(out),
        Command::RemoveView(command) => command.encode_into(out),
        Command::RenameView(command) => command.encode_into(out),
        Command::ApplyExternalSnapshot(command) => command.encode_into(out),
        Command::EditPathPoint(command) => command.encode_into(out),
        #[cfg(test)]
        Command::TestProbe(probe) => {
            out.id(probe.node);
            out.str(&probe.name);
            out.bool(probe.fail_after_apply);
        }
    }
}

/// [`encode_command`] bytes에서 command를 복원합니다. 입력 전체를 소비해야 합니다.
pub fn decode_command(bytes: &[u8]) -> Result<Command, CommandCodecError> {
    let mut reader = Reader::new(bytes);
    let command = decode_command_from(&mut reader)?;
    reader.expect_end()?;
    Ok(command)
}

/// reader에서 command 하나를 읽습니다.
pub(crate) fn decode_command_from(reader: &mut Reader<'_>) -> Result<Command, CommandCodecError> {
    let value = reader.u16()?;
    let kind = CommandKind::from_u16(value).ok_or(CommandCodecError::UnknownKind { value })?;
    let payload = reader.bytes()?;
    let mut payload = Reader::new(payload);
    let command = decode_payload(kind, &mut payload)?;
    payload.expect_end()?;
    Ok(command)
}

fn decode_payload(kind: CommandKind, input: &mut Reader<'_>) -> Result<Command, CommandCodecError> {
    match kind {
        CommandKind::InsertNode => InsertNode::decode_from(input).map(Command::InsertNode),
        CommandKind::DeleteSubtree => DeleteSubtree::decode_from(input).map(Command::DeleteSubtree),
        CommandKind::MoveNode => MoveNode::decode_from(input).map(Command::MoveNode),
        CommandKind::SetProperty => SetProperty::decode_from(input).map(Command::SetProperty),
        CommandKind::InsertModifier => {
            InsertModifier::decode_from(input).map(Command::InsertModifier)
        }
        CommandKind::RemoveModifier => {
            RemoveModifier::decode_from(input).map(Command::RemoveModifier)
        }
        CommandKind::MoveModifier => MoveModifier::decode_from(input).map(Command::MoveModifier),
        CommandKind::SetModifierProperty => {
            SetModifierProperty::decode_from(input).map(Command::SetModifierProperty)
        }
        CommandKind::ConvertFragmentToFrame => {
            ConvertFragmentToFrame::decode_from(input).map(Command::ConvertFragmentToFrame)
        }
        CommandKind::ConvertShapeToPath => {
            ConvertShapeToPath::decode_from(input).map(Command::ConvertShapeToPath)
        }
        CommandKind::RestoreNodeRecords => {
            RestoreNodeRecords::decode_from(input).map(Command::RestoreNodeRecords)
        }
        CommandKind::ReplaceText => {
            let node = input.id()?;
            let range = stellar_document::text::TextRange::new(input.u32()?, input.u32()?).ok_or(
                CommandCodecError::Malformed {
                    reason: "Text 교체 범위 역전",
                },
            )?;
            Ok(Command::ReplaceText(crate::text_edit::ReplaceText {
                node,
                range,
                replacement: input.string()?,
            }))
        }
        CommandKind::DuplicateSubtree => {
            DuplicateSubtree::decode_from(input).map(Command::DuplicateSubtree)
        }
        CommandKind::AddPage => AddPage::decode_from(input).map(Command::AddPage),
        CommandKind::RemovePage => RemovePage::decode_from(input).map(Command::RemovePage),
        CommandKind::RenamePage => RenamePage::decode_from(input).map(Command::RenamePage),
        CommandKind::AddView => AddView::decode_from(input).map(Command::AddView),
        CommandKind::RemoveView => RemoveView::decode_from(input).map(Command::RemoveView),
        CommandKind::RenameView => RenameView::decode_from(input).map(Command::RenameView),
        CommandKind::ApplyExternalSnapshot => {
            crate::snapshot_apply::ApplyExternalSnapshot::decode_from(input)
                .map(Command::ApplyExternalSnapshot)
        }
        CommandKind::EditPathPoint => {
            crate::path_edit::EditPathPoint::decode_from(input).map(Command::EditPathPoint)
        }
        #[cfg(test)]
        CommandKind::TestProbe => Ok(Command::TestProbe(crate::command::TestProbe {
            node: input.id()?,
            name: input.string()?,
            fail_after_apply: input.bool()?,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_fixtures::sid;
    use crate::command::TestProbe;

    fn probe() -> Command {
        Command::TestProbe(TestProbe {
            node: sid(11),
            name: "이름 🎨".into(),
            fail_after_apply: true,
        })
    }

    #[test]
    fn command_round_trips() {
        let command = probe();
        let bytes = encode_command(&command);
        assert_eq!(&bytes[..2], &0xFFF0u16.to_le_bytes(), "kind LE");
        assert_eq!(decode_command(&bytes).unwrap(), command);
        assert_eq!(encode_command(&decode_command(&bytes).unwrap()), bytes);
    }

    #[test]
    fn unknown_command_kind_is_rejected() {
        // 검증 기준(VII.1.2): unknown command kind 거부.
        let mut bytes = encode_command(&probe());
        for value in [0x0000u16, 0x00FF, 0x7777, 0xFFFF] {
            bytes[..2].copy_from_slice(&value.to_le_bytes());
            assert_eq!(
                decode_command(&bytes),
                Err(CommandCodecError::UnknownKind { value })
            );
        }
        // payload가 비어 있어도 kind 검사가 먼저입니다.
        assert_eq!(
            decode_command(&[0x34, 0x12, 0, 0, 0, 0]),
            Err(CommandCodecError::UnknownKind { value: 0x1234 })
        );
    }

    #[test]
    fn malformed_framing_is_rejected() {
        let bytes = encode_command(&probe());
        for cut in [0, 1, 2, 5, bytes.len() - 1] {
            assert!(decode_command(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            decode_command(&trailing),
            Err(CommandCodecError::Malformed {
                reason: "trailing bytes"
            })
        );
        // payload 길이가 실제보다 길면 거부.
        let mut long = bytes.clone();
        long[2..6].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_command(&long).is_err());
        // bool flag 2는 거부.
        let mut flag = bytes;
        let last = flag.len() - 1;
        flag[last] = 2;
        assert_eq!(
            decode_command(&flag),
            Err(CommandCodecError::Malformed {
                reason: "bool 값은 0 또는 1"
            })
        );
    }
}
