//! ComponentInstance 캔버스 편집 코어 (XII.10.34, ADR-0118 결정 4).
//!
//! - **깊은 선택.** 인스턴스 안 정의 node는 문서 node가 아니라 선택할 수 없습니다. [`pick_inside`]는 커서 아래 가장
//!   위의 정의 node를 찾아 읽기 전용 윤곽([`InstancePick::outline`], world 사각형 네 꼭짓점)으로 돌려줍니다. 윤곽은
//!   transient 값이며 저장·history와 무관합니다.
//! - **허용 편집.** 정의 node 속성 중 공개 Property에 연결된 값만 바꿀 수 있습니다. 연결 목록([`PropertyLink`])은
//!   binding 저장소가 넘깁니다(정의 node 속성 = Property 참조 하나인 binding). 편집은 그 Property의 override Set이며
//!   IX.2.3 규칙(`check_override`: 공개·선언·타입)을 그대로 씁니다. 거부하면 이유와 Detach 가능 여부를 돌려주고
//!   문서는 바뀌지 않습니다. nested instance 안 값은 직접 편집하지 않습니다(IX.4.2, drilling 필요).
//! - **기록.** override Set·속성별 Reset·전체 Reset은 모두 인스턴스 payload 교체 command(`swap-instance`,
//!   ADR-0118 결정 5) 하나입니다. 동작마다 history 1건입니다.
//! - **Slot 끌어 놓기.** 인스턴스 자식은 그 인스턴스 slot override에 투영된 Slot content입니다(문서 validator,
//!   ADR-0129 결정 3). [`plan_slot_drop`]은 node를 공개 Slot(컴포넌트가 선언한 Slot)에 넣는 `place-slot-content`
//!   command 하나를 만들고, [`plan_slot_take_out`]은 반대로 꺼냅니다. slot 종류·개수·가림 규칙은 IX.3 함수
//!   (`ComponentSlot::check_content`, `check_hidden_slot_content`)를 그대로 씁니다.
//! - **문맥 동작.** [`instance_actions`]가 속성별·전체 Reset, Detach(IX.2.6), 정의로 이동의 사용 가능 여부를 정하고,
//!   [`InstanceAction`]이 그 동작의 코어 command ID(메뉴 막대·문맥 메뉴가 같은 이름)를 정합니다. Detach 실행은
//!   [`plan_detach_instance`]가 `detach-instance` command 하나로 만듭니다(ADR-0129 결정 4·5).

use stellar_commands::command::Command;
use stellar_commands::detach::{plan_detach, DetachPlanError, InstanceRestructure, NodePlacement};
use stellar_commands::lock::{edit_permission, EditIntent};
use stellar_commands::swap::SwapInstance;
use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::{StableId, StableIdGenerator};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Transform2D, Vec2};
use stellar_document::component::definition::{
    ComponentEntry, ComponentLibrary, ComponentPropertyId, ComponentSlotId,
};
use stellar_document::component::instance::{
    placed_free_frame, ComponentInstancePayload, OverrideKey, SlotOverride,
};
use stellar_document::component::override_rules::{check_override, PropertyOverrideError};
use stellar_document::component::slot::SlotError;
use stellar_document::component::slot_override::{check_hidden_slot_content, SlotOverrideError};
use stellar_document::frame::Clipping;
use stellar_document::layout_record::LayoutRecord;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierStack;
use stellar_document::node_kind::NodeKind;
use stellar_document::records::{NodePayload, NodeRecord};

use super::CanvasScene;

/// 인스턴스 안 깊은 선택 결과(읽기 전용).
#[derive(Debug, Clone, PartialEq)]
pub struct InstancePick {
    /// 문서의 인스턴스 node.
    pub instance: StableId,
    /// 참조 컴포넌트.
    pub component: ComponentRef,
    /// 정의 root부터 고른 node까지의 정의 node 경로.
    pub path: Vec<StableId>,
    /// 고른 정의 node.
    pub node: StableId,
    /// 읽기 전용 윤곽: local 사각형 네 꼭짓점(왼쪽 위부터 시계 방향)의 world 위치.
    pub outline: [Vec2; 4],
}

fn payload_of(model: &DocumentModel, instance: StableId) -> Option<&ComponentInstancePayload> {
    match model.payload(instance) {
        Some(NodePayload::ComponentInstance(payload)) => Some(payload),
        _ => None,
    }
}

fn corners(world: &Transform2D, width: f32, height: f32) -> Option<[Vec2; 4]> {
    let at = |x: f32, y: f32| world.apply(Vec2::new(x, y).ok()?).ok();
    Some([
        at(0., 0.)?,
        at(width, 0.)?,
        at(width, height)?,
        at(0., height)?,
    ])
}

/// 커서 아래 가장 위(그리기 순서 마지막)의 정의 node를 고릅니다. 인스턴스 투영 행(scene의 root 행 world)에서
/// 정의 tree를 같은 규칙(`placed_free_frame`)으로 내려가며, 자식을 자르는 Frame 밖의 자식은 보지 않습니다.
pub fn pick_inside(
    model: &DocumentModel,
    library: &ComponentLibrary,
    scene: &CanvasScene,
    instance: StableId,
    world_point: Vec2,
) -> Option<InstancePick> {
    let component = payload_of(model, instance)?.component();
    let entry = library.get(component)?;
    let root_row = scene
        .frames
        .iter()
        .find(|f| f.id == instance && f.instance_frame)?;
    let tree = entry.tree();
    let root = tree.tree_root()?;
    let mut best: Option<(Vec<StableId>, StableId, [Vec2; 4])> = None;
    // (node, world, path)
    let mut pending = vec![(root, root_row.world, vec![root])];
    while let Some((id, world, path)) = pending.pop() {
        let node = tree.node(id)?;
        if !node.visible {
            continue;
        }
        let frame = placed_free_frame(node, None)?;
        let world = if id == root {
            world
        } else {
            frame.transform.then(&world).ok()?
        };
        let (w, h) = (frame.size.width(), frame.size.height());
        let local = world.inverse().and_then(|inv| inv.apply(world_point).ok());
        let inside = local.is_some_and(|p| p.x() >= 0. && p.y() >= 0. && p.x() <= w && p.y() <= h);
        if inside {
            best = Some((path.clone(), id, corners(&world, w, h)?));
        }
        if !inside && !matches!(frame.clipping, Clipping::Disabled) {
            continue;
        }
        // 그리기 순서(앞 형제 먼저)대로 보도록 뒤 형제부터 쌓습니다.
        for child in tree.children(id).iter().rev() {
            let mut child_path = path.clone();
            child_path.push(*child);
            pending.push((*child, world, child_path));
        }
    }
    let (path, node, outline) = best?;
    Some(InstancePick {
        instance,
        component,
        path,
        node,
        outline,
    })
}

/// 정의 node 속성과 공개 Property의 연결: 이 정의 node의 `key` 속성 값이 Property `property`를 그대로 씁니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyLink {
    /// 정의 node.
    pub node: StableId,
    /// 속성.
    pub key: PropertyKey,
    /// 연결된 Component Property.
    pub property: ComponentPropertyId,
}

/// 인스턴스 편집을 적용하지 않은 이유.
#[derive(Debug, Clone, PartialEq)]
pub enum InstanceEditRefusal {
    /// 대상이 ComponentInstance가 아닙니다.
    NotInstance,
    /// 인스턴스나 조상이 잠겼습니다.
    Locked,
    /// 참조 컴포넌트가 library에 없습니다.
    UnresolvedComponent,
    /// 정의에 없는 node입니다.
    UnknownNode,
    /// 정의 node 속성이 공개 Property에 연결되지 않았습니다(정의 고유 값).
    NotLinked,
    /// nested instance 안 값입니다(상위 정의의 drilling Property로만 바꿉니다, IX.4.2).
    NestedInstance,
    /// override 규칙 위반(비공개 Property, 정의에 없는 Property, 값 타입 불일치).
    Rule(PropertyOverrideError),
    /// 되돌릴 override가 없습니다.
    NotOverridden,
    /// 인스턴스 컴포넌트가 선언하지 않은 Slot입니다.
    UnknownSlot,
    /// Slot 종류·개수 규칙 위반(IX.3.1).
    SlotRule(SlotError),
    /// 가려진 Slot(override된 바깥 Slot 안)에는 content를 둘 수 없습니다(IX.3.1).
    HiddenSlot(SlotOverrideError),
    /// 옮길 수 없는 content입니다(없는 node, View root, 인스턴스 자신·조상, 잠김, 그 Slot의 content가 아님).
    ContentNotMovable,
    /// Detach 계획 실패(IX.2.6 규칙).
    Detach(DetachPlanError),
}

/// 거부 결과: 이유와 Detach로 자유 편집할 수 있는지.
#[derive(Debug, Clone, PartialEq)]
pub struct Refused {
    /// 이유.
    pub reason: InstanceEditRefusal,
    /// Detach(IX.2.6)로 실제 node로 풀어 편집할 수 있는지.
    pub detach: bool,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match &self.reason {
            InstanceEditRefusal::NotInstance => "ComponentInstance가 아닙니다".to_string(),
            InstanceEditRefusal::Locked => "잠긴 인스턴스는 편집할 수 없습니다".to_string(),
            InstanceEditRefusal::UnresolvedComponent => {
                "컴포넌트 정의를 찾을 수 없습니다".to_string()
            }
            InstanceEditRefusal::UnknownNode => "정의에 없는 node입니다".to_string(),
            InstanceEditRefusal::NotLinked => {
                "공개 Property에 연결되지 않은 값은 인스턴스에서 바꿀 수 없습니다".to_string()
            }
            InstanceEditRefusal::NestedInstance => {
                "nested instance 안 값은 직접 바꿀 수 없습니다(drilling Property 필요)".to_string()
            }
            InstanceEditRefusal::Rule(error) => error.to_string(),
            InstanceEditRefusal::NotOverridden => "되돌릴 override가 없습니다".to_string(),
            InstanceEditRefusal::UnknownSlot => "컴포넌트에 없는 Slot입니다".to_string(),
            InstanceEditRefusal::SlotRule(error) => error.to_string(),
            InstanceEditRefusal::HiddenSlot(error) => error.to_string(),
            InstanceEditRefusal::ContentNotMovable => {
                "이 node는 Slot으로 옮기거나 Slot에서 꺼낼 수 없습니다".to_string()
            }
            InstanceEditRefusal::Detach(error) => error.to_string(),
        };
        if self.detach {
            write!(
                f,
                "{reason}. Detach Instance로 풀면 자유롭게 편집할 수 있습니다"
            )
        } else {
            f.write_str(&reason)
        }
    }
}

impl std::error::Error for Refused {}

/// Detach(IX.2.6)할 수 있는지: 잠기지 않은 인스턴스이고 정의가 있음.
pub fn detach_available(
    model: &DocumentModel,
    library: &ComponentLibrary,
    instance: StableId,
) -> bool {
    payload_of(model, instance).is_some_and(|p| library.get(p.component()).is_some())
        && edit_permission(model, instance, EditIntent::Edit).is_ok()
}

/// 편집 가능한 인스턴스와 정의.
fn editable<'a>(
    model: &'a DocumentModel,
    library: &'a ComponentLibrary,
    instance: StableId,
) -> Result<(&'a ComponentInstancePayload, &'a ComponentEntry), Refused> {
    let refuse = |reason| Refused {
        reason,
        detach: detach_available(model, library, instance),
    };
    let payload = payload_of(model, instance).ok_or(refuse(InstanceEditRefusal::NotInstance))?;
    edit_permission(model, instance, EditIntent::Edit)
        .map_err(|_| refuse(InstanceEditRefusal::Locked))?;
    let entry = library
        .get(payload.component())
        .ok_or(refuse(InstanceEditRefusal::UnresolvedComponent))?;
    Ok((payload, entry))
}

fn swap(instance: StableId, payload: ComponentInstancePayload) -> Command {
    Command::SwapInstance(SwapInstance {
        node: instance,
        payload,
    })
}

/// 인스턴스 안 정의 node `node`의 `key` 속성을 `value`로 바꾸는 편집. 공개 Property에 연결된 값이면 그 Property의
/// override Set command(인스턴스 payload 교체 하나)이고, 아니면 이유와 Detach 경로를 돌려줍니다.
pub fn plan_inner_edit(
    model: &DocumentModel,
    library: &ComponentLibrary,
    links: &[PropertyLink],
    instance: StableId,
    node: StableId,
    key: PropertyKey,
    value: TypedValue,
) -> Result<Command, Refused> {
    let (payload, entry) = editable(model, library, instance)?;
    let refuse = |reason| Refused {
        reason,
        detach: true,
    };
    let tree = entry.tree();
    let mut current = Some(
        tree.node(node)
            .ok_or(refuse(InstanceEditRefusal::UnknownNode))?,
    );
    while let Some(n) = current {
        if n.kind == NodeKind::ComponentInstance {
            return Err(refuse(InstanceEditRefusal::NestedInstance));
        }
        current = n.parent.and_then(|p| tree.node(p));
    }
    let link = links
        .iter()
        .find(|l| l.node == node && l.key == key)
        .ok_or(refuse(InstanceEditRefusal::NotLinked))?;
    let override_key = OverrideKey::direct(link.property);
    check_override(entry, instance, &override_key, &value)
        .map_err(|e| refuse(InstanceEditRefusal::Rule(e)))?;
    let mut next = payload.clone();
    next.set_property_override(override_key, value);
    Ok(swap(instance, next))
}

/// 속성 하나의 override Reset(정의 기본값으로).
pub fn plan_reset(
    model: &DocumentModel,
    library: &ComponentLibrary,
    instance: StableId,
    property: ComponentPropertyId,
) -> Result<Command, Refused> {
    let (payload, _) = editable(model, library, instance)?;
    let mut next = payload.clone();
    next.remove_property_override(&OverrideKey::direct(property))
        .ok_or(Refused {
            reason: InstanceEditRefusal::NotOverridden,
            detach: true,
        })?;
    Ok(swap(instance, next))
}

/// 모든 Property override Reset. Slot content는 인스턴스 소유 node라 그대로 둡니다.
pub fn plan_reset_all(
    model: &DocumentModel,
    library: &ComponentLibrary,
    instance: StableId,
) -> Result<Command, Refused> {
    let (payload, _) = editable(model, library, instance)?;
    if payload.property_overrides().next().is_none() {
        return Err(Refused {
            reason: InstanceEditRefusal::NotOverridden,
            detach: true,
        });
    }
    let mut next = ComponentInstancePayload::new(payload.component());
    for (slot, content) in payload.slot_overrides() {
        next.set_slot_override(slot, content.clone());
    }
    Ok(swap(instance, next))
}

/// 인스턴스 문맥 동작의 사용 가능 여부(문맥 메뉴·Object 메뉴 코어).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceActions {
    /// 속성별 Reset 대상(override가 있는 직접 Property, id 순).
    pub reset_properties: Vec<ComponentPropertyId>,
    /// 전체 Reset 가능.
    pub reset_all: bool,
    /// Detach 가능.
    pub detach: bool,
    /// 정의로 이동 대상(컴포넌트와 정의 root). 정의가 없으면 `None`.
    pub definition: Option<(ComponentRef, NodeRef)>,
}

/// 인스턴스 하나의 문맥 동작. ComponentInstance가 아니면 `None`.
pub fn instance_actions(
    model: &DocumentModel,
    library: &ComponentLibrary,
    instance: StableId,
) -> Option<InstanceActions> {
    let payload = payload_of(model, instance)?;
    let editable = edit_permission(model, instance, EditIntent::Edit).is_ok();
    let entry = library.get(payload.component());
    let reset_properties: Vec<ComponentPropertyId> = if editable && entry.is_some() {
        payload
            .property_overrides()
            .filter(|(key, _)| key.is_direct())
            .map(|(key, _)| key.property())
            .collect()
    } else {
        Vec::new()
    };
    Some(InstanceActions {
        reset_all: editable && entry.is_some() && payload.property_overrides().next().is_some(),
        reset_properties,
        detach: detach_available(model, library, instance),
        definition: entry.map(|e| (payload.component(), e.definition().root)),
    })
}

/// 인스턴스 편집 Object 동작. 메뉴 막대 Object 메뉴·캔버스 문맥 메뉴·단축키 정책이 [`InstanceAction::name`]을 공유
/// command ID로 씁니다(ADR-0129 결정 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InstanceAction {
    /// 모든 Property override Reset(Reset Overrides).
    ResetOverrides,
    /// Property 하나의 override Reset(문맥 메뉴 하위 항목, 대상은 [`InstanceActions::reset_properties`]).
    ResetOverride,
    /// Detach Instance.
    DetachInstance,
    /// 정의로 이동.
    GoToDefinition,
}

impl InstanceAction {
    /// 전체 동작(메뉴 순서).
    pub const ALL: [InstanceAction; 4] = [
        InstanceAction::ResetOverrides,
        InstanceAction::ResetOverride,
        InstanceAction::DetachInstance,
        InstanceAction::GoToDefinition,
    ];

    /// 메뉴 명령과 공유하는 command ID.
    pub const fn name(self) -> &'static str {
        match self {
            InstanceAction::ResetOverrides => "object.reset_overrides",
            InstanceAction::ResetOverride => "object.reset_override",
            InstanceAction::DetachInstance => "object.detach_instance",
            InstanceAction::GoToDefinition => "object.go_to_definition",
        }
    }

    /// command ID로 찾습니다.
    pub fn from_name(name: &str) -> Option<InstanceAction> {
        InstanceAction::ALL.into_iter().find(|a| a.name() == name)
    }
}

impl InstanceActions {
    /// 동작을 지금 쓸 수 있는지(메뉴 항목 활성).
    pub fn enabled(&self, action: InstanceAction) -> bool {
        match action {
            InstanceAction::ResetOverrides => self.reset_all,
            InstanceAction::ResetOverride => !self.reset_properties.is_empty(),
            InstanceAction::DetachInstance => self.detach,
            InstanceAction::GoToDefinition => self.definition.is_some(),
        }
    }
}

/// 인스턴스 Detach 실행 command(`detach-instance`, IX.2.6). 잠긴 인스턴스·미해결 정의는 거부합니다. 복제 node id는
/// `generator`·`clock`이 정합니다.
pub fn plan_detach_instance<C: MonotonicClock>(
    model: &DocumentModel,
    library: &ComponentLibrary,
    instance: StableId,
    generator: &mut StableIdGenerator,
    clock: &C,
) -> Result<Command, Refused> {
    editable(model, library, instance)?;
    plan_detach(model, library, instance, generator, clock)
        .map(|plan| Command::DetachInstance(Box::new(plan)))
        .map_err(|error| Refused {
            reason: InstanceEditRefusal::Detach(error),
            detach: false,
        })
}

/// Slot에 넣을 content.
#[derive(Debug, Clone, PartialEq)]
pub enum SlotContent {
    /// 문서에 있는 node를 옮깁니다(캔버스·레이어에서 끌어 놓기).
    Existing(StableId),
    /// 새 subtree를 만듭니다(전위 순서, 첫 레코드가 content root이고 부모는 인스턴스).
    New {
        /// node 레코드.
        nodes: Vec<NodeRecord>,
        /// 만들 node의 modifier stack.
        modifier_stacks: Vec<ModifierStack>,
        /// 만들 node의 명시 layout record(소유 node StableId 순).
        layout_records: Vec<LayoutRecord>,
    },
}

fn not_movable() -> Refused {
    Refused {
        reason: InstanceEditRefusal::ContentNotMovable,
        detach: false,
    }
}

/// `node`가 `instance` 자신이거나 그 조상인지.
fn is_self_or_ancestor(model: &DocumentModel, node: StableId, instance: StableId) -> bool {
    let mut current = Some(instance);
    while let Some(id) = current {
        if id == node {
            return true;
        }
        current = model.record(id).and_then(|r| r.parent);
    }
    false
}

/// 만들 node·옮길 node 목록.
type SlotInsert = (Vec<NodeRecord>, Vec<ModifierStack>, Vec<LayoutRecord>);

/// 새 payload가 Slot 규칙(종류·개수·가림)을 지키는지 확인하고 `place-slot-content` command를 만듭니다.
fn slot_command(
    model: &DocumentModel,
    entry: &ComponentEntry,
    instance: StableId,
    payload: ComponentInstancePayload,
    kinds: &dyn Fn(StableId) -> Option<NodeKind>,
    insert: SlotInsert,
    moves: Vec<NodePlacement>,
) -> Result<Command, Refused> {
    let refuse = |reason| Refused {
        reason,
        detach: true,
    };
    for (slot_id, content) in payload.slot_overrides() {
        let slot = entry
            .slot(slot_id)
            .ok_or(refuse(InstanceEditRefusal::UnknownSlot))?;
        let mut checked = Vec::with_capacity(content.len());
        for node in content.content() {
            checked.push((*node, kinds(*node).ok_or_else(not_movable)?));
        }
        slot.check_content(checked)
            .map_err(|e| refuse(InstanceEditRefusal::SlotRule(e)))?;
    }
    check_hidden_slot_content(entry, &payload, instance)
        .map_err(|e| refuse(InstanceEditRefusal::HiddenSlot(e)))?;
    let mut record = model.record(instance).ok_or_else(not_movable)?;
    record.payload = NodePayload::ComponentInstance(payload);
    let (insert, insert_stacks, insert_layouts) = insert;
    Ok(Command::PlaceSlotContent(Box::new(InstanceRestructure {
        node: instance,
        record,
        layout: model
            .layout_record(instance)
            .cloned()
            .unwrap_or_else(|| LayoutRecord::new(instance)),
        insert,
        insert_stacks,
        insert_layouts,
        moves,
        remove: Vec::new(),
    })))
}

/// 공개 Slot(인스턴스 컴포넌트가 선언한 Slot) `slot`의 `index` 자리(투영 순서, 개수보다 크면 끝)에 content를 넣는
/// command 하나(끌어 놓기, history 1건). override가 없던 Slot은 이 content 하나짜리 override가 됩니다(default content
/// 대신 보임). 같은 인스턴스의 다른 Slot에 있던 content면 그 Slot에서 빠집니다.
pub fn plan_slot_drop(
    model: &DocumentModel,
    library: &ComponentLibrary,
    instance: StableId,
    slot: ComponentSlotId,
    index: usize,
    content: SlotContent,
) -> Result<Command, Refused> {
    let (payload, entry) = editable(model, library, instance)?;
    if entry.slot(slot).is_none() {
        return Err(Refused {
            reason: InstanceEditRefusal::UnknownSlot,
            detach: true,
        });
    }
    let mut next = payload.clone();
    let (root, insert, moves): (StableId, SlotInsert, Vec<NodePlacement>) = match content {
        SlotContent::Existing(node) => {
            let record = model.record(node).ok_or_else(not_movable)?;
            let parent = record.parent.ok_or_else(not_movable)?;
            if is_self_or_ancestor(model, node, instance)
                || edit_permission(model, node, EditIntent::Move).is_err()
                || edit_permission(model, parent, EditIntent::Edit).is_err()
            {
                return Err(not_movable());
            }
            let moves = if parent == instance {
                // 같은 인스턴스 안 이동: 원래 Slot에서 뺍니다(부모는 그대로).
                let from: Vec<_> = next
                    .slot_overrides()
                    .filter(|(_, c)| c.content().contains(&node))
                    .map(|(id, c)| (id, c.content().to_vec()))
                    .collect();
                for (id, list) in from {
                    let kept = list.into_iter().filter(|n| *n != node).collect();
                    next.set_slot_override(id, SlotOverride::new(kept).map_err(|_| not_movable())?);
                }
                Vec::new()
            } else {
                vec![NodePlacement {
                    node,
                    parent: instance,
                    order_key: model.next_child_order_key(instance),
                }]
            };
            (node, (Vec::new(), Vec::new(), Vec::new()), moves)
        }
        SlotContent::New {
            nodes,
            modifier_stacks,
            layout_records,
        } => {
            let root = nodes.first().ok_or_else(not_movable)?;
            if root.parent != Some(instance) {
                return Err(not_movable());
            }
            (
                root.id,
                (nodes, modifier_stacks, layout_records),
                Vec::new(),
            )
        }
    };
    let mut list = next
        .slot_override(slot)
        .map(|c| c.content().to_vec())
        .unwrap_or_default();
    list.insert(index.min(list.len()), root);
    next.set_slot_override(slot, SlotOverride::new(list).map_err(|_| not_movable())?);
    let new_kinds: Vec<(StableId, NodeKind)> = insert.0.iter().map(|r| (r.id, r.kind())).collect();
    let kinds = |id: StableId| {
        new_kinds
            .iter()
            .find(|(n, _)| *n == id)
            .map(|(_, k)| *k)
            .or_else(|| model.record(id).map(|r| r.kind()))
    };
    slot_command(model, entry, instance, next, &kinds, insert, moves)
}

/// Slot content 하나를 Slot에서 꺼내 `parent` 아래 `order_key` 자리로 옮기는 command 하나(끌어 내기). 마지막 content를
/// 빼면 `restore_default`일 때 override를 지워 default content로 돌아가고, 아니면 빈 override(Slot 비움)로 남깁니다.
pub fn plan_slot_take_out(
    model: &DocumentModel,
    library: &ComponentLibrary,
    instance: StableId,
    node: StableId,
    parent: StableId,
    order_key: OrderKey,
    restore_default: bool,
) -> Result<Command, Refused> {
    let (payload, entry) = editable(model, library, instance)?;
    let (slot, list) = payload
        .slot_overrides()
        .find(|(_, c)| c.content().contains(&node))
        .map(|(id, c)| (id, c.content().to_vec()))
        .ok_or_else(not_movable)?;
    if edit_permission(model, node, EditIntent::Move).is_err()
        || edit_permission(model, parent, EditIntent::Edit).is_err()
    {
        return Err(not_movable());
    }
    let kept: Vec<StableId> = list.into_iter().filter(|n| *n != node).collect();
    let mut next = payload.clone();
    if kept.is_empty() && restore_default {
        next.remove_slot_override(slot);
    } else {
        next.set_slot_override(slot, SlotOverride::new(kept).map_err(|_| not_movable())?);
    }
    let kinds = |id: StableId| model.record(id).map(|r| r.kind());
    slot_command(
        model,
        entry,
        instance,
        next,
        &kinds,
        (Vec::new(), Vec::new(), Vec::new()),
        vec![NodePlacement {
            node,
            parent,
            order_key,
        }],
    )
}

#[cfg(test)]
#[path = "instance_tests.rs"]
mod tests;
