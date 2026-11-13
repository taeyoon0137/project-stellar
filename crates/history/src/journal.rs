//! 영속 history journal schema (XXV.1.1, 로드맵).
//!
//! 세션 메모리 전용 [`HistoryEntry`](VII.4.1)를 세션 간 복원(XXV.1.2~)에 쓸 수 있도록 **안정된 binary
//! schema와 version**을 정의합니다. 이 형식은 project serializer(문서 chunk·manifest)와 별개이며, 기록
//! 위치(ignored path·OS app data)와 복원 정책은 후속 작업(XXV.1.2~XXV.1.5)의 몫입니다.
//!
//! canonical 규칙(V.1): 고정 폭 little-endian, `u32` 길이 프리픽스, trailing bytes 거부.
//!
//! | 필드 | 형식 |
//! |---|---|
//! | magic | `b"STLRHJNL"` 8바이트 |
//! | format version | `u32` — reader가 지원하는 값과 **정확히** 같아야 함 |
//! | schema version | `u32` — 기록 시점 schema |
//! | minimum reader version | `u32` — 이 journal을 읽는 데 필요한 최소 schema(V.8.1과 같은 규칙) |
//! | window | 16바이트 [`WindowSessionId`] |
//! | base generation | `u64` [`StorageGeneration`] — journal이 기준으로 삼은 저장 세대 |
//! | 항목 수 | `u32` |
//! | 항목 | `u32` 길이 + 항목 bytes |
//!
//! 항목: sequence `u64` + origin([`CommandOrigin`] canonical) + grouping key(tag + 필드) + payload(tag 1
//! 문서: 정방향·역방향 [`CommandTransaction`] bytes / tag 2 선택: 정방향·역방향 [`SetSelection`] bytes) +
//! 선택 복원 정보(flag + 편집 전·후 선택). 모든 tag는 저장 계약이라 바꾸거나 재사용하지 않습니다.

use std::fmt;

use stellar_commands::codec::CommandCodecError;
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

/// journal magic.
pub const JOURNAL_MAGIC: [u8; 8] = *b"STLRHJNL";
/// 이 reader가 지원하는 journal 컨테이너 형식 버전.
pub const JOURNAL_FORMAT_VERSION: u32 = 1;
/// 이 writer가 기록하는 journal schema 버전.
pub const JOURNAL_SCHEMA_VERSION: u32 = 1;
/// 이 writer가 기록하는 journal을 읽는 데 필요한 최소 schema 버전.
pub const JOURNAL_MINIMUM_READER_VERSION: u32 = 1;

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
    /// 버전 선언 자체가 모순입니다(최소 reader > schema).
    InvalidVersions,
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// 안에 담긴 command·선택 bytes 오류.
    Command(CommandCodecError),
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
        }
    }
}

impl std::error::Error for JournalError {}

impl From<CommandCodecError> for JournalError {
    fn from(error: CommandCodecError) -> JournalError {
        JournalError::Command(error)
    }
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
    /// 기록한 창.
    pub window: WindowSessionId,
    /// journal이 기준으로 삼은 저장 세대.
    pub base_generation: StorageGeneration,
}

impl JournalHeader {
    /// 현재 writer 버전으로 header를 만듭니다.
    pub fn current(window: WindowSessionId, base_generation: StorageGeneration) -> JournalHeader {
        JournalHeader {
            format_version: JOURNAL_FORMAT_VERSION,
            schema_version: JOURNAL_SCHEMA_VERSION,
            minimum_reader_version: JOURNAL_MINIMUM_READER_VERSION,
            window,
            base_generation,
        }
    }
}

/// 창 하나의 history journal.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryJournal {
    /// header.
    pub header: JournalHeader,
    /// 항목(오래된 것부터).
    pub entries: Vec<HistoryEntry>,
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

impl HistoryJournal {
    /// 현재 버전 header와 항목으로 journal을 만듭니다.
    pub fn new(
        window: WindowSessionId,
        base_generation: StorageGeneration,
        entries: Vec<HistoryEntry>,
    ) -> HistoryJournal {
        HistoryJournal {
            header: JournalHeader::current(window, base_generation),
            entries,
        }
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Out::default();
        out.0.extend_from_slice(&JOURNAL_MAGIC);
        out.u32(self.header.format_version);
        out.u32(self.header.schema_version);
        out.u32(self.header.minimum_reader_version);
        out.id(self.header.window.stable_id());
        out.u64(self.header.base_generation.0);
        out.len(self.entries.len());
        for entry in &self.entries {
            out.bytes(&encode_entry(entry));
        }
        out.0
    }

    /// 이 reader(schema [`JOURNAL_SCHEMA_VERSION`])로 디코딩합니다.
    pub fn decode(bytes: &[u8]) -> Result<HistoryJournal, JournalError> {
        HistoryJournal::decode_with_reader(bytes, JOURNAL_SCHEMA_VERSION)
    }

    /// reader schema `reader_version`으로 디코딩합니다(버전 판정 검증용).
    pub fn decode_with_reader(
        bytes: &[u8],
        reader_version: u32,
    ) -> Result<HistoryJournal, JournalError> {
        let mut input = In {
            input: bytes,
            pos: 0,
        };
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
        if minimum_reader_version > schema_version {
            return Err(JournalError::InvalidVersions);
        }
        if reader_version < minimum_reader_version {
            return Err(JournalError::ReaderTooOld {
                required: minimum_reader_version,
                reader: reader_version,
            });
        }
        let window = WindowSessionId::from_stable_id(input.id()?);
        let base_generation = StorageGeneration(input.u64()?);
        let count = input.len()?;
        let mut entries = Vec::with_capacity(count);
        let mut previous: Option<HistorySequence> = None;
        for _ in 0..count {
            let entry = decode_entry(input.bytes()?)?;
            if previous.is_some_and(|p| p >= entry.sequence) {
                return Err(JournalError::Malformed {
                    reason: "sequence가 단조 증가하지 않습니다",
                });
            }
            previous = Some(entry.sequence);
            entries.push(entry);
        }
        input.end()?;
        Ok(HistoryJournal {
            header: JournalHeader {
                format_version,
                schema_version,
                minimum_reader_version,
                window,
                base_generation,
            },
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::test_fixtures::{sid, small_state, tx};
    use crate::window::WindowHistory;
    use stellar_commands::command::Command;
    use stellar_commands::id::AgentProvenance;
    use stellar_commands::kind::CommandKind;
    use stellar_core_types::clock::ManualClock;

    /// 모든 grouping·payload 종류를 담은 창 history.
    fn rich_history() -> WindowHistory {
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
        history
    }

    #[test]
    fn journal_round_trips_every_entry_kind() {
        // 검증 기준(XXV.1.1): round-trip.
        let history = rich_history();
        let entries = history.history().entries().to_vec();
        assert_eq!(entries.len(), 6);
        assert!(
            entries.iter().any(|e| e.selection.is_some()),
            "선택 복원 정보 포함"
        );
        let journal = HistoryJournal::new(history.window(), StorageGeneration(42), entries.clone());
        let bytes = journal.encode();
        assert_eq!(&bytes[..8], b"STLRHJNL");
        let decoded = HistoryJournal::decode(&bytes).unwrap();
        assert_eq!(decoded, journal);
        assert_eq!(decoded.encode(), bytes, "재인코딩 동일");
        assert_eq!(decoded.header.base_generation, StorageGeneration(42));
        assert_eq!(decoded.header.schema_version, JOURNAL_SCHEMA_VERSION);
        for entry in &entries {
            assert_eq!(&decode_entry(&encode_entry(entry)).unwrap(), entry);
        }
        // 추가 grouping 종류도 왕복합니다.
        let mut area = entries[1].clone();
        area.grouping = Some(GroupingKey::AreaSelection);
        assert_eq!(decode_entry(&encode_entry(&area)).unwrap(), area);
        let mut no_key = entries[0].clone();
        no_key.grouping = Some(GroupingKey::KeyboardRepeat(CommandSignature {
            kind: CommandKind::MoveNode,
            target: sid(11),
            key: None,
        }));
        assert_eq!(decode_entry(&encode_entry(&no_key)).unwrap(), no_key);
    }

    #[test]
    fn version_rules_and_malformed_input_are_rejected() {
        let history = rich_history();
        let journal = HistoryJournal::new(
            history.window(),
            StorageGeneration(1),
            history.history().entries().to_vec(),
        );
        let bytes = journal.encode();
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert_eq!(HistoryJournal::decode(&bad), Err(JournalError::BadMagic));
        assert_eq!(
            HistoryJournal::decode(&bytes[..4]),
            Err(JournalError::BadMagic)
        );
        let mut format = bytes.clone();
        format[8..12].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            HistoryJournal::decode(&format),
            Err(JournalError::UnsupportedFormat { found: 2 })
        );
        // 미래 writer(schema 3, 최소 reader 2) → 현재 reader(1) 거부, reader 2는 허용.
        let mut future = bytes.clone();
        future[12..16].copy_from_slice(&3u32.to_le_bytes());
        future[16..20].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            HistoryJournal::decode(&future),
            Err(JournalError::ReaderTooOld {
                required: 2,
                reader: 1
            })
        );
        assert!(HistoryJournal::decode_with_reader(&future, 2).is_ok());
        let mut contradictory = bytes.clone();
        contradictory[16..20].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(
            HistoryJournal::decode_with_reader(&contradictory, 9),
            Err(JournalError::InvalidVersions)
        );
        for cut in [20, 40, bytes.len() - 1] {
            assert!(HistoryJournal::decode(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(HistoryJournal::decode(&trailing).is_err());
        // sequence 역순 항목 거부.
        let mut entries = history.history().entries().to_vec();
        entries.swap(0, 1);
        let shuffled =
            HistoryJournal::new(history.window(), StorageGeneration(1), entries).encode();
        assert_eq!(
            HistoryJournal::decode(&shuffled),
            Err(JournalError::Malformed {
                reason: "sequence가 단조 증가하지 않습니다"
            })
        );
    }
}
