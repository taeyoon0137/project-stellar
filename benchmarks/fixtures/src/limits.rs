//! 규모 상한 fixture: 가시 node 2만, 컴포넌트 인스턴스 5만, 중첩 64 (XVI.3.2).
//!
//! 아키텍처 §5.3의 "한 프레임의 가시 노드 20,000", "활성 문서의 컴포넌트 인스턴스 50,000",
//! "컴포넌트 중첩 깊이 64"를 각각 따로 만듭니다. 각 fixture는 해당 validator를 통과합니다.
//!
//! - [`visible_nodes`]: 문서 fixture. viewport 안 **유효 가시**(자신과 모든 조상의 visible이
//!   true이고 world bounds가 viewport와 겹침) node가 정확히 목표 수이고, 숨김 subtree와 viewport
//!   밖 subtree를 함께 둡니다. 문서 validator(VI.10.1)를 통과합니다.
//! - [`component_instances`]: 컴포넌트 fixture. 영속 문서 레코드에는 아직 ComponentInstance
//!   payload variant가 없으므로(VI.10), IX.2 scene 표현(`NodeColumns` + `ScenePayloads` +
//!   `ComponentLibrary`)으로 만듭니다. library 등록 검증(IX.1.1)과 모든 인스턴스의 expansion
//!   (IX.2.2, 미해결·재귀·중첩 상한 검사)을 통과합니다.
//! - [`nesting_chain`]: 컴포넌트 인스턴스 chain 깊이(scene 인스턴스 포함)가 정확히 지정 값인
//!   fixture와, 문서 node depth chain([`deep_document`])입니다. 64가 상한이고 65는 거부됩니다.

use std::collections::{BTreeMap, HashMap, HashSet};

use stellar_core_types::ids::{RuntimeEntityId, StableId};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{AssetRef, ComponentRef, NodeRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentDefinitionError, ComponentLibrary, ComponentPropertyId,
    DefinitionNode, DefinitionTree,
};
use stellar_document::component::expansion::{
    ExpandedNodeId, InstanceExpansion, SceneIndex, ScenePayloads,
};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey};
use stellar_document::component::property::{ComponentProperty, PropertyVisibility};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::node_columns::{NodeColumns, NodeFields};
use stellar_document::node_kind::NodeKind;
use stellar_document::path::PathCommand;
use stellar_document::record_codec::encode_payload;
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::shape::{EllipseParams, RectangleParams, ShapeGeometry};
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst};
use stellar_expressions::types::{ExpressionType, TypeSpec};
use stellar_storage::hash::ContentHash;

use crate::build::{self, finish_document, BuiltDocument, Quarter};
use crate::digest::FixtureDigest;
use crate::error::FixtureError;
use crate::ids::IdAllocator;
use crate::rng::SplitMix64;

/// 한 프레임의 가시 node 목표(아키텍처 §5.3).
pub const VISIBLE_NODES_TARGET: usize = 20_000;

/// 활성 문서의 컴포넌트 인스턴스 목표(아키텍처 §5.3).
pub const COMPONENT_INSTANCES_TARGET: usize = 50_000;

/// 컴포넌트 중첩 깊이 목표(아키텍처 §5.3). 문서 node depth 상한과 같은 값입니다.
pub const NESTING_TARGET: usize = 64;

/// 가시 fixture 목표 수 상한(오입력 방지).
pub const MAX_VISIBLE_TARGET: usize = 1_000_000;

/// 인스턴스 fixture 목표 수 상한(오입력 방지).
pub const MAX_INSTANCE_TARGET: usize = 1_000_000;

/// chain fixture 깊이 상한(상한 초과 거부를 확인하는 음성 대조용으로 64보다 크게 허용).
pub const MAX_CHAIN_LEVELS: usize = 256;

// ---------------------------------------------------------------------------
// 가시 node fixture
// ---------------------------------------------------------------------------

/// viewport 크기: 1920 × 1080 px.
pub const VIEWPORT_W: f32 = 1920.0;
/// viewport 높이.
pub const VIEWPORT_H: f32 = 1080.0;

/// 격자 cell 크기(px). card는 cell 안쪽 2 px 여백을 둡니다.
const CELL: Quarter = 20 * 4;
const CARD: Quarter = 18 * 4;
/// card 하나의 node 수(card Frame + leaf 4).
const CARD_NODES: usize = 5;
/// 숨김 section과 viewport 밖 section의 card 수.
const HIDDEN_CARDS: usize = 200;
const OFFSCREEN_CARDS: usize = 400;

/// 가시 node fixture.
#[derive(Debug)]
pub struct VisibleFixture {
    /// 문서(validator 통과).
    pub built: BuiltDocument,
    /// 측정 대상 View.
    pub view: StableId,
    /// View root Frame.
    pub root: StableId,
    /// viewport(View root 좌표계).
    pub viewport: Rect,
    /// 구성상 유효 가시 node 수(= 목표).
    pub visible_nodes: usize,
    /// 숨김 subtree node 수(section 포함).
    pub hidden_nodes: usize,
    /// viewport 밖 subtree node 수(section 포함).
    pub offscreen_nodes: usize,
    /// content hash.
    pub content_hash: ContentHash,
}

/// 가시 node가 정확히 `target`개인 문서 fixture를 만듭니다.
///
/// View root(viewport 크기 Frame) 아래 격자로 card Frame을 놓고, card 안에 배경 Rectangle·
/// Ellipse·Image·Path leaf를 둡니다. 모든 geometry가 card 안, 모든 card가 viewport 안이라 가시
/// 판정이 위치에 좌우되지 않습니다. 가시 판정을 흐리는 Text(bounds가 텍스트 레이아웃에 의존)와
/// Fragment(자체 geometry 없음)는 쓰지 않습니다. 숨김 section(visible=false)과 viewport 밖
/// section은 culling 대상입니다.
pub fn visible_nodes(seed: u64, target: usize) -> Result<VisibleFixture, FixtureError> {
    if target == 0 || target > MAX_VISIBLE_TARGET {
        return Err(FixtureError::InvalidSpec {
            reason: "가시 node 목표는 1..=1,000,000이어야 합니다",
        });
    }
    let cols = (VIEWPORT_W as Quarter * 4) / CELL;
    let rows = (VIEWPORT_H as Quarter * 4) / CELL;
    // root 1 + card(5 node)들. 마지막 card는 남은 수만큼만.
    let cards = (target - 1).div_ceil(CARD_NODES);
    if cards > (cols * rows) as usize {
        return Err(FixtureError::InvalidSpec {
            reason: "가시 node 목표가 viewport 격자 용량을 넘습니다",
        });
    }

    let mut rng = SplitMix64::new(seed ^ 0x7151_B1E0_0000_0002);
    let mut ids = IdAllocator::new(seed ^ 0x7151_B1E0);
    let document = ids.next_id();
    let view = ids.next_id();
    let root = ids.next_id();
    let mut definition = DocumentDefinition::new(document, "Visible 20k".to_string());
    let mut page = PageDefinition::new(ids.next_id(), "Dashboard".to_string());
    page.add_view(ViewDefinition::new(view, "Viewport".to_string(), root))
        .map_err(FixtureError::Definition)?;
    definition
        .add_page(page)
        .map_err(FixtureError::Definition)?;
    let assets: Vec<AssetRef> = (0..16).map(|_| AssetRef::new(ids.next_id())).collect();

    let mut records = DocumentRecords::new(definition);
    let mut root_payload = FramePayload {
        size: build::size(VIEWPORT_W as Quarter * 4, VIEWPORT_H as Quarter * 4),
        ..FramePayload::default()
    };
    root_payload.background = Some(build::color(&mut rng));
    records.nodes.push(build::record(
        root,
        None,
        OrderKey::between(None, None),
        true,
        "Viewport",
        NodePayload::Frame(root_payload),
    ));

    // root 자식: 가시 card들 → 숨김 section → viewport 밖 section.
    let keys = OrderKey::evenly_spaced(cards + 2);
    let mut remaining = target - 1;
    for (i, key) in keys.iter().take(cards).enumerate() {
        let col = i as Quarter % cols;
        let row = i as Quarter / cols;
        let nodes = remaining.min(CARD_NODES);
        remaining -= nodes;
        push_card(
            &mut rng,
            &mut ids,
            &assets,
            &mut records,
            root,
            key.clone(),
            col * CELL,
            row * CELL,
            nodes,
        );
    }

    // 숨김 section: viewport 안이지만 visible=false.
    let hidden_section = ids.next_id();
    records.nodes.push(build::record(
        hidden_section,
        Some(root),
        keys[cards].clone(),
        false,
        "Hidden",
        NodePayload::Frame(FramePayload {
            size: build::size(VIEWPORT_W as Quarter * 4, VIEWPORT_H as Quarter * 4),
            ..FramePayload::default()
        }),
    ));
    let hidden_keys = OrderKey::evenly_spaced(HIDDEN_CARDS);
    for (i, key) in hidden_keys.into_iter().enumerate() {
        let i = i as Quarter;
        push_card(
            &mut rng,
            &mut ids,
            &assets,
            &mut records,
            hidden_section,
            key,
            (i % cols) * CELL,
            (i / cols) * CELL,
            CARD_NODES,
        );
    }

    // viewport 밖 section: visible=true, 오른쪽 바깥.
    let offscreen_section = ids.next_id();
    let offscreen_x = VIEWPORT_W as Quarter * 4 + 400 * 4;
    records.nodes.push(build::record(
        offscreen_section,
        Some(root),
        keys[cards + 1].clone(),
        true,
        "Offscreen",
        NodePayload::Frame(FramePayload {
            size: build::size(VIEWPORT_W as Quarter * 4, VIEWPORT_H as Quarter * 4),
            transform: build::translation(offscreen_x, 0),
            ..FramePayload::default()
        }),
    ));
    let offscreen_keys = OrderKey::evenly_spaced(OFFSCREEN_CARDS);
    for (i, key) in offscreen_keys.into_iter().enumerate() {
        let i = i as Quarter;
        push_card(
            &mut rng,
            &mut ids,
            &assets,
            &mut records,
            offscreen_section,
            key,
            (i % cols) * CELL,
            (i / cols) * CELL,
            CARD_NODES,
        );
    }

    let built = finish_document(records)?;
    let mut digest = FixtureDigest::new("visible-nodes", seed);
    digest.parameter("target", target as u64);
    digest.document(&built.serialized);
    Ok(VisibleFixture {
        built,
        view,
        root,
        viewport: Rect::new(
            Vec2::ZERO,
            Size2::new(VIEWPORT_W, VIEWPORT_H).unwrap_or(Size2::ZERO),
        ),
        visible_nodes: target,
        hidden_nodes: 1 + HIDDEN_CARDS * CARD_NODES,
        offscreen_nodes: 1 + OFFSCREEN_CARDS * CARD_NODES,
        content_hash: digest.finish(),
    })
}

/// card Frame 하나와 leaf `nodes - 1`개(최대 4)를 넣습니다.
#[allow(clippy::too_many_arguments)]
fn push_card(
    rng: &mut SplitMix64,
    ids: &mut IdAllocator,
    assets: &[AssetRef],
    records: &mut DocumentRecords,
    parent: StableId,
    key: OrderKey,
    x: Quarter,
    y: Quarter,
    nodes: usize,
) {
    let card = ids.next_id();
    let mut payload = build::frame(rng, CARD, CARD, x, y);
    payload.corner_radius = build::uniform_radius(8);
    records.nodes.push(build::record(
        card,
        Some(parent),
        key,
        true,
        "Cell",
        NodePayload::Frame(payload),
    ));
    let leaves = nodes.saturating_sub(1).min(CARD_NODES - 1);
    let keys = OrderKey::evenly_spaced(leaves);
    for (i, key) in keys.into_iter().enumerate() {
        let payload = match i {
            0 => NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: build::size(CARD, CARD),
                corner_radius: build::uniform_radius(8),
            })),
            1 => {
                let d = 8 * 4 + build::quarter_below(rng, 4 * 4);
                NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
                    bounds: Rect::new(build::vec2(4, 4), build::size(d, d)),
                }))
            }
            2 => {
                let asset = assets[rng.below(assets.len() as u64) as usize];
                NodePayload::Image(build::image(rng, asset, CARD / 2, CARD / 2))
            }
            _ => NodePayload::Path(build::path(rng, ids, CARD, CARD)),
        };
        let id = ids.next_id();
        records
            .nodes
            .push(build::record(id, Some(card), key, true, "Mark", payload));
        if rng.one_in(3) {
            let modifier =
                build::plain_modifier(rng, stellar_document::modifier::ModifierKind::Fill);
            records
                .modifier_stacks
                .push(build::modifier_stack(ids, id, vec![modifier]));
        }
    }
}

/// local 좌표의 node geometry bounds(축 정렬). Text·Fragment·빈 path는 `None`입니다.
fn local_bounds(payload: &NodePayload) -> Option<(Vec2, Vec2)> {
    let rect = |origin: Vec2, size: Size2| {
        Some((
            origin,
            Vec2::new(origin.x() + size.width(), origin.y() + size.height()).ok()?,
        ))
    };
    match payload {
        NodePayload::Frame(frame) => rect(Vec2::ZERO, frame.size),
        NodePayload::Image(image) => rect(Vec2::ZERO, image.size),
        NodePayload::Shape(ShapeGeometry::Rectangle(params)) => rect(Vec2::ZERO, params.size),
        NodePayload::Shape(ShapeGeometry::Ellipse(params)) => {
            rect(params.bounds.origin(), params.bounds.size())
        }
        NodePayload::Shape(ShapeGeometry::Polygon(params)) => {
            rect(params.bounds().origin(), params.bounds().size())
        }
        NodePayload::Shape(ShapeGeometry::Line(params)) => Some((
            Vec2::new(
                params.start.x().min(params.end.x()),
                params.start.y().min(params.end.y()),
            )
            .ok()?,
            Vec2::new(
                params.start.x().max(params.end.x()),
                params.start.y().max(params.end.y()),
            )
            .ok()?,
        )),
        NodePayload::Path(path) => {
            let mut points = Vec::new();
            for command in path.data().commands() {
                match command {
                    PathCommand::MoveTo(p) | PathCommand::LineTo(p) => points.push(*p),
                    PathCommand::CubicTo { c1, c2, to } => points.extend([*c1, *c2, *to]),
                    PathCommand::Close => {}
                }
            }
            aabb(&points)
        }
        NodePayload::Text(_) | NodePayload::Fragment(_) | NodePayload::ComponentInstance(_) => None,
    }
}

fn aabb(points: &[Vec2]) -> Option<(Vec2, Vec2)> {
    let first = points.first()?;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (first.x(), first.y(), first.x(), first.y());
    for p in points {
        min_x = min_x.min(p.x());
        min_y = min_y.min(p.y());
        max_x = max_x.max(p.x());
        max_y = max_y.max(p.y());
    }
    Some((Vec2::new(min_x, min_y).ok()?, Vec2::new(max_x, max_y).ok()?))
}

/// 변환 후 축 정렬 bounds.
fn transformed(transform: &Transform2D, (min, max): (Vec2, Vec2)) -> Option<(Vec2, Vec2)> {
    let corners = [
        Vec2::new(min.x(), min.y()).ok()?,
        Vec2::new(max.x(), min.y()).ok()?,
        Vec2::new(min.x(), max.y()).ok()?,
        Vec2::new(max.x(), max.y()).ok()?,
    ];
    let mapped: Option<Vec<Vec2>> = corners.iter().map(|c| transform.apply(*c).ok()).collect();
    aabb(&mapped?)
}

fn intersects(bounds: (Vec2, Vec2), viewport: &Rect) -> bool {
    let (min, max) = bounds;
    let v_min = viewport.origin();
    let v_max_x = v_min.x() + viewport.size().width();
    let v_max_y = v_min.y() + viewport.size().height();
    // 경계에 닿는 경우도 겹침으로 봅니다(크기 0 geometry가 viewport 경계에 있을 때).
    min.x() <= v_max_x && max.x() >= v_min.x() && min.y() <= v_max_y && max.y() >= v_min.y()
}

/// View root 아래에서 viewport와 겹치는 유효 가시 node 수를 셉니다(fixture 독립 검산).
///
/// 자신과 모든 조상의 visible이 true이고, 부모 world 변환(Frame의 transform 누적)을 적용한
/// geometry bounds가 viewport와 겹치는 node를 셉니다. bounds가 없는 node(Text·Fragment·빈 path)는
/// 세지 않습니다. 영속 레코드로 부모→자식 index를 한 번 만들고(O(N)) 반복(비재귀) 순회합니다.
pub fn count_visible_in_viewport(
    model: &DocumentModel,
    root: StableId,
    viewport: &Rect,
) -> Result<usize, FixtureError> {
    let records = model.to_records().map_err(FixtureError::Model)?;
    let mut by_id: HashMap<StableId, &NodeRecord> = HashMap::with_capacity(records.nodes.len());
    let mut children: HashMap<StableId, Vec<StableId>> = HashMap::new();
    for node in &records.nodes {
        by_id.insert(node.id, node);
        if let Some(parent) = node.parent {
            children.entry(parent).or_default().push(node.id);
        }
    }
    let mut count = 0usize;
    // (node, 부모 world 변환)
    let mut stack: Vec<(StableId, Transform2D)> = vec![(root, Transform2D::identity())];
    while let Some((id, parent_world)) = stack.pop() {
        let Some(record) = by_id.get(&id) else {
            continue;
        };
        if !record.visible {
            continue;
        }
        let world = match &record.payload {
            NodePayload::Frame(frame) => frame.transform.then(&parent_world).ok(),
            _ => Some(parent_world),
        };
        let Some(world) = world else {
            continue;
        };
        if local_bounds(&record.payload)
            .and_then(|b| transformed(&world, b))
            .is_some_and(|bounds| intersects(bounds, viewport))
        {
            count += 1;
        }
        for child in children.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
            stack.push((*child, world));
        }
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// 컴포넌트 인스턴스 fixture
// ---------------------------------------------------------------------------

/// 컴포넌트 scene fixture(IX.2 표현).
#[derive(Debug)]
pub struct ComponentFixture {
    /// 컴포넌트 정의 library.
    pub library: ComponentLibrary,
    /// scene node column.
    pub columns: NodeColumns,
    /// scene node payload bytes.
    pub payloads: ScenePayloads,
    /// scene root Frame.
    pub root: StableId,
    /// scene ComponentInstance node(생성 순).
    pub instances: Vec<StableId>,
    /// 인스턴스가 쓸 수 있는 Property(컴포넌트별 선언).
    pub properties: BTreeMap<StableId, Vec<ComponentPropertyId>>,
    /// content hash.
    pub content_hash: ContentHash,
}

/// 컴포넌트 fixture 검증 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentValidation {
    /// 펼친 인스턴스 수.
    pub instances: usize,
    /// library의 정의 node 수.
    pub definition_nodes: usize,
    /// 모든 인스턴스가 노출한 가상 node 수.
    pub virtual_nodes: usize,
    /// 가상 node가 참조한 서로 다른 정의 node 저장소 수(복제 없음 = 정의 node 수 이하).
    pub distinct_definition_nodes: usize,
    /// 가장 긴 인스턴스 chain(scene 인스턴스 포함).
    pub max_instance_chain: usize,
    /// override 수.
    pub property_overrides: usize,
}

fn def_node(
    id: StableId,
    kind: NodeKind,
    parent: Option<StableId>,
    order_key: OrderKey,
    name: &str,
    payload: Vec<u8>,
) -> DefinitionNode {
    DefinitionNode {
        id,
        kind,
        parent,
        order_key,
        visible: true,
        locked: false,
        display_name: name.to_string(),
        payload,
    }
}

/// 정의 tree 조립 도우미: root Frame 아래 자식(종류, payload)들.
struct TreeBuilder<'a> {
    ids: &'a mut IdAllocator,
    component: StableId,
    tree: DefinitionTree,
    root: StableId,
}

impl<'a> TreeBuilder<'a> {
    fn new(
        ids: &'a mut IdAllocator,
        rng: &mut SplitMix64,
        w: Quarter,
        h: Quarter,
    ) -> Result<TreeBuilder<'a>, FixtureError> {
        let component = ids.next_id();
        let root = ids.next_id();
        let mut tree = DefinitionTree::new(ComponentRef::new(component));
        let payload = encode_payload(&NodePayload::Frame(build::frame(rng, w, h, 0, 0)));
        tree.insert(def_node(
            root,
            NodeKind::Frame,
            None,
            OrderKey::between(None, None),
            "root",
            payload,
        ))
        .map_err(component_error)?;
        Ok(TreeBuilder {
            ids,
            component,
            tree,
            root,
        })
    }

    /// root 자식들을 (종류, payload bytes) 순서로 넣습니다.
    fn children(
        &mut self,
        parent: StableId,
        children: Vec<(NodeKind, Vec<u8>)>,
    ) -> Result<Vec<StableId>, FixtureError> {
        let keys = OrderKey::evenly_spaced(children.len());
        let mut out = Vec::with_capacity(children.len());
        for ((kind, payload), key) in children.into_iter().zip(keys) {
            let id = self.ids.next_id();
            self.tree
                .insert(def_node(id, kind, Some(parent), key, kind.name(), payload))
                .map_err(component_error)?;
            out.push(id);
        }
        Ok(out)
    }

    fn finish(
        self,
        library: &mut ComponentLibrary,
        name: &str,
        properties: &[(ComponentPropertyId, &str, TypedValue)],
    ) -> Result<StableId, FixtureError> {
        let mut definition =
            ComponentDefinition::new(self.component, name.to_string(), NodeRef::new(self.root));
        for (property, _, _) in properties {
            definition
                .add_property(*property)
                .map_err(component_error)?;
        }
        library
            .insert(definition, self.tree)
            .map_err(component_error)?;
        // IX.1.2: 선언한 공개 Property마다 이름·타입·닫힌 default 표현식 record가 있어야 합니다.
        for (property, property_name, default) in properties {
            let record = ComponentProperty::new(
                *property,
                (*property_name).to_string(),
                TypeSpec::required(ExpressionType::of(default)),
                literal_default(*property, default.clone()),
                PropertyVisibility::Public,
            )
            .map_err(FixtureError::ComponentProperty)?;
            library
                .add_property(ComponentRef::new(self.component), record)
                .map_err(FixtureError::ComponentProperty)?;
        }
        Ok(self.component)
    }
}

/// Property default용 단일 literal 표현식. node id는 Property id에서 결정론적으로 만듭니다.
fn literal_default(property: ComponentPropertyId, value: TypedValue) -> ExpressionAst {
    let root = ExprNodeId::new(property.stable_id());
    ExpressionAst::new(root, vec![ExprNode::literal(root, value)])
        .expect("단일 literal 표현식은 항상 유효합니다")
}

fn component_error(error: ComponentDefinitionError) -> FixtureError {
    FixtureError::Component(error)
}

fn instance_bytes(component: StableId) -> Vec<u8> {
    ComponentInstancePayload::new(ComponentRef::new(component)).encode()
}

fn shape_bytes(rng: &mut SplitMix64, w: Quarter, h: Quarter) -> (NodeKind, Vec<u8>) {
    (
        NodeKind::Shape,
        encode_payload(&NodePayload::Shape(build::shape(rng, w, h))),
    )
}

fn text_bytes(rng: &mut SplitMix64, words: usize) -> (NodeKind, Vec<u8>) {
    (
        NodeKind::Text,
        encode_payload(&NodePayload::Text(build::text(rng, words, 0))),
    )
}

/// 컴포넌트 id와 그 컴포넌트가 선언한 Property.
type DeclaredComponent = (StableId, Vec<ComponentPropertyId>);

/// 디자인 시스템형 library: Icon, Button(Icon 중첩), Card(Button 중첩), ListRow(Icon 중첩).
/// (컴포넌트 id, 선언 Property) 목록과 library를 반환합니다.
fn design_system_library(
    rng: &mut SplitMix64,
    ids: &mut IdAllocator,
) -> Result<(ComponentLibrary, Vec<DeclaredComponent>), FixtureError> {
    let mut library = ComponentLibrary::new();
    let mut out = Vec::new();

    // Icon: Frame ← Path, Shape.
    let mut icon = TreeBuilder::new(ids, rng, 24 * 4, 24 * 4)?;
    let path = encode_payload(&NodePayload::Path(build::path(
        rng,
        icon.ids,
        24 * 4,
        24 * 4,
    )));
    let root = icon.root;
    let shape = shape_bytes(rng, 24 * 4, 24 * 4);
    icon.children(root, vec![(NodeKind::Path, path), shape])?;
    let icon_id = icon.finish(&mut library, "Icon", &[])?;
    out.push((icon_id, Vec::new()));

    // Button: Frame ← Shape(배경), Text(label), Instance(Icon). Property: label, disabled.
    let label = ComponentPropertyId::new(ids.next_id());
    let disabled = ComponentPropertyId::new(ids.next_id());
    let mut button = TreeBuilder::new(ids, rng, 120 * 4, 40 * 4)?;
    let root = button.root;
    let bg = shape_bytes(rng, 120 * 4, 40 * 4);
    let text = text_bytes(rng, 2);
    button.children(
        root,
        vec![
            bg,
            text,
            (NodeKind::ComponentInstance, instance_bytes(icon_id)),
        ],
    )?;
    let button_id = button.finish(
        &mut library,
        "Button",
        &[
            (label, "label", TypedValue::String("Button".into())),
            (disabled, "disabled", TypedValue::Boolean(false)),
        ],
    )?;
    out.push((button_id, vec![label, disabled]));

    // Card: Frame ← Image, Text(title), Text(body), Frame(actions) ← Instance(Button).
    let title = ComponentPropertyId::new(ids.next_id());
    let mut card = TreeBuilder::new(ids, rng, 320 * 4, 240 * 4)?;
    let root = card.root;
    let asset = AssetRef::new(card.ids.next_id());
    let image = encode_payload(&NodePayload::Image(build::image(
        rng,
        asset,
        320 * 4,
        160 * 4,
    )));
    let title_text = text_bytes(rng, 3);
    let body_text = text_bytes(rng, 12);
    let actions = encode_payload(&NodePayload::Frame(build::frame(
        rng,
        320 * 4,
        48 * 4,
        0,
        192 * 4,
    )));
    let kids = card.children(
        root,
        vec![
            (NodeKind::Image, image),
            title_text,
            body_text,
            (NodeKind::Frame, actions),
        ],
    )?;
    card.children(
        kids[3],
        vec![(NodeKind::ComponentInstance, instance_bytes(button_id))],
    )?;
    let card_id = card.finish(
        &mut library,
        "Card",
        &[(title, "title", TypedValue::String("Card".into()))],
    )?;
    out.push((card_id, vec![title]));

    // ListRow: Frame ← Instance(Icon), Text, Text.
    let primary = ComponentPropertyId::new(ids.next_id());
    let mut row = TreeBuilder::new(ids, rng, 360 * 4, 56 * 4)?;
    let root = row.root;
    let t1 = text_bytes(rng, 3);
    let t2 = text_bytes(rng, 5);
    row.children(
        root,
        vec![
            (NodeKind::ComponentInstance, instance_bytes(icon_id)),
            t1,
            t2,
        ],
    )?;
    let row_id = row.finish(
        &mut library,
        "ListRow",
        &[(primary, "primary", TypedValue::String("Row".into()))],
    )?;
    out.push((row_id, vec![primary]));

    Ok((library, out))
}

fn scene_fields(
    id: StableId,
    kind: NodeKind,
    parent: Option<RuntimeEntityId>,
    key: OrderKey,
    name: &str,
) -> NodeFields {
    NodeFields {
        stable_id: id,
        kind,
        parent,
        order_key: key,
        visible: true,
        locked: false,
        display_name: name.to_string(),
    }
}

/// section 하나의 인스턴스 수.
const INSTANCES_PER_SECTION: usize = 500;

/// 인스턴스가 정확히 `count`개인 컴포넌트 scene fixture를 만듭니다.
///
/// scene: root Frame ← section Frame(500 인스턴스씩) ← ComponentInstance. 인스턴스는 Button 40%,
/// ListRow 30%, Card 20%, Icon 10%이고, 절반 정도가 선언된 Property에 override 값을 가집니다.
pub fn component_instances(seed: u64, count: usize) -> Result<ComponentFixture, FixtureError> {
    if count == 0 || count > MAX_INSTANCE_TARGET {
        return Err(FixtureError::InvalidSpec {
            reason: "인스턴스 목표는 1..=1,000,000이어야 합니다",
        });
    }
    let mut rng = SplitMix64::new(seed ^ 0xC0DE_5CA1_E000_0003);
    let mut ids = IdAllocator::new(seed ^ 0xC0DE_5CA1);
    let (library, components) = design_system_library(&mut rng, &mut ids)?;
    // 가중치 순서: Icon, Button, Card, ListRow.
    let weights: [(usize, u32); 4] = [(0, 10), (1, 40), (2, 20), (3, 30)];

    let mut columns = NodeColumns::new();
    let mut payloads = ScenePayloads::new();
    let root = ids.next_id();
    let root_handle = columns.insert(scene_fields(
        root,
        NodeKind::Frame,
        None,
        OrderKey::between(None, None),
        "Page",
    ));
    payloads.insert(
        root,
        encode_payload(&NodePayload::Frame(build::frame(
            &mut rng,
            8_000 * 4,
            8_000 * 4,
            0,
            0,
        ))),
    );
    let sections = count.div_ceil(INSTANCES_PER_SECTION);
    let section_keys = OrderKey::evenly_spaced(sections);
    let mut instances = Vec::with_capacity(count);
    let mut remaining = count;
    for (s, key) in section_keys.into_iter().enumerate() {
        let section = ids.next_id();
        let handle = columns.insert(scene_fields(
            section,
            NodeKind::Frame,
            Some(root_handle),
            key,
            "Section",
        ));
        let s = s as Quarter;
        payloads.insert(
            section,
            encode_payload(&NodePayload::Frame(build::frame(
                &mut rng,
                800 * 4,
                800 * 4,
                (s % 10) * 800 * 4,
                (s / 10) * 800 * 4,
            ))),
        );
        let here = remaining.min(INSTANCES_PER_SECTION);
        remaining -= here;
        for key in OrderKey::evenly_spaced(here) {
            let id = ids.next_id();
            let (component, properties) = &components[rng.weighted(&weights)];
            let mut payload = ComponentInstancePayload::new(ComponentRef::new(*component));
            for property in properties {
                if rng.one_in(2) {
                    let value = if rng.one_in(2) {
                        TypedValue::String(rng.pick(&build::WORD_POOL).to_string())
                    } else {
                        TypedValue::Boolean(rng.one_in(2))
                    };
                    payload.set_property_override(OverrideKey::direct(*property), value);
                }
            }
            columns.insert(scene_fields(
                id,
                NodeKind::ComponentInstance,
                Some(handle),
                key,
                "",
            ));
            payloads.insert(id, payload.encode());
            instances.push(id);
        }
    }

    let properties = components.into_iter().collect();
    let mut digest = FixtureDigest::new("component-instances", seed);
    digest.parameter("count", count as u64);
    digest.resource("component-library", StableId::NIL, &library.encode());
    digest.resource("scene", root, &scene_bytes(&columns, &payloads));
    Ok(ComponentFixture {
        library,
        columns,
        payloads,
        root,
        instances,
        properties,
        content_hash: digest.finish(),
    })
}

/// scene column과 payload의 canonical bytes(StableId 순, 부모는 StableId).
fn scene_bytes(columns: &NodeColumns, payloads: &ScenePayloads) -> Vec<u8> {
    let mut by_handle: HashMap<RuntimeEntityId, StableId> = HashMap::with_capacity(columns.len());
    let mut fields: Vec<NodeFields> = Vec::with_capacity(columns.len());
    for handle in columns.iter_live() {
        if let Some(f) = columns.get(handle) {
            by_handle.insert(handle, f.stable_id);
            fields.push(f);
        }
    }
    fields.sort_by_key(|f| f.stable_id);
    let mut buf = Vec::new();
    for f in fields {
        buf.extend_from_slice(&f.stable_id.to_bytes());
        buf.extend_from_slice(&f.kind.as_u16().to_le_bytes());
        let parent = f
            .parent
            .and_then(|p| by_handle.get(&p))
            .copied()
            .unwrap_or(StableId::NIL);
        buf.extend_from_slice(&parent.to_bytes());
        buf.extend_from_slice(&(f.order_key.byte_len() as u32).to_le_bytes());
        buf.extend_from_slice(f.order_key.as_bytes());
        buf.push(u8::from(f.visible) | (u8::from(f.locked) << 1));
        buf.extend_from_slice(&(f.display_name.len() as u32).to_le_bytes());
        buf.extend_from_slice(f.display_name.as_bytes());
        let payload = payloads.get(&f.stable_id).map(Vec::as_slice).unwrap_or(&[]);
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(payload);
    }
    buf
}

/// 모든 인스턴스를 펼쳐 검증합니다.
///
/// - library canonical round-trip(인코딩 → 디코딩 → 같은 값).
/// - 각 인스턴스 `InstanceExpansion::new`(payload 해석·정의 해결)와 `walk`(미해결·재귀·중첩 상한).
/// - override는 그 컴포넌트가 선언한 Property에만 있습니다.
/// - 가상 node는 정의 node를 참조만 합니다(서로 다른 저장소 주소 수 ≤ 정의 node 수).
pub fn validate_component_fixture(
    fixture: &ComponentFixture,
) -> Result<ComponentValidation, FixtureError> {
    let decoded = ComponentLibrary::decode(&fixture.library.encode()).map_err(component_error)?;
    if decoded != fixture.library {
        return Err(FixtureError::InvalidSpec {
            reason: "component library canonical round-trip 불일치",
        });
    }
    let definition_nodes: usize = fixture.library.iter().map(|e| e.tree().len()).sum();
    let index = SceneIndex::build(&fixture.columns, &fixture.payloads);
    let mut virtual_nodes = 0usize;
    let mut distinct: HashSet<usize> = HashSet::new();
    let mut max_chain = 0usize;
    let mut overrides = 0usize;
    for id in &fixture.instances {
        let expansion = InstanceExpansion::new(&fixture.library, &index, *id)
            .map_err(FixtureError::Expansion)?;
        let component = expansion.payload().component().id();
        let declared = fixture
            .properties
            .get(&component)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for (key, _) in expansion.payload().property_overrides() {
            if !key.is_direct() || !declared.contains(&key.property()) {
                return Err(FixtureError::InvalidSpec {
                    reason: "선언되지 않은 Property override",
                });
            }
            overrides += 1;
        }
        for node in expansion.walk().map_err(FixtureError::Expansion)? {
            if let ExpandedNodeId::Virtual { instances, .. } = &node {
                max_chain = max_chain.max(instances.len());
            }
            let expanded = expansion.node(&node).map_err(FixtureError::Expansion)?;
            if let Some(definition) = expanded.definition_node() {
                distinct.insert(definition as *const DefinitionNode as usize);
            }
            virtual_nodes += 1;
        }
    }
    Ok(ComponentValidation {
        instances: fixture.instances.len(),
        definition_nodes,
        virtual_nodes,
        distinct_definition_nodes: distinct.len(),
        max_instance_chain: max_chain,
        property_overrides: overrides,
    })
}

// ---------------------------------------------------------------------------
// 중첩 fixture
// ---------------------------------------------------------------------------

/// 컴포넌트 chain: `C0 ⊃ C1 ⊃ … ⊃ C(levels-1)`의 scene 인스턴스 하나.
///
/// 각 정의는 root Frame ← Text, (다음 컴포넌트 인스턴스)이고, scene은 root Frame ← C0 인스턴스입니다.
/// 인스턴스 chain 깊이(scene 인스턴스 포함)가 정확히 `levels`입니다. 64가 상한이며, 음성 대조를
/// 위해 [`MAX_CHAIN_LEVELS`]까지 만들 수 있습니다(64 초과 chain은 expansion이 거부합니다).
pub fn nesting_chain(seed: u64, levels: usize) -> Result<ComponentFixture, FixtureError> {
    if levels == 0 || levels > MAX_CHAIN_LEVELS {
        return Err(FixtureError::InvalidSpec {
            reason: "chain 깊이는 1..=256이어야 합니다",
        });
    }
    let mut rng = SplitMix64::new(seed ^ 0x0E57_0000_0000_0004);
    let mut ids = IdAllocator::new(seed ^ 0x0E57);
    // 안쪽(마지막)부터 만들어 바깥 정의가 안쪽 id를 참조하게 합니다.
    let mut library = ComponentLibrary::new();
    let mut inner: Option<StableId> = None;
    for level in (0..levels).rev() {
        let mut tree = TreeBuilder::new(&mut ids, &mut rng, 200 * 4, 40 * 4)?;
        let root = tree.root;
        let mut kids = vec![text_bytes(&mut rng, 2)];
        if let Some(inner) = inner {
            kids.push((NodeKind::ComponentInstance, instance_bytes(inner)));
        }
        tree.children(root, kids)?;
        let name = format!("Level {level}");
        inner = Some(tree.finish(&mut library, &name, &[])?);
    }
    let outer = inner.ok_or(FixtureError::InvalidSpec {
        reason: "chain이 비었습니다",
    })?;
    let mut columns = NodeColumns::new();
    let mut payloads = ScenePayloads::new();
    let root = ids.next_id();
    let instance = ids.next_id();
    let handle = columns.insert(scene_fields(
        root,
        NodeKind::Frame,
        None,
        OrderKey::between(None, None),
        "Page",
    ));
    payloads.insert(
        root,
        encode_payload(&NodePayload::Frame(build::frame(
            &mut rng,
            400 * 4,
            400 * 4,
            0,
            0,
        ))),
    );
    columns.insert(scene_fields(
        instance,
        NodeKind::ComponentInstance,
        Some(handle),
        OrderKey::between(None, None),
        "",
    ));
    payloads.insert(instance, instance_bytes(outer));

    let mut digest = FixtureDigest::new("nesting-chain", seed);
    digest.parameter("levels", levels as u64);
    digest.resource("component-library", StableId::NIL, &library.encode());
    digest.resource("scene", root, &scene_bytes(&columns, &payloads));
    let properties = library
        .iter()
        .map(|e| (e.definition().id, Vec::new()))
        .collect();
    Ok(ComponentFixture {
        library,
        columns,
        payloads,
        root,
        instances: vec![instance],
        properties,
        content_hash: digest.finish(),
    })
}

/// 깊이 chain 문서 fixture.
#[derive(Debug)]
pub struct DeepDocument {
    /// 문서. `depth ≤ 64`이면 validator 통과.
    pub built: BuiltDocument,
    /// 가장 깊은 node.
    pub deepest: StableId,
    /// content hash.
    pub content_hash: ContentHash,
}

/// root Frame 아래 Frame을 `depth`단 중첩한 문서(가장 깊은 node depth = `depth`, root = 0).
/// 각 단에는 형제 leaf(Shape·Text)도 둡니다.
///
/// 64 초과는 문서 validator가 거부하므로 [`FixtureError::Load`]가 됩니다(음성 대조).
pub fn deep_document(seed: u64, depth: u32) -> Result<DeepDocument, FixtureError> {
    if depth == 0 || depth as usize > MAX_CHAIN_LEVELS {
        return Err(FixtureError::InvalidSpec {
            reason: "depth는 1..=256이어야 합니다",
        });
    }
    let mut rng = SplitMix64::new(seed ^ 0xDEE9_0000_0000_0005);
    let mut ids = IdAllocator::new(seed ^ 0xDEE9);
    let document = ids.next_id();
    let view = ids.next_id();
    let root = ids.next_id();
    let mut definition = DocumentDefinition::new(document, "Deep".to_string());
    let mut page = PageDefinition::new(ids.next_id(), "Page".to_string());
    page.add_view(ViewDefinition::new(view, "Deep".to_string(), root))
        .map_err(FixtureError::Definition)?;
    definition
        .add_page(page)
        .map_err(FixtureError::Definition)?;
    let mut records = DocumentRecords::new(definition);
    let side: Quarter = 2_048 * 4;
    records.nodes.push(build::record(
        root,
        None,
        OrderKey::between(None, None),
        true,
        "Root",
        NodePayload::Frame(build::frame(&mut rng, side, side, 0, 0)),
    ));
    let mut parent = root;
    for level in 1..=depth {
        // 한 단마다 크기를 조금씩 줄입니다(최소 4 px).
        let s = side.saturating_sub(level * 16 * 4).max(16);
        let keys = OrderKey::evenly_spaced(3);
        let frame = ids.next_id();
        records.nodes.push(build::record(
            frame,
            Some(parent),
            keys[1].clone(),
            true,
            "Level",
            NodePayload::Frame(build::frame(&mut rng, s, s, 8 * 4, 8 * 4)),
        ));
        records.nodes.push(build::record(
            ids.next_id(),
            Some(parent),
            keys[0].clone(),
            true,
            "Mark",
            NodePayload::Shape(build::shape(&mut rng, 32, 32)),
        ));
        records.nodes.push(build::record(
            ids.next_id(),
            Some(parent),
            keys[2].clone(),
            true,
            "Caption",
            NodePayload::Text(build::text(&mut rng, 2, 0)),
        ));
        parent = frame;
    }
    let built = finish_document(records)?;
    let mut digest = FixtureDigest::new("deep-document", seed);
    digest.parameter("depth", u64::from(depth));
    digest.document(&built.serialized);
    Ok(DeepDocument {
        built,
        deepest: parent,
        content_hash: digest.finish(),
    })
}
