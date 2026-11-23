//! 에이전트 편집 lease와 판정 (XIX.4.2~XIX.4.4, XIX.4.6, XIX.5.2, ADR-0093).
//!
//! lease 하나는 에이전트 편집 세션 하나(`AgentProvenance::edit_session`)가 쥔 잠금 묶음입니다.
//!
//! - **subtree 잠금**([`SubtreeLock`]): 대상 root node와 그 모든 자손입니다. 자손은 lease를 만들 때 목록으로
//!   적지 않고 판정할 때 조상을 따라 올라가 찾으므로, 나중에 추가된 자손도 같은 lease에 속합니다(XIX.4.2).
//! - **외부 resource 잠금**([`ExternalResource`]): token, component definition, asset은 문서 node가 아니므로
//!   따로 요청합니다. instance node를 잠가도 그 정의를 자동으로 잠그지 않습니다(XIX.4.4).
//! - **겹침**: 같은 문서에서 root가 같거나 한쪽 root가 다른 쪽의 조상이면 겹칩니다(XIX.4.6). 조상 목록은
//!   잠금을 만들 때의 문서로 [`SubtreeLock::capture`]가 적어 두므로, 문서 모델이 없는 다른 프로세스도 같은
//!   판정을 합니다([`super::coordinator`]).
//!
//! 판정 규칙은 [`LeaseTable::judge`] 하나입니다. 사용자 편집 거부(XIX.4.3)와 에이전트 proposal 범위 밖 거부
//! (XIX.5.2)가 같은 함수를 쓰며, 둘 다 transaction 실행 결과의 [`ChangeSet`](바뀐·추가·삭제 node, 속성 경로)을
//! 봅니다. command 종류별로 대상을 따로 계산하지 않습니다.
//!
//! - 사용자([`Actor::User`]): 바뀐 node가 적용 전이나 적용 후 어느 lease에 속하면 거부합니다. lease root의
//!   조상을 옮기거나(부모·순서 변경) 삭제하는 것도 거부합니다. 조회·선택은 command가 아니므로 막지 않습니다.
//! - 에이전트([`Actor::Agent`]): 바뀐 node는 적용 전과 후 모두(존재하는 쪽) 자기 lease에 속해야 합니다. 자기
//!   lease root를 지우는 것도 root의 부모(lease 밖)를 바꾸므로 거부합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;

use crate::change::{node_keys, ChangeSet};
use crate::id::CommandOrigin;
use crate::state::DocumentState;
use crate::transaction::{AppliedTransaction, CommandTransaction, TransactionError};
use crate::tree::parent_of;

/// lease 식별자. 에이전트 편집 세션 id(`AgentProvenance::edit_session`)와 같은 값입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseId(pub StableId);

impl fmt::Display for LeaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// 문서 밖 전역 resource 종류(XIX.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExternalKind {
    /// design token.
    Token,
    /// component definition(`design/components` 원본).
    ComponentDefinition,
    /// asset(이미지·글꼴 원본).
    Asset,
}

impl ExternalKind {
    /// 모든 종류(고정 순서).
    pub const ALL: [ExternalKind; 3] = [
        ExternalKind::Token,
        ExternalKind::ComponentDefinition,
        ExternalKind::Asset,
    ];

    /// 고정 이름(tool 인자·잠금 파일에 씀).
    pub fn name(self) -> &'static str {
        match self {
            ExternalKind::Token => "token",
            ExternalKind::ComponentDefinition => "component-definition",
            ExternalKind::Asset => "asset",
        }
    }

    /// 이름으로 찾습니다.
    pub fn from_name(name: &str) -> Option<ExternalKind> {
        ExternalKind::ALL
            .into_iter()
            .find(|kind| kind.name() == name)
    }
}

/// 전역 resource 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalResource {
    /// 종류.
    pub kind: ExternalKind,
    /// resource id.
    pub id: StableId,
}

/// 문서 subtree 하나의 잠금.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtreeLock {
    /// 문서.
    pub document: StableId,
    /// 잠근 root node.
    pub root: StableId,
    /// 잠금을 만들 때 root의 조상(부모부터 View root까지). 겹침 판정에만 씁니다.
    pub ancestors: Vec<StableId>,
}

impl SubtreeLock {
    /// 문서에 있는 `root`의 잠금을 만듭니다. 없으면 `None`입니다.
    pub fn capture(model: &DocumentModel, root: StableId) -> Option<SubtreeLock> {
        parent_of(model, root)?;
        let mut ancestors = Vec::new();
        let mut current = root;
        let mut remaining = model.node_count();
        while let Some(Some(parent)) = parent_of(model, current) {
            if remaining == 0 {
                break;
            }
            remaining -= 1;
            ancestors.push(parent);
            current = parent;
        }
        Some(SubtreeLock {
            document: model.document_id(),
            root,
            ancestors,
        })
    }

    /// 두 subtree가 겹치는지(같은 root, 또는 한쪽 root가 다른 쪽의 조상, XIX.4.6).
    pub fn overlaps(&self, other: &SubtreeLock) -> bool {
        self.document == other.document
            && (self.root == other.root
                || self.ancestors.contains(&other.root)
                || other.ancestors.contains(&self.root))
    }
}

/// lease 하나: subtree 잠금과 전역 resource 잠금.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// 식별자.
    pub id: LeaseId,
    /// 문서 subtree 잠금.
    pub subtrees: Vec<SubtreeLock>,
    /// 전역 resource 잠금.
    pub externals: Vec<ExternalResource>,
}

impl Lease {
    /// `other`와 겹치는 첫 대상. 자기 대상끼리의 겹침은 보지 않습니다.
    pub fn conflict_with(&self, other: &Lease) -> Option<Conflict> {
        for mine in &self.subtrees {
            if let Some(theirs) = other.subtrees.iter().find(|s| s.overlaps(mine)) {
                return Some(Conflict::Subtree {
                    holder: other.id,
                    requested: mine.root,
                    held: theirs.root,
                });
            }
        }
        self.externals
            .iter()
            .find(|resource| other.externals.contains(resource))
            .map(|resource| Conflict::External {
                holder: other.id,
                resource: *resource,
            })
    }

    /// 대상 목록 자체가 올바른지: 대상이 하나 이상이고, 자기 subtree끼리 겹치지 않으며, 전역 resource가
    /// 중복되지 않습니다.
    pub fn check_shape(&self) -> Result<(), LeaseShapeError> {
        if self.subtrees.is_empty() && self.externals.is_empty() {
            return Err(LeaseShapeError::Empty);
        }
        for (index, subtree) in self.subtrees.iter().enumerate() {
            if let Some(other) = self.subtrees[..index]
                .iter()
                .find(|other| other.overlaps(subtree))
            {
                return Err(LeaseShapeError::SelfOverlap {
                    first: other.root,
                    second: subtree.root,
                });
            }
        }
        let mut seen = BTreeSet::new();
        for resource in &self.externals {
            if !seen.insert(*resource) {
                return Err(LeaseShapeError::DuplicateExternal(*resource));
            }
        }
        Ok(())
    }
}

/// 대상 목록 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseShapeError {
    /// 대상이 없습니다.
    Empty,
    /// 같은 요청 안의 두 subtree가 겹칩니다.
    SelfOverlap {
        /// 앞 root.
        first: StableId,
        /// 뒤 root.
        second: StableId,
    },
    /// 같은 전역 resource를 두 번 요청했습니다.
    DuplicateExternal(ExternalResource),
}

impl fmt::Display for LeaseShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LeaseShapeError::Empty => f.write_str("잠글 대상이 없습니다"),
            LeaseShapeError::SelfOverlap { first, second } => {
                write!(f, "요청 대상 {first}와 {second}가 겹칩니다")
            }
            LeaseShapeError::DuplicateExternal(resource) => write!(
                f,
                "전역 resource {} {}를 두 번 요청했습니다",
                resource.kind.name(),
                resource.id
            ),
        }
    }
}

impl std::error::Error for LeaseShapeError {}

/// 잠금 획득을 막는 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// 다른 lease의 subtree와 겹칩니다.
    Subtree {
        /// 겹친 lease.
        holder: LeaseId,
        /// 요청 root.
        requested: StableId,
        /// 이미 잠긴 root.
        held: StableId,
    },
    /// 다른 lease가 같은 전역 resource를 쥐고 있습니다.
    External {
        /// 쥔 lease.
        holder: LeaseId,
        /// resource.
        resource: ExternalResource,
    },
    /// 사용자 상호작용 미리보기(gesture)가 대상과 겹칩니다. gesture가 끝난 뒤 다시 시도합니다(XIX.4.7).
    Gesture {
        /// gesture id.
        gesture: StableId,
        /// 요청 root.
        requested: StableId,
    },
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Conflict::Subtree {
                holder,
                requested,
                held,
            } => write!(
                f,
                "대상 {requested}가 lease {holder}의 잠금 {held}와 겹칩니다"
            ),
            Conflict::External { holder, resource } => write!(
                f,
                "전역 resource {} {}는 lease {holder}가 잠갔습니다",
                resource.kind.name(),
                resource.id
            ),
            Conflict::Gesture { gesture, requested } => write!(
                f,
                "대상 {requested}는 진행 중인 사용자 조작 {gesture}가 끝난 뒤 잠글 수 있습니다"
            ),
        }
    }
}

/// 편집 주체.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// 사용자 편집. 어느 lease 안도 바꿀 수 없습니다.
    User,
    /// 에이전트 편집. 자기 lease 안만 바꿀 수 있습니다.
    Agent(LeaseId),
}

impl Actor {
    /// transaction 출처의 판정 주체. 시스템(외부 변경 반영)·migration 편집은 lease 판정 대상이 아니라
    /// `None`입니다. 외부 변경이 lease와 겹치면 거부하지 않고 세션을 끝냅니다(ADR-0005 §4.2,
    /// [`LeaseTable::touched_leases`]).
    pub fn of(origin: CommandOrigin) -> Option<Actor> {
        match origin {
            CommandOrigin::User => Some(Actor::User),
            CommandOrigin::FutureAgent(provenance) => {
                Some(Actor::Agent(LeaseId(provenance.edit_session())))
            }
            CommandOrigin::System | CommandOrigin::Migration => None,
        }
    }
}

/// lease 판정 위반.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseViolation {
    /// 위반한 node(전역 resource면 resource id).
    pub node: StableId,
    /// 이유.
    pub reason: ViolationReason,
}

/// 위반 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationReason {
    /// 다른 lease가 잠근 subtree를 바꿉니다.
    Leased(LeaseId),
    /// lease root의 조상을 옮기거나 지웁니다.
    AncestorOfLease(LeaseId),
    /// 에이전트가 자기 lease 밖을 바꿉니다(XIX.5.2).
    OutsideLease,
    /// 에이전트가 자기 lease root를 지웁니다.
    RemovesLeaseRoot,
    /// 판정 주체의 lease가 없습니다(끝났거나 승인 전).
    UnknownLease(LeaseId),
    /// 잠그지 않은 전역 resource를 바꿉니다(XIX.4.4).
    ExternalNotLocked(ExternalKind),
    /// 다른 lease가 잠근 전역 resource를 바꿉니다.
    ExternalLeased(LeaseId),
}

impl fmt::Display for LeaseViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let node = self.node;
        match self.reason {
            ViolationReason::Leased(lease) => {
                write!(f, "node {node}는 에이전트 lease {lease}가 잠갔습니다")
            }
            ViolationReason::AncestorOfLease(lease) => write!(
                f,
                "node {node}는 lease {lease}가 잠근 subtree의 조상이라 옮기거나 지울 수 없습니다"
            ),
            ViolationReason::OutsideLease => write!(f, "node {node}는 이 lease 범위 밖입니다"),
            ViolationReason::RemovesLeaseRoot => {
                write!(f, "lease root {node}는 lease 안에서 지울 수 없습니다")
            }
            ViolationReason::UnknownLease(lease) => write!(f, "lease {lease}가 없습니다"),
            ViolationReason::ExternalNotLocked(kind) => write!(
                f,
                "전역 resource {} {node}는 따로 잠그지 않았습니다",
                kind.name()
            ),
            ViolationReason::ExternalLeased(lease) => {
                write!(f, "전역 resource {node}는 lease {lease}가 잠갔습니다")
            }
        }
    }
}

impl std::error::Error for LeaseViolation {}

/// 활성 lease 목록.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeaseTable {
    leases: BTreeMap<LeaseId, Lease>,
}

/// 적용 전 문서의 lease 소속(판정 입력). 비용은 잠긴 subtree 크기에 비례합니다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeaseSnapshot {
    /// 잠긴 subtree의 node → lease.
    covered: BTreeMap<StableId, LeaseId>,
    /// lease root의 진조상 → lease.
    ancestors: BTreeMap<StableId, LeaseId>,
}

impl LeaseTable {
    /// 빈 목록.
    pub fn new() -> LeaseTable {
        LeaseTable::default()
    }

    /// lease가 없는지.
    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }

    /// lease 수.
    pub fn len(&self) -> usize {
        self.leases.len()
    }

    /// lease 하나.
    pub fn get(&self, id: LeaseId) -> Option<&Lease> {
        self.leases.get(&id)
    }

    /// 모든 lease(id 순).
    pub fn iter(&self) -> impl Iterator<Item = &Lease> {
        self.leases.values()
    }

    /// `candidate`와 겹치는 첫 기존 lease 대상(XIX.4.6). 같은 id의 lease는 보지 않습니다.
    pub fn first_conflict(&self, candidate: &Lease) -> Option<Conflict> {
        self.leases
            .values()
            .filter(|lease| lease.id != candidate.id)
            .find_map(|lease| candidate.conflict_with(lease))
    }

    /// lease를 더합니다. 대상 목록이 틀렸거나 다른 lease와 겹치면 더하지 않습니다(XIX.4.6).
    pub fn insert(&mut self, lease: Lease) -> Result<(), InsertError> {
        lease.check_shape().map_err(InsertError::Shape)?;
        if self.leases.contains_key(&lease.id) {
            return Err(InsertError::DuplicateId(lease.id));
        }
        if let Some(conflict) = self.first_conflict(&lease) {
            return Err(InsertError::Conflict(conflict));
        }
        self.leases.insert(lease.id, lease);
        Ok(())
    }

    /// lease를 지웁니다(완료·세션 종료).
    pub fn remove(&mut self, id: LeaseId) -> Option<Lease> {
        self.leases.remove(&id)
    }

    /// `document`의 lease root → lease.
    fn roots(&self, document: StableId) -> BTreeMap<StableId, LeaseId> {
        self.leases
            .values()
            .flat_map(|lease| {
                lease
                    .subtrees
                    .iter()
                    .filter(move |s| s.document == document)
                    .map(move |s| (s.root, lease.id))
            })
            .collect()
    }

    /// `document`에 lease가 있는지.
    pub fn covers_document(&self, document: StableId) -> bool {
        self.leases
            .values()
            .any(|lease| lease.subtrees.iter().any(|s| s.document == document))
    }

    /// 문서 node가 속한 lease. 자신이나 조상이 lease root이면 그 lease입니다. 나중에 추가된 자손도 조상을 따라
    /// 같은 lease에 속합니다(XIX.4.2). 비용은 조상 수에 비례합니다.
    pub fn holder(&self, model: &DocumentModel, node: StableId) -> Option<LeaseId> {
        holder_in(&self.roots(model.document_id()), model, node)
    }

    /// 전역 resource를 잠근 lease.
    pub fn external_holder(&self, resource: ExternalResource) -> Option<LeaseId> {
        self.leases
            .values()
            .find(|lease| lease.externals.contains(&resource))
            .map(|lease| lease.id)
    }

    /// 전역 resource 편집 판정(XIX.4.4). 사용자는 아무 lease도 쥐지 않은 resource만, 에이전트는 자기 lease가
    /// 명시적으로 잠근 resource만 바꿀 수 있습니다. node 잠금에서 자동으로 이어지지 않습니다.
    pub fn check_external(
        &self,
        resource: ExternalResource,
        actor: Actor,
    ) -> Result<(), LeaseViolation> {
        let violation = |reason| LeaseViolation {
            node: resource.id,
            reason,
        };
        let holder = self.external_holder(resource);
        match actor {
            Actor::User => match holder {
                Some(lease) => Err(violation(ViolationReason::ExternalLeased(lease))),
                None => Ok(()),
            },
            Actor::Agent(me) => {
                if !self.leases.contains_key(&me) {
                    return Err(violation(ViolationReason::UnknownLease(me)));
                }
                match holder {
                    Some(lease) if lease == me => Ok(()),
                    Some(lease) => Err(violation(ViolationReason::ExternalLeased(lease))),
                    None => Err(violation(ViolationReason::ExternalNotLocked(resource.kind))),
                }
            }
        }
    }

    /// 적용 전 문서의 lease 소속을 기록합니다.
    pub fn snapshot(&self, model: &DocumentModel) -> LeaseSnapshot {
        let mut snapshot = LeaseSnapshot::default();
        for (root, lease) in self.roots(model.document_id()) {
            if !model.contains(root) {
                continue;
            }
            let mut pending = vec![root];
            while let Some(node) = pending.pop() {
                if snapshot.covered.insert(node, lease).is_none() {
                    pending.extend(model.children_in_order(node));
                }
            }
            for ancestor in proper_ancestors(model, root) {
                snapshot.ancestors.insert(ancestor, lease);
            }
        }
        snapshot
    }

    /// transaction 결과를 판정합니다(모듈 문서 규칙). `before`는 같은 문서의 적용 전 [`LeaseTable::snapshot`],
    /// `after`는 적용 후 모델입니다.
    pub fn judge(
        &self,
        before: &LeaseSnapshot,
        after: &DocumentModel,
        changes: &ChangeSet,
        actor: Actor,
    ) -> Result<(), LeaseViolation> {
        if let Actor::Agent(me) = actor {
            if !self.leases.contains_key(&me) {
                return Err(LeaseViolation {
                    node: StableId::NIL,
                    reason: ViolationReason::UnknownLease(me),
                });
            }
        }
        let roots = self.roots(after.document_id());
        if roots.is_empty() && actor == Actor::User {
            return Ok(());
        }
        for node in changes.removed_nodes() {
            if let Actor::Agent(me) = actor {
                if roots.get(&node) == Some(&me) {
                    return Err(LeaseViolation {
                        node,
                        reason: ViolationReason::RemovesLeaseRoot,
                    });
                }
            }
            check_node(node, actor, Some(before.covered.get(&node).copied()), None)?;
        }
        let inserted: BTreeSet<StableId> = changes.inserted_nodes().into_iter().collect();
        for node in changes.changed_nodes() {
            let existed = (!inserted.contains(&node)).then(|| before.covered.get(&node).copied());
            let exists = after.contains(node).then(|| holder_in(&roots, after, node));
            check_node(node, actor, existed, exists)?;
        }
        if actor == Actor::User {
            let mut after_ancestors: Option<BTreeMap<StableId, LeaseId>> = None;
            for path in changes.changed_properties() {
                let positioned =
                    path.keys() == [node_keys::PARENT] || path.keys() == [node_keys::ORDER_KEY];
                if !positioned {
                    continue;
                }
                let node = path.root();
                let ancestors = after_ancestors.get_or_insert_with(|| {
                    let mut map = BTreeMap::new();
                    for (root, lease) in &roots {
                        for ancestor in proper_ancestors(after, *root) {
                            map.insert(ancestor, *lease);
                        }
                    }
                    map
                });
                if let Some(lease) = before.ancestors.get(&node).or_else(|| ancestors.get(&node)) {
                    return Err(LeaseViolation {
                        node,
                        reason: ViolationReason::AncestorOfLease(*lease),
                    });
                }
            }
        }
        Ok(())
    }

    /// 변경이 닿은 lease(적용 전 또는 후 lease 안의 node를 바꿈). 외부 변경 반영(시스템 출처)이 겹친 세션을
    /// 끝낼 때 씁니다(ADR-0005 §4.2). 판정과 같은 소속 계산을 씁니다.
    pub fn touched_leases(
        &self,
        before: &LeaseSnapshot,
        after: &DocumentModel,
        changes: &ChangeSet,
    ) -> BTreeSet<LeaseId> {
        let roots = self.roots(after.document_id());
        let mut out = BTreeSet::new();
        for node in changes
            .changed_nodes()
            .into_iter()
            .chain(changes.removed_nodes())
        {
            out.extend(before.covered.get(&node).copied());
            if after.contains(node) {
                out.extend(holder_in(&roots, after, node));
            }
        }
        out
    }
}

/// [`LeaseTable::insert`] 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertError {
    /// 대상 목록이 틀렸습니다.
    Shape(LeaseShapeError),
    /// 같은 id의 lease가 이미 있습니다.
    DuplicateId(LeaseId),
    /// 다른 lease와 겹칩니다.
    Conflict(Conflict),
}

impl fmt::Display for InsertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InsertError::Shape(e) => write!(f, "{e}"),
            InsertError::DuplicateId(id) => write!(f, "lease {id}가 이미 있습니다"),
            InsertError::Conflict(c) => write!(f, "{c}"),
        }
    }
}

impl std::error::Error for InsertError {}

/// node 하나의 적용 전(`existed`)·후(`exists`) 소속 판정. 바깥 `None`은 그 시점에 node가 없음입니다.
fn check_node(
    node: StableId,
    actor: Actor,
    existed: Option<Option<LeaseId>>,
    exists: Option<Option<LeaseId>>,
) -> Result<(), LeaseViolation> {
    for holder in [existed, exists].into_iter().flatten() {
        let reason = match (actor, holder) {
            (Actor::User, None) => continue,
            (Actor::User, Some(lease)) => ViolationReason::Leased(lease),
            (Actor::Agent(me), Some(lease)) if lease == me => continue,
            (Actor::Agent(_), Some(lease)) => ViolationReason::Leased(lease),
            (Actor::Agent(_), None) => ViolationReason::OutsideLease,
        };
        return Err(LeaseViolation { node, reason });
    }
    Ok(())
}

fn holder_in(
    roots: &BTreeMap<StableId, LeaseId>,
    model: &DocumentModel,
    node: StableId,
) -> Option<LeaseId> {
    if roots.is_empty() {
        return None;
    }
    let mut current = node;
    let mut remaining = model.node_count();
    loop {
        if let Some(lease) = roots.get(&current) {
            return Some(*lease);
        }
        match parent_of(model, current) {
            Some(Some(parent)) if remaining > 0 => {
                remaining -= 1;
                current = parent;
            }
            _ => return None,
        }
    }
}

fn proper_ancestors(model: &DocumentModel, node: StableId) -> Vec<StableId> {
    SubtreeLock::capture(model, node).map_or_else(Vec::new, |lock| lock.ancestors)
}

/// lease 판정을 거친 transaction 실행 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum LeasedExecutionError {
    /// command validator가 거부했습니다(문서는 실행 전 상태).
    Transaction(TransactionError),
    /// lease 판정이 거부했습니다(문서와 revision은 실행 전 상태).
    Lease(LeaseViolation),
    /// lease 판정이 거부한 뒤 되돌리기가 실패했습니다(내부 불일치 신호).
    Restore {
        /// 원래 위반.
        violation: LeaseViolation,
        /// 되돌리기 실패.
        error: TransactionError,
    },
}

impl fmt::Display for LeasedExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LeasedExecutionError::Transaction(e) => write!(f, "{e}"),
            LeasedExecutionError::Lease(v) => write!(f, "{v}"),
            LeasedExecutionError::Restore { violation, error } => {
                write!(f, "{violation}; 되돌리기 실패: {error}")
            }
        }
    }
}

impl std::error::Error for LeasedExecutionError {}

/// lease 판정을 거쳐 transaction을 실행합니다(XIX.4.3 user command validator, XIX.5.2 범위 검사).
///
/// 판정 주체는 transaction 출처입니다([`Actor::of`]). 시스템·migration 출처이거나, 사용자 편집인데 이 문서에
/// lease가 없으면 판정 없이 [`DocumentState::execute_transaction`]과 같습니다. 판정할 때는 같은 실행 경로로
/// 적용한 뒤 결과 [`ChangeSet`]을 [`LeaseTable::judge`]로 보고, 거부하면 역 transaction으로 되돌리고
/// revision을 실행 전 값으로 돌립니다. 거부된 실행은 밖에서 관찰되지 않으므로(change set·역 transaction을
/// 내보내지 않음) 문서는 실행 전과 같은 의미 상태·revision입니다.
pub fn execute_leased(
    state: &mut DocumentState,
    transaction: &CommandTransaction,
    table: &LeaseTable,
) -> Result<AppliedTransaction, LeasedExecutionError> {
    let Some(actor) = Actor::of(transaction.origin) else {
        return state
            .execute_transaction(transaction)
            .map_err(LeasedExecutionError::Transaction);
    };
    if actor == Actor::User && !table.covers_document(state.model().document_id()) {
        return state
            .execute_transaction(transaction)
            .map_err(LeasedExecutionError::Transaction);
    }
    let before = table.snapshot(state.model());
    let start = state.revision;
    let applied = state
        .execute_transaction(transaction)
        .map_err(LeasedExecutionError::Transaction)?;
    match table.judge(&before, state.model(), &applied.changes, actor) {
        Ok(()) => Ok(applied),
        Err(violation) => {
            let restored = state.execute_transaction(&applied.inverse);
            state.revision = start;
            match restored {
                Ok(_) => Err(LeasedExecutionError::Lease(violation)),
                Err(error) => Err(LeasedExecutionError::Restore { violation, error }),
            }
        }
    }
}

/// 승인 전 미리보기 결과(XIX.5.3).
#[derive(Debug, Clone, PartialEq)]
pub struct LeasePreview {
    /// 적용 전 문서 레코드(canonical 순).
    pub before: stellar_document::records::DocumentRecords,
    /// 적용했을 때의 문서 레코드(canonical 순).
    pub after: stellar_document::records::DocumentRecords,
    /// 적용했을 때의 change set.
    pub changes: ChangeSet,
    /// lease 판정. 위반이어도 미리보기는 만들어 보여 줍니다.
    pub scope: Result<(), LeaseViolation>,
}

/// 미리보기 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewError {
    /// 문서 사본을 만들 수 없습니다.
    Copy(String),
    /// command validator가 거부했습니다.
    Transaction(TransactionError),
}

impl fmt::Display for PreviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreviewError::Copy(e) => write!(f, "문서 사본을 만들 수 없습니다: {e}"),
            PreviewError::Transaction(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PreviewError {}

/// transaction을 문서 사본에 적용해 결과와 lease 판정을 만듭니다. `state`는 읽기만 하므로 문서 상태와
/// revision은 바뀌지 않습니다(XIX.5.3). 시스템·migration 출처는 판정하지 않습니다(`scope`는 `Ok`).
pub fn preview(
    state: &DocumentState,
    transaction: &CommandTransaction,
    table: &LeaseTable,
) -> Result<LeasePreview, PreviewError> {
    let mut before = state
        .model()
        .to_records()
        .map_err(|e| PreviewError::Copy(e.to_string()))?;
    before.sort_canonical();
    let (copy, _) = DocumentModel::from_records(before.clone())
        .map_err(|e| PreviewError::Copy(format!("{e:?}")))?;
    let snapshot = table.snapshot(state.model());
    let mut scratch = DocumentState::with_revision(copy, state.revision());
    let applied = scratch
        .execute_transaction(transaction)
        .map_err(PreviewError::Transaction)?;
    let scope = match Actor::of(transaction.origin) {
        Some(actor) => table.judge(&snapshot, scratch.model(), &applied.changes, actor),
        None => Ok(()),
    };
    let mut after = scratch
        .model()
        .to_records()
        .map_err(|e| PreviewError::Copy(e.to_string()))?;
    after.sort_canonical();
    Ok(LeasePreview {
        before,
        after,
        changes: applied.changes,
        scope,
    })
}

#[cfg(test)]
#[path = "lease_tests.rs"]
mod tests;
