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
//!
//! **계획(XIV.5.3)**: [`plan_stage`]는 현재 Index → WorkingTree diff에서 dependency graph(XIV.5.2)로
//! closure를 구하고, 선택만 Index에 적용한 snapshot을 validator(XIV.5.3 [`validate`](crate::validate))로
//! 검사해 [`StagePlan`]을 돌려줍니다. 필수 dependency는 `required_additions`로 **반환만** 하고 자동으로
//! 포함하지 않습니다. Index·WorkingTree snapshot은 읽기만 합니다. graph는 validator가 볼 수 없는 참조
//! (binding payload 안 token 등)까지 의존으로 보므로, validator가 통과해도 필수 변경이 있을 수 있습니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;

use stellar_document::validate::{validate_document, DocumentViolation};

use crate::apply::{apply, ApplyError};
use crate::change::SemanticChange;
use crate::codec::{put_path, read_path, DecodeError, Reader};
use crate::dependency::DependencyGraph;
use crate::diff::{diff, SemanticDiff};
use crate::records::{document_records, DocumentProjection, ProjectionError};
use crate::snapshot::{ObjectKind, SemanticSnapshot};
use crate::validate::{introduced_violations, SnapshotViolation};

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

/// 선택만 적용했을 때의 결과(XIV.5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedOnlyOutcome {
    /// validator가 새 위반을 찾지 못함.
    Valid,
    /// 기준(Index·WorkingTree) 어디에도 없던 위반이 생김.
    Invalid(BTreeSet<SnapshotViolation>),
    /// 선택만으로는 적용할 수 없음(예: 교체의 추가만 선택).
    NotApplicable(ApplyError),
}

/// 선택 stage·unstage 계획(XIV.5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagePlan {
    /// 선택과 dependency closure.
    pub selection: StageSelection,
    /// 선택만 기준 snapshot에 적용했을 때의 결과.
    pub selected_only: SelectedOnlyOutcome,
}

impl StagePlan {
    /// 호출자 승인이 필요한 필수 함께-변경.
    pub fn required_additions(&self) -> BTreeSet<ChangeId> {
        self.selection.required_additions()
    }

    /// 승인이 필요한지(필수 함께-변경이 있음).
    pub fn needs_approval(&self) -> bool {
        !self.selection.is_self_contained()
    }
}

/// staging 계획·candidate 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageError {
    /// 요청 ID가 현재 변경 목록에 없음(오래된 요청).
    UnknownChange(UnknownChange),
    /// 요청 작업이 이 함수와 맞지 않음.
    WrongOperation {
        /// 이 함수가 받는 작업.
        expected: SelectionOperation,
        /// 요청 작업.
        found: SelectionOperation,
    },
    /// 필수 함께-변경을 호출자가 모두 승인하지 않음(candidate를 만들지 않음).
    ApprovalRequired {
        /// 승인이 필요한 전체 필수 변경.
        required: BTreeSet<ChangeId>,
    },
    /// 승인 목록에 필수 변경이 아닌 ID가 있음(오래된 승인).
    UnexpectedApproval {
        /// 필수 변경이 아닌 ID.
        id: ChangeId,
    },
    /// closure 적용 실패.
    Apply(ApplyError),
    /// candidate가 snapshot 무결성 위반을 새로 만듦.
    CandidateInvalid {
        /// 새 위반.
        violations: BTreeSet<SnapshotViolation>,
    },
    /// candidate 문서를 레코드로 복원하지 못함.
    Projection(ProjectionError),
    /// candidate 문서가 문서 validator(VI.10.1)를 통과하지 못함.
    DocumentInvalid {
        /// 문서.
        document: StableId,
        /// 기준 Index 문서에 없던 위반(발견 순).
        violations: Vec<DocumentViolation>,
    },
}

impl fmt::Display for StageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StageError::UnknownChange(e) => write!(f, "{e}"),
            StageError::WrongOperation { expected, found } => {
                write!(
                    f,
                    "요청 작업 {found:?}은 {expected:?} 계획에 쓸 수 없습니다"
                )
            }
            StageError::ApprovalRequired { required } => {
                write!(f, "필수 함께-변경 {}개의 승인이 필요합니다", required.len())
            }
            StageError::UnexpectedApproval { id } => {
                write!(f, "필수 변경이 아닌 승인 ID(객체 {})", id.object)
            }
            StageError::Apply(e) => write!(f, "{e}"),
            StageError::CandidateInvalid { violations } => {
                write!(f, "candidate 무결성 위반 {}건", violations.len())
            }
            StageError::Projection(e) => write!(f, "{e}"),
            StageError::DocumentInvalid {
                document,
                violations,
            } => write!(
                f,
                "candidate 문서 {document} 검증 실패({}건)",
                violations.len()
            ),
        }
    }
}

impl std::error::Error for StageError {}

impl From<UnknownChange> for StageError {
    fn from(e: UnknownChange) -> StageError {
        StageError::UnknownChange(e)
    }
}

/// `base → target` diff에서 선택의 closure와 선택만 적용한 결과를 계산합니다.
///
/// `baselines`의 위반은 이미 있던 것으로 보고 제외합니다.
pub(crate) fn plan_selection(
    base: &SemanticSnapshot,
    target: &SemanticSnapshot,
    operation: SelectionOperation,
    selected: &BTreeSet<ChangeId>,
) -> Result<StagePlan, StageError> {
    plan_selection_with(base, target, &diff(base, target), operation, selected)
}

/// [`plan_selection`]을 이미 계산한 `base → target` 변경 목록으로 실행합니다.
pub(crate) fn plan_selection_with(
    base: &SemanticSnapshot,
    target: &SemanticSnapshot,
    changes: &[SemanticChange],
    operation: SelectionOperation,
    selected: &BTreeSet<ChangeId>,
) -> Result<StagePlan, StageError> {
    let graph = DependencyGraph::build(base, target, changes);
    let selection = graph.select(operation, selected)?;
    let chosen: Vec<SemanticChange> = resolve_changes(changes, selection.selected())?
        .into_iter()
        .cloned()
        .collect();
    let selected_only = match apply(base, &chosen) {
        Ok(candidate) => {
            let violations = introduced_violations(&candidate, &[base, target]);
            if violations.is_empty() {
                SelectedOnlyOutcome::Valid
            } else {
                SelectedOnlyOutcome::Invalid(violations)
            }
        }
        Err(error) => SelectedOnlyOutcome::NotApplicable(error),
    };
    Ok(StagePlan {
        selection,
        selected_only,
    })
}

/// 선택 stage 계획(XIV.5.3): Index → WorkingTree 변경 중 선택의 필수 dependency를 계산합니다.
///
/// 필수 dependency는 [`StagePlan::required_additions`]로 반환만 하고 자동으로 stage하지 않습니다.
/// `request.approved`는 이 단계에서 쓰지 않습니다(candidate 단계에서 검사).
pub fn plan_stage(
    index: &SemanticSnapshot,
    working: &SemanticSnapshot,
    request: &StageSelectionRequest,
) -> Result<StagePlan, StageError> {
    if request.operation != SelectionOperation::Stage {
        return Err(StageError::WrongOperation {
            expected: SelectionOperation::Stage,
            found: request.operation,
        });
    }
    plan_selection(index, working, SelectionOperation::Stage, &request.selected)
}

/// 승인된 선택(선택 + 필수 변경)을 기준 snapshot에 적용한 candidate(XIV.5.4).
///
/// stage면 기준은 현재 Index이고, candidate는 Git index에 기록할 다음 Index 상태입니다. candidate는
/// semantic 무결성(XIV.5.3)과 문서 validator(VI.10.1)를 통과한 상태로만 만들어집니다.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateIndexSnapshot {
    selection: StageSelection,
    snapshot: SemanticSnapshot,
    applied: SemanticDiff,
    documents: Vec<DocumentProjection>,
}

impl CandidateIndexSnapshot {
    /// 선택과 closure.
    pub fn selection(&self) -> &StageSelection {
        &self.selection
    }

    /// candidate snapshot.
    pub fn snapshot(&self) -> &SemanticSnapshot {
        &self.snapshot
    }

    /// 기준 snapshot → candidate 변경(canonical 순서).
    pub fn applied(&self) -> &SemanticDiff {
        &self.applied
    }

    /// 변경이 닿은 문서의 검증된 레코드(문서 StableId 오름차순).
    pub fn documents(&self) -> &[DocumentProjection] {
        &self.documents
    }

    /// candidate snapshot을 꺼냅니다.
    pub fn into_snapshot(self) -> SemanticSnapshot {
        self.snapshot
    }
}

/// 문서 validator 위반 목록(통과면 빈 목록). 문서가 없거나 복원에 실패하면 `None`.
fn document_violations(
    snapshot: &SemanticSnapshot,
    document: StableId,
) -> Option<Vec<DocumentViolation>> {
    let projection = document_records(snapshot, document).ok()?;
    Some(match validate_document(&projection.records) {
        Ok(_) => Vec::new(),
        Err(error) => error.violations,
    })
}

/// 승인된 선택으로 candidate를 만듭니다(공통: stage·unstage).
///
/// `base`는 적용 기준(Index), `target`은 선택 변경이 가리키는 상태(stage: WorkingTree, unstage: HEAD)입니다.
/// semantic 위반은 `base`·`target` 어디에도 없던 것만, 문서 위반은 `base` 문서에 없던 것만 실패로 봅니다.
pub(crate) fn build_candidate(
    base: &SemanticSnapshot,
    target: &SemanticSnapshot,
    operation: SelectionOperation,
    request: &StageSelectionRequest,
) -> Result<CandidateIndexSnapshot, StageError> {
    let changes = diff(base, target);
    let plan = plan_selection_with(base, target, &changes, operation, &request.selected)?;
    let required = plan.required_additions();
    if let Some(id) = request.approved.iter().find(|id| !required.contains(*id)) {
        return Err(StageError::UnexpectedApproval { id: id.clone() });
    }
    if !required.is_subset(&request.approved) {
        return Err(StageError::ApprovalRequired { required });
    }
    let closure: Vec<SemanticChange> = resolve_changes(&changes, plan.selection.closure())?
        .into_iter()
        .cloned()
        .collect();
    let candidate = apply(base, &closure).map_err(StageError::Apply)?;
    let violations = introduced_violations(&candidate, &[base, target]);
    if !violations.is_empty() {
        return Err(StageError::CandidateInvalid { violations });
    }
    let touched: BTreeSet<StableId> = closure
        .iter()
        .filter_map(SemanticChange::document)
        .collect();
    let mut documents = Vec::new();
    for document in touched {
        if candidate
            .get(document)
            .is_none_or(|o| o.kind() != ObjectKind::Document)
        {
            // 문서 자체를 지우는 선택: 남은 레코드가 없습니다.
            continue;
        }
        let projection = document_records(&candidate, document).map_err(StageError::Projection)?;
        if let Err(error) = validate_document(&projection.records) {
            let existing = document_violations(base, document).unwrap_or_default();
            let introduced: Vec<DocumentViolation> = error
                .violations
                .into_iter()
                .filter(|v| !existing.contains(v))
                .collect();
            if !introduced.is_empty() {
                return Err(StageError::DocumentInvalid {
                    document,
                    violations: introduced,
                });
            }
        }
        documents.push(projection);
    }
    Ok(CandidateIndexSnapshot {
        selection: plan.selection,
        applied: diff(base, &candidate),
        snapshot: candidate,
        documents,
    })
}

/// 승인된 선택 stage candidate(XIV.5.4): Index에 선택 + 필수 변경을 적용합니다.
///
/// 필수 변경은 `request.approved`가 모두 포함해야 하며(아니면 [`StageError::ApprovalRequired`]), 필수가
/// 아닌 승인 ID는 오래된 승인으로 거부합니다. 결과는 semantic validator와 문서 validator를 통과합니다.
/// Index·WorkingTree snapshot은 바꾸지 않습니다.
pub fn build_stage_candidate(
    index: &SemanticSnapshot,
    working: &SemanticSnapshot,
    request: &StageSelectionRequest,
) -> Result<CandidateIndexSnapshot, StageError> {
    if request.operation != SelectionOperation::Stage {
        return Err(StageError::WrongOperation {
            expected: SelectionOperation::Stage,
            found: request.operation,
        });
    }
    build_candidate(index, working, SelectionOperation::Stage, request)
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

    /// staging fixture(Index, WorkingTree).
    ///
    /// - Index: 문서 1 ─ 페이지 2 ─ View 3(root frame 10), frame 10 아래 frame 11·인스턴스 400(컴포넌트 300,
    ///   override 310), token 200, binding expression 600(token 200 참조).
    /// - WorkingTree: 새 페이지 4 ─ View 5 ─ root frame 20 ─ 자식 21(문서 페이지 목록 변경), interface에
    ///   Property 311 추가와 그 override, frame 11 숨김(독립 변경).
    pub(crate) fn staging_fixture() -> (SemanticSnapshot, SemanticSnapshot) {
        use crate::dependency::tests::{component_base, frame, object, replace};
        use crate::snapshot::{ObjectKind, Origin};
        use crate::value::SemanticValue;
        let doc = Origin::Document(sid(1));
        let mut index = component_base();
        for o in [
            object(
                1,
                ObjectKind::Document,
                doc,
                None,
                None,
                vec![(
                    path(1, keys::DOCUMENT_PAGES),
                    SemanticValue::Ids(vec![sid(2)]),
                )],
            ),
            object(
                2,
                ObjectKind::Page,
                doc,
                Some(1),
                None,
                vec![(path(2, keys::PAGE_VIEWS), SemanticValue::Ids(vec![sid(3)]))],
            ),
            object(
                3,
                ObjectKind::View,
                doc,
                Some(2),
                None,
                vec![(path(3, keys::VIEW_ROOT_FRAME), SemanticValue::Id(sid(10)))],
            ),
            frame(11, Some(10), 0x40),
        ] {
            index.insert(o).unwrap();
        }
        let mut working = index.clone();
        let mut document = working.get(sid(1)).unwrap().clone();
        document
            .set_property(
                path(1, keys::DOCUMENT_PAGES),
                SemanticValue::Ids(vec![sid(2), sid(4)]),
            )
            .unwrap();
        replace(&mut working, document);
        for o in [
            object(
                4,
                ObjectKind::Page,
                doc,
                Some(1),
                None,
                vec![(path(4, keys::PAGE_VIEWS), SemanticValue::Ids(vec![sid(5)]))],
            ),
            object(
                5,
                ObjectKind::View,
                doc,
                Some(4),
                None,
                vec![(path(5, keys::VIEW_ROOT_FRAME), SemanticValue::Id(sid(20)))],
            ),
            frame(20, None, 0x80),
            frame(21, Some(20), 0x80),
        ] {
            working.insert(o).unwrap();
        }
        let mut component = working.get(sid(300)).unwrap().clone();
        component
            .set_property(
                path(300, keys::COMPONENT_PROPERTIES),
                SemanticValue::Ids(vec![sid(310), sid(311)]),
            )
            .unwrap();
        replace(&mut working, component);
        let mut instance = working.get(sid(400)).unwrap().clone();
        instance
            .set_property(override_path(311), SemanticValue::Bool(true))
            .unwrap();
        replace(&mut working, instance);
        let mut hidden = frame(11, Some(10), 0x40);
        hidden
            .set_property(path(11, keys::VISIBLE), SemanticValue::Bool(false))
            .unwrap();
        replace(&mut working, hidden);
        (index, working)
    }

    pub(crate) fn override_path(property: u64) -> PropertyPath {
        path(400, keys::instance::PROPERTY_OVERRIDES).with_key_segment(&sid(property).to_string())
    }

    #[test]
    fn invalid_partial_selection_returns_required_dependencies_without_staging() {
        let (index, working) = staging_fixture();
        let index_before = index.clone();
        // fixture 자체는 Index·WorkingTree 모두 위반 없음(인스턴스·root 정상).
        assert!(crate::validate::validate_snapshot(&index).is_empty());
        assert!(crate::validate::validate_snapshot(&working).is_empty());

        // 새 자식 21만 선택: parent 20이 없어 invalid → 새 subtree와 페이지 목록이 필수.
        let plan = plan_stage(
            &index,
            &working,
            &StageSelectionRequest::stage([ChangeId::add(sid(21))]),
        )
        .unwrap();
        assert!(plan.needs_approval());
        assert_eq!(
            plan.required_additions(),
            [
                ChangeId::property(path(1, keys::DOCUMENT_PAGES)),
                ChangeId::add(sid(4)),
                ChangeId::add(sid(5)),
                ChangeId::add(sid(20)),
            ]
            .into()
        );
        assert_eq!(
            plan.selected_only,
            SelectedOnlyOutcome::Invalid(
                [SnapshotViolation::MissingParent {
                    object: sid(21),
                    parent: sid(20)
                }]
                .into()
            )
        );

        // override 311만 선택: interface에 311이 없어 invalid → interface 변경이 필수.
        let plan = plan_stage(
            &index,
            &working,
            &StageSelectionRequest::stage([ChangeId::property(override_path(311))]),
        )
        .unwrap();
        assert_eq!(
            plan.required_additions(),
            [ChangeId::property(path(300, keys::COMPONENT_PROPERTIES))].into()
        );
        assert_eq!(
            plan.selected_only,
            SelectedOnlyOutcome::Invalid(
                [SnapshotViolation::UnknownInterfaceMember {
                    path: override_path(311),
                    member: sid(311)
                }]
                .into()
            )
        );

        // 독립 변경은 필수 변경 없이 유효.
        let plan = plan_stage(
            &index,
            &working,
            &StageSelectionRequest::stage([ChangeId::property(path(11, keys::VISIBLE))]),
        )
        .unwrap();
        assert!(!plan.needs_approval());
        assert_eq!(plan.selected_only, SelectedOnlyOutcome::Valid);

        // 계획은 Index를 바꾸지 않습니다.
        assert_eq!(index, index_before);
    }

    #[test]
    fn stale_or_wrong_requests_are_rejected() {
        let (index, working) = staging_fixture();
        assert_eq!(
            plan_stage(
                &index,
                &working,
                &StageSelectionRequest::stage([ChangeId::add(sid(99))])
            ),
            Err(StageError::UnknownChange(UnknownChange {
                id: ChangeId::add(sid(99))
            }))
        );
        assert_eq!(
            plan_stage(
                &index,
                &working,
                &StageSelectionRequest::unstage([ChangeId::add(sid(21))])
            ),
            Err(StageError::WrongOperation {
                expected: SelectionOperation::Stage,
                found: SelectionOperation::Unstage
            })
        );
    }

    /// 레코드 기반 staging fixture(Index, WorkingTree) — 문서 validator 대상.
    ///
    /// Index: HEAD fixture(frame 10 아래 rect 11·image 12·fragment 13, rect 11 fill modifier 50).
    /// WorkingTree: 새 페이지 4 ─ View 5 ─ root frame 20 ─ rect 21(+ modifier 60), rect 11 크기 변경.
    pub(crate) fn record_staging_fixture() -> (SemanticSnapshot, SemanticSnapshot) {
        use crate::build::tests::{head_records, node, rect};
        use stellar_document::definitions::{PageDefinition, ViewDefinition};
        use stellar_document::frame::FramePayload;
        use stellar_document::modifier::{ModifierId, ModifierKind, ModifierStack};
        use stellar_document::records::NodePayload;
        let index = head_records();
        let mut working = index.clone();
        let mut page = PageDefinition::new(sid(4), "새 페이지".into());
        page.add_view(ViewDefinition::new(sid(5), "새 View".into(), sid(20)))
            .unwrap();
        working.definition.add_page(page).unwrap();
        working.nodes.push(node(
            20,
            None,
            0x80,
            "new-root",
            NodePayload::Frame(FramePayload::default()),
        ));
        working
            .nodes
            .push(node(21, Some(20), 0x80, "new-box", rect(5.0, 5.0)));
        let mut stack = ModifierStack::new(sid(21));
        stack
            .push_back(ModifierId::new(sid(60)), ModifierKind::Opacity)
            .unwrap();
        working.modifier_stacks.push(stack);
        working.nodes[1].payload = rect(40.0, 20.0);
        (build(&index), build(&working))
    }

    #[test]
    fn approved_selection_builds_a_candidate_that_passes_the_document_validator() {
        let (index, working) = record_staging_fixture();
        let index_before = index.clone();
        let working_before = working.clone();
        let select = StageSelectionRequest::stage([ChangeId::add(sid(21))]);
        let plan = plan_stage(&index, &working, &select).unwrap();
        let required = plan.required_additions();
        assert_eq!(
            required,
            [
                ChangeId::property(path(1, keys::DOCUMENT_PAGES)),
                ChangeId::add(sid(4)),
                ChangeId::add(sid(5)),
                ChangeId::add(sid(20)),
            ]
            .into()
        );
        // 승인 전에는 candidate를 만들지 않습니다.
        assert_eq!(
            build_stage_candidate(&index, &working, &select),
            Err(StageError::ApprovalRequired {
                required: required.clone()
            })
        );
        // 일부만 승인해도 거부, 필수가 아닌 ID 승인은 오래된 승인.
        let partial = select.clone().with_approved([ChangeId::add(sid(20))]);
        assert!(matches!(
            build_stage_candidate(&index, &working, &partial),
            Err(StageError::ApprovalRequired { .. })
        ));
        let stale = select
            .clone()
            .with_approved(required.iter().cloned().chain([ChangeId::add(sid(60))]));
        assert_eq!(
            build_stage_candidate(&index, &working, &stale),
            Err(StageError::UnexpectedApproval {
                id: ChangeId::add(sid(60))
            })
        );

        // 승인 후 candidate.
        let approved = select.with_approved(required.iter().cloned());
        let candidate = build_stage_candidate(&index, &working, &approved).unwrap();
        let snapshot = candidate.snapshot();
        for id in [4, 5, 20, 21] {
            assert!(snapshot.contains(sid(id)), "{id}");
        }
        // 선택하지 않은 변경(modifier 60, rect 11 크기)은 들어가지 않습니다.
        assert!(!snapshot.contains(sid(60)));
        let size = path(11, keys::RECT_SIZE);
        assert_eq!(
            snapshot.get(sid(11)).unwrap().property(&size),
            index.get(sid(11)).unwrap().property(&size)
        );
        assert_eq!(candidate.applied().len(), 5);
        assert_eq!(candidate.selection().closure().len(), 5);
        // candidate 문서는 문서 validator(VI.10.1)를 통과합니다.
        assert_eq!(candidate.documents().len(), 1);
        let records = &candidate.documents()[0].records;
        assert!(validate_document(records).is_ok());
        assert_eq!(records.definition.pages.len(), 2);
        assert!(crate::validate::validate_snapshot(snapshot)
            .iter()
            .all(|v| matches!(v, SnapshotViolation::DanglingReference { target, .. } if *target == sid(90))));
        // 입력 snapshot은 그대로입니다.
        assert_eq!(index, index_before);
        assert_eq!(working, working_before);
    }

    #[test]
    fn staging_every_change_reproduces_the_working_tree() {
        use stellar_document::test_support::random_document::random_valid_document;
        for seed in 0..20 {
            let index = build(&random_valid_document(seed).to_records().unwrap());
            let working = build(&random_valid_document(seed + 1000).to_records().unwrap());
            let all: BTreeSet<ChangeId> = diff(&index, &working).iter().map(|c| c.id()).collect();
            let candidate =
                build_stage_candidate(&index, &working, &StageSelectionRequest::stage(all))
                    .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            assert_eq!(candidate.snapshot(), &working, "seed {seed}");
            assert!(candidate.selection().is_self_contained());
        }
    }

    #[test]
    fn candidate_failing_the_document_validator_is_rejected() {
        use crate::build::tests::{head_records, node};
        use stellar_document::fragment::FragmentPayload;
        use stellar_document::records::NodePayload;
        // WorkingTree가 Shape(rect 11) 아래에 자식을 둠: semantic 무결성은 통과하지만 문서 validator
        // (parent consistency: Shape는 자식을 가질 수 없음)는 거부합니다.
        let index = head_records();
        let mut working = index.clone();
        working.nodes.push(node(
            30,
            Some(11),
            0x80,
            "bad",
            NodePayload::Fragment(FragmentPayload),
        ));
        let (index, working) = (build(&index), build(&working));
        let request = StageSelectionRequest::stage([ChangeId::add(sid(30))]);
        match build_stage_candidate(&index, &working, &request) {
            Err(StageError::DocumentInvalid {
                document,
                violations,
            }) => {
                assert_eq!(document, sid(1));
                assert!(!violations.is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            build_stage_candidate(
                &index,
                &working,
                &StageSelectionRequest::unstage([ChangeId::add(sid(30))])
            ),
            Err(StageError::WrongOperation {
                expected: SelectionOperation::Stage,
                found: SelectionOperation::Unstage
            })
        );
    }

    #[test]
    fn replacement_half_is_not_applicable_alone() {
        use crate::snapshot::{ObjectKind, Origin, SemanticObject};
        let (index, _) = staging_fixture();
        let mut working = index.clone();
        working.remove(sid(11)).unwrap();
        working
            .insert(SemanticObject::new(
                sid(11),
                ObjectKind::Token,
                Origin::Project,
            ))
            .unwrap();
        let plan = plan_stage(
            &index,
            &working,
            &StageSelectionRequest::stage([ChangeId::add(sid(11))]),
        )
        .unwrap();
        assert_eq!(
            plan.required_additions(),
            [ChangeId::remove(sid(11))].into()
        );
        assert_eq!(
            plan.selected_only,
            SelectedOnlyOutcome::NotApplicable(ApplyError::ObjectExists { id: sid(11) })
        );
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
