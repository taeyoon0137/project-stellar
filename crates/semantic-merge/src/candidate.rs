//! 독립 `CandidateSnapshot C`와 해결 모델 (XXIII.2, XIV.12.1~XIV.12.3).
//!
//! Semantic Merge 결과를 입력(Source A = ours, Source B = theirs, base)과 분리된 메모리 값으로 둡니다
//! (아키텍처 §27.4 "결과 C는 A·B를 직접 변경하지 않는 가상 Snapshot"). C는 입력 세 snapshot을 복사해
//! 가지므로 A·B는 바뀌지 않고, 파일·Git index도 읽거나 쓰지 않습니다. 해결(XIV.12)은 C에만 적용합니다.
//!
//! **단위와 출처(XXIII.2.2)**: 단위는 `(StableId, 대상)`입니다(대상 = 객체 존재·위치·속성 경로, conflict 대상과
//! 같은 [`ConflictTarget`]). C는 base와 다른 단위와 conflict 단위마다 출처 [`Source`]를 기록합니다: A·B 한쪽
//! 상태와 같으면 그쪽, 양쪽과 같거나(같은 변경) 엔진이 계산한 값(key 재배정·정의 목록 병합)이면 `Auto`,
//! conflict로 base를 유지하면 `Base`, 사용자가 직접 넣은 값이면 `Custom`입니다. 기록이 없는 단위는 base 그대로입니다.
//!
//! **해결(XIV.12.1, ADR-0081 결정 4)**: [`Resolution`]은 `ChooseBase`·`ChooseOurs`·`ChooseTheirs`·
//! `CustomValue`·`Delete` 닫힌 enum입니다. 허용 action은 conflict 대상과 세 상태로 정합니다
//! ([`allowed_resolutions`]). 계획서의 `KeepModified`는 수정 쪽 선택과 같아 따로 두지 않습니다.
//!
//! **command(XXIII.2.3, XIV.12.2)**: [`CandidateCommand`]는 conflict 해결(`Resolve`), 임의 단위의 A·B·base
//! 선택(`Select`), 역 command(`Restore`)입니다. [`CandidateSnapshot::apply`]는 C 사본에서 모두 성공할 때만 C를
//! 바꾸고 역 command를 돌려줍니다: `apply(inverse)`는 C를 이전과 같은 값으로 되돌립니다. C 밖(파일·index)은
//! 건드리지 않습니다. Property 삭제 conflict(전용 종류 7·9)를 삭제 쪽으로 해결하면 C의 scene·정의 인스턴스에서
//! 지운 Property를 가리키는 override도 함께 지웁니다(삭제 쪽 상태와 같음).
//!
//! **쓰기 조건(XIV.12.3)**: [`CandidateSnapshot::validate`]는 unresolved conflict가 0이고 C가 세 입력
//! 어디에도 없던 snapshot 위반을 갖지 않을 때만 성공합니다.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use stellar_semantic_diff::change::{Position, SemanticChange};
use stellar_semantic_diff::core_types::ids::StableId;
use stellar_semantic_diff::diff::diff;
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_diff::validate::override_member;
use stellar_semantic_diff::validate::{introduced_violations, SnapshotViolation};
use stellar_semantic_diff::value::SemanticValue;

use crate::merge::merge;
use crate::model::ConflictKind;
use crate::model::{ConflictTarget, SideState};
use crate::model::{MergeError, MergeInput, MergeResult, SemanticConflict};

/// 단위 `(StableId, 대상)`.
pub type UnitKey = (StableId, ConflictTarget);

/// 단위 값의 출처(XXIII.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Source {
    /// base 상태(conflict로 유지했거나 base를 골랐음).
    Base = 0,
    /// Source A(ours).
    A = 1,
    /// Source B(theirs).
    B = 2,
    /// 양쪽이 같게 바꿨거나 엔진이 계산함.
    Auto = 3,
    /// 사용자가 직접 넣은 값.
    Custom = 4,
}

impl Source {
    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            Source::Base => "base",
            Source::A => "a",
            Source::B => "b",
            Source::Auto => "auto",
            Source::Custom => "custom",
        }
    }
}

/// 병합 입력의 한쪽.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    /// 공통 조상.
    Base,
    /// Source A(ours).
    Ours,
    /// Source B(theirs).
    Theirs,
}

impl Side {
    fn source(self) -> Source {
        match self {
            Side::Base => Source::Base,
            Side::Ours => Source::A,
            Side::Theirs => Source::B,
        }
    }
}

/// snapshot에서 단위 하나의 상태. 객체가 없으면 `Missing`.
pub fn unit_state(snapshot: &SemanticSnapshot, (object, target): &UnitKey) -> SideState {
    let Some(found) = snapshot.get(*object) else {
        return SideState::Missing;
    };
    match target {
        ConflictTarget::Object => SideState::Object(Box::new(found.clone())),
        ConflictTarget::Position => SideState::Position(Position::of(found)),
        ConflictTarget::Property(path) => SideState::Value(found.property(path).cloned()),
    }
}

/// 변경 하나의 단위.
fn unit_of(change: &SemanticChange) -> UnitKey {
    use stellar_semantic_diff::change::SemanticChangeKind as K;
    let target = match change.kind() {
        K::AddObject | K::RemoveObject | K::AddToken | K::RemoveToken => ConflictTarget::Object,
        K::MoveObject | K::ReorderObject => ConflictTarget::Position,
        _ => change
            .property_path()
            .map_or(ConflictTarget::Object, |path| {
                ConflictTarget::Property(path.clone())
            }),
    };
    (change.object_id(), target)
}

/// conflict 해결 action(XIV.12.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// base 상태.
    ChooseBase,
    /// Source A(ours) 상태.
    ChooseOurs,
    /// Source B(theirs) 상태.
    ChooseTheirs,
    /// 직접 넣는 속성 값.
    CustomValue(SemanticValue),
    /// 객체 또는 속성 삭제.
    Delete,
}

/// 해결 action 종류(안정 numeric code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ResolutionKind {
    /// [`Resolution::ChooseBase`].
    ChooseBase = 1,
    /// [`Resolution::ChooseOurs`].
    ChooseOurs = 2,
    /// [`Resolution::ChooseTheirs`].
    ChooseTheirs = 3,
    /// [`Resolution::CustomValue`].
    CustomValue = 4,
    /// [`Resolution::Delete`].
    Delete = 5,
}

impl ResolutionKind {
    /// 정의된 모든 종류(code 오름차순).
    pub const ALL: [ResolutionKind; 5] = [
        ResolutionKind::ChooseBase,
        ResolutionKind::ChooseOurs,
        ResolutionKind::ChooseTheirs,
        ResolutionKind::CustomValue,
        ResolutionKind::Delete,
    ];

    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            ResolutionKind::ChooseBase => "base",
            ResolutionKind::ChooseOurs => "ours",
            ResolutionKind::ChooseTheirs => "theirs",
            ResolutionKind::CustomValue => "custom",
            ResolutionKind::Delete => "delete",
        }
    }
}

impl Resolution {
    /// action 종류.
    pub fn kind(&self) -> ResolutionKind {
        match self {
            Resolution::ChooseBase => ResolutionKind::ChooseBase,
            Resolution::ChooseOurs => ResolutionKind::ChooseOurs,
            Resolution::ChooseTheirs => ResolutionKind::ChooseTheirs,
            Resolution::CustomValue(_) => ResolutionKind::CustomValue,
            Resolution::Delete => ResolutionKind::Delete,
        }
    }
}

/// conflict 하나에 허용되는 해결 action(code 오름차순, XIV.12.1).
///
/// | 대상 | side 선택 | `CustomValue` | `Delete` |
/// |---|---|---|---|
/// | 객체 | 세 side 모두(`Missing` = 삭제) | 없음 | 어느 side든 `Missing`일 때 |
/// | 위치 | 그 side가 위치 상태일 때 | 없음 | 없음 |
/// | 속성 | 그 side가 속성 상태일 때 | 있음 | 어느 side든 속성이 없거나 객체가 없을 때 |
pub fn allowed_resolutions(conflict: &SemanticConflict) -> Vec<ResolutionKind> {
    let sides = [
        (ResolutionKind::ChooseBase, &conflict.base),
        (ResolutionKind::ChooseOurs, &conflict.ours),
        (ResolutionKind::ChooseTheirs, &conflict.theirs),
    ];
    let mut out = Vec::new();
    for (kind, state) in sides {
        let applicable = match conflict.target {
            ConflictTarget::Object => true,
            ConflictTarget::Position => matches!(state, SideState::Position(_)),
            ConflictTarget::Property(_) => matches!(state, SideState::Value(_)),
        };
        if applicable {
            out.push(kind);
        }
    }
    let states = [&conflict.base, &conflict.ours, &conflict.theirs];
    match conflict.target {
        ConflictTarget::Object => {
            if states.iter().any(|s| matches!(s, SideState::Missing)) {
                out.push(ResolutionKind::Delete);
            }
        }
        ConflictTarget::Position => {}
        ConflictTarget::Property(_) => {
            out.push(ResolutionKind::CustomValue);
            if states
                .iter()
                .any(|s| matches!(s, SideState::Missing | SideState::Value(None)))
            {
                out.push(ResolutionKind::Delete);
            }
        }
    }
    out
}

/// 해결·선택 실패. 실패한 command는 C를 바꾸지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionError {
    /// 그 `(StableId, 대상)`의 conflict가 없음.
    UnknownConflict,
    /// conflict 대상·상태가 허용하지 않는 action.
    NotAllowed(ResolutionKind),
    /// 상태를 그 단위에 넣을 수 없음(예: 객체가 없는 쪽의 속성, 위치가 없는 객체).
    NotApplicable(&'static str),
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolutionError::UnknownConflict => f.write_str("해당 대상의 conflict가 없습니다"),
            ResolutionError::NotAllowed(kind) => {
                write!(f, "이 conflict에 허용하지 않는 해결입니다: {}", kind.name())
            }
            ResolutionError::NotApplicable(reason) => write!(f, "적용할 수 없는 상태: {reason}"),
        }
    }
}

impl std::error::Error for ResolutionError {}

/// C에 적용하는 command(XXIII.2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateCommand {
    /// conflict 하나를 해결합니다.
    Resolve {
        /// conflict 객체.
        object: StableId,
        /// conflict 대상.
        target: ConflictTarget,
        /// 해결 action.
        resolution: Resolution,
    },
    /// 임의 단위를 한 side의 상태로 둡니다. 그 단위가 conflict면 그 side 선택 해결과 같습니다.
    Select {
        /// 객체.
        object: StableId,
        /// 대상.
        target: ConflictTarget,
        /// 고를 side.
        side: Side,
    },
    /// 역 command: 단위 상태·출처·해결 기록을 되돌립니다(적용 순서대로).
    Restore(Restore),
}

/// [`CandidateCommand::Restore`]의 내용.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Restore {
    /// 되돌릴 단위 상태(앞에서부터 적용).
    pub units: Vec<(UnitKey, SideState)>,
    /// 되돌릴 출처(`None` = 기록 없음).
    pub sources: Vec<(UnitKey, Option<Source>)>,
    /// 되돌릴 해결 기록(`None` = 미해결).
    pub resolutions: Vec<(UnitKey, Option<Resolution>)>,
}

/// 적용 중인 C 사본과 역 command 기록.
struct Edit<'a> {
    candidate: &'a mut CandidateSnapshot,
    undo: Restore,
}

impl Edit<'_> {
    fn set_unit(&mut self, key: &UnitKey, state: SideState) -> Result<(), ResolutionError> {
        let previous = unit_state(&self.candidate.snapshot, key);
        put_unit(&mut self.candidate.snapshot, key, state)?;
        self.undo.units.push((key.clone(), previous));
        Ok(())
    }

    fn set_source(&mut self, key: &UnitKey, source: Option<Source>) {
        let previous = match source {
            Some(source) => self.candidate.provenance.insert(key.clone(), source),
            None => self.candidate.provenance.remove(key),
        };
        self.undo.sources.push((key.clone(), previous));
    }

    fn set_resolution(&mut self, key: &UnitKey, resolution: Option<Resolution>) {
        let previous = match resolution {
            Some(resolution) => self.candidate.resolutions.insert(key.clone(), resolution),
            None => self.candidate.resolutions.remove(key),
        };
        self.undo.resolutions.push((key.clone(), previous));
    }

    /// 단위 하나를 상태로 두고 출처를 기록합니다. 객체 단위면 그 객체의 위치·속성 출처 기록도 지웁니다(객체
    /// 전체가 한 출처).
    fn place(
        &mut self,
        key: &UnitKey,
        state: SideState,
        source: Source,
    ) -> Result<(), ResolutionError> {
        if key.1 == ConflictTarget::Object {
            let stale: Vec<UnitKey> = self
                .candidate
                .provenance
                .keys()
                .filter(|(id, target)| *id == key.0 && *target != ConflictTarget::Object)
                .cloned()
                .collect();
            for unit in stale {
                self.set_source(&unit, None);
            }
        }
        self.set_unit(key, state)?;
        self.set_source(key, Some(source));
        Ok(())
    }

    /// Property 삭제 쪽 해결(전용 종류 7·9) 뒤, C의 인스턴스에서 목록에 없는 Property를 가리키는 override를
    /// 지웁니다.
    fn release_overrides(
        &mut self,
        component: StableId,
        source: Source,
    ) -> Result<(), ResolutionError> {
        let snapshot = &self.candidate.snapshot;
        let Some(owner) = snapshot.get(component) else {
            return Ok(());
        };
        let listed: Vec<StableId> = match owner.property(&owner.path(&[keys::COMPONENT_PROPERTIES]))
        {
            Some(SemanticValue::Ids(ids)) => ids.clone(),
            _ => Vec::new(),
        };
        let reference = SemanticValue::Id(component);
        let mut released = Vec::new();
        for object in snapshot.objects() {
            if object.property(&object.path(&[keys::instance::COMPONENT_REF])) != Some(&reference) {
                continue;
            }
            for path in object.properties().keys() {
                let is_override = path.keys().first() == Some(&keys::instance::PROPERTY_OVERRIDES);
                if is_override
                    && override_member(path).is_some_and(|(_, member)| !listed.contains(&member))
                {
                    released.push((object.id(), ConflictTarget::Property(path.clone())));
                }
            }
        }
        for key in released {
            self.place(&key, SideState::Value(None), source)?;
        }
        Ok(())
    }
}

/// 단위에 상태를 넣습니다.
fn put_unit(
    snapshot: &mut SemanticSnapshot,
    (object, target): &UnitKey,
    state: SideState,
) -> Result<(), ResolutionError> {
    match (target, state) {
        (ConflictTarget::Object, SideState::Missing) => {
            snapshot.remove(*object);
            Ok(())
        }
        (ConflictTarget::Object, SideState::Object(value)) if value.id() == *object => {
            snapshot.remove(*object);
            snapshot
                .insert(*value)
                .map_err(|_| ResolutionError::NotApplicable("객체를 넣을 수 없음"))
        }
        (ConflictTarget::Position, SideState::Position(position)) => {
            let mut found = snapshot
                .remove(*object)
                .ok_or(ResolutionError::NotApplicable("위치를 바꿀 객체가 없음"))?;
            found.set_position(position.parent, position.order_key);
            snapshot
                .insert(found)
                .map_err(|_| ResolutionError::NotApplicable("객체를 넣을 수 없음"))
        }
        (ConflictTarget::Property(path), SideState::Value(value)) => {
            let mut found = snapshot
                .remove(*object)
                .ok_or(ResolutionError::NotApplicable("속성을 바꿀 객체가 없음"))?;
            let outcome = match value {
                Some(value) => found
                    .set_property(path.clone(), value)
                    .map(|_| ())
                    .map_err(|_| ResolutionError::NotApplicable("속성 경로가 객체와 맞지 않음")),
                None => {
                    found.remove_property(path);
                    Ok(())
                }
            };
            snapshot
                .insert(found)
                .map_err(|_| ResolutionError::NotApplicable("객체를 넣을 수 없음"))?;
            outcome
        }
        (ConflictTarget::Position, SideState::Missing)
        | (ConflictTarget::Property(_), SideState::Missing) => {
            Err(ResolutionError::NotApplicable("그 side에는 객체가 없음"))
        }
        _ => Err(ResolutionError::NotApplicable("대상과 상태 종류가 다름")),
    }
}

/// 쓰기를 막는 이유(XIV.12.3, XXIII.2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateBlocked {
    /// 해결하지 않은 conflict 단위(순서대로).
    pub unresolved: Vec<UnitKey>,
    /// C에 새로 생긴 snapshot 위반.
    pub violations: BTreeSet<SnapshotViolation>,
}

impl fmt::Display for CandidateBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "해결하지 않은 conflict {}개, 새 snapshot 위반 {}개",
            self.unresolved.len(),
            self.violations.len()
        )
    }
}

impl std::error::Error for CandidateBlocked {}

/// 병합 결과 candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSnapshot {
    input: MergeInput,
    snapshot: SemanticSnapshot,
    conflicts: Vec<SemanticConflict>,
    provenance: BTreeMap<UnitKey, Source>,
    resolutions: BTreeMap<UnitKey, Resolution>,
}

impl CandidateSnapshot {
    /// 입력을 병합해 C를 만듭니다(XXIII.2.1).
    pub fn build(input: MergeInput) -> Result<CandidateSnapshot, MergeError> {
        let result = merge(&input)?;
        Ok(CandidateSnapshot::from_result(input, result))
    }

    /// 이미 계산한 병합 결과로 C를 만듭니다. `result`는 `input`의 [`merge`] 결과여야 합니다.
    pub fn from_result(input: MergeInput, result: MergeResult) -> CandidateSnapshot {
        let provenance = initial_provenance(&input, &result);
        CandidateSnapshot {
            input,
            snapshot: result.merged,
            conflicts: result.conflicts,
            provenance,
            resolutions: BTreeMap::new(),
        }
    }

    /// 병합 입력(base·A·B). C를 바꿔도 그대로입니다.
    pub fn input(&self) -> &MergeInput {
        &self.input
    }

    /// 현재 candidate snapshot.
    pub fn snapshot(&self) -> &SemanticSnapshot {
        &self.snapshot
    }

    /// 엔진이 남긴 conflict(`(StableId, 대상)` 순서).
    pub fn conflicts(&self) -> &[SemanticConflict] {
        &self.conflicts
    }

    /// 아직 해결하지 않은 conflict 수.
    pub fn unresolved_count(&self) -> usize {
        self.conflicts
            .iter()
            .filter(|c| !self.resolutions.contains_key(&(c.object, c.target.clone())))
            .count()
    }

    /// 단위의 출처(기록이 없으면 base 그대로).
    pub fn provenance(&self, object: StableId, target: &ConflictTarget) -> Source {
        self.provenance
            .get(&(object, target.clone()))
            .copied()
            .unwrap_or(Source::Base)
    }

    /// 기록한 출처(단위 순서).
    pub fn provenance_entries(&self) -> impl Iterator<Item = (&UnitKey, Source)> + '_ {
        self.provenance.iter().map(|(key, source)| (key, *source))
    }

    /// conflict의 해결 기록(미해결이면 `None`).
    pub fn resolution(&self, object: StableId, target: &ConflictTarget) -> Option<&Resolution> {
        self.resolutions.get(&(object, target.clone()))
    }

    /// command를 적용하고 역 command를 돌려줍니다(XXIII.2.3). 실패하면 C는 그대로입니다.
    pub fn apply(
        &mut self,
        command: &CandidateCommand,
    ) -> Result<CandidateCommand, ResolutionError> {
        let mut next = self.clone();
        let mut edit = Edit {
            candidate: &mut next,
            undo: Restore::default(),
        };
        match command {
            CandidateCommand::Resolve {
                object,
                target,
                resolution,
            } => resolve(&mut edit, &(*object, target.clone()), resolution)?,
            CandidateCommand::Select {
                object,
                target,
                side,
            } => {
                let key = (*object, target.clone());
                let is_conflict = edit
                    .candidate
                    .conflicts
                    .iter()
                    .any(|c| c.object == *object && &c.target == target);
                if is_conflict {
                    let resolution = match side {
                        Side::Base => Resolution::ChooseBase,
                        Side::Ours => Resolution::ChooseOurs,
                        Side::Theirs => Resolution::ChooseTheirs,
                    };
                    resolve(&mut edit, &key, &resolution)?;
                } else {
                    let state = unit_state(side_snapshot(&edit.candidate.input, *side), &key);
                    edit.place(&key, state, side.source())?;
                }
            }
            CandidateCommand::Restore(restore) => {
                for (key, state) in &restore.units {
                    edit.set_unit(key, state.clone())?;
                }
                for (key, source) in &restore.sources {
                    edit.set_source(key, *source);
                }
                for (key, resolution) in &restore.resolutions {
                    edit.set_resolution(key, resolution.clone());
                }
            }
        }
        let mut undo = edit.undo;
        undo.units.reverse();
        undo.sources.reverse();
        undo.resolutions.reverse();
        *self = next;
        Ok(CandidateCommand::Restore(undo))
    }

    /// 쓰기 조건 검사(XIV.12.3, XXIII.2.4): unresolved conflict 0과 새 snapshot 위반 0.
    pub fn validate(&self) -> Result<(), CandidateBlocked> {
        let unresolved: Vec<UnitKey> = self
            .conflicts
            .iter()
            .map(|c| (c.object, c.target.clone()))
            .filter(|key| !self.resolutions.contains_key(key))
            .collect();
        let input = &self.input;
        let violations =
            introduced_violations(&self.snapshot, &[&input.base, &input.ours, &input.theirs]);
        if unresolved.is_empty() && violations.is_empty() {
            Ok(())
        } else {
            Err(CandidateBlocked {
                unresolved,
                violations,
            })
        }
    }
}

/// 초기 출처: base와 다른 단위는 A·B 상태 비교, conflict 단위는 base.
fn initial_provenance(input: &MergeInput, result: &MergeResult) -> BTreeMap<UnitKey, Source> {
    let mut out = BTreeMap::new();
    for change in diff(&input.base, &result.merged).iter() {
        let key = unit_of(change);
        let state = unit_state(&result.merged, &key);
        let a = state == unit_state(&input.ours, &key);
        let b = state == unit_state(&input.theirs, &key);
        let source = match (a, b) {
            (true, false) => Source::A,
            (false, true) => Source::B,
            _ => Source::Auto,
        };
        out.insert(key, source);
    }
    for conflict in &result.conflicts {
        out.entry((conflict.object, conflict.target.clone()))
            .or_insert(Source::Base);
    }
    out
}

fn side_snapshot(input: &MergeInput, side: Side) -> &SemanticSnapshot {
    match side {
        Side::Base => &input.base,
        Side::Ours => &input.ours,
        Side::Theirs => &input.theirs,
    }
}

/// conflict 하나를 해결합니다(XIV.12.1·XIV.12.2).
fn resolve(
    edit: &mut Edit<'_>,
    key: &UnitKey,
    resolution: &Resolution,
) -> Result<(), ResolutionError> {
    let conflict = edit
        .candidate
        .conflicts
        .iter()
        .find(|c| c.object == key.0 && c.target == key.1)
        .cloned()
        .ok_or(ResolutionError::UnknownConflict)?;
    if !allowed_resolutions(&conflict).contains(&resolution.kind()) {
        return Err(ResolutionError::NotAllowed(resolution.kind()));
    }
    let sides = [
        (Source::Base, &conflict.base),
        (Source::A, &conflict.ours),
        (Source::B, &conflict.theirs),
    ];
    let (state, source) = match resolution {
        Resolution::ChooseBase => (conflict.base.clone(), Source::Base),
        Resolution::ChooseOurs => (conflict.ours.clone(), Source::A),
        Resolution::ChooseTheirs => (conflict.theirs.clone(), Source::B),
        Resolution::CustomValue(value) => (SideState::Value(Some(value.clone())), Source::Custom),
        Resolution::Delete => {
            let state = match key.1 {
                ConflictTarget::Object => SideState::Missing,
                _ => SideState::Value(None),
            };
            let source = sides
                .iter()
                .find(|(_, side)| **side == state)
                .map_or(Source::Custom, |(source, _)| *source);
            (state, source)
        }
    };
    edit.place(key, state, source)?;
    if matches!(
        conflict.kind,
        ConflictKind::ComponentPropertyRemovedVsOverride | ConflictKind::DrillingChainBreak
    ) {
        edit.release_overrides(key.0, source)?;
    }
    edit.set_resolution(key, Some(resolution.clone()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::tests::{base, edit, set};
    use crate::model::tests::text;
    use stellar_semantic_diff::keys;

    #[test]
    fn candidate_is_independent_of_sources() {
        let base = base();
        let ours = edit(&base, 11, |o| set(o, keys::DISPLAY_NAME, text("ours")));
        let theirs = edit(&base, 12, |o| set(o, keys::DISPLAY_NAME, text("theirs")));
        let (before_a, before_b) = (ours.clone(), theirs.clone());
        let candidate = CandidateSnapshot::build(MergeInput {
            base: base.clone(),
            ours: ours.clone(),
            theirs: theirs.clone(),
        })
        .unwrap();
        assert_eq!(candidate.unresolved_count(), 0);
        assert_eq!((&ours, &theirs), (&before_a, &before_b));
        assert_eq!(candidate.input().ours, before_a);
        assert_ne!(candidate.snapshot(), &before_a);
        assert_ne!(candidate.snapshot(), &before_b);
    }
}
