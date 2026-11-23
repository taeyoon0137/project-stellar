//! 영속 history journal schema (XXV.1.1, 로드맵).
//!
//! 세션 메모리 전용 [`HistoryEntry`](VII.4.1)를 세션 간 복원(XXV.1.2~)에 쓸 수 있도록 **안정된 binary
//! schema와 version**을 정의합니다. 이 형식은 project serializer(문서 chunk·manifest)와 별개이며, 기록
//! 위치(ignored path·OS app data)와 복원 정책은 후속 작업(XXV.1.2~XXV.1.5)의 몫입니다.
//!
//! journal 하나는 창 history의 **복원 가능한 전체 상태**입니다: undo stack, redo stack, 다음 sequence와
//! 그 상태가 기준으로 삼은 저장 세대([`JournalBase`]). undo stack 맨 위 상태의 문서가 base 세대 내용과
//! 같아야 하며(기록 쪽 계약), 복원은 현재 세대가 base와 같을 때만 합니다(XXV.1.3).
//!
//! canonical 규칙(V.1): 고정 폭 little-endian, `u32` 길이 프리픽스, trailing bytes 거부.
//!
//! | 필드 | 형식 |
//! |---|---|
//! | magic | `b"STLRHJNL"` 8바이트 |
//! | format version | `u32` — reader가 지원하는 값과 **정확히** 같아야 함 |
//! | schema version | `u32` — 기록 시점 schema |
//! | minimum reader version | `u32` — 이 journal을 읽는 데 필요한 최소 schema |
//! | minimum writer version | `u32` — 이 journal을 이어 기록(수정 저장)해도 안전한 최소 schema |
//! | command codec version | `u32` — schema 2 이상에만 있음. 항목 command를 기록한 codec 버전(ADR-0079) |
//! | window | 16바이트 [`WindowSessionId`] |
//! | base generation | `u64` [`StorageGeneration`] |
//! | base manifest hash | 32바이트 — base 세대 manifest content hash(`design/current`가 가리키는 값) |
//! | next sequence | `u64` — 복원한 history가 이어 할당할 다음 [`HistorySequence`] |
//! | undo 항목 수 | `u32` + (`u32` 길이 + 항목 bytes)* — 오래된 것부터 |
//! | redo 항목 수 | `u32` + (`u32` 길이 + 항목 bytes)* — 마지막이 다음 redo |
//!
//! version 규칙(V.8.1 `assess_compatibility`와 같음): 선언은 `minimum reader <= minimum writer <=
//! schema`여야 하고, reader schema `A`가 `A < minimum reader`이면 거부, `A < minimum writer`이면 읽기
//! 전용([`JournalAccess::ReadOnly`]: 복원은 할 수 있으나 이 reader가 같은 journal을 다시 쓰면 안 됨),
//! 그 밖은 편집 가능입니다.
//!
//! sequence 무결성: undo는 엄격 증가, redo는 엄격 감소(맨 위가 가장 작음), 마지막 undo < 마지막 redo,
//! next sequence > 모든 항목 sequence.
//!
//! 항목: sequence `u64` + origin([`CommandOrigin`] canonical) + grouping key(tag + 필드) + payload(tag 1
//! 문서: 정방향·역방향 [`CommandTransaction`] bytes / tag 2 선택: 정방향·역방향 [`SetSelection`] bytes) +
//! 선택 복원 정보(flag + 편집 전·후 선택). 모든 tag는 저장 계약이라 바꾸거나 재사용하지 않습니다.
//!
//! 개정(2026-11-14, 기록 경로 XXV.1.2 이전이라 기록된 journal 없음): redo stack, next sequence, minimum
//! writer version, base manifest hash를 schema 1에 추가했습니다.
//!
//! schema 2(ADR-0079): header에 command codec version([`COMMAND_CODEC_VERSION`])을 더했습니다. schema 1
//! journal은 codec 1로 읽습니다(새 reader는 옛 journal을 그대로 decode). 새 layout은 옛 reader가 해석할 수
//! 없으므로 minimum reader·writer를 2로 기록합니다. 옛 reader는 schema 판정(`ReaderTooOld`)에서 복원만
//! 포기하고 문서 원본은 건드리지 않습니다. reader의 codec보다 새 codec([`JournalError::CommandCodecTooNew`])
//! 이거나, header가 선언한 codec 안에 없는 command 종류([`JournalError::KindOutsideCodec`])·registry에 없는
//! 종류가 들어 있으면 같은 방식으로 복원을 포기하고 진단(봉인 사유 `unknown-command`)을 남깁니다.

use std::fmt;

use stellar_commands::codec::{CommandCodecError, COMMAND_CODEC_VERSION};
use stellar_commands::id::{CommandOrigin, WindowSessionId};
use stellar_commands::kind::CommandKind;
use stellar_commands::selection::{Selection, SetSelection};
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::revision::StorageGeneration;

use crate::entry::{
    CommandSignature, GroupingKey, HistoryEntry, HistoryPayload, HistorySequence, SelectionRestore,
    TextInputKind,
};
use crate::history::History;

/// journal magic.
pub const JOURNAL_MAGIC: [u8; 8] = *b"STLRHJNL";
/// 이 reader가 지원하는 journal 컨테이너 형식 버전.
pub const JOURNAL_FORMAT_VERSION: u32 = 1;
/// 이 writer가 기록하는 journal schema 버전.
pub const JOURNAL_SCHEMA_VERSION: u32 = 2;
/// 이 writer가 기록하는 journal을 읽는 데 필요한 최소 schema 버전.
pub const JOURNAL_MINIMUM_READER_VERSION: u32 = 2;
/// 이 writer가 기록하는 journal을 이어 기록해도 안전한 최소 schema 버전.
pub const JOURNAL_MINIMUM_WRITER_VERSION: u32 = 2;
/// header에 command codec version 필드가 생긴 schema 버전(ADR-0079).
pub const JOURNAL_CODEC_FIELD_SCHEMA: u32 = 2;

/// journal 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalError {
    /// magic이 다릅니다(journal이 아님).
    BadMagic,
    /// 지원하지 않는 컨테이너 형식 버전.
    UnsupportedFormat {
        /// 발견된 값.
        found: u32,
    },
    /// 이 reader의 schema가 journal이 요구하는 최소 버전보다 낮습니다.
    ReaderTooOld {
        /// journal이 요구하는 최소 reader schema.
        required: u32,
        /// 이 reader의 schema.
        reader: u32,
    },
    /// 버전 선언 자체가 모순입니다(`최소 reader <= 최소 writer <= schema` 위반).
    InvalidVersions,
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// 안에 담긴 command·선택 bytes 오류.
    Command(CommandCodecError),
    /// journal의 command codec이 이 reader보다 새것입니다(ADR-0079).
    CommandCodecTooNew {
        /// journal header의 codec 버전.
        found: u32,
        /// 이 reader의 codec 버전.
        supported: u32,
    },
    /// header가 선언한 codec 버전에 없는 command 종류가 있습니다(ADR-0079).
    KindOutsideCodec {
        /// command 종류 번호.
        kind: u16,
        /// header의 codec 버전.
        codec: u32,
    },
}

impl JournalError {
    /// 이 reader가 모르는 command를 담아 복원을 포기한 경우인지(ADR-0079). 문서 원본과 무관한 journal
    /// 한정 진단입니다.
    pub fn is_unknown_command(&self) -> bool {
        matches!(
            self,
            JournalError::CommandCodecTooNew { .. }
                | JournalError::KindOutsideCodec { .. }
                | JournalError::Command(CommandCodecError::UnknownKind { .. })
        )
    }
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalError::BadMagic => f.write_str("history journal magic이 아닙니다"),
            JournalError::UnsupportedFormat { found } => {
                write!(f, "지원하지 않는 journal 형식 버전 {found}")
            }
            JournalError::ReaderTooOld { required, reader } => write!(
                f,
                "journal은 schema {required} 이상 reader가 필요합니다(현재 {reader})"
            ),
            JournalError::InvalidVersions => f.write_str("journal 버전 선언이 모순입니다"),
            JournalError::Malformed { reason } => write!(f, "journal 형식 위반: {reason}"),
            JournalError::Command(error) => write!(f, "journal 항목 command 오류: {error}"),
            JournalError::CommandCodecTooNew { found, supported } => write!(
                f,
                "journal command codec {found}이 이 앱의 codec {supported}보다 새것입니다"
            ),
            JournalError::KindOutsideCodec { kind, codec } => write!(
                f,
                "journal codec {codec}에 없는 command 종류 {kind:#06x}가 있습니다"
            ),
        }
    }
}

impl std::error::Error for JournalError {}

impl From<CommandCodecError> for JournalError {
    fn from(error: CommandCodecError) -> JournalError {
        JournalError::Command(error)
    }
}

/// journal이 기준으로 삼은 저장 세대. 두 값이 모두 같아야 같은 base입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JournalBase {
    /// 창이 기준으로 삼은 storage generation 번호(§22.2 BaseGeneration).
    pub generation: StorageGeneration,
    /// base 세대 manifest content hash(32바이트, `design/current`가 가리키는 값). history crate는
    /// storage에 의존하지 않으므로 값을 해석하지 않고 같음만 비교합니다.
    pub manifest_hash: [u8; 32],
}

impl JournalBase {
    /// base를 만듭니다.
    pub const fn new(generation: StorageGeneration, manifest_hash: [u8; 32]) -> JournalBase {
        JournalBase {
            generation,
            manifest_hash,
        }
    }
}

/// reader가 journal을 어떻게 다룰 수 있는지(V.8.1 판정).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalAccess {
    /// 복원하고 이어 기록할 수 있습니다.
    Editable,
    /// 복원만 할 수 있고 이 reader가 같은 journal을 다시 쓰면 안 됩니다.
    ReadOnly,
}

/// journal header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalHeader {
    /// 컨테이너 형식 버전.
    pub format_version: u32,
    /// 기록 시점 schema 버전.
    pub schema_version: u32,
    /// 읽는 데 필요한 최소 schema 버전.
    pub minimum_reader_version: u32,
    /// 이어 기록(수정 저장)에 필요한 최소 schema 버전.
    pub minimum_writer_version: u32,
    /// 항목 command를 기록한 codec 버전(schema 1 journal은 1, ADR-0079).
    pub command_codec_version: u32,
    /// 기록한 창.
    pub window: WindowSessionId,
    /// journal이 기준으로 삼은 저장 세대.
    pub base: JournalBase,
}

impl JournalHeader {
    /// 현재 writer 버전으로 header를 만듭니다.
    pub fn current(window: WindowSessionId, base: JournalBase) -> JournalHeader {
        JournalHeader {
            format_version: JOURNAL_FORMAT_VERSION,
            schema_version: JOURNAL_SCHEMA_VERSION,
            minimum_reader_version: JOURNAL_MINIMUM_READER_VERSION,
            minimum_writer_version: JOURNAL_MINIMUM_WRITER_VERSION,
            command_codec_version: COMMAND_CODEC_VERSION,
            window,
            base,
        }
    }

    /// header bytes 길이(schema에 따라 codec 필드 유무가 다름).
    pub fn encoded_len(&self) -> usize {
        header_len(self.schema_version)
    }

    /// reader schema `reader_version`의 접근 판정(V.8.1). 읽을 수 없으면 오류입니다.
    pub fn access(&self, reader_version: u32) -> Result<JournalAccess, JournalError> {
        if self.format_version != JOURNAL_FORMAT_VERSION {
            return Err(JournalError::UnsupportedFormat {
                found: self.format_version,
            });
        }
        if self.minimum_reader_version > self.minimum_writer_version
            || self.minimum_writer_version > self.schema_version
        {
            return Err(JournalError::InvalidVersions);
        }
        if reader_version < self.minimum_reader_version {
            return Err(JournalError::ReaderTooOld {
                required: self.minimum_reader_version,
                reader: reader_version,
            });
        }
        if self.command_codec_version == 0 {
            return Err(JournalError::InvalidVersions);
        }
        if self.command_codec_version > COMMAND_CODEC_VERSION {
            return Err(JournalError::CommandCodecTooNew {
                found: self.command_codec_version,
                supported: COMMAND_CODEC_VERSION,
            });
        }
        Ok(if reader_version < self.minimum_writer_version {
            JournalAccess::ReadOnly
        } else {
            JournalAccess::Editable
        })
    }
}

/// 창 하나의 history journal(복원 가능한 history 전체 상태).
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryJournal {
    /// header.
    pub header: JournalHeader,
    /// 복원한 history가 이어 할당할 다음 sequence 값.
    pub next_sequence: u64,
    /// undo stack(오래된 것부터).
    pub undo: Vec<HistoryEntry>,
    /// redo stack(마지막이 다음 redo).
    pub redo: Vec<HistoryEntry>,
}

// --- LE writer/reader ---

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn id(&mut self, id: StableId) {
        self.0.extend_from_slice(&id.to_bytes());
    }
    fn len(&mut self, n: usize) {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.0.extend_from_slice(bytes);
    }
}

struct In<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], JournalError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.input.len())
            .ok_or(JournalError::Malformed {
                reason: "입력 부족",
            })?;
        let slice = &self.input[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, JournalError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, JournalError> {
        let mut raw = [0u8; 2];
        raw.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(raw))
    }
    fn u32(&mut self) -> Result<u32, JournalError> {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(raw))
    }
    fn u64(&mut self) -> Result<u64, JournalError> {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(raw))
    }
    fn id(&mut self) -> Result<StableId, JournalError> {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(self.take(16)?);
        Ok(StableId::from_bytes(raw))
    }
    fn len(&mut self) -> Result<usize, JournalError> {
        let n = self.u32()? as usize;
        if n > self.input.len() - self.pos {
            return Err(JournalError::Malformed {
                reason: "길이가 남은 입력보다 큽니다",
            });
        }
        Ok(n)
    }
    fn bytes(&mut self) -> Result<&'a [u8], JournalError> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn end(&self) -> Result<(), JournalError> {
        if self.pos == self.input.len() {
            Ok(())
        } else {
            Err(JournalError::Malformed {
                reason: "trailing bytes",
            })
        }
    }
}

// --- grouping key ---

const GROUP_NONE: u8 = 0;
const GROUP_POINTER: u8 = 1;
const GROUP_TEXT: u8 = 2;
const GROUP_KEYBOARD: u8 = 3;
const GROUP_SCRUB: u8 = 4;
const GROUP_AREA: u8 = 5;

fn text_kind_tag(kind: TextInputKind) -> u8 {
    match kind {
        TextInputKind::Insert => 1,
        TextInputKind::DeleteBackward => 2,
        TextInputKind::DeleteForward => 3,
        TextInputKind::Replace => 4,
    }
}

fn write_grouping(out: &mut Out, grouping: Option<&GroupingKey>) {
    match grouping {
        None => out.u8(GROUP_NONE),
        Some(GroupingKey::PointerGesture { gesture }) => {
            out.u8(GROUP_POINTER);
            out.u64(*gesture);
        }
        Some(GroupingKey::TextInput {
            node,
            kind,
            caret_before,
            caret_after,
        }) => {
            out.u8(GROUP_TEXT);
            out.id(*node);
            out.u8(text_kind_tag(*kind));
            out.u32(*caret_before);
            out.u32(*caret_after);
        }
        Some(GroupingKey::KeyboardRepeat(signature)) => {
            out.u8(GROUP_KEYBOARD);
            out.u16(signature.kind.as_u16());
            out.id(signature.target);
            match signature.key {
                None => out.u8(0),
                Some(key) => {
                    out.u8(1);
                    out.u32(key.0);
                }
            }
        }
        Some(GroupingKey::Scrub { control }) => {
            out.u8(GROUP_SCRUB);
            out.u64(*control);
        }
        Some(GroupingKey::AreaSelection) => out.u8(GROUP_AREA),
    }
}

fn read_grouping(input: &mut In<'_>) -> Result<Option<GroupingKey>, JournalError> {
    let malformed = |reason| JournalError::Malformed { reason };
    Ok(match input.u8()? {
        GROUP_NONE => None,
        GROUP_POINTER => Some(GroupingKey::PointerGesture {
            gesture: input.u64()?,
        }),
        GROUP_TEXT => {
            let node = input.id()?;
            let kind = match input.u8()? {
                1 => TextInputKind::Insert,
                2 => TextInputKind::DeleteBackward,
                3 => TextInputKind::DeleteForward,
                4 => TextInputKind::Replace,
                _ => return Err(malformed("알 수 없는 텍스트 입력 종류")),
            };
            Some(GroupingKey::TextInput {
                node,
                kind,
                caret_before: input.u32()?,
                caret_after: input.u32()?,
            })
        }
        GROUP_KEYBOARD => {
            let kind =
                CommandKind::from_u16(input.u16()?).ok_or(malformed("알 수 없는 command kind"))?;
            let target = input.id()?;
            let key = match input.u8()? {
                0 => None,
                1 => Some(PropertyKey(input.u32()?)),
                _ => return Err(malformed("signature key flag")),
            };
            Some(GroupingKey::KeyboardRepeat(CommandSignature {
                kind,
                target,
                key,
            }))
        }
        GROUP_SCRUB => Some(GroupingKey::Scrub {
            control: input.u64()?,
        }),
        GROUP_AREA => Some(GroupingKey::AreaSelection),
        _ => return Err(malformed("알 수 없는 grouping tag")),
    })
}

// --- 항목 ---

const PAYLOAD_DOCUMENT: u8 = 1;
const PAYLOAD_SELECTION: u8 = 2;

fn write_selection(out: &mut Out, selection: &Selection) {
    out.bytes(&SetSelection::new(selection.clone()).encode());
}

fn read_selection(input: &mut In<'_>) -> Result<Selection, JournalError> {
    Ok(SetSelection::decode(input.bytes()?)?.selection)
}

/// 항목 하나의 canonical bytes.
pub fn encode_entry(entry: &HistoryEntry) -> Vec<u8> {
    let mut out = Out::default();
    out.u64(entry.sequence.get());
    let mut origin = Vec::new();
    entry.origin.encode_into(&mut origin);
    out.0.extend_from_slice(&origin);
    write_grouping(&mut out, entry.grouping.as_ref());
    match &entry.payload {
        HistoryPayload::Document { forward, inverse } => {
            out.u8(PAYLOAD_DOCUMENT);
            out.bytes(&forward.encode());
            out.bytes(&inverse.encode());
        }
        HistoryPayload::Selection { forward, inverse } => {
            out.u8(PAYLOAD_SELECTION);
            out.bytes(&forward.encode());
            out.bytes(&inverse.encode());
        }
    }
    match &entry.selection {
        None => out.u8(0),
        Some(restore) => {
            out.u8(1);
            write_selection(&mut out, &restore.before);
            write_selection(&mut out, &restore.after);
        }
    }
    out.0
}

/// [`encode_entry`]의 역.
pub fn decode_entry(bytes: &[u8]) -> Result<HistoryEntry, JournalError> {
    let mut input = In {
        input: bytes,
        pos: 0,
    };
    let sequence = HistorySequence::from_u64(input.u64()?);
    let (origin, used) =
        CommandOrigin::decode_prefix(&bytes[input.pos..]).map_err(|_| JournalError::Malformed {
            reason: "잘못된 origin",
        })?;
    input.take(used)?;
    let grouping = read_grouping(&mut input)?;
    let payload = match input.u8()? {
        PAYLOAD_DOCUMENT => HistoryPayload::Document {
            forward: CommandTransaction::decode(input.bytes()?)?,
            inverse: CommandTransaction::decode(input.bytes()?)?,
        },
        PAYLOAD_SELECTION => HistoryPayload::Selection {
            forward: SetSelection::decode(input.bytes()?)?,
            inverse: SetSelection::decode(input.bytes()?)?,
        },
        _ => {
            return Err(JournalError::Malformed {
                reason: "알 수 없는 payload tag",
            })
        }
    };
    let selection = match input.u8()? {
        0 => None,
        1 => Some(SelectionRestore {
            before: read_selection(&mut input)?,
            after: read_selection(&mut input)?,
        }),
        _ => {
            return Err(JournalError::Malformed {
                reason: "선택 복원 flag",
            })
        }
    };
    input.end()?;
    Ok(HistoryEntry {
        sequence,
        origin,
        grouping,
        payload,
        selection,
    })
}

fn malformed(reason: &'static str) -> JournalError {
    JournalError::Malformed { reason }
}

/// undo·redo stack과 next sequence의 sequence 무결성을 검사합니다.
fn check_sequences(
    undo: &[HistoryEntry],
    redo: &[HistoryEntry],
    next_sequence: u64,
) -> Result<(), JournalError> {
    if undo.windows(2).any(|w| w[0].sequence >= w[1].sequence) {
        return Err(malformed("undo sequence가 단조 증가하지 않습니다"));
    }
    if redo.windows(2).any(|w| w[0].sequence <= w[1].sequence) {
        return Err(malformed("redo sequence가 단조 감소하지 않습니다"));
    }
    if let (Some(last_undo), Some(next_redo)) = (undo.last(), redo.last()) {
        if last_undo.sequence >= next_redo.sequence {
            return Err(malformed("redo 항목이 undo 항목보다 오래되었습니다"));
        }
    }
    let newest = undo
        .last()
        .into_iter()
        .chain(redo.first())
        .map(|entry| entry.sequence.get())
        .max()
        .unwrap_or(0);
    if next_sequence == 0 || next_sequence <= newest {
        return Err(malformed(
            "next sequence가 기록된 sequence보다 크지 않습니다",
        ));
    }
    Ok(())
}

/// schema 1 header 길이(magic 8 + version 4×4 + window 16 + generation 8 + manifest hash 32).
pub const JOURNAL_HEADER_V1_LEN: usize = 8 + 4 * 4 + 16 + 8 + 32;

/// 이 writer가 기록하는 header 길이(schema 1 + command codec version 4). 어떤 schema의 header든 이 길이
/// 안에 들어갑니다.
pub const JOURNAL_HEADER_LEN: usize = JOURNAL_HEADER_V1_LEN + 4;

/// schema의 header 길이.
fn header_len(schema_version: u32) -> usize {
    if schema_version >= JOURNAL_CODEC_FIELD_SCHEMA {
        JOURNAL_HEADER_LEN
    } else {
        JOURNAL_HEADER_V1_LEN
    }
}

/// 이 writer가 기록하는, 항목이 없는 journal 길이(header + next sequence 8 + undo·redo 항목 수 4×2).
pub const JOURNAL_EMPTY_LEN: usize = JOURNAL_HEADER_LEN + 8 + 4 + 4;

/// compaction 결과(XXV.1.5): 버린 항목 수.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JournalCompaction {
    /// 버린 가장 오래된 undo 항목 수.
    pub dropped_undo: usize,
    /// 버린 가장 먼 redo 항목 수.
    pub dropped_redo: usize,
    /// compaction 뒤 encoding 길이.
    pub encoded_len: u64,
}

impl JournalCompaction {
    /// 버린 항목이 있는지.
    pub fn is_compacted(&self) -> bool {
        self.dropped_undo + self.dropped_redo > 0
    }
}

/// 현재 위치에서 먼 항목부터 버리는 계획(항목 bytes는 길이 프리픽스 제외).
fn plan_compaction(
    undo: &[Vec<u8>],
    redo: &[Vec<u8>],
    max_bytes: u64,
    max_entries: usize,
) -> JournalCompaction {
    let entry_len = |bytes: &Vec<u8>| 4 + bytes.len() as u64;
    let mut total = JOURNAL_EMPTY_LEN as u64
        + undo.iter().map(entry_len).sum::<u64>()
        + redo.iter().map(entry_len).sum::<u64>();
    let (mut dropped_undo, mut dropped_redo) = (0, 0);
    loop {
        let (left_undo, left_redo) = (undo.len() - dropped_undo, redo.len() - dropped_redo);
        if (left_undo + left_redo <= max_entries && total <= max_bytes)
            || left_undo + left_redo == 0
        {
            break;
        }
        if left_undo >= left_redo {
            total -= entry_len(&undo[dropped_undo]);
            dropped_undo += 1;
        } else {
            total -= entry_len(&redo[dropped_redo]);
            dropped_redo += 1;
        }
    }
    JournalCompaction {
        dropped_undo,
        dropped_redo,
        encoded_len: total,
    }
}

/// header를 읽습니다. format version 불일치는 그 뒤 layout을 알 수 없으므로 바로 거부합니다.
fn read_header(input: &mut In<'_>) -> Result<JournalHeader, JournalError> {
    if input.take(8).map_err(|_| JournalError::BadMagic)? != JOURNAL_MAGIC {
        return Err(JournalError::BadMagic);
    }
    let format_version = input.u32()?;
    if format_version != JOURNAL_FORMAT_VERSION {
        return Err(JournalError::UnsupportedFormat {
            found: format_version,
        });
    }
    let schema_version = input.u32()?;
    let minimum_reader_version = input.u32()?;
    let minimum_writer_version = input.u32()?;
    let command_codec_version = if schema_version >= JOURNAL_CODEC_FIELD_SCHEMA {
        input.u32()?
    } else {
        1
    };
    let window = WindowSessionId::from_stable_id(input.id()?);
    let generation = StorageGeneration(input.u64()?);
    let mut manifest_hash = [0u8; 32];
    manifest_hash.copy_from_slice(input.take(32)?);
    Ok(JournalHeader {
        format_version,
        schema_version,
        minimum_reader_version,
        minimum_writer_version,
        command_codec_version,
        window,
        base: JournalBase::new(generation, manifest_hash),
    })
}

/// 항목 안 command 종류가 header가 선언한 codec 버전 안에 있는지 확인합니다(ADR-0079).
fn check_codec(entries: &[HistoryEntry], codec: u32) -> Result<(), JournalError> {
    let check = |kind: CommandKind| {
        if kind.codec_version() > codec {
            Err(JournalError::KindOutsideCodec {
                kind: kind.as_u16(),
                codec,
            })
        } else {
            Ok(())
        }
    };
    for entry in entries {
        if let Some(GroupingKey::KeyboardRepeat(signature)) = &entry.grouping {
            check(signature.kind)?;
        }
        if let HistoryPayload::Document { forward, inverse } = &entry.payload {
            for request in forward.requests.iter().chain(&inverse.requests) {
                check(request.command.kind())?;
            }
        }
    }
    Ok(())
}

impl JournalHeader {
    /// journal bytes 앞부분([`JOURNAL_HEADER_LEN`] 이상)에서 header만 읽습니다(덮어쓰기 가능 여부 판정용).
    /// 버전 판정은 하지 않으므로 [`JournalHeader::access`]로 확인합니다.
    pub fn decode_prefix(bytes: &[u8]) -> Result<JournalHeader, JournalError> {
        read_header(&mut In {
            input: bytes,
            pos: 0,
        })
    }
}

fn write_stack(out: &mut Out, entries: &[HistoryEntry]) {
    out.len(entries.len());
    for entry in entries {
        out.bytes(&encode_entry(entry));
    }
}

fn read_stack(input: &mut In<'_>) -> Result<Vec<HistoryEntry>, JournalError> {
    let count = input.len()?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(decode_entry(input.bytes()?)?);
    }
    Ok(entries)
}

impl HistoryJournal {
    /// 현재 버전 header로 journal을 만듭니다. sequence 무결성은 디코딩([`HistoryJournal::decode`])이
    /// 검사합니다.
    pub fn new(
        window: WindowSessionId,
        base: JournalBase,
        next_sequence: u64,
        undo: Vec<HistoryEntry>,
        redo: Vec<HistoryEntry>,
    ) -> HistoryJournal {
        HistoryJournal {
            header: JournalHeader::current(window, base),
            next_sequence,
            undo,
            redo,
        }
    }

    /// 창 history의 현재 undo·redo stack과 다음 sequence를 담은 journal(현재 writer 버전). 마지막 기준점
    /// 변경 이전 항목은 담지 않습니다([`History::journal_stacks`], XXV.1.4).
    pub fn capture(
        window: WindowSessionId,
        base: JournalBase,
        history: &History,
    ) -> HistoryJournal {
        let (undo, redo) = history.journal_stacks();
        HistoryJournal::new(
            window,
            base,
            history.next_sequence(),
            undo.to_vec(),
            redo.to_vec(),
        )
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Out::default();
        self.write_prefix(&mut out);
        write_stack(&mut out, &self.undo);
        write_stack(&mut out, &self.redo);
        out.0
    }

    /// header와 next sequence.
    fn write_prefix(&self, out: &mut Out) {
        out.0.extend_from_slice(&JOURNAL_MAGIC);
        out.u32(self.header.format_version);
        out.u32(self.header.schema_version);
        out.u32(self.header.minimum_reader_version);
        out.u32(self.header.minimum_writer_version);
        if self.header.schema_version >= JOURNAL_CODEC_FIELD_SCHEMA {
            out.u32(self.header.command_codec_version);
        }
        out.id(self.header.window.stable_id());
        out.u64(self.header.base.generation.0);
        out.0.extend_from_slice(&self.header.base.manifest_hash);
        out.u64(self.next_sequence);
    }

    /// 경계 안으로 compaction한 canonical bytes(XXV.1.5). 항목 수가 `max_entries`, 전체 bytes가 `max_bytes`
    /// 이하가 될 때까지 **현재 위치에서 가장 먼 항목**부터 버립니다: 남은 undo 수가 redo 수 이상이면 가장
    /// 오래된 undo(맨 아래), 아니면 가장 먼 redo(맨 아래)를 버립니다. 남은 항목은 현재 위치 앞뒤로 연속이라
    /// 그대로 undo·redo할 수 있고 sequence 무결성도 유지됩니다. 항목을 모두 버려도 넘치면(경계가
    /// [`JOURNAL_EMPTY_LEN`]보다 작음) 빈 journal입니다. 각 항목은 한 번만 인코딩합니다.
    pub fn encode_bounded(
        &self,
        max_bytes: u64,
        max_entries: usize,
    ) -> (Vec<u8>, JournalCompaction) {
        let undo: Vec<Vec<u8>> = self.undo.iter().map(encode_entry).collect();
        let redo: Vec<Vec<u8>> = self.redo.iter().map(encode_entry).collect();
        let compaction = plan_compaction(&undo, &redo, max_bytes, max_entries);
        let mut out = Out::default();
        self.write_prefix(&mut out);
        for (stack, dropped) in [
            (&undo, compaction.dropped_undo),
            (&redo, compaction.dropped_redo),
        ] {
            out.len(stack.len() - dropped);
            for bytes in &stack[dropped..] {
                out.bytes(bytes);
            }
        }
        (out.0, compaction)
    }

    /// journal bytes에서 항목 수(undo, redo)만 셉니다(XXV.1.5 retention 판정). 항목은 디코딩·할당하지 않고
    /// 길이 프리픽스만큼 건너뛰며(buffer 밖이면 seek), redo 항목은 읽지 않습니다. 이 형식이 아니거나
    /// (magic·format version) 중간에 끝나면 `Ok(None)`입니다. 버전 접근·항목 내용·sequence 무결성은 검사하지
    /// 않습니다([`HistoryJournal::decode`]의 몫).
    pub(crate) fn read_entry_counts<R: std::io::Read + std::io::Seek>(
        reader: &mut std::io::BufReader<R>,
    ) -> std::io::Result<Option<(usize, usize)>> {
        use std::io::Read;
        fn count<R: Read>(reader: &mut R) -> std::io::Result<u32> {
            let mut raw = [0u8; 4];
            reader.read_exact(&mut raw)?;
            Ok(u32::from_le_bytes(raw))
        }
        let mut prefix = [0u8; JOURNAL_HEADER_V1_LEN + 8];
        let counts = (|| -> std::io::Result<Option<(usize, usize)>> {
            reader.read_exact(&mut prefix)?;
            let Ok(header) = JournalHeader::decode_prefix(&prefix) else {
                return Ok(None);
            };
            // schema 2 이상은 header가 codec 필드만큼 깁니다(next sequence까지 건너뜀).
            let rest = header.encoded_len() + 8 - prefix.len();
            reader.seek_relative(rest as i64)?;
            let undo = count(reader)? as usize;
            for _ in 0..undo {
                let len = count(reader)?;
                reader.seek_relative(i64::from(len))?;
            }
            let redo = count(reader)? as usize;
            Ok(Some((undo, redo)))
        })();
        match counts {
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            other => other,
        }
    }

    /// [`HistoryJournal::encode_bounded`]과 같은 규칙으로 이 journal 자체를 줄입니다.
    pub fn compact(&mut self, max_bytes: u64, max_entries: usize) -> JournalCompaction {
        let undo: Vec<Vec<u8>> = self.undo.iter().map(encode_entry).collect();
        let redo: Vec<Vec<u8>> = self.redo.iter().map(encode_entry).collect();
        let compaction = plan_compaction(&undo, &redo, max_bytes, max_entries);
        self.undo.drain(..compaction.dropped_undo);
        self.redo.drain(..compaction.dropped_redo);
        compaction
    }

    /// 이 reader(schema [`JOURNAL_SCHEMA_VERSION`])로 디코딩합니다.
    pub fn decode(bytes: &[u8]) -> Result<HistoryJournal, JournalError> {
        HistoryJournal::decode_with_reader(bytes, JOURNAL_SCHEMA_VERSION)
    }

    /// reader schema `reader_version`으로 디코딩합니다. 읽을 수 없으면(형식·선언 모순·reader가 낮음)
    /// 오류이고, 읽기 전용 여부는 [`JournalHeader::access`]로 확인합니다.
    pub fn decode_with_reader(
        bytes: &[u8],
        reader_version: u32,
    ) -> Result<HistoryJournal, JournalError> {
        let mut input = In {
            input: bytes,
            pos: 0,
        };
        let header = read_header(&mut input)?;
        header.access(reader_version)?;
        let next_sequence = input.u64()?;
        let undo = read_stack(&mut input)?;
        let redo = read_stack(&mut input)?;
        input.end()?;
        check_sequences(&undo, &redo, next_sequence)?;
        check_codec(&undo, header.command_codec_version)?;
        check_codec(&redo, header.command_codec_version)?;
        Ok(HistoryJournal {
            header,
            next_sequence,
            undo,
            redo,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::history::test_fixtures::{sid, small_state, tx};
    use crate::window::WindowHistory;
    use stellar_commands::command::Command;
    use stellar_commands::id::AgentProvenance;
    use stellar_commands::kind::CommandKind;
    use stellar_core_types::clock::ManualClock;

    /// 테스트 base(generation `n`, hash 바이트 `n`).
    pub(crate) fn base(n: u8) -> JournalBase {
        JournalBase::new(StorageGeneration(u64::from(n)), [n; 32])
    }

    /// 모든 grouping·payload 종류를 담은 창 history. 마지막 두 항목은 undo해 redo stack에 있습니다.
    pub(crate) fn rich_history() -> WindowHistory {
        let mut history = WindowHistory::new(WindowSessionId::from_stable_id(sid(500)));
        let mut doc = small_state();
        let clock = ManualClock::new(0);
        let t = tx(&doc, 1_000, Command::rename_node(sid(11), "일반"));
        history.record_document(&mut doc, &t, None).unwrap();
        history
            .record_selection(
                &doc,
                &SetSelection::new(Selection::new([sid(12), sid(13)], Some(sid(12)))),
            )
            .unwrap();
        let mut agent = tx(&doc, 1_001, Command::set_locked(sid(12), true));
        agent.origin = CommandOrigin::FutureAgent(AgentProvenance::new(sid(77)));
        let key = GroupingKey::KeyboardRepeat(CommandSignature {
            kind: CommandKind::SetProperty,
            target: sid(12),
            key: Some(PropertyKey(0x0002)),
        });
        history
            .record_grouped(&mut doc, &agent, key, &clock)
            .unwrap();
        let t = tx(&doc, 1_002, Command::rename_node(sid(13), "텍스트"));
        let text = GroupingKey::TextInput {
            node: sid(13),
            kind: TextInputKind::DeleteForward,
            caret_before: 4,
            caret_after: 4,
        };
        history.record_grouped(&mut doc, &t, text, &clock).unwrap();
        history.begin_pointer_gesture(9).unwrap();
        let t = tx(&doc, 1_003, Command::set_visibility(sid(13), false));
        history.update_preview(&mut doc, &t).unwrap();
        history.end_preview().unwrap();
        history.begin_scrub(3).unwrap();
        let t = tx(&doc, 1_004, Command::rename_node(sid(11), "scrub"));
        history.update_preview(&mut doc, &t).unwrap();
        history.end_preview().unwrap();
        history.undo(&mut doc).unwrap().unwrap();
        history.undo(&mut doc).unwrap().unwrap();
        history
    }

    fn rich_journal() -> HistoryJournal {
        let history = rich_history();
        HistoryJournal::capture(history.window(), base(42), history.history())
    }

    /// header 필드 위치(magic 8 + format 4 + schema 4 + 최소 reader 4 + 최소 writer 4).
    const SCHEMA_AT: usize = 12;
    const MIN_READER_AT: usize = 16;
    const MIN_WRITER_AT: usize = 20;
    /// command codec version(schema 2 이상).
    pub(crate) const CODEC_AT: usize = 24;
    /// codec 4 + window 16 + generation 8 + hash 32 뒤 next sequence.
    const NEXT_SEQUENCE_AT: usize = 28 + 16 + 8 + 32;

    fn set_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn journal_round_trips_undo_redo_and_next_sequence() {
        // 검증 기준(XXV.1.1): round-trip. 복원에 필요한 redo stack·next sequence·base도 왕복합니다.
        let journal = rich_journal();
        assert_eq!(journal.undo.len(), 4);
        assert_eq!(journal.redo.len(), 2, "undo 2회 → redo stack");
        assert_eq!(journal.next_sequence, 7);
        assert!(
            journal.undo.iter().any(|e| e.selection.is_some()),
            "선택 복원 정보 포함"
        );
        let bytes = journal.encode();
        assert_eq!(&bytes[..8], b"STLRHJNL");
        let decoded = HistoryJournal::decode(&bytes).unwrap();
        assert_eq!(decoded, journal);
        assert_eq!(decoded.encode(), bytes, "재인코딩 동일");
        assert_eq!(decoded.header.base, base(42));
        assert_eq!(decoded.header.schema_version, JOURNAL_SCHEMA_VERSION);
        assert_eq!(
            decoded.header.minimum_writer_version,
            JOURNAL_MINIMUM_WRITER_VERSION
        );
        assert_eq!(
            decoded.header.access(JOURNAL_SCHEMA_VERSION),
            Ok(JournalAccess::Editable)
        );
        for entry in journal.undo.iter().chain(&journal.redo) {
            assert_eq!(&decode_entry(&encode_entry(entry)).unwrap(), entry);
        }
        // 추가 grouping 종류도 왕복합니다.
        let mut area = journal.undo[1].clone();
        area.grouping = Some(GroupingKey::AreaSelection);
        assert_eq!(decode_entry(&encode_entry(&area)).unwrap(), area);
        let mut no_key = journal.undo[0].clone();
        no_key.grouping = Some(GroupingKey::KeyboardRepeat(CommandSignature {
            kind: CommandKind::MoveNode,
            target: sid(11),
            key: None,
        }));
        assert_eq!(decode_entry(&encode_entry(&no_key)).unwrap(), no_key);
        // 빈 history도 왕복합니다(next sequence 1).
        let empty = HistoryJournal::capture(
            WindowSessionId::from_stable_id(sid(501)),
            base(1),
            &History::new(),
        );
        assert_eq!(empty.next_sequence, 1);
        assert_eq!(HistoryJournal::decode(&empty.encode()).unwrap(), empty);
    }

    /// 기준 commit(ce60bb92, schema 1 writer)이 [`rich_history`]를 base(42)로 기록한 bytes(ADR-0079).
    const SCHEMA1_FIXTURE: &[u8] = include_bytes!("../fixtures/journal-schema1-codec1.stlhjnl");

    #[test]
    fn schema1_journal_fixture_decodes_as_codec1() {
        // 검증(ADR-0079): 옛 codec version journal fixture가 새 reader에서 그대로 decode됩니다.
        let old = HistoryJournal::decode(SCHEMA1_FIXTURE).unwrap();
        assert_eq!(old.header.schema_version, 1);
        assert_eq!(old.header.minimum_reader_version, 1);
        assert_eq!(old.header.command_codec_version, 1);
        assert_eq!(old.header.encoded_len(), JOURNAL_HEADER_V1_LEN);
        assert_eq!(
            old.header.access(JOURNAL_SCHEMA_VERSION),
            Ok(JournalAccess::Editable)
        );
        let current = rich_journal();
        assert_eq!(old.undo, current.undo);
        assert_eq!(old.redo, current.redo);
        assert_eq!(old.next_sequence, current.next_sequence);
        assert_eq!(old.header.base, current.header.base);
        // 같은 내용을 새 writer로 쓰면 schema 2 + codec 필드 4바이트만 다릅니다.
        let bytes = current.encode();
        assert_eq!(bytes.len(), SCHEMA1_FIXTURE.len() + 4);
        assert_eq!(&bytes[..SCHEMA_AT], &SCHEMA1_FIXTURE[..SCHEMA_AT]);
        assert_eq!(
            &bytes[CODEC_AT..CODEC_AT + 4],
            &COMMAND_CODEC_VERSION.to_le_bytes()
        );
        assert_eq!(&bytes[CODEC_AT + 4..], &SCHEMA1_FIXTURE[CODEC_AT..]);
        // 옛 journal의 항목 수 세기(retention)도 header 길이를 따릅니다.
        let counts = |bytes: &[u8]| {
            HistoryJournal::read_entry_counts(&mut std::io::BufReader::new(std::io::Cursor::new(
                bytes.to_vec(),
            )))
            .unwrap()
        };
        assert_eq!(counts(SCHEMA1_FIXTURE), Some((4, 2)));
        assert_eq!(counts(&bytes), Some((4, 2)));
    }

    #[test]
    fn newer_codec_and_kinds_outside_the_declared_codec_are_rejected() {
        let bytes = rich_journal().encode();
        // 이 reader보다 새 codec.
        let mut newer = bytes.clone();
        set_u32(&mut newer, CODEC_AT, COMMAND_CODEC_VERSION + 1);
        let error = HistoryJournal::decode(&newer).unwrap_err();
        assert_eq!(
            error,
            JournalError::CommandCodecTooNew {
                found: COMMAND_CODEC_VERSION + 1,
                supported: COMMAND_CODEC_VERSION
            }
        );
        assert!(error.is_unknown_command());
        let mut zero = bytes.clone();
        set_u32(&mut zero, CODEC_AT, 0);
        assert_eq!(
            HistoryJournal::decode(&zero),
            Err(JournalError::InvalidVersions)
        );
        // codec 1을 선언했는데 codec 2 종류(View 이름 변경)가 들어 있으면 거부합니다.
        let mut history = WindowHistory::new(WindowSessionId::from_stable_id(sid(510)));
        let mut doc = small_state();
        let rename = Command::RenameView(stellar_commands::page_view::RenameView {
            view: sid(3),
            name: "홈".into(),
        });
        let t = tx(&doc, 2_000, rename);
        history.record_document(&mut doc, &t, None).unwrap();
        let journal = HistoryJournal::capture(history.window(), base(1), history.history());
        let mut declared_old = journal.encode();
        set_u32(&mut declared_old, CODEC_AT, 1);
        let error = HistoryJournal::decode(&declared_old).unwrap_err();
        assert_eq!(
            error,
            JournalError::KindOutsideCodec {
                kind: CommandKind::RenameView.as_u16(),
                codec: 1
            }
        );
        assert!(error.is_unknown_command());
        assert_eq!(
            HistoryJournal::decode(&journal.encode()).unwrap(),
            journal,
            "codec 2 선언이면 정상"
        );
    }

    #[test]
    fn version_rules_follow_v81_matrix() {
        // V.8.1과 같은 판정: format 정확 일치, reader < 최소 reader 거부, reader < 최소 writer 읽기 전용.
        let bytes = rich_journal().encode();
        let mut format = bytes.clone();
        set_u32(&mut format, 8, 2);
        assert_eq!(
            HistoryJournal::decode(&format),
            Err(JournalError::UnsupportedFormat { found: 2 })
        );
        // 미래 writer: schema 5, 최소 reader 3, 최소 writer 4.
        let mut future = bytes.clone();
        set_u32(&mut future, SCHEMA_AT, 5);
        set_u32(&mut future, MIN_READER_AT, 3);
        set_u32(&mut future, MIN_WRITER_AT, 4);
        for (reader, expected) in [
            (5, Ok(JournalAccess::Editable)),
            (4, Ok(JournalAccess::Editable)),
            (3, Ok(JournalAccess::ReadOnly)),
            (
                2,
                Err(JournalError::ReaderTooOld {
                    required: 3,
                    reader: 2,
                }),
            ),
        ] {
            let decoded = HistoryJournal::decode_with_reader(&future, reader);
            match expected {
                Ok(access) => {
                    assert_eq!(
                        decoded.unwrap().header.access(reader),
                        Ok(access),
                        "{reader}"
                    )
                }
                Err(error) => assert_eq!(decoded, Err(error), "{reader}"),
            }
        }
        assert_eq!(
            HistoryJournal::decode(&future),
            Err(JournalError::ReaderTooOld {
                required: 3,
                reader: JOURNAL_SCHEMA_VERSION
            })
        );
        // 모순 선언: 최소 reader > 최소 writer, 최소 writer > schema.
        let mut reader_above_writer = future.clone();
        set_u32(&mut reader_above_writer, MIN_READER_AT, 5);
        let mut writer_above_schema = future.clone();
        set_u32(&mut writer_above_schema, MIN_WRITER_AT, 6);
        for bad in [reader_above_writer, writer_above_schema] {
            assert_eq!(
                HistoryJournal::decode_with_reader(&bad, 9),
                Err(JournalError::InvalidVersions)
            );
        }
    }

    #[test]
    fn malformed_input_and_sequence_integrity_are_rejected() {
        let journal = rich_journal();
        let bytes = journal.encode();
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert_eq!(HistoryJournal::decode(&bad), Err(JournalError::BadMagic));
        assert_eq!(
            HistoryJournal::decode(&bytes[..4]),
            Err(JournalError::BadMagic)
        );
        for cut in [20, 40, NEXT_SEQUENCE_AT + 4, bytes.len() - 1] {
            assert!(HistoryJournal::decode(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(HistoryJournal::decode(&trailing).is_err());

        let rejected = |journal: HistoryJournal| HistoryJournal::decode(&journal.encode());
        let reason = |reason| Err(JournalError::Malformed { reason });
        // undo 역순.
        let mut undo_swapped = journal.clone();
        undo_swapped.undo.swap(0, 1);
        assert_eq!(
            rejected(undo_swapped),
            reason("undo sequence가 단조 증가하지 않습니다")
        );
        // redo 순서 뒤집힘(다음 redo가 가장 작아야 함).
        let mut redo_swapped = journal.clone();
        redo_swapped.redo.swap(0, 1);
        assert_eq!(
            rejected(redo_swapped),
            reason("redo sequence가 단조 감소하지 않습니다")
        );
        // redo 항목이 undo 맨 위보다 오래됨.
        let mut crossed = journal.clone();
        let oldest = crossed.undo.remove(0);
        crossed.redo.push(oldest);
        assert_eq!(
            rejected(crossed),
            reason("redo 항목이 undo 항목보다 오래되었습니다")
        );
        // next sequence가 기록된 sequence 이하이거나 0.
        for next in [0, 6] {
            let mut stale = journal.clone();
            stale.next_sequence = next;
            assert_eq!(
                rejected(stale),
                reason("next sequence가 기록된 sequence보다 크지 않습니다"),
                "next {next}"
            );
        }
    }
}
