//! 선택 변경의 Staging 계획 (XIV.5.1~).
//!
//! 사용자는 semantic status(XIV.4)의 변경 중 일부만 골라 stage하거나(unstaged → Index) unstage할 수
//! 있습니다(staged → Index에서 제거, XIV.7). 이 모듈은 그 선택을 표현하는 값과 결과 dependency
//! closure를 정의합니다.
//!
//! **변경 ID**: [`ChangeId`]는 한 diff 안에서 변경 하나를 가리키는 `(객체 StableId, 대상 단위)`입니다.
//! 대상 단위는 객체 삭제·객체 추가·위치·속성 경로 중 하나이며, 올바른 diff에서 같은 ID의 변경은 하나뿐입니다
//! (적용 단계 XIV.3.9가 같은 대상을 두 번 다루는 목록을 거부하는 것과 같은 단위). 교체(같은 StableId의 삭제 +
//! 추가)는 삭제·추가가 서로 다른 ID입니다. ID는 변경의 이전·이후 값을 담지 않으므로, 요청을 받은 쪽은 현재
//! status를 다시 계산해 ID를 변경으로 되찾고 없으면 오래된(stale) 요청으로 거부합니다
//! ([`resolve_changes`]).
//!
//! **요청과 결과**: [`StageSelectionRequest`]는 고른 ID(`selected`)와 이전 계획에서 받은 필수 함께-변경을
//! 호출자가 승인한 ID(`approved`)를 담습니다. 계획 결과 [`StageSelection`]은 선택과 그 dependency closure,
//! closure에서 선택을 뺀 `required_additions`입니다. 필수 변경은 자동으로 stage하지 않고 호출자 승인을
//! 거칩니다(XIV.5.3).
//!
//! canonical bytes(LE, 저장 포맷 아님):
//!
//! | 항목 | 형식 |
//! |---|---|
//! | ChangeId | 객체 id 16 + 단위 tag `u8`(1 삭제·2 추가·3 위치·4 속성) + (속성이면 경로) |
//! | ID 집합 | 개수 `u32` + ChangeId* (오름차순·중복 없음) |
//! | 요청 | `SSRQ` + version `u16` + 작업 `u8` + selected 집합 + approved 집합 |
//! | 결과 | `SSEL` + version `u16` + 작업 `u8` + selected 집합 + closure 집합(selected ⊆ closure) |

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;

use crate::change::SemanticChange;
use crate::codec::{put_path, read_path, DecodeError, Reader};

/// 변경이 다루는 대상 단위.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeUnit {
    /// 객체 삭제(RemoveObject·RemoveToken).
    Remove,
    /// 객체 추가(AddObject·AddToken).
    Add,
    /// 위치(MoveObject·ReorderObject).
    Position,
    /// 속성 하나(SetProperty·ChangeToken·ComponentInterfaceChange·AssetRefChange).
    Property(PropertyPath),
}

impl ChangeUnit {
    fn tag(&self) -> u8 {
        match self {
            ChangeUnit::Remove => 1,
            ChangeUnit::Add => 2,
            ChangeUnit::Position => 3,
            ChangeUnit::Property(_) => 4,
        }
    }
}

/// diff 안 변경 하나의 ID.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChangeId {
    /// 대상 객체.
    pub object: StableId,
    /// 대상 단위.
    pub unit: ChangeUnit,
}

impl ChangeId {
    /// 객체 삭제 ID.
    pub fn remove(object: StableId) -> ChangeId {
        ChangeId {
            object,
            unit: ChangeUnit::Remove,
        }
    }

    /// 객체 추가 ID.
    pub fn add(object: StableId) -> ChangeId {
        ChangeId {
            object,
            unit: ChangeUnit::Add,
        }
    }

    /// 위치 변경 ID.
    pub fn position(object: StableId) -> ChangeId {
        ChangeId {
            object,
            unit: ChangeUnit::Position,
        }
    }

    /// 속성 변경 ID(객체는 경로 root).
    pub fn property(path: PropertyPath) -> ChangeId {
        ChangeId {
            object: path.root(),
            unit: ChangeUnit::Property(path),
        }
    }

    /// canonical bytes를 기록합니다.
    pub fn write_canonical(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.object.to_bytes());
        out.push(self.unit.tag());
        if let ChangeUnit::Property(path) = &self.unit {
            put_path(out, path);
        }
    }

    /// canonical bytes를 읽습니다. 속성 경로 root는 객체여야 합니다.
    pub fn read_canonical(reader: &mut Reader<'_>) -> Result<ChangeId, DecodeError> {
        let object = reader.id()?;
        let unit = match reader.u8()? {
            1 => ChangeUnit::Remove,
            2 => ChangeUnit::Add,
            3 => ChangeUnit::Position,
            4 => {
                let path = read_path(reader)?;
                if path.root() != object {
                    return Err(DecodeError::InvalidValue {
                        field: "change_id.path_root",
                    });
                }
                ChangeUnit::Property(path)
            }
            other => {
                return Err(DecodeError::UnknownTag {
                    field: "change_id.unit",
                    value: u32::from(other),
                })
            }
        };
        Ok(ChangeId { object, unit })
    }
}

impl SemanticChange {
    /// 이 변경의 ID.
    pub fn id(&self) -> ChangeId {
        match self {
            SemanticChange::RemoveObject(c) | SemanticChange::RemoveToken(c) => {
                ChangeId::remove(c.object.id())
            }
            SemanticChange::AddObject(c) | SemanticChange::AddToken(c) => {
                ChangeId::add(c.object.id())
            }
            SemanticChange::MoveObject(c) | SemanticChange::ReorderObject(c) => {
                ChangeId::position(c.object)
            }
            SemanticChange::SetProperty(c)
            | SemanticChange::ChangeToken(c)
            | SemanticChange::ComponentInterfaceChange(c)
            | SemanticChange::AssetRefChange(c) => ChangeId::property(c.path.clone()),
        }
    }
}

/// 요청 ID를 현재 변경 목록에서 찾지 못함(오래된 요청).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownChange {
    /// 찾지 못한 ID.
    pub id: ChangeId,
}

impl fmt::Display for UnknownChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "현재 변경 목록에 없는 변경 ID입니다(객체 {}): {:?}",
            self.id.object, self.id.unit
        )
    }
}

impl std::error::Error for UnknownChange {}

/// ID들을 현재 변경 목록의 변경으로 되찾습니다(ID 오름차순). 하나라도 없으면 오래된 요청입니다.
pub fn resolve_changes<'a>(
    changes: &'a [SemanticChange],
    ids: &BTreeSet<ChangeId>,
) -> Result<Vec<&'a SemanticChange>, UnknownChange> {
    let mut by_id: std::collections::BTreeMap<ChangeId, &SemanticChange> =
        std::collections::BTreeMap::new();
    for change in changes {
        by_id.insert(change.id(), change);
    }
    ids.iter()
        .map(|id| {
            by_id
                .get(id)
                .copied()
                .ok_or_else(|| UnknownChange { id: id.clone() })
        })
        .collect()
}

/// 선택 변경 작업.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionOperation {
    /// unstaged 변경을 Index에 반영(XIV.5).
    Stage,
    /// staged 변경을 Index에서 되돌림(XIV.7).
    Unstage,
}

impl SelectionOperation {
    /// 안정 numeric code.
    pub fn code(self) -> u8 {
        match self {
            SelectionOperation::Stage => 1,
            SelectionOperation::Unstage => 2,
        }
    }

    /// code에서 복원합니다.
    pub fn from_code(code: u8) -> Option<SelectionOperation> {
        match code {
            1 => Some(SelectionOperation::Stage),
            2 => Some(SelectionOperation::Unstage),
            _ => None,
        }
    }
}

/// 선택 stage·unstage 요청(XIV.5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSelectionRequest {
    /// 작업.
    pub operation: SelectionOperation,
    /// 사용자가 고른 변경.
    pub selected: BTreeSet<ChangeId>,
    /// 이전 계획의 필수 함께-변경 중 호출자가 승인한 변경.
    pub approved: BTreeSet<ChangeId>,
}

impl StageSelectionRequest {
    /// stage 요청(승인 없음).
    pub fn stage(selected: impl IntoIterator<Item = ChangeId>) -> StageSelectionRequest {
        StageSelectionRequest {
            operation: SelectionOperation::Stage,
            selected: selected.into_iter().collect(),
            approved: BTreeSet::new(),
        }
    }

    /// unstage 요청(승인 없음).
    pub fn unstage(selected: impl IntoIterator<Item = ChangeId>) -> StageSelectionRequest {
        StageSelectionRequest {
            operation: SelectionOperation::Unstage,
            ..StageSelectionRequest::stage(selected)
        }
    }

    /// 필수 함께-변경 승인을 더한 요청.
    pub fn with_approved(
        mut self,
        approved: impl IntoIterator<Item = ChangeId>,
    ) -> StageSelectionRequest {
        self.approved.extend(approved);
        self
    }

    /// canonical bytes.
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_header(&mut out, REQUEST_MAGIC, self.operation);
        put_ids(&mut out, &self.selected);
        put_ids(&mut out, &self.approved);
        out
    }

    /// canonical bytes를 읽습니다(trailing·비정렬·중복 거부).
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<StageSelectionRequest, DecodeError> {
        let mut reader = Reader::new(bytes);
        let operation = read_header(&mut reader, REQUEST_MAGIC)?;
        let selected = read_ids(&mut reader, "request.selected")?;
        let approved = read_ids(&mut reader, "request.approved")?;
        reader.finish()?;
        Ok(StageSelectionRequest {
            operation,
            selected,
            approved,
        })
    }
}

/// 선택과 dependency closure(XIV.5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSelection {
    operation: SelectionOperation,
    selected: BTreeSet<ChangeId>,
    closure: BTreeSet<ChangeId>,
}

/// closure가 선택을 모두 포함하지 않음.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureMissingSelection {
    /// closure에 없는 선택 ID.
    pub id: ChangeId,
}

impl StageSelection {
    /// 선택과 closure로 만듭니다. closure는 선택을 모두 포함해야 합니다.
    pub fn new(
        operation: SelectionOperation,
        selected: BTreeSet<ChangeId>,
        mut closure: BTreeSet<ChangeId>,
    ) -> Result<StageSelection, ClosureMissingSelection> {
        if let Some(id) = selected.iter().find(|id| !closure.contains(*id)) {
            return Err(ClosureMissingSelection { id: id.clone() });
        }
        closure.extend(selected.iter().cloned());
        Ok(StageSelection {
            operation,
            selected,
            closure,
        })
    }

    /// 작업.
    pub fn operation(&self) -> SelectionOperation {
        self.operation
    }

    /// 사용자가 고른 변경.
    pub fn selected(&self) -> &BTreeSet<ChangeId> {
        &self.selected
    }

    /// 선택과 그 모든 필수 dependency.
    pub fn closure(&self) -> &BTreeSet<ChangeId> {
        &self.closure
    }

    /// closure에서 선택을 뺀 필수 함께-변경(오름차순).
    pub fn required_additions(&self) -> BTreeSet<ChangeId> {
        self.closure.difference(&self.selected).cloned().collect()
    }

    /// 필수 함께-변경이 없는지(선택만으로 완결).
    pub fn is_self_contained(&self) -> bool {
        self.closure.len() == self.selected.len()
    }

    /// canonical bytes.
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_header(&mut out, SELECTION_MAGIC, self.operation);
        put_ids(&mut out, &self.selected);
        put_ids(&mut out, &self.closure);
        out
    }

    /// canonical bytes를 읽습니다(selected ⊆ closure 검사).
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<StageSelection, DecodeError> {
        let mut reader = Reader::new(bytes);
        let operation = read_header(&mut reader, SELECTION_MAGIC)?;
        let selected = read_ids(&mut reader, "selection.selected")?;
        let closure = read_ids(&mut reader, "selection.closure")?;
        reader.finish()?;
        StageSelection::new(operation, selected, closure).map_err(|_| DecodeError::InvalidValue {
            field: "selection.closure",
        })
    }
}

const REQUEST_MAGIC: &[u8; 4] = b"SSRQ";
const SELECTION_MAGIC: &[u8; 4] = b"SSEL";
const FORMAT_VERSION: u16 = 1;
/// ChangeId 최소 크기: id 16 + tag 1.
const MIN_CHANGE_ID: usize = 17;

fn put_header(out: &mut Vec<u8>, magic: &[u8; 4], operation: SelectionOperation) {
    out.extend_from_slice(magic);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.push(operation.code());
}

fn read_header(
    reader: &mut Reader<'_>,
    magic: &[u8; 4],
) -> Result<SelectionOperation, DecodeError> {
    if reader.take(4)? != magic {
        return Err(DecodeError::BadMagic);
    }
    let version = reader.u16()?;
    if version != FORMAT_VERSION {
        return Err(DecodeError::UnsupportedVersion { version });
    }
    let code = reader.u8()?;
    SelectionOperation::from_code(code).ok_or(DecodeError::UnknownTag {
        field: "selection.operation",
        value: u32::from(code),
    })
}

/// ID 집합을 기록합니다(오름차순).
pub fn put_ids(out: &mut Vec<u8>, ids: &BTreeSet<ChangeId>) {
    out.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    for id in ids {
        id.write_canonical(out);
    }
}

/// ID 집합을 읽습니다(엄격한 오름차순이 아니면 거부).
pub fn read_ids(
    reader: &mut Reader<'_>,
    field: &'static str,
) -> Result<BTreeSet<ChangeId>, DecodeError> {
    let count = reader.count(MIN_CHANGE_ID)?;
    let mut ids = BTreeSet::new();
    let mut last: Option<ChangeId> = None;
    for _ in 0..count {
        let id = ChangeId::read_canonical(reader)?;
        if last.as_ref().is_some_and(|previous| *previous >= id) {
            return Err(DecodeError::NonCanonicalOrder { field });
        }
        last = Some(id.clone());
        ids.insert(id);
    }
    Ok(ids)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::build::tests::{build, sid};
    use crate::diff::diff;
    use crate::keys;
    use crate::status::tests::partial_staged;

    pub(crate) fn path(id: u64, key: stellar_core_types::property::PropertyKey) -> PropertyPath {
        PropertyPath::with_keys(sid(id), vec![key])
    }

    #[test]
    fn change_ids_are_unique_within_a_diff_and_resolve_back() {
        let [head, index, working] = partial_staged().map(|r| build(&r));
        for (base, target) in [(&head, &index), (&index, &working), (&head, &working)] {
            let changes = diff(base, target);
            let ids: BTreeSet<ChangeId> = changes.iter().map(SemanticChange::id).collect();
            assert_eq!(ids.len(), changes.len(), "diff 안 ID 유일");
            let resolved = resolve_changes(&changes, &ids).unwrap();
            assert_eq!(resolved.len(), changes.len());
            for change in resolved {
                assert!(changes.contains(change));
            }
        }
        let unstaged = diff(&index, &working);
        assert_eq!(
            unstaged.iter().map(SemanticChange::id).collect::<Vec<_>>(),
            vec![
                ChangeId::property(path(11, keys::VISIBLE)),
                ChangeId::remove(sid(13)),
            ]
        );
        // 현재 목록에 없는 ID는 오래된 요청입니다.
        let stale: BTreeSet<ChangeId> = [ChangeId::add(sid(99))].into();
        assert_eq!(
            resolve_changes(&unstaged, &stale),
            Err(UnknownChange {
                id: ChangeId::add(sid(99))
            })
        );
    }

    #[test]
    fn replacement_has_distinct_remove_and_add_ids() {
        use crate::snapshot::{ObjectKind, Origin, SemanticObject};
        let [_, index, _] = partial_staged().map(|r| build(&r));
        let mut replaced = index.clone();
        replaced.remove(sid(13)).unwrap();
        replaced
            .insert(SemanticObject::new(
                sid(13),
                ObjectKind::Token,
                Origin::Project,
            ))
            .unwrap();
        let ids: Vec<ChangeId> = diff(&index, &replaced).iter().map(|c| c.id()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&ChangeId::remove(sid(13))));
        assert!(ids.contains(&ChangeId::add(sid(13))));
    }

    pub(crate) fn sample_ids() -> Vec<ChangeId> {
        vec![
            ChangeId::remove(sid(13)),
            ChangeId::add(sid(14)),
            ChangeId::position(sid(12)),
            ChangeId::property(path(11, keys::VISIBLE)),
            ChangeId::property(path(11, keys::RECT_SIZE).with_index(1)),
            ChangeId::property(
                PropertyPath::with_keys(sid(40), vec![keys::instance::PROPERTY_OVERRIDES])
                    .with_key_segment("a/b"),
            ),
        ]
    }

    #[test]
    fn request_and_selection_round_trip() {
        let ids = sample_ids();
        let request = StageSelectionRequest::stage(ids[..3].iter().cloned())
            .with_approved(ids[3..].iter().cloned());
        assert_eq!(request.operation, SelectionOperation::Stage);
        let bytes = request.to_canonical_bytes();
        assert_eq!(&bytes[..4], b"SSRQ");
        assert_eq!(
            StageSelectionRequest::from_canonical_bytes(&bytes),
            Ok(request)
        );
        let unstage = StageSelectionRequest::unstage(ids.iter().cloned());
        assert_eq!(unstage.operation, SelectionOperation::Unstage);
        assert!(unstage.approved.is_empty());
        assert_eq!(
            StageSelectionRequest::from_canonical_bytes(&unstage.to_canonical_bytes()),
            Ok(unstage)
        );

        let selected: BTreeSet<ChangeId> = ids[..2].iter().cloned().collect();
        let closure: BTreeSet<ChangeId> = ids.iter().cloned().collect();
        let selection =
            StageSelection::new(SelectionOperation::Stage, selected.clone(), closure.clone())
                .unwrap();
        assert_eq!(selection.selected(), &selected);
        assert_eq!(selection.closure(), &closure);
        assert_eq!(
            selection.required_additions(),
            ids[2..].iter().cloned().collect::<BTreeSet<_>>()
        );
        assert!(!selection.is_self_contained());
        let bytes = selection.to_canonical_bytes();
        assert_eq!(StageSelection::from_canonical_bytes(&bytes), Ok(selection));

        let alone = StageSelection::new(
            SelectionOperation::Unstage,
            selected.clone(),
            selected.clone(),
        )
        .unwrap();
        assert!(alone.is_self_contained());
        assert!(alone.required_additions().is_empty());
        // closure가 선택을 빠뜨리면 거부합니다.
        assert_eq!(
            StageSelection::new(SelectionOperation::Stage, selected, BTreeSet::new()),
            Err(ClosureMissingSelection {
                id: ids[0].clone().min(ids[1].clone())
            })
        );
    }

    #[test]
    fn malformed_bytes_are_rejected_without_panic() {
        let ids = sample_ids();
        let request = StageSelectionRequest::stage(ids.iter().cloned());
        let bytes = request.to_canonical_bytes();
        // 모든 길이로 자른 입력은 오류이고 panic하지 않습니다.
        for len in 0..bytes.len() {
            assert!(StageSelectionRequest::from_canonical_bytes(&bytes[..len]).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            StageSelectionRequest::from_canonical_bytes(&trailing),
            Err(DecodeError::TrailingBytes { remaining: 1 })
        );
        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert_eq!(
            StageSelectionRequest::from_canonical_bytes(&magic),
            Err(DecodeError::BadMagic)
        );
        let mut operation = bytes.clone();
        operation[6] = 9;
        assert!(matches!(
            StageSelectionRequest::from_canonical_bytes(&operation),
            Err(DecodeError::UnknownTag { .. })
        ));
        // 비정렬 집합.
        let mut out = Vec::new();
        put_header(&mut out, REQUEST_MAGIC, SelectionOperation::Stage);
        out.extend_from_slice(&2u32.to_le_bytes());
        ChangeId::add(sid(2)).write_canonical(&mut out);
        ChangeId::add(sid(1)).write_canonical(&mut out);
        out.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            StageSelectionRequest::from_canonical_bytes(&out),
            Err(DecodeError::NonCanonicalOrder {
                field: "request.selected"
            })
        );
        // 경로 root가 객체가 아닌 속성 ID.
        let mut out = Vec::new();
        put_header(&mut out, REQUEST_MAGIC, SelectionOperation::Stage);
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&sid(1).to_bytes());
        out.push(4);
        put_path(&mut out, &path(2, keys::VISIBLE));
        out.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            StageSelectionRequest::from_canonical_bytes(&out),
            Err(DecodeError::InvalidValue {
                field: "change_id.path_root"
            })
        );
        // 개수 필드가 남은 bytes보다 큰 경우 미리 할당하지 않고 거부합니다.
        let mut out = Vec::new();
        put_header(&mut out, REQUEST_MAGIC, SelectionOperation::Stage);
        out.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            StageSelectionRequest::from_canonical_bytes(&out),
            Err(DecodeError::Truncated)
        );
        // selection: closure가 선택을 빠뜨린 bytes.
        let mut out = Vec::new();
        put_header(&mut out, SELECTION_MAGIC, SelectionOperation::Stage);
        put_ids(&mut out, &[ChangeId::add(sid(1))].into());
        put_ids(&mut out, &BTreeSet::new());
        assert_eq!(
            StageSelection::from_canonical_bytes(&out),
            Err(DecodeError::InvalidValue {
                field: "selection.closure"
            })
        );
    }

    #[test]
    fn operation_codes_round_trip() {
        for op in [SelectionOperation::Stage, SelectionOperation::Unstage] {
            assert_eq!(SelectionOperation::from_code(op.code()), Some(op));
        }
        assert_eq!(SelectionOperation::from_code(0), None);
    }
}
