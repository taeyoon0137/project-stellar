//! 3-way Semantic Merge 입력·결과·conflict 모델 (XIV.10.1).
//!
//! 입력은 공통 조상(base)과 양쪽(ours·theirs)의 [`SemanticSnapshot`]입니다(아키텍처 §27.1).
//! 결과는 자동 병합된 candidate snapshot, base → candidate 변경, 그리고 자동 병합하지 못한
//! [`SemanticConflict`] 목록입니다. conflict 대상은 candidate에서 **base 상태를 유지**하고
//! base·ours·theirs 세 상태를 모두 기록합니다 — 해결(XIV.12)은 이 기록만으로 선택지를 만듭니다.
//! 예외로 정의 목록(페이지·View·Slot 목록)은 candidate에서 항상 실제로 남은 소속 객체와 같은 원소를
//! 가집니다. 목록 재정렬 conflict면 남은 객체의 base 순서를 유지하고, owner가 객체 conflict여도 목록
//! 원소는 남은 소속 객체에 맞춥니다(merge 엔진 "정의 목록").
//!
//! canonical bytes(LE, semantic-diff codec 재사용):
//!
//! | 항목 | 형식 |
//! |---|---|
//! | MergeInput | `SMIN` + version `u16` + snapshot × 3(base, ours, theirs) |
//! | MergeResult | `SMRS` + version `u16` + merged snapshot + 변경 목록 + conflict 수 `u32` + conflict* |
//! | conflict | 종류 `u8` + 출처 + 객체 id 16 + 대상(tag `u8` + 경로) + 상태 × 3(base, ours, theirs) |
//! | 상태 | tag `u8`(0 없음·1 객체·2 위치·3 속성 값) + 내용 |

use std::fmt;

use stellar_semantic_diff::change::{Position, Provenance};
use stellar_semantic_diff::codec::{
    put_changes, put_object, put_opt_value, put_path, put_position, put_provenance, put_snapshot,
    read_changes, read_object, read_opt_value, read_path, read_position, read_provenance,
    read_snapshot, DecodeError, Reader,
};
use stellar_semantic_diff::core_types::ids::StableId;
use stellar_semantic_diff::core_types::property::PropertyPath;
use stellar_semantic_diff::diff::SemanticDiff;
use stellar_semantic_diff::snapshot::{SemanticObject, SemanticSnapshot};
use stellar_semantic_diff::value::SemanticValue;

/// 3-way merge 입력.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeInput {
    /// 공통 조상.
    pub base: SemanticSnapshot,
    /// 우리 쪽.
    pub ours: SemanticSnapshot,
    /// 상대 쪽.
    pub theirs: SemanticSnapshot,
}

/// conflict 종류(안정 numeric code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ConflictKind {
    /// 같은 속성을 양쪽이 서로 다른 값으로 바꿈(XIV.10.4).
    PropertyValue = 1,
    /// 한쪽 삭제 대 다른 쪽 수정(속성·subtree 추가 포함, XIV.10.5).
    DeleteVsModify = 2,
    /// 한쪽 삭제 대 다른 쪽 이동·재정렬(XIV.10.5).
    DeleteVsMove = 3,
    /// 한쪽 값 타입 변경 대 다른 쪽 같은 타입 값 변경(XIV.10.5).
    TypeChangeVsValueChange = 4,
    /// 같은 객체를 양쪽이 다른 위치로 이동·재정렬(XIV.10.6, 대상 = 위치), 또는 같은 정의 목록
    /// (페이지·View·Slot 목록)을 양쪽이 서로 다른 순서로 재정렬(대상 = 목록 속성, 상태 = 목록 값).
    ConflictingReorder = 5,
    /// 같은 StableId를 양쪽이 서로 다르게 정의(추가·교체, 아키텍처 §27.1).
    InconsistentDefinition = 6,
}

impl ConflictKind {
    /// 정의된 모든 종류(code 오름차순).
    pub const ALL: [ConflictKind; 6] = [
        ConflictKind::PropertyValue,
        ConflictKind::DeleteVsModify,
        ConflictKind::DeleteVsMove,
        ConflictKind::TypeChangeVsValueChange,
        ConflictKind::ConflictingReorder,
        ConflictKind::InconsistentDefinition,
    ];

    /// 안정 numeric code.
    pub fn code(self) -> u8 {
        self as u8
    }

    /// code에서 복원합니다.
    pub fn from_code(code: u8) -> Option<ConflictKind> {
        ConflictKind::ALL.into_iter().find(|k| k.code() == code)
    }

    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            ConflictKind::PropertyValue => "property-value",
            ConflictKind::DeleteVsModify => "delete-vs-modify",
            ConflictKind::DeleteVsMove => "delete-vs-move",
            ConflictKind::TypeChangeVsValueChange => "type-change-vs-value-change",
            ConflictKind::ConflictingReorder => "conflicting-reorder",
            ConflictKind::InconsistentDefinition => "inconsistent-definition",
        }
    }
}

/// conflict가 걸린 대상.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConflictTarget {
    /// 객체 전체(존재·정의).
    Object,
    /// 객체 위치(parent·OrderKey).
    Position,
    /// 속성 하나.
    Property(PropertyPath),
}

/// 한 쪽에서 본 대상 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideState {
    /// 객체가 없음(삭제됐거나 추가되지 않음).
    Missing,
    /// 객체 전체.
    Object(Box<SemanticObject>),
    /// 객체 위치.
    Position(Position),
    /// 속성 값(`None` = 속성 없음).
    Value(Option<SemanticValue>),
}

/// 자동 병합하지 못한 의미 충돌 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticConflict {
    /// 종류.
    pub kind: ConflictKind,
    /// 출처(대상 객체 기준).
    pub provenance: Provenance,
    /// 대상 객체.
    pub object: StableId,
    /// 대상.
    pub target: ConflictTarget,
    /// base 상태.
    pub base: SideState,
    /// ours 상태.
    pub ours: SideState,
    /// theirs 상태.
    pub theirs: SideState,
}

/// 속성 conflict의 세 값(XIV.10.4). `None`은 그쪽에 속성이 없음을 뜻합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyValues<'a> {
    /// 속성 경로.
    pub path: &'a PropertyPath,
    /// base 값.
    pub base: Option<&'a SemanticValue>,
    /// ours 값.
    pub ours: Option<&'a SemanticValue>,
    /// theirs 값.
    pub theirs: Option<&'a SemanticValue>,
}

impl SemanticConflict {
    /// 소유 문서(프로젝트 범위면 `None`).
    pub fn document(&self) -> Option<StableId> {
        self.provenance.document()
    }

    /// 속성 대상 conflict의 base·ours·theirs 값(XIV.10.4).
    ///
    /// 대상이 속성이 아니거나 어느 쪽 상태가 속성 값이 아니면(객체 자체가 없는 등) `None`.
    pub fn property_values(&self) -> Option<PropertyValues<'_>> {
        let ConflictTarget::Property(path) = &self.target else {
            return None;
        };
        fn value(side: &SideState) -> Option<Option<&SemanticValue>> {
            match side {
                SideState::Value(value) => Some(value.as_ref()),
                _ => None,
            }
        }
        Some(PropertyValues {
            path,
            base: value(&self.base)?,
            ours: value(&self.ours)?,
            theirs: value(&self.theirs)?,
        })
    }
}

/// 3-way merge 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeResult {
    /// 자동 병합한 candidate snapshot. conflict 대상은 base 상태를 유지합니다(정의 목록 원소는 예외로
    /// candidate 소속 객체와 맞춤, 모듈 문서).
    pub merged: SemanticSnapshot,
    /// base → merged 변경(canonical 순서).
    pub changes: SemanticDiff,
    /// conflict 목록.
    pub conflicts: Vec<SemanticConflict>,
}

impl MergeResult {
    /// conflict가 없는지.
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// merge 실패(입력이 snapshot 불변식을 어겨 병합 변경을 적용할 수 없는 경우 등).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// 병합 변경 적용 실패.
    Apply(stellar_semantic_diff::apply::ApplyError),
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::Apply(error) => write!(f, "병합 변경을 적용할 수 없습니다: {error}"),
        }
    }
}

impl std::error::Error for MergeError {}

const INPUT_MAGIC: &[u8; 4] = b"SMIN";
const RESULT_MAGIC: &[u8; 4] = b"SMRS";
const FORMAT_VERSION: u16 = 1;

fn put_header(out: &mut Vec<u8>, magic: &[u8; 4]) {
    out.extend_from_slice(magic);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
}

fn read_header(reader: &mut Reader<'_>, magic: &[u8; 4]) -> Result<(), DecodeError> {
    if reader.take(4)? != magic {
        return Err(DecodeError::BadMagic);
    }
    match reader.u16()? {
        FORMAT_VERSION => Ok(()),
        version => Err(DecodeError::UnsupportedVersion { version }),
    }
}

fn put_side(out: &mut Vec<u8>, side: &SideState) {
    match side {
        SideState::Missing => out.push(0),
        SideState::Object(object) => {
            out.push(1);
            put_object(out, object);
        }
        SideState::Position(position) => {
            out.push(2);
            put_position(out, position);
        }
        SideState::Value(value) => {
            out.push(3);
            put_opt_value(out, value.as_ref());
        }
    }
}

fn read_side(reader: &mut Reader<'_>) -> Result<SideState, DecodeError> {
    match reader.u8()? {
        0 => Ok(SideState::Missing),
        1 => Ok(SideState::Object(Box::new(read_object(reader)?))),
        2 => Ok(SideState::Position(read_position(reader)?)),
        3 => Ok(SideState::Value(read_opt_value(reader)?)),
        other => Err(DecodeError::UnknownTag {
            field: "conflict.side",
            value: u32::from(other),
        }),
    }
}

/// conflict 하나를 기록합니다.
pub fn put_conflict(out: &mut Vec<u8>, conflict: &SemanticConflict) {
    out.push(conflict.kind.code());
    put_provenance(out, &conflict.provenance);
    out.extend_from_slice(&conflict.object.to_bytes());
    match &conflict.target {
        ConflictTarget::Object => out.push(1),
        ConflictTarget::Position => out.push(2),
        ConflictTarget::Property(path) => {
            out.push(3);
            put_path(out, path);
        }
    }
    for side in [&conflict.base, &conflict.ours, &conflict.theirs] {
        put_side(out, side);
    }
}

/// conflict 하나를 읽습니다.
pub fn read_conflict(reader: &mut Reader<'_>) -> Result<SemanticConflict, DecodeError> {
    let code = reader.u8()?;
    let kind = ConflictKind::from_code(code).ok_or(DecodeError::UnknownTag {
        field: "conflict.kind",
        value: u32::from(code),
    })?;
    let provenance = read_provenance(reader)?;
    let object = reader.id()?;
    let target = match reader.u8()? {
        1 => ConflictTarget::Object,
        2 => ConflictTarget::Position,
        3 => {
            let path = read_path(reader)?;
            if path.root() != object {
                return Err(DecodeError::InvalidValue {
                    field: "conflict.path_root",
                });
            }
            ConflictTarget::Property(path)
        }
        other => {
            return Err(DecodeError::UnknownTag {
                field: "conflict.target",
                value: u32::from(other),
            })
        }
    };
    Ok(SemanticConflict {
        kind,
        provenance,
        object,
        target,
        base: read_side(reader)?,
        ours: read_side(reader)?,
        theirs: read_side(reader)?,
    })
}

impl MergeInput {
    /// canonical bytes.
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_header(&mut out, INPUT_MAGIC);
        for snapshot in [&self.base, &self.ours, &self.theirs] {
            put_snapshot(&mut out, snapshot);
        }
        out
    }

    /// canonical bytes를 읽습니다(trailing 거부).
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<MergeInput, DecodeError> {
        let mut reader = Reader::new(bytes);
        read_header(&mut reader, INPUT_MAGIC)?;
        let input = MergeInput {
            base: read_snapshot(&mut reader)?,
            ours: read_snapshot(&mut reader)?,
            theirs: read_snapshot(&mut reader)?,
        };
        reader.finish()?;
        Ok(input)
    }
}

impl MergeResult {
    /// canonical bytes.
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_header(&mut out, RESULT_MAGIC);
        put_snapshot(&mut out, &self.merged);
        put_changes(&mut out, self.changes.changes());
        out.extend_from_slice(&(self.conflicts.len() as u32).to_le_bytes());
        for conflict in &self.conflicts {
            put_conflict(&mut out, conflict);
        }
        out
    }

    /// canonical bytes를 읽습니다(trailing 거부). 변경 목록은 canonical 순서여야 합니다.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<MergeResult, DecodeError> {
        let mut reader = Reader::new(bytes);
        read_header(&mut reader, RESULT_MAGIC)?;
        let merged = read_snapshot(&mut reader)?;
        let changes = read_changes(&mut reader)?;
        let sorted = SemanticDiff::new(changes.clone());
        if sorted.changes() != changes.as_slice() {
            return Err(DecodeError::NonCanonicalOrder {
                field: "merge.changes",
            });
        }
        // conflict 최소 크기: 종류 1 + 출처 1 + id 16 + 대상 1 + 상태 3.
        let count = reader.count(22)?;
        let mut conflicts = Vec::new();
        for _ in 0..count {
            conflicts.push(read_conflict(&mut reader)?);
        }
        reader.finish()?;
        Ok(MergeResult {
            merged,
            changes: sorted,
            conflicts,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use stellar_semantic_diff::core_types::order_key::OrderKey;
    use stellar_semantic_diff::core_types::property::PropertyKey;
    use stellar_semantic_diff::diff::diff;
    use stellar_semantic_diff::keys;
    use stellar_semantic_diff::snapshot::{NodeKind, ObjectKind, Origin};

    pub(crate) fn sid(n: u64) -> StableId {
        StableId::new_v7(1_700_000_000_000 + n, 0x0abc, 0x0123_4567_89ab_cdef ^ n)
    }

    pub(crate) fn key(byte: u8) -> OrderKey {
        OrderKey::from_bytes(vec![byte]).unwrap()
    }

    /// 문서 1의 node 객체(Frame 또는 Fragment).
    pub(crate) fn node(
        id: u64,
        parent: Option<u64>,
        order: u8,
        props: &[(PropertyKey, SemanticValue)],
    ) -> SemanticObject {
        let kind = if parent.is_none() {
            NodeKind::Frame
        } else {
            NodeKind::Fragment
        };
        let mut object =
            SemanticObject::new(sid(id), ObjectKind::Node(kind), Origin::Document(sid(1)))
                .with_position(parent.map(sid), Some(key(order)));
        for (k, value) in props {
            object
                .set_property(object.path(&[*k]), value.clone())
                .unwrap();
        }
        object
    }

    pub(crate) fn snapshot(objects: Vec<SemanticObject>) -> SemanticSnapshot {
        let mut snapshot = SemanticSnapshot::new();
        for object in objects {
            snapshot.insert(object).unwrap();
        }
        snapshot
    }

    pub(crate) fn text(value: &str) -> SemanticValue {
        SemanticValue::Text(value.into())
    }

    #[test]
    fn conflict_kind_codes_golden() {
        let golden = [
            (1, "property-value"),
            (2, "delete-vs-modify"),
            (3, "delete-vs-move"),
            (4, "type-change-vs-value-change"),
            (5, "conflicting-reorder"),
            (6, "inconsistent-definition"),
        ];
        assert_eq!(ConflictKind::ALL.len(), golden.len());
        for (kind, (code, name)) in ConflictKind::ALL.into_iter().zip(golden) {
            assert_eq!((kind.code(), kind.name()), (code, name));
            assert_eq!(ConflictKind::from_code(code), Some(kind));
        }
        assert_eq!(ConflictKind::from_code(0), None);
    }

    fn sample_result() -> MergeResult {
        let base = snapshot(vec![
            node(10, None, 0x80, &[(keys::DISPLAY_NAME, text("root"))]),
            node(11, Some(10), 0x40, &[(keys::DISPLAY_NAME, text("a"))]),
        ]);
        let merged = snapshot(vec![
            node(10, None, 0x80, &[(keys::DISPLAY_NAME, text("root"))]),
            node(11, Some(10), 0x40, &[(keys::DISPLAY_NAME, text("a"))]),
            node(12, Some(10), 0x80, &[]),
        ]);
        let changes = diff(&base, &merged);
        let document = Provenance::Document { document: sid(1) };
        let conflicts = vec![
            SemanticConflict {
                kind: ConflictKind::PropertyValue,
                provenance: document,
                object: sid(11),
                target: ConflictTarget::Property(PropertyPath::with_keys(
                    sid(11),
                    vec![keys::DISPLAY_NAME],
                )),
                base: SideState::Value(Some(text("a"))),
                ours: SideState::Value(Some(text("b"))),
                theirs: SideState::Value(None),
            },
            SemanticConflict {
                kind: ConflictKind::DeleteVsMove,
                provenance: document,
                object: sid(10),
                target: ConflictTarget::Object,
                base: SideState::Object(Box::new(base.get(sid(10)).unwrap().clone())),
                ours: SideState::Missing,
                theirs: SideState::Position(Position {
                    parent: None,
                    order_key: Some(key(0x20)),
                }),
            },
        ];
        MergeResult {
            merged,
            changes,
            conflicts,
        }
    }

    #[test]
    fn property_values_accessor() {
        let result = sample_result();
        let values = result.conflicts[0].property_values().unwrap();
        assert_eq!(values.base, Some(&text("a")));
        assert_eq!(values.ours, Some(&text("b")));
        assert_eq!(values.theirs, None);
        assert_eq!(values.path.root(), sid(11));
        assert!(result.conflicts[1].property_values().is_none(), "객체 대상");
    }

    #[test]
    fn merge_result_round_trips() {
        let result = sample_result();
        assert!(!result.is_clean());
        assert_eq!(result.conflicts[0].document(), Some(sid(1)));
        let bytes = result.to_canonical_bytes();
        assert_eq!(MergeResult::from_canonical_bytes(&bytes).unwrap(), result);
        let clean = MergeResult::default();
        assert!(clean.is_clean());
        assert_eq!(
            MergeResult::from_canonical_bytes(&clean.to_canonical_bytes()).unwrap(),
            clean
        );
        for cut in 0..bytes.len() {
            assert!(
                MergeResult::from_canonical_bytes(&bytes[..cut]).is_err(),
                "cut {cut}"
            );
        }
    }

    #[test]
    fn merge_input_round_trips() {
        let result = sample_result();
        let input = MergeInput {
            base: snapshot(vec![node(10, None, 0x80, &[])]),
            ours: result.merged.clone(),
            theirs: SemanticSnapshot::new(),
        };
        let bytes = input.to_canonical_bytes();
        assert_eq!(MergeInput::from_canonical_bytes(&bytes).unwrap(), input);
        let mut trailing = bytes.clone();
        trailing.push(7);
        assert_eq!(
            MergeInput::from_canonical_bytes(&trailing),
            Err(DecodeError::TrailingBytes { remaining: 1 })
        );
        assert_eq!(
            MergeInput::from_canonical_bytes(&bytes[4..]),
            Err(DecodeError::BadMagic)
        );
    }

    #[test]
    fn malformed_conflict_is_rejected() {
        let result = sample_result();
        let mut bytes = Vec::new();
        put_conflict(&mut bytes, &result.conflicts[0]);
        let mut unknown = bytes.clone();
        unknown[0] = 99;
        assert!(matches!(
            read_conflict(&mut Reader::new(&unknown)),
            Err(DecodeError::UnknownTag {
                field: "conflict.kind",
                ..
            })
        ));
        // 경로 root가 conflict 객체와 다르면 거부.
        let mut foreign = result.conflicts[0].clone();
        foreign.object = sid(99);
        let mut bytes = Vec::new();
        put_conflict(&mut bytes, &foreign);
        assert_eq!(
            read_conflict(&mut Reader::new(&bytes)),
            Err(DecodeError::InvalidValue {
                field: "conflict.path_root"
            })
        );
    }
}
