//! 영속 레코드 → [`SemanticSnapshot`] 구성 (XIV.3.1).
//!
//! 입력은 Git snapshot(HEAD·Index·Working Tree)을 materialize한 뒤 project loader가 읽은
//! **영속 레코드**입니다(XIV.2.6). runtime 핸들·평가 cache는 입력에 없으므로 snapshot에도
//! 들어갈 수 없습니다.
//!
//! | 입력 | 객체 |
//! |---|---|
//! | [`ProjectMetadata`] | `Project` 1개(항목은 `PROJECT_METADATA` + 이름 segment) |
//! | [`DocumentRecords`] | `Document`·`Page`·`View`, scene `Node`, `Modifier` |
//! | [`ComponentLibrary`] | `Component`, 정의 tree `Node`(Origin::Component), `ComponentSlot` |
//! | [`TokenEntry`] | `Token` |
//! | [`AssetEntry`] | `Asset` |
//! | [`InstanceNode`] | scene ComponentInstance `Node`(override 항목별 속성) |
//! | [`ExpressionNodeEntry`] | `ExpressionNode`(AST node마다 객체 하나) |
//! | [`VariantEntry`] | `Variant`(patch 항목마다 속성 하나) |
//!
//! **AST·Variant는 StableId node 단위입니다(XIV.3.7).** expression AST는 node마다 객체이고
//! (payload·child ref 속성), Variant override 묶음은 대상 PropertyPath·node·slot마다 속성
//! 하나라서 operator 하나·patch 하나의 변경이 전체 blob 변경으로 축약되지 않습니다. binding의
//! expression도 binding을 owner로 하는 AST node 객체로 같은 경로를 탑니다.
//!
//! node payload는 필드별 속성으로 펼쳐 같은 객체의 서로 다른 필드 변경이 따로 비교·병합되게
//! 합니다(XIV.3.4, XIV.10.2). 단, Path·Text는 내부 offset·point id가 서로 묶여 있어 payload
//! 전체를 문서 canonical 인코딩 하나로 담습니다. 각 `add_*`는 원자적입니다 — 오류가 나면
//! builder는 바뀌지 않습니다.

use stellar_core_types::color::ColorSpace;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{FiniteF32, Rect, Size2, Transform2D, Vec2};
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey};
use stellar_document::component::variant_override::VariantOverrideSet;
use stellar_document::definitions::DocumentDefinition;
use stellar_document::frame::{Clipping, FramePayload};
use stellar_document::image::{FitMode, ImagePayload, SamplingMode};
use stellar_document::modifier::{ModifierEntry, ModifierPayload};
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::{decode_payload, encode_payload};
use stellar_document::records::{DocumentRecords, NodePayload};
use stellar_document::shape::{LineCap, ShapeGeometry};

use crate::keys;
use crate::snapshot::{ObjectKind, Origin, SemanticObject, SemanticSnapshot, SnapshotError};
use crate::value::SemanticValue;

/// 프로젝트 metadata(매니페스트 등에서 caller가 추출한 이름·값 쌍).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMetadata {
    /// 프로젝트 객체 StableId.
    pub id: StableId,
    /// 항목(이름 중복 불가).
    pub entries: Vec<(String, String)>,
}

/// token record 입력. token 모델(VIII.5)의 영속 필드를 caller가 옮겨 담습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenEntry {
    /// token StableId.
    pub id: StableId,
    /// 소유 문서. `None`이면 프로젝트 scope.
    pub document: Option<StableId>,
    /// 이름.
    pub name: String,
    /// token 타입 numeric id.
    pub value_type: u32,
    /// literal 값.
    pub value: Option<TypedValue>,
    /// 값 expression root node.
    pub expression: Option<StableId>,
}

/// scene ComponentInstance node 입력(XIV.3.6).
///
/// 문서 레코드(`DocumentRecords`)는 아직 ComponentInstance payload variant가 없어 instance
/// node를 담지 못하므로 별도 입력으로 받습니다. 공통 필드는 [`NodeRecord`]와 같습니다.
///
/// [`NodeRecord`]: stellar_document::records::NodeRecord
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceNode {
    /// node StableId.
    pub id: StableId,
    /// 부모 node.
    pub parent: Option<StableId>,
    /// 형제 순서 키.
    pub order_key: OrderKey,
    /// 표시 여부.
    pub visible: bool,
    /// 잠금 여부.
    pub locked: bool,
    /// 표시 이름.
    pub display_name: String,
    /// 정의 참조와 sparse override.
    pub payload: ComponentInstancePayload,
}

/// expression AST node 입력(XIV.3.7).
///
/// semantic-diff는 expressions crate에 의존하지 않으므로(II.2.8) caller가 AST node의 canonical
/// payload bytes와 typed child ref(slot code, 대상 id)를 옮겨 담습니다. AST는 DAG일 수 있고
/// 공유 node도 객체 하나입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionNodeEntry {
    /// AST node StableId.
    pub id: StableId,
    /// AST를 소유한 객체(token·binding·variant 등). snapshot에서 parent가 됩니다.
    pub owner: StableId,
    /// 소유 문서. `None`이면 프로젝트 범위.
    pub document: Option<StableId>,
    /// node 종류·operator·literal·reference의 canonical payload bytes.
    pub payload: Vec<u8>,
    /// typed child ref(slot code, 대상 node) — slot 순서.
    pub children: Vec<(u32, StableId)>,
}

/// View Variant override 묶음 입력(XIV.3.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantEntry {
    /// Variant StableId.
    pub id: StableId,
    /// 소유 문서.
    pub document: StableId,
    /// 소유 객체(View 등). snapshot에서 parent가 됩니다.
    pub owner: StableId,
    /// sparse override.
    pub overrides: VariantOverrideSet,
}

/// asset record 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    /// asset StableId(문서의 `AssetRef`가 가리키는 값).
    pub id: StableId,
    /// content hash bytes.
    pub content_hash: Vec<u8>,
    /// media type.
    pub media_type: String,
}

/// [`SemanticSnapshot`] builder.
#[derive(Debug, Default)]
pub struct SnapshotBuilder {
    snapshot: SemanticSnapshot,
}

impl SnapshotBuilder {
    /// 빈 builder.
    pub fn new() -> SnapshotBuilder {
        SnapshotBuilder::default()
    }

    /// 프로젝트 metadata를 추가합니다.
    pub fn add_project(&mut self, metadata: &ProjectMetadata) -> Result<(), SnapshotError> {
        let mut object = SemanticObject::new(metadata.id, ObjectKind::Project, Origin::Project);
        for (name, value) in &metadata.entries {
            let path = object
                .path(&[keys::PROJECT_METADATA])
                .with_key_segment(name);
            object.insert_property(path, SemanticValue::Text(value.clone()))?;
        }
        self.commit(vec![object])
    }

    /// 문서 하나의 정의·node·modifier를 추가합니다. 레코드 순서와 무관합니다.
    pub fn add_document(&mut self, records: &DocumentRecords) -> Result<(), SnapshotError> {
        let document = records.definition.id;
        let origin = Origin::Document(document);
        let mut objects = definition_objects(&records.definition)?;
        for node in &records.nodes {
            let mut object = node_object(
                node.id,
                node.kind(),
                origin,
                (node.parent, node.order_key.clone()),
                (node.visible, node.locked, &node.display_name),
            )?;
            project_payload(&mut object, &node.payload)?;
            objects.push(object);
        }
        for stack in &records.modifier_stacks {
            for entry in stack.entries() {
                objects.push(modifier_object(origin, stack.node, entry)?);
            }
        }
        self.commit(objects)
    }

    /// 문서의 컴포넌트 정의 library를 추가합니다.
    pub fn add_component_library(
        &mut self,
        document: StableId,
        library: &ComponentLibrary,
    ) -> Result<(), SnapshotError> {
        self.component_library(library, |component| Origin::Component {
            document,
            component,
        })
    }

    /// 특정 문서에 속하지 않는 프로젝트 공용 컴포넌트 정의를 추가합니다.
    pub fn add_project_component_library(
        &mut self,
        library: &ComponentLibrary,
    ) -> Result<(), SnapshotError> {
        self.component_library(library, Origin::ProjectComponent)
    }

    fn component_library(
        &mut self,
        library: &ComponentLibrary,
        owner: impl Fn(StableId) -> Origin,
    ) -> Result<(), SnapshotError> {
        let mut objects = Vec::new();
        for entry in library.iter() {
            let definition = entry.definition();
            let origin = owner(definition.id);
            let mut component = SemanticObject::new(definition.id, ObjectKind::Component, origin);
            let props: [(PropertyKey, SemanticValue); 5] = [
                (
                    keys::DISPLAY_NAME,
                    SemanticValue::Text(definition.display_name().to_string()),
                ),
                (
                    keys::COMPONENT_ROOT,
                    SemanticValue::Id(definition.root.id()),
                ),
                (
                    keys::COMPONENT_PROPERTIES,
                    SemanticValue::Ids(
                        definition
                            .properties()
                            .iter()
                            .map(|p| p.stable_id())
                            .collect(),
                    ),
                ),
                (
                    keys::COMPONENT_SLOTS,
                    SemanticValue::Ids(definition.slots().iter().map(|s| s.stable_id()).collect()),
                ),
                (
                    keys::COMPONENT_LOCAL_TOKENS,
                    SemanticValue::Ids(definition.local_tokens().iter().map(|t| t.id()).collect()),
                ),
            ];
            for (key, value) in props {
                component.insert_property(component.path(&[key]), value)?;
            }
            objects.push(component);

            for node in entry.tree().iter() {
                let mut object = node_object(
                    node.id,
                    node.kind,
                    origin,
                    (node.parent, node.order_key.clone()),
                    (node.visible, node.locked, &node.display_name),
                )?;
                project_definition_payload(&mut object, node.kind, &node.payload)?;
                objects.push(object);
            }

            for slot in entry.slots() {
                let mut object =
                    SemanticObject::new(slot.id.stable_id(), ObjectKind::ComponentSlot, origin);
                let mut allowed = Vec::new();
                for kind in slot.allowed_kinds() {
                    allowed.extend_from_slice(&kind.as_u16().to_le_bytes());
                }
                object.insert_property(
                    object.path(&[keys::SLOT_HOST]),
                    SemanticValue::Id(slot.host.id()),
                )?;
                object.insert_property(
                    object.path(&[keys::SLOT_ALLOWED_KINDS]),
                    SemanticValue::Bytes(allowed),
                )?;
                object.insert_property(
                    object.path(&[keys::SLOT_MIN]),
                    SemanticValue::Uint(u64::from(slot.cardinality.min())),
                )?;
                if let Some(max) = slot.cardinality.max() {
                    object.insert_property(
                        object.path(&[keys::SLOT_MAX]),
                        SemanticValue::Uint(u64::from(max)),
                    )?;
                }
                object.insert_property(
                    object.path(&[keys::SLOT_DEFAULT_CONTENT]),
                    SemanticValue::Ids(slot.default_content().iter().map(|r| r.id()).collect()),
                )?;
                objects.push(object);
            }
        }
        self.commit(objects)
    }

    /// 문서 scene의 ComponentInstance node를 추가합니다(XIV.3.6). override는 항목마다 속성
    /// 하나(`PROPERTY_OVERRIDES` + override key segment, `SLOT_OVERRIDES` + slot segment)입니다.
    pub fn add_instance(
        &mut self,
        document: StableId,
        instance: &InstanceNode,
    ) -> Result<(), SnapshotError> {
        let mut object = node_object(
            instance.id,
            NodeKind::ComponentInstance,
            Origin::Document(document),
            (instance.parent, instance.order_key.clone()),
            (instance.visible, instance.locked, &instance.display_name),
        )?;
        project_instance(&mut object, &instance.payload)?;
        self.commit(vec![object])
    }

    /// token을 추가합니다.
    pub fn add_token(&mut self, token: &TokenEntry) -> Result<(), SnapshotError> {
        let origin = token.document.map_or(Origin::Project, Origin::Document);
        let mut object = SemanticObject::new(token.id, ObjectKind::Token, origin);
        object.insert_property(
            object.path(&[keys::TOKEN_NAME]),
            SemanticValue::Text(token.name.clone()),
        )?;
        object.insert_property(
            object.path(&[keys::TOKEN_TYPE]),
            SemanticValue::Uint(u64::from(token.value_type)),
        )?;
        if let Some(value) = &token.value {
            object.insert_property(
                object.path(&[keys::TOKEN_VALUE]),
                SemanticValue::Typed(value.clone()),
            )?;
        }
        if let Some(root) = token.expression {
            object.insert_property(
                object.path(&[keys::TOKEN_EXPRESSION]),
                SemanticValue::Id(root),
            )?;
        }
        self.commit(vec![object])
    }

    /// asset record를 추가합니다.
    pub fn add_asset(&mut self, asset: &AssetEntry) -> Result<(), SnapshotError> {
        let mut object = SemanticObject::new(asset.id, ObjectKind::Asset, Origin::Project);
        object.insert_property(
            object.path(&[keys::ASSET_CONTENT_HASH]),
            SemanticValue::Bytes(asset.content_hash.clone()),
        )?;
        object.insert_property(
            object.path(&[keys::ASSET_MEDIA_TYPE]),
            SemanticValue::Text(asset.media_type.clone()),
        )?;
        self.commit(vec![object])
    }

    /// expression AST node들을 추가합니다(XIV.3.7). 묶음 전체가 원자적입니다.
    pub fn add_expression_nodes(
        &mut self,
        nodes: &[ExpressionNodeEntry],
    ) -> Result<(), SnapshotError> {
        let mut objects = Vec::with_capacity(nodes.len());
        for node in nodes {
            let origin = node.document.map_or(Origin::Project, Origin::Document);
            let mut object = SemanticObject::new(node.id, ObjectKind::ExpressionNode, origin)
                .with_position(Some(node.owner), None);
            put(
                &mut object,
                keys::EXPR_PAYLOAD,
                SemanticValue::Bytes(node.payload.clone()),
            )?;
            let mut children = Vec::with_capacity(4 + node.children.len() * 20);
            children.extend_from_slice(&(node.children.len() as u32).to_le_bytes());
            for (slot, target) in &node.children {
                children.extend_from_slice(&slot.to_le_bytes());
                children.extend_from_slice(&target.to_bytes());
            }
            put(
                &mut object,
                keys::EXPR_CHILDREN,
                SemanticValue::Bytes(children),
            )?;
            objects.push(object);
        }
        self.commit(objects)
    }

    /// View Variant override 묶음을 추가합니다(XIV.3.7).
    ///
    /// Property patch는 대상 경로마다 TypedValue 속성, 구조 patch·slot change는 항목마다 그
    /// 항목 하나만 담은 [`VariantOverrideSet`]의 canonical 인코딩(Bytes)입니다.
    pub fn add_variant(&mut self, variant: &VariantEntry) -> Result<(), SnapshotError> {
        let mut object = SemanticObject::new(
            variant.id,
            ObjectKind::Variant,
            Origin::Document(variant.document),
        )
        .with_position(Some(variant.owner), None);
        for (path, value) in variant.overrides.property_patches() {
            let segment: String = path
                .to_canonical_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let key = object
                .path(&[keys::VARIANT_PROPERTY_PATCH])
                .with_key_segment(&segment);
            object.insert_property(key, SemanticValue::Typed(value.clone()))?;
        }
        for (node, patch) in variant.overrides.structure_patches() {
            let mut single = VariantOverrideSet::new();
            single.set_structure(node, patch.clone());
            let key = object
                .path(&[keys::VARIANT_STRUCTURE_PATCH])
                .with_key_segment(&node.to_string());
            object.insert_property(key, SemanticValue::Bytes(single.encode()))?;
        }
        for (instance, slot, change) in variant.overrides.slot_changes() {
            let mut single = VariantOverrideSet::new();
            single.set_slot(instance, slot, change.clone());
            let key = object
                .path(&[keys::VARIANT_SLOT_CHANGE])
                .with_key_segment(&format!("{instance}/{}", slot.stable_id()));
            object.insert_property(key, SemanticValue::Bytes(single.encode()))?;
        }
        self.commit(vec![object])
    }

    /// 완성된 snapshot.
    pub fn finish(self) -> SemanticSnapshot {
        self.snapshot
    }

    /// 객체 묶음을 원자적으로 넣습니다: 기존·묶음 안 중복이 있으면 하나도 넣지 않습니다.
    fn commit(&mut self, objects: Vec<SemanticObject>) -> Result<(), SnapshotError> {
        let mut seen = std::collections::BTreeSet::new();
        for object in &objects {
            if self.snapshot.contains(object.id()) || !seen.insert(object.id()) {
                return Err(SnapshotError::DuplicateObject { id: object.id() });
            }
        }
        for object in objects {
            self.snapshot.insert(object)?;
        }
        Ok(())
    }
}

/// 문서·페이지·View 정의 객체.
fn definition_objects(
    definition: &DocumentDefinition,
) -> Result<Vec<SemanticObject>, SnapshotError> {
    let origin = Origin::Document(definition.id);
    let mut objects = Vec::new();
    let mut document = SemanticObject::new(definition.id, ObjectKind::Document, origin);
    document.insert_property(
        document.path(&[keys::DISPLAY_NAME]),
        SemanticValue::Text(definition.display_name().to_string()),
    )?;
    document.insert_property(
        document.path(&[keys::DOCUMENT_PAGES]),
        SemanticValue::Ids(definition.pages.iter().map(|p| p.id).collect()),
    )?;
    objects.push(document);
    for page in &definition.pages {
        let mut object = SemanticObject::new(page.id, ObjectKind::Page, origin)
            .with_position(Some(definition.id), None);
        object.insert_property(
            object.path(&[keys::DISPLAY_NAME]),
            SemanticValue::Text(page.display_name().to_string()),
        )?;
        object.insert_property(
            object.path(&[keys::PAGE_VIEWS]),
            SemanticValue::Ids(page.views.iter().map(|v| v.id).collect()),
        )?;
        objects.push(object);
        for view in &page.views {
            let mut object = SemanticObject::new(view.id, ObjectKind::View, origin)
                .with_position(Some(page.id), None);
            object.insert_property(
                object.path(&[keys::DISPLAY_NAME]),
                SemanticValue::Text(view.display_name().to_string()),
            )?;
            object.insert_property(
                object.path(&[keys::VIEW_ROOT_FRAME]),
                SemanticValue::Id(view.root_frame),
            )?;
            objects.push(object);
        }
    }
    Ok(objects)
}

/// 공통 node 필드(VI.2.2)를 담은 node 객체.
fn node_object(
    id: StableId,
    kind: NodeKind,
    origin: Origin,
    (parent, order_key): (Option<StableId>, OrderKey),
    (visible, locked, name): (bool, bool, &str),
) -> Result<SemanticObject, SnapshotError> {
    let mut object = SemanticObject::new(id, ObjectKind::Node(kind), origin)
        .with_position(parent, Some(order_key));
    object.insert_property(object.path(&[keys::VISIBLE]), SemanticValue::Bool(visible))?;
    object.insert_property(object.path(&[keys::LOCKED]), SemanticValue::Bool(locked))?;
    object.insert_property(
        object.path(&[keys::DISPLAY_NAME]),
        SemanticValue::Text(name.to_string()),
    )?;
    Ok(object)
}

/// float 목록 값. 유한하지 않으면 오류(저장 레코드는 이미 유한).
fn floats(
    object: StableId,
    key: PropertyKey,
    values: &[f32],
) -> Result<SemanticValue, SnapshotError> {
    values
        .iter()
        .map(|v| FiniteF32::new(*v).map_err(|_| SnapshotError::NonFiniteValue { object, key }))
        .collect::<Result<Vec<_>, _>>()
        .map(SemanticValue::Floats)
}

fn vec2_values(value: Vec2) -> [f32; 2] {
    [value.x(), value.y()]
}

fn size_values(value: Size2) -> [f32; 2] {
    [value.width(), value.height()]
}

fn rect_values(value: Rect) -> [f32; 4] {
    [
        value.origin().x(),
        value.origin().y(),
        value.size().width(),
        value.size().height(),
    ]
}

fn transform_values(value: Transform2D) -> [f32; 6] {
    let (a, b, c, d, tx, ty) = value.components();
    [a, b, c, d, tx, ty]
}

fn quad((a, b, c, d): (f32, f32, f32, f32)) -> [f32; 4] {
    [a, b, c, d]
}

fn color_space_tag(space: ColorSpace) -> u64 {
    u64::from(space.tag())
}

/// key에 float 목록 속성을 추가합니다.
fn put_floats(
    object: &mut SemanticObject,
    key: PropertyKey,
    values: &[f32],
) -> Result<(), SnapshotError> {
    let value = floats(object.id(), key, values)?;
    object.insert_property(object.path(&[key]), value)
}

/// key에 속성을 추가합니다.
fn put(
    object: &mut SemanticObject,
    key: PropertyKey,
    value: SemanticValue,
) -> Result<(), SnapshotError> {
    object.insert_property(object.path(&[key]), value)
}

/// scene·정의 node payload를 필드별 속성으로 펼칩니다.
fn project_payload(
    object: &mut SemanticObject,
    payload: &NodePayload,
) -> Result<(), SnapshotError> {
    match payload {
        NodePayload::Frame(frame) => project_frame(object, frame),
        NodePayload::Fragment(_) => Ok(()),
        NodePayload::Shape(shape) => project_shape(object, shape),
        NodePayload::Path(_) => put(
            object,
            keys::PATH_DATA,
            SemanticValue::Bytes(encode_payload(payload)),
        ),
        NodePayload::Text(_) => put(
            object,
            keys::TEXT_PAYLOAD,
            SemanticValue::Bytes(encode_payload(payload)),
        ),
        NodePayload::Image(image) => project_image(object, image),
    }
}

fn project_frame(object: &mut SemanticObject, frame: &FramePayload) -> Result<(), SnapshotError> {
    let FramePayload {
        size,
        transform,
        clipping,
        background,
        border,
        corner_radius,
        layout_mode,
    } = frame;
    put_floats(object, keys::frame::SIZE, &size_values(*size))?;
    put_floats(
        object,
        keys::frame::TRANSFORM,
        &transform_values(*transform),
    )?;
    let clipping = match clipping {
        Clipping::Disabled => SemanticValue::Uint(0),
        Clipping::FrameBounds => SemanticValue::Uint(1),
        Clipping::Path(target) => SemanticValue::Id(target.id()),
    };
    put(object, keys::frame::CLIPPING, clipping)?;
    if let Some(color) = background {
        put(
            object,
            keys::frame::BACKGROUND,
            SemanticValue::Typed(TypedValue::Color(*color)),
        )?;
    }
    if let Some(border) = border {
        // border는 색(index 0)과 두께(index 1) 하위 경로로 나눕니다.
        let base = object.path(&[keys::frame::BORDER]);
        object.insert_property(
            base.clone().with_index(0),
            SemanticValue::Typed(TypedValue::Color(border.color)),
        )?;
        let thickness = floats(object.id(), keys::frame::BORDER, &[border.thickness()])?;
        object.insert_property(base.with_index(1), thickness)?;
    }
    put_floats(
        object,
        keys::frame::CORNER_RADIUS,
        &quad(corner_radius.components()),
    )?;
    if let Some(mode) = layout_mode {
        put(object, keys::frame::LAYOUT_MODE, SemanticValue::Id(*mode))?;
    }
    Ok(())
}

fn project_shape(object: &mut SemanticObject, shape: &ShapeGeometry) -> Result<(), SnapshotError> {
    put(
        object,
        keys::SHAPE_KIND,
        SemanticValue::Uint(u64::from(shape.kind().tag())),
    )?;
    match shape {
        ShapeGeometry::Rectangle(params) => {
            put_floats(object, keys::RECT_SIZE, &size_values(params.size))?;
            put_floats(
                object,
                keys::RECT_CORNER_RADIUS,
                &quad(params.corner_radius.components()),
            )
        }
        ShapeGeometry::Ellipse(params) => {
            put_floats(object, keys::ELLIPSE_BOUNDS, &rect_values(params.bounds))
        }
        ShapeGeometry::Line(params) => {
            put_floats(object, keys::LINE_START, &vec2_values(params.start))?;
            put_floats(object, keys::LINE_END, &vec2_values(params.end))?;
            let cap = match params.cap {
                LineCap::Butt => 0,
                LineCap::Round => 1,
                LineCap::Square => 2,
            };
            put(object, keys::LINE_CAP, SemanticValue::Uint(cap))
        }
        ShapeGeometry::Polygon(params) => {
            put_floats(object, keys::POLYGON_BOUNDS, &rect_values(params.bounds()))?;
            put(
                object,
                keys::POLYGON_SIDES,
                SemanticValue::Uint(u64::from(params.sides())),
            )?;
            put_floats(object, keys::POLYGON_ROTATION, &[params.rotation_degrees()])?;
            put_floats(
                object,
                keys::POLYGON_CORNER_RADIUS,
                &[params.corner_radius()],
            )
        }
    }
}

fn project_image(object: &mut SemanticObject, image: &ImagePayload) -> Result<(), SnapshotError> {
    put(
        object,
        keys::IMAGE_ASSET,
        SemanticValue::Id(image.asset.id()),
    )?;
    put_floats(object, keys::IMAGE_SIZE, &size_values(image.size))?;
    let fit = match image.fit {
        FitMode::Fill => 0,
        FitMode::Contain => 1,
        FitMode::Cover => 2,
        FitMode::None => 3,
    };
    put(object, keys::IMAGE_FIT, SemanticValue::Uint(fit))?;
    put_floats(object, keys::IMAGE_CROP, &quad(image.crop.components()))?;
    let sampling = match image.sampling {
        SamplingMode::Linear => 0,
        SamplingMode::Nearest => 1,
    };
    put(object, keys::IMAGE_SAMPLING, SemanticValue::Uint(sampling))?;
    if let Some(space) = image.color_space_override {
        put(
            object,
            keys::IMAGE_COLOR_SPACE,
            SemanticValue::Uint(color_space_tag(space)),
        )?;
    }
    Ok(())
}

/// override key의 경로 segment: nested instance 경로와 Property StableId를 `/`로 잇습니다.
pub(crate) fn override_segment(key: &OverrideKey) -> String {
    let mut segment = String::new();
    for instance in key.path() {
        segment.push_str(&instance.to_string());
        segment.push('/');
    }
    segment.push_str(&key.property().stable_id().to_string());
    segment
}

/// ComponentInstance payload를 속성으로 펼칩니다: 정의 참조, override 항목별, slot override별.
pub(crate) fn project_instance(
    object: &mut SemanticObject,
    payload: &ComponentInstancePayload,
) -> Result<(), SnapshotError> {
    put(
        object,
        keys::instance::COMPONENT_REF,
        SemanticValue::Id(payload.component().id()),
    )?;
    for (key, value) in payload.property_overrides() {
        let path = object
            .path(&[keys::instance::PROPERTY_OVERRIDES])
            .with_key_segment(&override_segment(key));
        object.insert_property(path, SemanticValue::Typed(value.clone()))?;
    }
    for (slot, content) in payload.slot_overrides() {
        let path = object
            .path(&[keys::instance::SLOT_OVERRIDES])
            .with_key_segment(&slot.stable_id().to_string());
        object.insert_property(path, SemanticValue::Ids(content.content().to_vec()))?;
    }
    Ok(())
}

/// 정의 tree node의 canonical payload bytes를 해석해 펼칩니다.
fn project_definition_payload(
    object: &mut SemanticObject,
    kind: NodeKind,
    bytes: &[u8],
) -> Result<(), SnapshotError> {
    let error = |reason: String| SnapshotError::DefinitionPayload {
        node: object.id(),
        kind,
        reason,
    };
    if kind == NodeKind::ComponentInstance {
        let payload = ComponentInstancePayload::decode(bytes).map_err(|e| error(e.to_string()))?;
        project_instance(object, &payload)
    } else {
        let payload = decode_payload(kind, bytes).map_err(|e| error(e.to_string()))?;
        project_payload(object, &payload)
    }
}

/// modifier stack 항목 객체(부모 = 소유 node, 순서 = 항목 OrderKey).
fn modifier_object(
    origin: Origin,
    owner: StableId,
    entry: &ModifierEntry,
) -> Result<SemanticObject, SnapshotError> {
    use keys::modifier as m;
    let mut object = SemanticObject::new(
        entry.id.stable_id(),
        ObjectKind::Modifier(entry.kind),
        origin,
    )
    .with_position(Some(owner), Some(entry.order.clone()));
    put(&mut object, m::ENABLED, SemanticValue::Bool(entry.enabled))?;
    let finite = |v: FiniteF32| SemanticValue::Floats(vec![v]);
    let color = |c| SemanticValue::Typed(TypedValue::Color(c));
    match &entry.payload {
        ModifierPayload::Transform(matrix) => {
            put_floats(&mut object, m::TRANSFORM_MATRIX, &transform_values(*matrix))?
        }
        ModifierPayload::Boolean { op, operands } => {
            put(
                &mut object,
                m::BOOLEAN_OP,
                SemanticValue::Uint(u64::from(*op as u8)),
            )?;
            put(
                &mut object,
                m::BOOLEAN_OPERANDS,
                SemanticValue::Ids(operands.iter().map(|r| r.id()).collect()),
            )?;
        }
        ModifierPayload::StrokeExpand { width } => {
            put(&mut object, m::STROKE_EXPAND_WIDTH, finite(*width))?
        }
        ModifierPayload::Corner { radius } => put(&mut object, m::CORNER_RADIUS, finite(*radius))?,
        ModifierPayload::Mask { source } => {
            put(&mut object, m::MASK_SOURCE, SemanticValue::Id(source.id()))?
        }
        ModifierPayload::Clip { source } => {
            put(&mut object, m::CLIP_SOURCE, SemanticValue::Id(source.id()))?
        }
        ModifierPayload::Fill { color: c } => put(&mut object, m::FILL_COLOR, color(*c))?,
        ModifierPayload::Stroke { color: c, width } => {
            put(&mut object, m::STROKE_COLOR, color(*c))?;
            put(&mut object, m::STROKE_WIDTH, finite(*width))?;
        }
        ModifierPayload::Shadow {
            color: c,
            offset,
            blur,
        } => {
            put(&mut object, m::SHADOW_COLOR, color(*c))?;
            put_floats(&mut object, m::SHADOW_OFFSET, &vec2_values(*offset))?;
            put(&mut object, m::SHADOW_BLUR, finite(*blur))?;
        }
        ModifierPayload::Blur { radius } => put(&mut object, m::BLUR_RADIUS, finite(*radius))?,
        ModifierPayload::Blend { mode } => put(
            &mut object,
            m::BLEND_MODE,
            SemanticValue::Uint(u64::from(*mode as u8)),
        )?,
        ModifierPayload::Opacity { value } => put(&mut object, m::OPACITY_VALUE, finite(*value))?,
    }
    Ok(object)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use stellar_core_types::property::{PathSegment, PropertyPath};
    use stellar_core_types::reference::{AssetRef, ComponentRef, NodeRef};
    use stellar_core_types::value::FiniteF32;
    use stellar_document::component::definition::{
        ComponentDefinition, ComponentPropertyId, DefinitionNode, DefinitionTree,
    };
    use stellar_document::component::variant_override::StructurePatch;
    use stellar_document::definitions::{PageDefinition, ViewDefinition};
    use stellar_document::fragment::FragmentPayload;
    use stellar_document::frame::FramePayload;
    use stellar_document::model::DocumentModel;
    use stellar_document::modifier::{ModifierId, ModifierKind, ModifierStack};
    use stellar_document::record_codec::SubtreeRecords;
    use stellar_document::records::NodeRecord;
    use stellar_document::shape::{EllipseParams, RectangleParams};
    use stellar_document::test_support::random_document::random_valid_document;

    pub(crate) fn sid(n: u64) -> StableId {
        StableId::new_v7(1_700_000_000_000 + n, 0x0abc, 0x0123_4567_89ab_cdef ^ n)
    }

    pub(crate) fn key(byte: u8) -> OrderKey {
        OrderKey::from_bytes(vec![byte]).unwrap()
    }

    pub(crate) fn size(w: f32, h: f32) -> Size2 {
        Size2::new(w, h).unwrap()
    }

    pub(crate) fn node(
        n: u64,
        parent: Option<u64>,
        k: u8,
        name: &str,
        payload: NodePayload,
    ) -> NodeRecord {
        NodeRecord {
            id: sid(n),
            parent: parent.map(sid),
            order_key: key(k),
            visible: true,
            locked: false,
            display_name: name.to_string(),
            payload,
        }
    }

    pub(crate) fn rect(w: f32, h: f32) -> NodePayload {
        NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: size(w, h),
            corner_radius: Default::default(),
        }))
    }

    /// 고정 fixture(HEAD): 문서 1, 페이지 2, View 3(root frame 10). frame 10 아래 rect 11,
    /// image 12, fragment 13. rect 11에 fill modifier 50.
    pub(crate) fn head_records() -> DocumentRecords {
        let mut definition = DocumentDefinition::new(sid(1), "문서".into());
        let mut page = PageDefinition::new(sid(2), "페이지".into());
        page.add_view(ViewDefinition::new(sid(3), "홈".into(), sid(10)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut records = DocumentRecords::new(definition);
        let frame = FramePayload {
            size: size(320.0, 240.0),
            ..Default::default()
        };
        records
            .nodes
            .push(node(10, None, 0x80, "root", NodePayload::Frame(frame)));
        records
            .nodes
            .push(node(11, Some(10), 0x40, "box", rect(10.0, 20.0)));
        records.nodes.push(node(
            12,
            Some(10),
            0x80,
            "photo",
            NodePayload::Image(ImagePayload::new(AssetRef::new(sid(90)), size(64.0, 64.0))),
        ));
        records.nodes.push(node(
            13,
            Some(10),
            0xC0,
            "group",
            NodePayload::Fragment(FragmentPayload),
        ));
        let mut stack = ModifierStack::new(sid(11));
        stack
            .push_back(ModifierId::new(sid(50)), ModifierKind::Fill)
            .unwrap();
        records.modifier_stacks.push(stack);
        records
    }

    pub(crate) fn build(records: &DocumentRecords) -> SemanticSnapshot {
        let mut builder = SnapshotBuilder::new();
        builder.add_document(records).unwrap();
        builder.finish()
    }

    /// Git blob materialize 경로 모사: View subtree chunk bytes로 직렬화 후 다시 읽습니다.
    fn through_chunks(records: &DocumentRecords) -> DocumentRecords {
        let subtrees = records
            .view_subtrees()
            .unwrap()
            .into_iter()
            .map(|(_, subtree)| SubtreeRecords::decode(&subtree.encode()).unwrap())
            .collect();
        DocumentRecords::from_subtrees(records.definition.clone(), subtrees)
    }

    #[test]
    fn head_index_working_tree_snapshots_build() {
        // HEAD → Index(staged: rect 크기) → WorkingTree(unstaged: ellipse 추가).
        let head = head_records();
        let mut index = head.clone();
        index.nodes[1].payload = rect(30.0, 20.0);
        let mut working = index.clone();
        working.nodes.push(node(
            14,
            Some(10),
            0xE0,
            "dot",
            NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
                bounds: Rect::new(Vec2::new(0.0, 0.0).unwrap(), size(4.0, 4.0)),
            })),
        ));

        let snapshots: Vec<SemanticSnapshot> = [&head, &index, &working]
            .into_iter()
            .map(|records| {
                let direct = build(records);
                // chunk bytes를 거쳐 다시 읽어도 같은 snapshot.
                assert_eq!(direct, build(&through_chunks(records)));
                direct
            })
            .collect();
        let [head_snapshot, index_snapshot, working_snapshot] = &snapshots[..] else {
            unreachable!("snapshot 3개");
        };

        // 문서 1 + 페이지 1 + View 1 + node 4 + modifier 1.
        assert_eq!(head_snapshot.len(), 8);
        assert_eq!(index_snapshot.len(), 8);
        assert_eq!(working_snapshot.len(), 9);
        let rect_size = PropertyPath::with_keys(sid(11), vec![keys::RECT_SIZE]);
        let width = |s: &SemanticSnapshot| s.get(sid(11)).unwrap().property(&rect_size).cloned();
        let floats = |w: f32, h: f32| {
            Some(SemanticValue::Floats(vec![
                FiniteF32::new(w).unwrap(),
                FiniteF32::new(h).unwrap(),
            ]))
        };
        assert_eq!(width(head_snapshot), floats(10.0, 20.0));
        assert_eq!(width(index_snapshot), floats(30.0, 20.0));
        assert_eq!(width(working_snapshot), floats(30.0, 20.0));
        assert!(!index_snapshot.contains(sid(14)));
        assert!(working_snapshot.contains(sid(14)));

        // 위치·종류·범위.
        let modifier = head_snapshot.get(sid(50)).unwrap();
        assert_eq!(modifier.kind(), ObjectKind::Modifier(ModifierKind::Fill));
        assert_eq!(modifier.parent(), Some(sid(11)));
        let view = head_snapshot.get(sid(3)).unwrap();
        assert_eq!(view.parent(), Some(sid(2)));
        assert_eq!(
            head_snapshot.children(sid(10)),
            vec![sid(11), sid(12), sid(13)]
        );
        assert!(head_snapshot
            .objects()
            .all(|o| o.document() == Some(sid(1))));

        // asset 참조 index.
        let refs = head_snapshot.asset_references();
        assert_eq!(refs.len(), 1);
        assert_eq!(
            refs[&sid(90)].iter().cloned().collect::<Vec<_>>(),
            vec![PropertyPath::with_keys(sid(12), vec![keys::IMAGE_ASSET])]
        );
    }

    #[test]
    fn record_order_and_runtime_history_do_not_matter() {
        // runtime 삽입 이력이 다른 두 모델(생성기 모델과, 역순 레코드로 다시 load한 모델)의
        // snapshot이 같습니다. 레코드 순서·chunk 경유도 결과를 바꾸지 않습니다.
        for seed in 0..20 {
            let model = random_valid_document(seed);
            let records = model.to_records().unwrap();
            let mut reversed = records.clone();
            reversed.nodes.reverse();
            reversed.modifier_stacks.reverse();
            assert_eq!(build(&records), build(&reversed), "seed {seed}");
            let (reloaded, _) = DocumentModel::from_records(reversed).unwrap();
            assert_eq!(
                build(&records),
                build(&reloaded.to_records().unwrap()),
                "seed {seed}"
            );
            assert_eq!(
                build(&records),
                build(&through_chunks(&records)),
                "seed {seed}"
            );
        }
    }

    /// `a + b` AST: root 600(Binary op) ← Lhs 601(literal), Rhs 602(token ref). owner = token 200.
    fn ast(operator: u8) -> Vec<ExpressionNodeEntry> {
        let entry =
            |id: u64, payload: Vec<u8>, children: Vec<(u32, StableId)>| ExpressionNodeEntry {
                id: sid(id),
                owner: sid(200),
                document: None,
                payload,
                children,
            };
        vec![
            entry(
                600,
                vec![0x04, operator],
                vec![(1, sid(601)), (2, sid(602))],
            ),
            entry(601, vec![0x01, 0x02, 0, 0, 0x80, 0x3F], vec![]),
            entry(602, vec![0x02, 0x01], vec![]),
        ]
    }

    fn snapshot_of(
        nodes: &[ExpressionNodeEntry],
        variant: &VariantOverrideSet,
    ) -> SemanticSnapshot {
        let mut builder = SnapshotBuilder::new();
        builder.add_expression_nodes(nodes).unwrap();
        builder
            .add_variant(&VariantEntry {
                id: sid(700),
                document: sid(1),
                owner: sid(3),
                overrides: variant.clone(),
            })
            .unwrap();
        builder.finish()
    }

    #[test]
    fn ast_operator_change_is_one_node_change() {
        let variant = VariantOverrideSet::new();
        let base = snapshot_of(&ast(0x01), &variant);
        let target = snapshot_of(&ast(0x02), &variant);
        let changes = crate::diff::diff(&base, &target);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].object_id(), sid(600));
        assert_eq!(
            changes[0].property_path(),
            Some(&PropertyPath::with_keys(sid(600), vec![keys::EXPR_PAYLOAD]))
        );
        // child 교체도 그 node의 child ref 변경 하나입니다.
        let mut swapped = ast(0x01);
        swapped[0].children.reverse();
        let changes = crate::diff::diff(&base, &snapshot_of(&swapped, &variant));
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].property_path().unwrap().keys(),
            &[keys::EXPR_CHILDREN]
        );
        assert_eq!(base.get(sid(601)).unwrap().parent(), Some(sid(200)));
    }

    #[test]
    fn variant_patch_change_is_one_entry_change() {
        let width = PropertyPath::with_keys(sid(11), vec![keys::RECT_SIZE]);
        let height = PropertyPath::with_keys(sid(12), vec![keys::IMAGE_SIZE]);
        let number = |v: f32| TypedValue::Number(FiniteF32::new(v).unwrap());
        let mut base_set = VariantOverrideSet::new();
        base_set.set_property(width.clone(), number(1.0));
        base_set.set_property(height.clone(), number(2.0));
        base_set.set_structure(sid(13), StructurePatch::Delete);
        let mut target_set = base_set.clone();
        target_set.set_property(width, number(5.0));
        let nodes = ast(0x01);
        let changes = crate::diff::diff(
            &snapshot_of(&nodes, &base_set),
            &snapshot_of(&nodes, &target_set),
        );
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].object_id(), sid(700));
        assert_eq!(
            changes[0].property_path().unwrap().keys(),
            &[keys::VARIANT_PROPERTY_PATCH]
        );

        // 구조 patch 하나 변경 → 그 node 항목 하나.
        let mut moved = base_set.clone();
        moved.set_structure(
            sid(13),
            StructurePatch::Move {
                parent: sid(10),
                order_key: key(0x10),
            },
        );
        let changes = crate::diff::diff(
            &snapshot_of(&nodes, &base_set),
            &snapshot_of(&nodes, &moved),
        );
        assert_eq!(changes.len(), 1);
        let path = changes[0].property_path().unwrap();
        assert_eq!(path.keys(), &[keys::VARIANT_STRUCTURE_PATCH]);
        assert_eq!(path.tail(), Some(&PathSegment::Key(sid(13).to_string())));
    }

    #[test]
    fn project_tokens_assets_and_components() {
        let mut builder = SnapshotBuilder::new();
        builder
            .add_project(&ProjectMetadata {
                id: sid(100),
                entries: vec![
                    ("name".into(), "Stellar".into()),
                    ("schema".into(), "1".into()),
                ],
            })
            .unwrap();
        builder
            .add_token(&TokenEntry {
                id: sid(200),
                document: None,
                name: "color.primary".into(),
                value_type: 9,
                value: Some(TypedValue::Number(FiniteF32::ONE)),
                expression: None,
            })
            .unwrap();
        builder
            .add_asset(&AssetEntry {
                id: sid(90),
                content_hash: vec![0xAB; 32],
                media_type: "image/png".into(),
            })
            .unwrap();
        // 컴포넌트: root frame 301, 자식 rect 302, Property 1개.
        let mut library = ComponentLibrary::new();
        let mut definition =
            ComponentDefinition::new(sid(300), "Button".into(), NodeRef::new(sid(301)));
        definition
            .add_property(ComponentPropertyId::new(sid(310)))
            .unwrap();
        let mut tree = DefinitionTree::new(ComponentRef::new(sid(300)));
        let def_node =
            |n: u64, parent: Option<u64>, kind: NodeKind, payload: Vec<u8>| DefinitionNode {
                id: sid(n),
                kind,
                parent: parent.map(sid),
                order_key: key(0x80),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload,
            };
        tree.insert(def_node(
            301,
            None,
            NodeKind::Frame,
            encode_payload(&NodePayload::Frame(FramePayload::default())),
        ))
        .unwrap();
        tree.insert(def_node(
            302,
            Some(301),
            NodeKind::Shape,
            encode_payload(&rect(1.0, 1.0)),
        ))
        .unwrap();
        library.insert(definition, tree).unwrap();
        builder.add_component_library(sid(1), &library).unwrap();

        let mut shared = SnapshotBuilder::new();
        shared.add_project_component_library(&library).unwrap();
        let shared = shared.finish();
        let empty = SemanticSnapshot::new();
        let diff = crate::diff::diff(&empty, &shared);
        assert!(diff
            .iter()
            .all(|c| c.document().is_none()
                && c.provenance().definition_component() == Some(sid(300))));
        assert_eq!(
            crate::codec::snapshot_from_bytes(&crate::codec::snapshot_to_bytes(&shared)).unwrap(),
            shared
        );
        assert_eq!(
            crate::diff::SemanticDiff::from_canonical_bytes(&diff.to_canonical_bytes()).unwrap(),
            diff
        );

        // 중복은 원자적으로 거부합니다.
        assert_eq!(
            builder.add_asset(&AssetEntry {
                id: sid(90),
                content_hash: vec![],
                media_type: String::new()
            }),
            Err(SnapshotError::DuplicateObject { id: sid(90) })
        );
        let snapshot = builder.finish();
        assert_eq!(snapshot.len(), 6);
        let project = snapshot.get(sid(100)).unwrap();
        let name = PropertyPath::with_keys(sid(100), vec![keys::PROJECT_METADATA])
            .with_key_segment("name");
        assert_eq!(
            project.property(&name),
            Some(&SemanticValue::Text("Stellar".into()))
        );
        assert_eq!(snapshot.get(sid(200)).unwrap().origin(), Origin::Project);
        let def = snapshot.get(sid(302)).unwrap();
        assert_eq!(def.kind(), ObjectKind::Node(NodeKind::Shape));
        assert_eq!(
            def.origin(),
            Origin::Component {
                document: sid(1),
                component: sid(300)
            }
        );
        let component = snapshot.get(sid(300)).unwrap();
        assert_eq!(
            component.property(&component.path(&[keys::COMPONENT_PROPERTIES])),
            Some(&SemanticValue::Ids(vec![sid(310)]))
        );
    }

    #[test]
    fn duplicate_metadata_names_are_rejected() {
        let mut builder = SnapshotBuilder::new();
        let result = builder.add_project(&ProjectMetadata {
            id: sid(100),
            entries: vec![("a".into(), "1".into()), ("a".into(), "2".into())],
        });
        assert!(matches!(
            result,
            Err(SnapshotError::DuplicateProperty { .. })
        ));
        assert!(builder.finish().is_empty());
    }
}
