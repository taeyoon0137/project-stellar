//! Orphaned Binding validator (VIII.8.1).
//!
//! target 속성이나 source가 참조하던 대상이 사라지거나 타입이 바뀌어 binding이 깨지면, binding을 **삭제하지 않고**
//! [`OrphanedBinding`]으로 바꿉니다(아키텍처 §14.4). orphan은 원래 binding 전체(ID·target 경로·마지막 유효 타입·source
//! AST·owner)와 끊어진 원인·위치를 보존합니다. 마지막 계산값은 보관하지 않습니다.
//!
//! 깨짐 판정(순서대로, 첫 원인 하나를 기록):
//!
//! 1. [`OrphanCause::TargetMissing`] — target 속성(또는 그 node)이 없습니다.
//! 2. [`OrphanCause::SourceMissing`] — source가 참조하는 대상이 없습니다(대상 정렬 순 첫 번째, 참조 표현식 node 포함).
//! 3. [`OrphanCause::SourceTypeChanged`] — 참조 대상 타입이 바뀌어 source가 type 검사를 통과하지 못합니다.
//! 4. [`OrphanCause::TargetTypeChanged`] — target 속성의 선언 타입이 바뀌어 source 결과를 둘 수 없습니다. 선언 타입이
//!    바뀌어도 source 결과를 그대로 둘 수 있으면 깨진 것이 아닙니다.
//!
//! 현재 상태는 [`BindingWorld`]가 알려 줍니다. validator는 live binding만 검사하며 이미 orphan인 binding을 자동으로
//! 다시 연결하지 않습니다(재연결은 명시 command, VIII.8.4).

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::{ExpectedRefType, UnresolvedRef};
use stellar_expressions::ast::{ExprNodeId, NodePayload, ReferenceTarget};
use stellar_expressions::typecheck::{ReferenceTypes, TypeError};
use stellar_expressions::types::TypeSpec;

use crate::binding::{
    check_source, BindingId, BindingSet, BindingSetError, BindingTypeError, ValueBinding,
};

/// binding validator가 보는 현재 상태.
pub trait BindingWorld {
    /// target 속성의 현재 선언 타입. 속성(또는 그 node)이 없으면 `None`.
    fn target_type(&self, target: &PropertyPath) -> Option<TypeSpec>;
    /// 참조 대상의 현재 타입. 대상이 없으면 `None`.
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec>;
}

/// binding이 끊어진 원인.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanCause {
    /// target 속성(또는 그 node)이 없습니다.
    TargetMissing,
    /// source가 참조하는 대상이 없습니다.
    SourceMissing {
        /// 없는 대상.
        target: ReferenceTarget,
        /// 그 대상을 참조하는 source 표현식 node(StableId 순).
        nodes: Vec<ExprNodeId>,
    },
    /// 참조 대상 타입이 바뀌어 source가 type 검사를 통과하지 못합니다.
    SourceTypeChanged {
        /// type 오류(오류 node StableId 포함).
        errors: Vec<TypeError>,
    },
    /// target 속성 선언 타입이 바뀌어 source 결과를 둘 수 없습니다.
    TargetTypeChanged {
        /// 현재 선언 타입.
        declared: TypeSpec,
        /// source 결과 타입.
        source: TypeSpec,
    },
}

impl fmt::Display for OrphanCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrphanCause::TargetMissing => f.write_str("target 속성이 없습니다"),
            OrphanCause::SourceMissing { target, .. } => {
                write!(f, "참조 대상이 없습니다: {target}")
            }
            OrphanCause::SourceTypeChanged { errors } => {
                write!(
                    f,
                    "참조 대상 타입 변경으로 source type 오류 {}개",
                    errors.len()
                )
            }
            OrphanCause::TargetTypeChanged { declared, source } => {
                write!(
                    f,
                    "target 타입 {declared}에 source 결과 {source}를 둘 수 없습니다"
                )
            }
        }
    }
}

/// 끊어진 binding (VIII.8.1). 원래 binding 전체와 원인을 보존합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanedBinding {
    binding: ValueBinding,
    cause: OrphanCause,
}

impl OrphanedBinding {
    /// orphan을 만듭니다.
    pub fn new(binding: ValueBinding, cause: OrphanCause) -> OrphanedBinding {
        OrphanedBinding { binding, cause }
    }

    /// 원래 binding(ID·경로·타입·source·owner 보존).
    pub fn binding(&self) -> &ValueBinding {
        &self.binding
    }

    /// 원래 binding을 꺼냅니다.
    pub fn into_binding(self) -> ValueBinding {
        self.binding
    }

    /// binding ID.
    pub fn id(&self) -> BindingId {
        self.binding.id()
    }

    /// target 속성 경로.
    pub fn target(&self) -> &PropertyPath {
        self.binding.target()
    }

    /// 마지막 유효 타입(binding이 깨지기 전 target 기대 타입).
    pub fn last_valid_type(&self) -> TypeSpec {
        self.binding.expected_type()
    }

    /// 끊어진 원인.
    pub fn cause(&self) -> &OrphanCause {
        &self.cause
    }

    /// 없는 참조 대상의 미해결 참조(III.2.4: 원래 ID·기대 종류·발견 위치 = target 경로). 환경 입력·대상이 남아 있는
    /// 원인은 `None`.
    pub fn unresolved(&self) -> Option<UnresolvedRef> {
        let OrphanCause::SourceMissing { target, .. } = &self.cause else {
            return None;
        };
        let location = self.binding.target().clone();
        match target {
            ReferenceTarget::Token(token) => Some(token.unresolved_at(location)),
            ReferenceTarget::Property(path) => Some(UnresolvedRef::new(
                path.root(),
                ExpectedRefType::Node,
                location,
            )),
            ReferenceTarget::ComponentProperty(id) => Some(UnresolvedRef::new(
                *id,
                ExpectedRefType::Component,
                location,
            )),
            ReferenceTarget::Environment(_) => None,
        }
    }
}

struct WorldTypes<'a>(&'a dyn BindingWorld);

impl ReferenceTypes for WorldTypes<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        self.0.reference_type(target)
    }
}

/// binding 하나가 현재 상태에서 깨졌는지 판정합니다. 깨지지 않았으면 `None`.
pub fn break_cause(binding: &ValueBinding, world: &dyn BindingWorld) -> Option<OrphanCause> {
    let Some(declared) = world.target_type(binding.target()) else {
        return Some(OrphanCause::TargetMissing);
    };
    for target in binding.references() {
        if world.reference_type(&target).is_none() {
            let nodes = binding
                .source()
                .nodes()
                .iter()
                .filter(|node| matches!(node.payload(), NodePayload::Reference(t) if *t == target))
                .map(|node| node.id())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            return Some(OrphanCause::SourceMissing { target, nodes });
        }
    }
    match check_source(declared, binding.source(), &WorldTypes(world)) {
        Ok(_) => None,
        Err(BindingTypeError::Expression(errors)) => {
            Some(OrphanCause::SourceTypeChanged { errors })
        }
        Err(BindingTypeError::Mismatch { expected, actual }) => {
            Some(OrphanCause::TargetTypeChanged {
                declared: expected,
                source: actual,
            })
        }
    }
}

/// live binding과 orphan binding 저장소 (VIII.8.1). 같은 ID는 둘 중 한 곳에만 있습니다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingStore {
    live: BindingSet,
    orphans: BTreeMap<BindingId, OrphanedBinding>,
}

impl BindingStore {
    /// live binding으로 만듭니다.
    pub fn new(live: BindingSet) -> BindingStore {
        BindingStore {
            live,
            orphans: BTreeMap::new(),
        }
    }

    /// live binding.
    pub fn live(&self) -> &BindingSet {
        &self.live
    }

    /// orphan binding(ID 순).
    pub fn orphans(&self) -> impl Iterator<Item = &OrphanedBinding> {
        self.orphans.values()
    }

    /// orphan 하나.
    pub fn orphan(&self, id: BindingId) -> Option<&OrphanedBinding> {
        self.orphans.get(&id)
    }

    /// 전체 binding 수(live + orphan).
    pub fn len(&self) -> usize {
        self.live.len() + self.orphans.len()
    }

    /// binding이 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// live binding을 추가합니다. 같은 ID의 orphan이 있거나 live 모음 규칙(ID·target 유일)을 어기면 거부합니다.
    pub fn insert_live(&mut self, binding: ValueBinding) -> Result<(), BindingSetError> {
        if self.orphans.contains_key(&binding.id()) {
            return Err(BindingSetError::DuplicateId(binding.id()));
        }
        self.live.insert(binding)
    }

    pub(crate) fn live_mut(&mut self) -> &mut BindingSet {
        &mut self.live
    }

    pub(crate) fn insert_orphan(&mut self, orphan: OrphanedBinding) {
        self.orphans.insert(orphan.id(), orphan);
    }

    pub(crate) fn remove_orphan(&mut self, id: BindingId) -> Option<OrphanedBinding> {
        self.orphans.remove(&id)
    }

    /// live binding을 검사해 깨진 binding을 orphan으로 옮기고, 새로 orphan이 된 ID를 돌려줍니다(ID 순). 아무것도
    /// 삭제하지 않습니다.
    pub fn orphan_broken(&mut self, world: &dyn BindingWorld) -> Vec<BindingId> {
        let broken: Vec<(BindingId, OrphanCause)> = self
            .live
            .iter()
            .filter_map(|binding| break_cause(binding, world).map(|cause| (binding.id(), cause)))
            .collect();
        let mut moved = Vec::with_capacity(broken.len());
        for (id, cause) in broken {
            if let Some(binding) = self.live.remove(id) {
                self.orphans
                    .insert(id, OrphanedBinding::new(binding, cause));
                moved.push(id);
            }
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::PropertyKey;
    use stellar_core_types::reference::{NodeRef, TokenRef};
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::Length;
    use stellar_expressions::ast::{ExprNode, ExpressionAst};
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::ExpressionType;

    use super::*;
    use crate::binding::ScopeOwner;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn prop(n: u64) -> PropertyPath {
        PropertyPath::new(sid(2_000 + n)).push(PropertyKey(1))
    }

    fn x(n: u64) -> ExprNodeId {
        ExprNodeId::new(sid(n))
    }

    fn token() -> ReferenceTarget {
        ReferenceTarget::Token(TokenRef::new(sid(1_001)))
    }

    fn cp() -> ReferenceTarget {
        ReferenceTarget::ComponentProperty(sid(3_001))
    }

    fn length() -> TypeSpec {
        TypeSpec::required(ExpressionType::Length)
    }

    /// `target + 1px`.
    fn plus_one(base: u64, target: ReferenceTarget) -> ExpressionAst {
        ExpressionAst::new(
            x(base),
            vec![
                ExprNode::binary(x(base), BinaryOp::Add, x(base + 1), x(base + 2)),
                ExprNode::reference(x(base + 1), target),
                ExprNode::literal(
                    x(base + 2),
                    TypedValue::Length(Length::new(1.0).expect("유한")),
                ),
            ],
        )
        .expect("유효")
    }

    fn bind(n: u64, target: u64, source: ExpressionAst) -> ValueBinding {
        ValueBinding::new(
            BindingId::new(sid(4_000 + n)),
            prop(target),
            source,
            length(),
            ScopeOwner::Node(NodeRef::new(sid(5_000 + n))),
        )
    }

    /// 상태: 속성 1~4 존재(Length), 토큰·Component Property 존재(Length).
    struct World {
        targets: BTreeMap<PropertyPath, TypeSpec>,
        references: BTreeMap<ReferenceTarget, TypeSpec>,
    }

    impl BindingWorld for World {
        fn target_type(&self, target: &PropertyPath) -> Option<TypeSpec> {
            self.targets.get(target).copied()
        }

        fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
            self.references.get(target).copied()
        }
    }

    fn world() -> World {
        World {
            targets: (1..=4).map(|n| (prop(n), length())).collect(),
            references: BTreeMap::from([(token(), length()), (cp(), length())]),
        }
    }

    /// b1: p1 ← token + 1px, b2: p2 ← cp + 1px, b3: p3 ← cp + 1px, b4: p4 ← token + 1px.
    fn store() -> BindingStore {
        let mut live = BindingSet::new();
        live.insert(bind(1, 1, plus_one(10, token()))).expect("b1");
        live.insert(bind(2, 2, plus_one(20, cp()))).expect("b2");
        live.insert(bind(3, 3, plus_one(30, cp()))).expect("b3");
        live.insert(bind(4, 4, plus_one(40, token()))).expect("b4");
        BindingStore::new(live)
    }

    #[test]
    fn deleted_references_and_targets_become_orphans_with_information() {
        let mut store = store();
        let originals: Vec<ValueBinding> = store.live().iter().cloned().collect();
        let mut world = world();
        assert!(
            store.orphan_broken(&world).is_empty(),
            "정상 상태는 orphan 없음"
        );
        // 토큰 삭제 + p4 node 삭제.
        world.references.remove(&token());
        world.targets.remove(&prop(4));
        let moved = store.orphan_broken(&world);
        assert_eq!(
            moved,
            vec![BindingId::new(sid(4_001)), BindingId::new(sid(4_004))]
        );
        // 삭제하지 않음: 전체 수 유지.
        assert_eq!(store.len(), 4);
        assert_eq!(store.live().len(), 2);
        let b1 = store.orphan(BindingId::new(sid(4_001))).expect("b1 orphan");
        assert_eq!(b1.binding(), &originals[0], "ID·경로·타입·source 보존");
        assert_eq!(
            b1.binding().to_bytes().expect("직렬화"),
            originals[0].to_bytes().expect("직렬화")
        );
        assert_eq!(b1.target(), &prop(1));
        assert_eq!(b1.last_valid_type(), length());
        assert_eq!(
            b1.cause(),
            &OrphanCause::SourceMissing {
                target: token(),
                nodes: vec![x(11)]
            }
        );
        let unresolved = b1.unresolved().expect("미해결 참조");
        assert_eq!(unresolved.original_id(), sid(1_001));
        assert_eq!(unresolved.expected(), ExpectedRefType::Token);
        assert_eq!(unresolved.location(), &prop(1));
        // p4는 target이 먼저 판정됩니다(토큰도 없지만 원인은 TargetMissing).
        let b4 = store.orphan(BindingId::new(sid(4_004))).expect("b4 orphan");
        assert_eq!(b4.cause(), &OrphanCause::TargetMissing);
        assert_eq!(b4.unresolved(), None);
        // 다시 검사해도 새 orphan 없음, orphan은 그대로(자동 재연결·삭제 없음).
        let snapshot = store.clone();
        world.references.insert(token(), length());
        assert!(store.orphan_broken(&world).is_empty());
        assert_eq!(store, snapshot);
    }

    #[test]
    fn type_changes_become_orphans_only_when_broken() {
        let mut store = store();
        let mut world = world();
        // p2 선언 타입 Length → Color: b2는 결과를 둘 수 없음.
        world
            .targets
            .insert(prop(2), TypeSpec::required(ExpressionType::Color));
        // p1 선언 타입 Length → Optional<Length>: 결과를 그대로 둘 수 있어 깨지지 않음.
        world
            .targets
            .insert(prop(1), TypeSpec::optional(ExpressionType::Length));
        let moved = store.orphan_broken(&world);
        assert_eq!(moved, vec![BindingId::new(sid(4_002))]);
        assert_eq!(
            store
                .orphan(BindingId::new(sid(4_002)))
                .expect("b2")
                .cause(),
            &OrphanCause::TargetTypeChanged {
                declared: TypeSpec::required(ExpressionType::Color),
                source: length(),
            }
        );
        // Component Property 타입 Length → Color: b3 source type 오류(오류 node 포함).
        world
            .references
            .insert(cp(), TypeSpec::required(ExpressionType::Color));
        let moved = store.orphan_broken(&world);
        assert_eq!(moved, vec![BindingId::new(sid(4_003))]);
        let b3 = store.orphan(BindingId::new(sid(4_003))).expect("b3");
        match b3.cause() {
            OrphanCause::SourceTypeChanged { errors } => {
                let nodes: BTreeSet<ExprNodeId> = errors.iter().map(|e| e.node).collect();
                assert!(nodes.contains(&x(30)), "덧셈 node에서 오류: {errors:?}");
            }
            other => panic!("SourceTypeChanged 기대: {other:?}"),
        }
        assert_eq!(b3.last_valid_type(), length(), "마지막 유효 타입 보존");
        assert_eq!(store.len(), 4);
    }
}
