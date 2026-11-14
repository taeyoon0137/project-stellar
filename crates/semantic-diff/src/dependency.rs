//! 변경 dependency graph (XIV.5.2).
//!
//! base → target diff의 변경 `a`가 변경 `b`에 **의존**한다는 것은 base에 `a`만 적용하면(`b` 없이) 참조·
//! 구조 무결성이 깨진다는 뜻입니다. 선택 staging(XIV.5.3)은 이 graph의 closure로 필수 함께-변경을 찾고,
//! unstage(XIV.7.2)는 같은 graph를 Index → HEAD 방향 diff에 씁니다.
//!
//! | 규칙 | 의존 | 이유 |
//! |---|---|---|
//! | 새 parent | 객체 추가·이동의 새 parent가 같은 diff에서 추가됨 → 그 추가 | [`DependencyReason::ParentAdded`] |
//! | 참조 대상 추가 | 새 값·경로가 같은 diff에서 추가되는 객체를 가리킴 → 그 추가 | [`DependencyReason::ReferenceAdded`] |
//! | 소속 목록 | 새 Page·View·Slot·root Frame이 문서 페이지·페이지 View·컴포넌트 Slot 목록·View/컴포넌트 root에 들어가는 변경 | [`DependencyReason::Membership`] |
//! | 자식 정리 | 객체 삭제 → base 자식의 삭제·이동 | [`DependencyReason::ChildDetached`] |
//! | 참조 해제 | 객체 삭제 → base에서 그 객체를 가리키던 속성의 변경(또는 그 객체 삭제) | [`DependencyReason::ReferenceReleased`] |
//! | 교체 | 같은 StableId의 삭제 ↔ 추가 | [`DependencyReason::Replacement`] |
//! | interface member 추가 | 컴포넌트 Property·Slot을 가리키는 override·binding → 그 member를 interface 목록에 넣는 변경 | [`DependencyReason::InterfaceMemberAdded`] |
//! | interface member 제거 | interface 목록에서 member를 빼는 변경 → 그 member를 가리키던 override·binding의 변경 | [`DependencyReason::InterfaceMemberReleased`] |
//! | OrderKey 비움 | 새 위치(parent, key)를 base에서 쓰던 형제의 이동·삭제 | [`DependencyReason::OrderKeyVacated`] |
//!
//! **참조 찾기(보수적)**: semantic-diff는 expressions·component 내부 형식에 의존하지 않으므로, 값의
//! canonical bytes와 경로 key segment(`/`로 나눈 StableId 문자열, 또는 16진수 bytes)에서 **16바이트
//! StableId 창**이 관심 대상 id와 같은지 봅니다. binding expression payload의 token·component Property
//! 참조, override 경로의 Property·Slot id, Variant patch 대상 node가 모두 이 방식으로 잡힙니다. StableId는
//! 전역 유일 값이라 우연 일치는 사실상 없고, 일치하면 의존을 **더하는** 방향(필수 변경이 늘어남)이므로
//! 무결성을 해치지 않습니다.
//!
//! graph 구성은 diff·base 크기에 선형(참조 scan은 값 bytes 길이에 선형)이고, closure는 반복 worklist로
//! 계산합니다(재귀 없음). 결과는 [`ChangeId`] 순서의 결정론적 BTreeMap입니다.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::{PathSegment, PropertyKey, PropertyPath};

use crate::change::SemanticChange;
use crate::keys;
use crate::snapshot::{ObjectKind, Origin, SemanticObject, SemanticSnapshot};
use crate::stage::{ChangeId, ChangeUnit, SelectionOperation, StageSelection, UnknownChange};
use crate::value::SemanticValue;

/// 의존 이유(우선순위 = 선언 순서).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DependencyReason {
    /// 같은 StableId의 삭제·추가(교체)는 함께 적용해야 합니다.
    Replacement,
    /// 새 parent가 같은 diff에서 추가됩니다.
    ParentAdded,
    /// 새 값이 같은 diff에서 추가되는 객체를 가리킵니다.
    ReferenceAdded,
    /// 새 Page·View·Slot·root Frame과 그것을 담는 목록·root 참조.
    Membership,
    /// 삭제되는 객체의 base 자식을 정리해야 합니다.
    ChildDetached,
    /// 삭제되는 객체를 가리키던 base 참조를 정리해야 합니다.
    ReferenceReleased,
    /// 컴포넌트 interface에 새 Property·Slot을 넣는 변경이 필요합니다.
    InterfaceMemberAdded,
    /// interface에서 빼는 Property·Slot을 가리키던 참조를 정리해야 합니다.
    InterfaceMemberReleased,
    /// 새 위치의 OrderKey를 base에서 쓰던 형제가 비켜야 합니다.
    OrderKeyVacated,
}

/// 변경 dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DependencyGraph {
    nodes: BTreeSet<ChangeId>,
    edges: BTreeMap<ChangeId, BTreeMap<ChangeId, DependencyReason>>,
}

/// 16바이트 StableId 창 집합.
type IdWindows = HashSet<[u8; 16]>;

fn windows_of(ids: impl IntoIterator<Item = StableId>) -> IdWindows {
    ids.into_iter().map(|id| id.to_bytes()).collect()
}

/// bytes 안에서 `candidates`와 같은 16바이트 창을 찾습니다.
fn scan_bytes(bytes: &[u8], candidates: &IdWindows, out: &mut BTreeSet<StableId>) {
    if candidates.is_empty() || bytes.len() < 16 {
        return;
    }
    for window in bytes.windows(16) {
        let mut key = [0u8; 16];
        key.copy_from_slice(window);
        if candidates.contains(&key) {
            out.insert(StableId::from_bytes(key));
        }
    }
}

/// 16진수 문자열을 bytes로 바꿉니다(짝수 길이·소문자/대문자 16진수만).
fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.len() < 32 {
        return None;
    }
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    text.as_bytes()
        .chunks(2)
        .map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?))
        .collect()
}

/// 속성 경로·값이 가리키는 `candidates` 안의 StableId(보수적 scan, 모듈 문서).
pub(crate) fn references(
    path: &PropertyPath,
    value: Option<&SemanticValue>,
    candidates: &IdWindows,
) -> BTreeSet<StableId> {
    let mut out = BTreeSet::new();
    if candidates.is_empty() {
        return out;
    }
    if let Some(PathSegment::Key(text)) = path.tail() {
        for part in text.split('/') {
            if let Ok(id) = part.parse::<StableId>() {
                if candidates.contains(&id.to_bytes()) {
                    out.insert(id);
                }
            } else if let Some(bytes) = hex_bytes(part) {
                scan_bytes(&bytes, candidates, &mut out);
            }
        }
    }
    if let Some(value) = value {
        scan_bytes(&value.to_canonical_bytes(), candidates, &mut out);
    }
    out
}

/// 객체 전체(모든 속성 경로·값)가 가리키는 id.
fn object_references(object: &SemanticObject, candidates: &IdWindows) -> BTreeSet<StableId> {
    let mut out = BTreeSet::new();
    for (path, value) in object.properties() {
        out.extend(references(path, Some(value), candidates));
    }
    out.remove(&object.id());
    out
}

/// 컴포넌트 interface 목록 key인지.
fn is_interface_list(key: PropertyKey) -> bool {
    key == keys::COMPONENT_PROPERTIES || key == keys::COMPONENT_SLOTS
}

/// 소속 목록 key(문서 페이지·페이지 View·컴포넌트 Slot).
fn is_membership_list(key: PropertyKey) -> bool {
    key == keys::DOCUMENT_PAGES || key == keys::PAGE_VIEWS || key == keys::COMPONENT_SLOTS
}

/// root 참조 key(View root Frame·컴포넌트 root).
fn is_root_reference(key: PropertyKey) -> bool {
    key == keys::VIEW_ROOT_FRAME || key == keys::COMPONENT_ROOT
}

fn ids_of(value: Option<&SemanticValue>) -> BTreeSet<StableId> {
    match value {
        Some(SemanticValue::Ids(ids)) => ids.iter().copied().collect(),
        Some(SemanticValue::Id(id)) => [*id].into(),
        _ => BTreeSet::new(),
    }
}

/// 소속 목록에 들어가야 하는 객체의 (목록 owner, 목록 key). 해당 없으면 `None`.
fn membership_list(object: &SemanticObject) -> Option<(StableId, PropertyKey)> {
    match object.kind() {
        ObjectKind::Page => object.parent().map(|doc| (doc, keys::DOCUMENT_PAGES)),
        ObjectKind::View => object.parent().map(|page| (page, keys::PAGE_VIEWS)),
        ObjectKind::ComponentSlot => match object.origin() {
            Origin::Component { component, .. } => Some((component, keys::COMPONENT_SLOTS)),
            _ => None,
        },
        _ => None,
    }
}

/// 같은 parent 아래 OrderKey가 같은 sequence를 공유하는 형제 class(modifier stack은 별도).
pub(crate) fn sibling_class(object: &SemanticObject) -> u8 {
    match object.kind() {
        ObjectKind::Modifier(_) => 1,
        _ => 0,
    }
}

/// 변경 하나의 이후(target) 상태 정보.
struct Target<'a> {
    /// 새 위치(추가·이동이면).
    position: Option<(Option<StableId>, Option<&'a OrderKey>, u8)>,
}

impl DependencyGraph {
    /// `base → target` diff의 변경 목록으로 graph를 만듭니다.
    ///
    /// `changes`는 `diff(base, target)`(또는 그 부분집합이 아닌 전체)여야 합니다. 같은 ID가 두 번이면
    /// 뒤의 것을 무시합니다.
    pub fn build(
        base: &SemanticSnapshot,
        target: &SemanticSnapshot,
        changes: &[SemanticChange],
    ) -> DependencyGraph {
        let mut graph = DependencyGraph::default();
        let mut by_id: BTreeMap<ChangeId, &SemanticChange> = BTreeMap::new();
        for change in changes {
            let id = change.id();
            graph.nodes.insert(id.clone());
            by_id.entry(id).or_insert(change);
        }
        let has = |id: &ChangeId| by_id.contains_key(id);

        // 추가·삭제 객체.
        let added: BTreeSet<StableId> = by_id
            .keys()
            .filter(|id| id.unit == ChangeUnit::Add)
            .map(|id| id.object)
            .collect();
        let removed: BTreeSet<StableId> = by_id
            .keys()
            .filter(|id| id.unit == ChangeUnit::Remove)
            .map(|id| id.object)
            .collect();

        // interface member 추가·제거: member id → interface 변경 ID.
        let mut members_added: BTreeMap<StableId, ChangeId> = BTreeMap::new();
        let mut members_removed: BTreeMap<StableId, ChangeId> = BTreeMap::new();
        for (id, change) in &by_id {
            let SemanticChange::ComponentInterfaceChange(c) = change else {
                continue;
            };
            if !c
                .path
                .keys()
                .first()
                .copied()
                .is_some_and(is_interface_list)
            {
                continue;
            }
            let (old, new) = (ids_of(c.old.as_ref()), ids_of(c.new.as_ref()));
            for member in new.difference(&old) {
                members_added.insert(*member, id.clone());
            }
            for member in old.difference(&new) {
                members_removed.insert(*member, id.clone());
            }
        }

        // 도입되는 참조 대상 → 도입 변경과 이유.
        let mut introduced: BTreeMap<StableId, (ChangeId, DependencyReason)> = BTreeMap::new();
        for object in &added {
            introduced.insert(
                *object,
                (ChangeId::add(*object), DependencyReason::ReferenceAdded),
            );
        }
        for (member, change) in &members_added {
            introduced
                .entry(*member)
                .or_insert((change.clone(), DependencyReason::InterfaceMemberAdded));
        }
        let introduced_windows = windows_of(introduced.keys().copied());

        for (id, change) in &by_id {
            let target_state = match change {
                SemanticChange::AddObject(c) | SemanticChange::AddToken(c) => Target {
                    position: Some((
                        c.object.parent(),
                        c.object.order_key(),
                        sibling_class(&c.object),
                    )),
                },
                SemanticChange::MoveObject(c) | SemanticChange::ReorderObject(c) => {
                    let class = target.get(c.object).map_or(0, sibling_class);
                    Target {
                        position: Some((c.to.parent, c.to.order_key.as_ref(), class)),
                    }
                }
                _ => Target { position: None },
            };

            // 교체.
            match id.unit {
                ChangeUnit::Add if removed.contains(&id.object) => {
                    graph.edge(
                        id,
                        &ChangeId::remove(id.object),
                        DependencyReason::Replacement,
                    );
                }
                ChangeUnit::Remove if added.contains(&id.object) => {
                    graph.edge(id, &ChangeId::add(id.object), DependencyReason::Replacement);
                }
                _ => {}
            }

            // 새 parent.
            if let Some((Some(parent), _, _)) = target_state.position {
                if parent != id.object && added.contains(&parent) {
                    graph.edge(id, &ChangeId::add(parent), DependencyReason::ParentAdded);
                }
            }

            // 참조 대상 추가·interface member 추가.
            let refs = match change {
                SemanticChange::AddObject(c) | SemanticChange::AddToken(c) => {
                    object_references(&c.object, &introduced_windows)
                }
                SemanticChange::SetProperty(c)
                | SemanticChange::ChangeToken(c)
                | SemanticChange::ComponentInterfaceChange(c)
                | SemanticChange::AssetRefChange(c) => match &c.new {
                    Some(value) => references(&c.path, Some(value), &introduced_windows),
                    None => BTreeSet::new(),
                },
                _ => BTreeSet::new(),
            };
            for referent in refs {
                if referent == id.object {
                    continue;
                }
                if let Some((introducer, reason)) = introduced.get(&referent) {
                    if introducer != id {
                        graph.edge(id, introducer, *reason);
                    }
                }
            }

            // 소속 목록: 새 Page·View·Slot → 목록에 넣는 변경, 새 root Frame → root 참조 변경.
            if let SemanticChange::AddObject(c) = change {
                if let Some((owner, list)) = membership_list(&c.object) {
                    let list_id = ChangeId::property(PropertyPath::with_keys(owner, vec![list]));
                    if has(&list_id) {
                        graph.edge(id, &list_id, DependencyReason::Membership);
                    }
                }
                if matches!(c.object.kind(), ObjectKind::Node(_)) && c.object.parent().is_none() {
                    for (other, other_change) in &by_id {
                        if other != id && introduces_root(other_change, c.object.id()) {
                            graph.edge(id, other, DependencyReason::Membership);
                        }
                    }
                }
            }
            // 목록에서 빠지는 Page·View·Slot → 그 객체 삭제.
            if let SemanticChange::SetProperty(c) | SemanticChange::ComponentInterfaceChange(c) =
                change
            {
                if c.path
                    .keys()
                    .first()
                    .copied()
                    .is_some_and(is_membership_list)
                {
                    let (old, new) = (ids_of(c.old.as_ref()), ids_of(c.new.as_ref()));
                    for dropped in old.difference(&new) {
                        if removed.contains(dropped) {
                            graph.edge(
                                id,
                                &ChangeId::remove(*dropped),
                                DependencyReason::Membership,
                            );
                        }
                    }
                }
            }
        }

        // 자식 정리: base 자식 index.
        if !removed.is_empty() {
            let mut children: BTreeMap<StableId, Vec<StableId>> = BTreeMap::new();
            for object in base.objects() {
                if let Some(parent) = object.parent() {
                    if removed.contains(&parent) {
                        children.entry(parent).or_default().push(object.id());
                    }
                }
            }
            for (parent, kids) in &children {
                let remove = ChangeId::remove(*parent);
                for kid in kids {
                    for detach in [ChangeId::remove(*kid), ChangeId::position(*kid)] {
                        if has(&detach) {
                            graph.edge(&remove, &detach, DependencyReason::ChildDetached);
                        }
                    }
                }
            }
        }

        // 참조 해제·interface member 제거: base에서 삭제 대상을 가리키던 (객체, 경로).
        let mut released: BTreeMap<StableId, (ChangeId, DependencyReason)> = BTreeMap::new();
        for object in &removed {
            released.insert(
                *object,
                (
                    ChangeId::remove(*object),
                    DependencyReason::ReferenceReleased,
                ),
            );
        }
        for (member, change) in &members_removed {
            released
                .entry(*member)
                .or_insert((change.clone(), DependencyReason::InterfaceMemberReleased));
        }
        if !released.is_empty() {
            let windows = windows_of(released.keys().copied());
            for object in base.objects() {
                for (path, value) in object.properties() {
                    for referent in references(path, Some(value), &windows) {
                        if referent == object.id() {
                            continue;
                        }
                        let Some((remover, reason)) = released.get(&referent) else {
                            continue;
                        };
                        let property = ChangeId::property(path.clone());
                        let fix = if has(&property) {
                            property
                        } else {
                            ChangeId::remove(object.id())
                        };
                        if &fix != remover && has(&fix) {
                            graph.edge(remover, &fix, *reason);
                        }
                    }
                }
            }
        }

        // OrderKey 비움: base에서 새 위치(parent, class, key)를 쓰던 형제.
        let mut slots: BTreeMap<(StableId, u8, &OrderKey), Vec<StableId>> = BTreeMap::new();
        for object in base.objects() {
            if let (Some(parent), Some(key)) = (object.parent(), object.order_key()) {
                slots
                    .entry((parent, sibling_class(object), key))
                    .or_default()
                    .push(object.id());
            }
        }
        for (id, change) in &by_id {
            let position = match change {
                SemanticChange::AddObject(c) | SemanticChange::AddToken(c) => c
                    .object
                    .parent()
                    .zip(c.object.order_key())
                    .map(|(p, k)| (p, sibling_class(&c.object), k)),
                SemanticChange::MoveObject(c) | SemanticChange::ReorderObject(c) => {
                    let class = target.get(c.object).map_or(0, sibling_class);
                    c.to.parent
                        .zip(c.to.order_key.as_ref())
                        .map(|(p, k)| (p, class, k))
                }
                _ => None,
            };
            let Some(slot) = position else { continue };
            for sibling in slots.get(&slot).into_iter().flatten() {
                if *sibling == id.object {
                    continue;
                }
                for vacate in [ChangeId::position(*sibling), ChangeId::remove(*sibling)] {
                    if has(&vacate) {
                        graph.edge(id, &vacate, DependencyReason::OrderKeyVacated);
                    }
                }
            }
        }
        graph
    }

    fn edge(&mut self, from: &ChangeId, to: &ChangeId, reason: DependencyReason) {
        if from == to {
            return;
        }
        let entry = self
            .edges
            .entry(from.clone())
            .or_default()
            .entry(to.clone())
            .or_insert(reason);
        if reason < *entry {
            *entry = reason;
        }
    }

    /// graph의 모든 변경 ID.
    pub fn changes(&self) -> &BTreeSet<ChangeId> {
        &self.nodes
    }

    /// 변경이 직접 의존하는 변경과 이유(ID 오름차순).
    pub fn dependencies<'a>(
        &'a self,
        id: &ChangeId,
    ) -> impl Iterator<Item = (&'a ChangeId, DependencyReason)> + 'a {
        self.edges
            .get(id)
            .into_iter()
            .flat_map(|deps| deps.iter().map(|(to, reason)| (to, *reason)))
    }

    /// 직접 의존 이유(없으면 `None`).
    pub fn reason(&self, from: &ChangeId, to: &ChangeId) -> Option<DependencyReason> {
        self.edges.get(from).and_then(|deps| deps.get(to)).copied()
    }

    /// 선택과 그 모든 전이 의존(반복 worklist).
    pub fn closure(&self, selected: &BTreeSet<ChangeId>) -> BTreeSet<ChangeId> {
        let mut closure = selected.clone();
        let mut work: Vec<ChangeId> = selected.iter().cloned().collect();
        while let Some(id) = work.pop() {
            for (dependency, _) in self.dependencies(&id) {
                if closure.insert(dependency.clone()) {
                    work.push(dependency.clone());
                }
            }
        }
        closure
    }

    /// 선택을 검증하고 closure로 [`StageSelection`]을 만듭니다. graph에 없는 ID는 오래된 요청입니다.
    pub fn select(
        &self,
        operation: SelectionOperation,
        selected: &BTreeSet<ChangeId>,
    ) -> Result<StageSelection, UnknownChange> {
        if let Some(id) = selected.iter().find(|id| !self.nodes.contains(*id)) {
            return Err(UnknownChange { id: id.clone() });
        }
        let closure = self.closure(selected);
        // closure ⊇ selected이므로 생성은 실패하지 않습니다.
        StageSelection::new(operation, selected.clone(), closure)
            .map_err(|missing| UnknownChange { id: missing.id })
    }
}

/// 변경이 `root`를 View root Frame·컴포넌트 root로 가리키게 만드는지.
fn introduces_root(change: &SemanticChange, root: StableId) -> bool {
    let points = |path: &PropertyPath, value: Option<&SemanticValue>| {
        path.keys().first().copied().is_some_and(is_root_reference)
            && value == Some(&SemanticValue::Id(root))
    };
    match change {
        SemanticChange::AddObject(c) => c
            .object
            .properties()
            .iter()
            .any(|(path, value)| points(path, Some(value))),
        SemanticChange::SetProperty(c) => points(&c.path, c.new.as_ref()),
        _ => false,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::build::tests::{build, head_records, key, sid};
    use crate::diff::diff;
    use crate::snapshot::NodeKind;
    use crate::stage::tests::path;
    use crate::status::tests::partial_staged;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::FiniteF32;

    pub(crate) fn object(
        id: u64,
        kind: ObjectKind,
        origin: Origin,
        parent: Option<u64>,
        order: Option<u8>,
        props: Vec<(PropertyPath, SemanticValue)>,
    ) -> SemanticObject {
        let mut object = SemanticObject::new(sid(id), kind, origin)
            .with_position(parent.map(sid), order.map(key));
        for (path, value) in props {
            object.set_property(path, value).unwrap();
        }
        object
    }

    pub(crate) fn doc() -> Origin {
        Origin::Document(sid(1))
    }

    pub(crate) fn frame(id: u64, parent: Option<u64>, order: u8) -> SemanticObject {
        object(
            id,
            ObjectKind::Node(if parent.is_none() {
                NodeKind::Frame
            } else {
                NodeKind::Fragment
            }),
            doc(),
            parent,
            Some(order),
            vec![(path(id, keys::VISIBLE), SemanticValue::Bool(true))],
        )
    }

    pub(crate) fn snapshot(objects: Vec<SemanticObject>) -> SemanticSnapshot {
        let mut snapshot = SemanticSnapshot::new();
        for object in objects {
            snapshot.insert(object).unwrap();
        }
        snapshot
    }

    pub(crate) fn replace(snapshot: &mut SemanticSnapshot, object: SemanticObject) {
        snapshot.remove(object.id());
        snapshot.insert(object).unwrap();
    }

    /// 컴포넌트 interface·instance override·binding fixture.
    ///
    /// - 컴포넌트 300(문서 1): Property 310, Slot 목록 없음.
    /// - 인스턴스 400(scene node, 300 참조): override 310 = 1.
    /// - token 200, binding expression node 600(owner 800: binding record, 객체 아님)이 token 200 참조.
    pub(crate) fn component_base() -> SemanticSnapshot {
        let component_origin = Origin::Component {
            document: sid(1),
            component: sid(300),
        };
        let number = |v: f32| SemanticValue::Typed(TypedValue::Number(FiniteF32::new(v).unwrap()));
        snapshot(vec![
            object(
                300,
                ObjectKind::Component,
                component_origin,
                None,
                None,
                vec![
                    (
                        path(300, keys::COMPONENT_PROPERTIES),
                        SemanticValue::Ids(vec![sid(310)]),
                    ),
                    (path(300, keys::COMPONENT_SLOTS), SemanticValue::Ids(vec![])),
                ],
            ),
            object(
                400,
                ObjectKind::Node(NodeKind::ComponentInstance),
                doc(),
                Some(10),
                Some(0x80),
                vec![
                    (
                        path(400, keys::instance::COMPONENT_REF),
                        SemanticValue::Id(sid(300)),
                    ),
                    (
                        path(400, keys::instance::PROPERTY_OVERRIDES)
                            .with_key_segment(&sid(310).to_string()),
                        number(1.0),
                    ),
                ],
            ),
            frame(10, None, 0x80),
            object(
                200,
                ObjectKind::Token,
                Origin::Project,
                None,
                None,
                vec![(path(200, keys::TOKEN_NAME), SemanticValue::Text("a".into()))],
            ),
            expression(600, &[sid(200)]),
        ])
    }

    /// binding expression node: payload = reference tag + 참조 id들.
    pub(crate) fn expression(id: u64, refs: &[StableId]) -> SemanticObject {
        let mut payload = vec![0x02];
        for r in refs {
            payload.extend_from_slice(&r.to_bytes());
        }
        object(
            id,
            ObjectKind::ExpressionNode,
            Origin::Project,
            Some(800),
            None,
            vec![(path(id, keys::EXPR_PAYLOAD), SemanticValue::Bytes(payload))],
        )
    }

    fn deps(graph: &DependencyGraph, id: &ChangeId) -> Vec<(ChangeId, DependencyReason)> {
        graph
            .dependencies(id)
            .map(|(to, reason)| (to.clone(), reason))
            .collect()
    }

    #[test]
    fn new_objects_depend_on_parents_lists_and_roots() {
        // base: HEAD fixture. target: 새 페이지 4·View 5·root frame 20·자식 21·modifier 60.
        let base = build(&head_records());
        let mut target = base.clone();
        let mut document = target.get(sid(1)).unwrap().clone();
        document
            .set_property(
                path(1, keys::DOCUMENT_PAGES),
                SemanticValue::Ids(vec![sid(2), sid(4)]),
            )
            .unwrap();
        replace(&mut target, document);
        target
            .insert(object(
                4,
                ObjectKind::Page,
                doc(),
                Some(1),
                None,
                vec![(path(4, keys::PAGE_VIEWS), SemanticValue::Ids(vec![sid(5)]))],
            ))
            .unwrap();
        target
            .insert(object(
                5,
                ObjectKind::View,
                doc(),
                Some(4),
                None,
                vec![(path(5, keys::VIEW_ROOT_FRAME), SemanticValue::Id(sid(20)))],
            ))
            .unwrap();
        target.insert(frame(20, None, 0x80)).unwrap();
        target.insert(frame(21, Some(20), 0x80)).unwrap();
        target
            .insert(object(
                60,
                ObjectKind::Modifier(crate::snapshot::ModifierKind::Fill),
                doc(),
                Some(21),
                Some(0x80),
                vec![],
            ))
            .unwrap();
        let changes = diff(&base, &target);
        let graph = DependencyGraph::build(&base, &target, &changes);
        let pages = ChangeId::property(path(1, keys::DOCUMENT_PAGES));
        use DependencyReason::*;
        assert_eq!(
            deps(&graph, &pages),
            vec![(ChangeId::add(sid(4)), ReferenceAdded)]
        );
        assert_eq!(
            deps(&graph, &ChangeId::add(sid(4))),
            vec![
                (pages.clone(), Membership),
                (ChangeId::add(sid(5)), ReferenceAdded)
            ]
        );
        assert_eq!(
            deps(&graph, &ChangeId::add(sid(5))),
            vec![
                (ChangeId::add(sid(4)), ParentAdded),
                (ChangeId::add(sid(20)), ReferenceAdded),
            ]
        );
        assert_eq!(
            deps(&graph, &ChangeId::add(sid(20))),
            vec![(ChangeId::add(sid(5)), Membership)]
        );
        assert_eq!(
            deps(&graph, &ChangeId::add(sid(21))),
            vec![(ChangeId::add(sid(20)), ParentAdded)]
        );
        assert_eq!(
            deps(&graph, &ChangeId::add(sid(60))),
            vec![(ChangeId::add(sid(21)), ParentAdded)]
        );
        // modifier 하나만 골라도 새 subtree 전체와 페이지 목록이 필요합니다.
        let closure = graph.closure(&[ChangeId::add(sid(60))].into());
        assert_eq!(
            closure,
            [
                pages,
                ChangeId::add(sid(4)),
                ChangeId::add(sid(5)),
                ChangeId::add(sid(20)),
                ChangeId::add(sid(21)),
                ChangeId::add(sid(60)),
            ]
            .into()
        );
    }

    #[test]
    fn binding_token_and_component_interface_dependencies() {
        let base = component_base();
        let mut target = base.clone();
        // 새 token 201을 binding 601이 참조, 기존 binding 600은 token 200 대신 201 참조.
        target
            .insert(object(
                201,
                ObjectKind::Token,
                Origin::Project,
                None,
                None,
                vec![(path(201, keys::TOKEN_NAME), SemanticValue::Text("b".into()))],
            ))
            .unwrap();
        target.insert(expression(601, &[sid(201)])).unwrap();
        // token 200 삭제(600의 참조가 201로 바뀜).
        target.remove(sid(200));
        replace(&mut target, expression(600, &[sid(201)]));
        // interface: Property 310 제거·311 추가, 인스턴스 override 310 제거·311 추가.
        let mut component = target.get(sid(300)).unwrap().clone();
        component
            .set_property(
                path(300, keys::COMPONENT_PROPERTIES),
                SemanticValue::Ids(vec![sid(311)]),
            )
            .unwrap();
        replace(&mut target, component);
        let override_path = |p: u64| {
            path(400, keys::instance::PROPERTY_OVERRIDES).with_key_segment(&sid(p).to_string())
        };
        let mut instance = target.get(sid(400)).unwrap().clone();
        instance.remove_property(&override_path(310));
        instance
            .set_property(override_path(311), SemanticValue::Bool(true))
            .unwrap();
        replace(&mut target, instance);
        // binding 602가 새 Property 311을 참조(component property reference).
        target.insert(expression(602, &[sid(311)])).unwrap();

        let changes = diff(&base, &target);
        let graph = DependencyGraph::build(&base, &target, &changes);
        use DependencyReason::*;
        let interface = ChangeId::property(path(300, keys::COMPONENT_PROPERTIES));
        let payload_600 = ChangeId::property(path(600, keys::EXPR_PAYLOAD));
        // binding → token 추가.
        assert_eq!(
            deps(&graph, &ChangeId::add(sid(601))),
            vec![(ChangeId::add(sid(201)), ReferenceAdded)]
        );
        assert_eq!(
            deps(&graph, &payload_600),
            vec![(ChangeId::add(sid(201)), ReferenceAdded)]
        );
        // token 삭제 → 참조하던 binding payload 변경.
        assert_eq!(
            deps(&graph, &ChangeId::remove(sid(200))),
            vec![(payload_600, ReferenceReleased)]
        );
        // override 추가 → interface member 추가, binding의 Property 참조도 같음.
        assert_eq!(
            deps(&graph, &ChangeId::property(override_path(311))),
            vec![(interface.clone(), InterfaceMemberAdded)]
        );
        assert_eq!(
            deps(&graph, &ChangeId::add(sid(602))),
            vec![(interface.clone(), InterfaceMemberAdded)]
        );
        // interface에서 310 제거 → 310 override 제거.
        assert_eq!(
            deps(&graph, &interface),
            vec![(
                ChangeId::property(override_path(310)),
                InterfaceMemberReleased
            )]
        );
        assert_eq!(
            graph.reason(&interface, &ChangeId::property(override_path(310))),
            Some(InterfaceMemberReleased)
        );
        // 의존 없는 변경.
        assert!(deps(&graph, &ChangeId::property(override_path(310))).is_empty());
        assert!(deps(&graph, &ChangeId::add(sid(201))).is_empty());
    }

    #[test]
    fn removals_replacements_and_order_keys() {
        // base: root 10 ─ 11(0x40) ─ 12(0x80 under 11), 13(0xC0). target: 11 삭제(12도), 13 교체,
        // 14 추가(0x40: 11이 쓰던 key), 15·16 OrderKey 교환.
        let base = snapshot(vec![
            frame(10, None, 0x80),
            frame(11, Some(10), 0x40),
            frame(12, Some(11), 0x80),
            frame(13, Some(10), 0xC0),
            frame(15, Some(10), 0x10),
            frame(16, Some(10), 0x20),
        ]);
        let mut target = base.clone();
        target.remove(sid(11));
        target.remove(sid(12));
        replace(
            &mut target,
            object(
                13,
                ObjectKind::Token,
                Origin::Project,
                Some(10),
                None,
                vec![],
            ),
        );
        target.insert(frame(14, Some(10), 0x40)).unwrap();
        replace(&mut target, frame(15, Some(10), 0x20));
        replace(&mut target, frame(16, Some(10), 0x10));
        let changes = diff(&base, &target);
        let graph = DependencyGraph::build(&base, &target, &changes);
        use DependencyReason::*;
        assert_eq!(
            deps(&graph, &ChangeId::remove(sid(11))),
            vec![(ChangeId::remove(sid(12)), ChildDetached)]
        );
        assert!(deps(&graph, &ChangeId::remove(sid(12))).is_empty());
        assert_eq!(
            deps(&graph, &ChangeId::add(sid(13))),
            vec![(ChangeId::remove(sid(13)), Replacement)]
        );
        assert_eq!(
            deps(&graph, &ChangeId::remove(sid(13))),
            vec![(ChangeId::add(sid(13)), Replacement)]
        );
        assert_eq!(
            deps(&graph, &ChangeId::add(sid(14))),
            vec![(ChangeId::remove(sid(11)), OrderKeyVacated)]
        );
        assert_eq!(
            deps(&graph, &ChangeId::position(sid(15))),
            vec![(ChangeId::position(sid(16)), OrderKeyVacated)]
        );
        assert_eq!(
            deps(&graph, &ChangeId::position(sid(16))),
            vec![(ChangeId::position(sid(15)), OrderKeyVacated)]
        );
        // 14만 골라도 11·12 삭제가 함께 필요합니다.
        let selection = graph
            .select(SelectionOperation::Stage, &[ChangeId::add(sid(14))].into())
            .unwrap();
        assert_eq!(
            selection.required_additions(),
            [ChangeId::remove(sid(11)), ChangeId::remove(sid(12))].into()
        );
        // graph에 없는 ID는 오래된 요청입니다.
        assert_eq!(
            graph.select(SelectionOperation::Stage, &[ChangeId::add(sid(99))].into()),
            Err(UnknownChange {
                id: ChangeId::add(sid(99))
            })
        );
    }

    #[test]
    fn independent_changes_have_no_dependencies_and_graph_is_deterministic() {
        let [_, index, working] = partial_staged().map(|r| build(&r));
        let changes = diff(&index, &working);
        let graph = DependencyGraph::build(&index, &working, &changes);
        assert_eq!(graph.changes().len(), changes.len());
        for id in graph.changes() {
            assert_eq!(graph.dependencies(id).count(), 0, "{id:?}");
        }
        let mut reversed: Vec<SemanticChange> = changes.to_vec();
        reversed.reverse();
        assert_eq!(DependencyGraph::build(&index, &working, &reversed), graph);
    }

    #[test]
    fn reference_scan_reads_values_and_path_segments() {
        let target_id = sid(77);
        let windows = windows_of([target_id]);
        // 값 bytes 안의 id.
        let mut bytes = vec![1, 2, 3];
        bytes.extend_from_slice(&target_id.to_bytes());
        assert_eq!(
            references(
                &path(1, keys::EXPR_PAYLOAD),
                Some(&SemanticValue::Bytes(bytes)),
                &windows
            ),
            [target_id].into()
        );
        // override 경로 segment(nested `a/b/property`).
        let nested = path(1, keys::instance::PROPERTY_OVERRIDES).with_key_segment(&format!(
            "{}/{}",
            sid(5),
            target_id
        ));
        assert_eq!(references(&nested, None, &windows), [target_id].into());
        // Variant property patch segment(경로 canonical bytes 16진수).
        let patched = path(77, keys::RECT_SIZE).to_canonical_bytes();
        let hex: String = patched.iter().map(|b| format!("{b:02x}")).collect();
        let variant = path(1, keys::VARIANT_PROPERTY_PATCH).with_key_segment(&hex);
        assert_eq!(references(&variant, None, &windows), [target_id].into());
        // 관계없는 값.
        assert!(references(
            &path(1, keys::DISPLAY_NAME),
            Some(&SemanticValue::Text("x".into())),
            &windows
        )
        .is_empty());
        assert!(references(&nested, None, &IdWindows::new()).is_empty());
        assert_eq!(hex_bytes("zz"), None);
    }
}
