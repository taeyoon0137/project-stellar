//! Context 시험용 문서·컴포넌트 구성 도우미(`test-support`).
//!
//! scene(View 하나)과 컴포넌트 library를 짧은 호출로 만듭니다. id는 [`sid`]로 만든 결정적 StableId입니다.

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::types::{ExpressionType, TypeSpec};

use crate::component::definition::{
    ComponentDefinition, ComponentLibrary, ComponentSlotId, DefinitionNode, DefinitionTree,
};
use crate::component::instance::{ComponentInstancePayload, SlotOverride};
use crate::component::slot::{ComponentSlot, SlotCardinality};
use crate::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use crate::frame::FramePayload;
use crate::model::DocumentModel;
use crate::node_kind::NodeKind;
use crate::record_codec::encode_payload;
use crate::records::{NodePayload, NodeRecord};

/// 결정적 StableId.
pub fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0x0c7e, n)
}

/// sRGB 불투명 색.
pub fn rgb(r: f32, g: f32, b: f32) -> TypedValue {
    TypedValue::Color(Color::new(ColorSpace::Srgb, r, g, b, 1.0).expect("유한 색"))
}

/// Color 필수 타입.
pub fn color_type() -> TypeSpec {
    TypeSpec::required(ExpressionType::Color)
}

fn expr_id(seed: u64, n: u64) -> ExprNodeId {
    ExprNodeId::new(StableId::new_v7(seed, 0x0e7e, n))
}

/// 리터럴 하나짜리 AST(`seed`로 node id를 정합니다).
pub fn literal(seed: u64, value: TypedValue) -> ExpressionAst {
    let id = expr_id(seed, 1);
    ExpressionAst::new(id, vec![ExprNode::literal(id, value)]).expect("리터럴 AST")
}

/// 참조 하나짜리 AST.
pub fn reference(seed: u64, target: ReferenceTarget) -> ExpressionAst {
    let id = expr_id(seed, 1);
    ExpressionAst::new(id, vec![ExprNode::reference(id, target)]).expect("참조 AST")
}

/// `Context(key)` 하나를 읽는 AST.
pub fn context_read(seed: u64, key: StableId) -> ExpressionAst {
    reference(seed, ReferenceTarget::Context(key))
}

/// `if Context(flag) then Context(a) else Context(b)` 같은 조합이 필요할 때 쓰는 `to_srgb(Context(key))` 형태 AST.
pub fn context_to_srgb(seed: u64, key: StableId) -> ExpressionAst {
    use stellar_expressions::functions::FunctionId;
    let read = expr_id(seed, 1);
    let call = expr_id(seed, 2);
    ExpressionAst::new(
        call,
        vec![
            ExprNode::reference(read, ReferenceTarget::Context(key)),
            ExprNode::call(call, FunctionId::ToSrgb, &[read]),
        ],
    )
    .expect("함수 AST")
}

fn order(n: u64) -> OrderKey {
    OrderKey::from_bytes(vec![0x10 + (n % 0xE0) as u8]).expect("order key")
}

fn frame_bytes() -> Vec<u8> {
    encode_payload(&NodePayload::Frame(FramePayload::default()))
}

fn instance_bytes(payload: &ComponentInstancePayload) -> Vec<u8> {
    encode_payload(&NodePayload::ComponentInstance(payload.clone()))
}

/// 컴포넌트 정의 하나를 쌓는 builder.
#[derive(Debug)]
pub struct ComponentBuilder {
    definition: ComponentDefinition,
    tree: DefinitionTree,
    slots: Vec<ComponentSlot>,
    count: u64,
}

impl ComponentBuilder {
    /// root Frame `root`를 가진 컴포넌트 `id`.
    pub fn new(id: StableId, name: &str, root: StableId) -> ComponentBuilder {
        let component = ComponentRef::new(id);
        let mut tree = DefinitionTree::new(component);
        tree.insert(DefinitionNode {
            id: root,
            kind: NodeKind::Frame,
            parent: None,
            order_key: order(0),
            visible: true,
            locked: false,
            display_name: format!("{name} root"),
            payload: frame_bytes(),
        })
        .expect("root");
        ComponentBuilder {
            definition: ComponentDefinition::new(id, name.to_owned(), NodeRef::new(root)),
            tree,
            slots: Vec::new(),
            count: 1,
        }
    }

    /// Frame node를 더합니다.
    pub fn frame(mut self, id: StableId, parent: StableId) -> ComponentBuilder {
        self.count += 1;
        self.tree
            .insert(DefinitionNode {
                id,
                kind: NodeKind::Frame,
                parent: Some(parent),
                order_key: order(self.count),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload: frame_bytes(),
            })
            .expect("frame");
        self
    }

    /// nested ComponentInstance node를 더합니다.
    pub fn instance(
        mut self,
        id: StableId,
        parent: StableId,
        payload: ComponentInstancePayload,
    ) -> ComponentBuilder {
        self.count += 1;
        self.tree
            .insert(DefinitionNode {
                id,
                kind: NodeKind::ComponentInstance,
                parent: Some(parent),
                order_key: order(self.count),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload: instance_bytes(&payload),
            })
            .expect("instance");
        self
    }

    /// host Frame에 Slot을 선언합니다.
    pub fn slot(mut self, slot: StableId, host: StableId) -> ComponentBuilder {
        let id = ComponentSlotId::new(slot);
        self.definition.add_slot(id).expect("slot 선언");
        self.slots.push(ComponentSlot::new(
            id,
            NodeRef::new(host),
            [NodeKind::Frame, NodeKind::Text, NodeKind::ComponentInstance],
            SlotCardinality::unbounded(),
            Vec::new(),
        ));
        self
    }

    /// library에 등록합니다.
    pub fn register(self, library: &mut ComponentLibrary) {
        let component = self.definition.reference();
        library
            .insert(self.definition, self.tree)
            .expect("컴포넌트 등록");
        for slot in self.slots {
            library.add_slot(component, slot).expect("slot record");
        }
    }
}

/// 인스턴스 payload(slot override 포함).
pub fn instance_of(component: StableId, slots: &[(StableId, Vec<StableId>)]) -> ComponentInstancePayload {
    let mut payload = ComponentInstancePayload::new(ComponentRef::new(component));
    for (slot, content) in slots {
        payload.set_slot_override(
            ComponentSlotId::new(*slot),
            SlotOverride::new(content.clone()).expect("slot content"),
        );
    }
    payload
}

/// View 하나짜리 문서 scene builder.
#[derive(Debug)]
pub struct SceneBuilder {
    model: DocumentModel,
    count: u64,
}

impl SceneBuilder {
    /// 문서 `document`, 페이지 `page`, View `view`(root Frame `root`).
    pub fn new(document: StableId, page: StableId, view: StableId, root: StableId) -> SceneBuilder {
        let mut definition = DocumentDefinition::new(document, "문서".into());
        let mut page_definition = PageDefinition::new(page, "페이지".into());
        page_definition
            .add_view(ViewDefinition::new(view, "View".into(), root))
            .expect("view");
        definition.add_page(page_definition).expect("page");
        let mut model = DocumentModel::new(definition);
        model
            .insert_node(NodeRecord {
                id: root,
                parent: None,
                order_key: order(0),
                visible: true,
                locked: false,
                display_name: "root".into(),
                payload: NodePayload::Frame(FramePayload::default()),
            })
            .expect("root");
        SceneBuilder { model, count: 1 }
    }

    fn insert(mut self, id: StableId, parent: StableId, payload: NodePayload) -> SceneBuilder {
        self.count += 1;
        self.model
            .insert_node(NodeRecord {
                id,
                parent: Some(parent),
                order_key: order(self.count),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload,
            })
            .expect("scene node");
        self
    }

    /// Frame을 더합니다.
    pub fn frame(self, id: StableId, parent: StableId) -> SceneBuilder {
        self.insert(id, parent, NodePayload::Frame(FramePayload::default()))
    }

    /// ComponentInstance를 더합니다.
    pub fn instance(
        self,
        id: StableId,
        parent: StableId,
        payload: ComponentInstancePayload,
    ) -> SceneBuilder {
        self.insert(id, parent, NodePayload::ComponentInstance(payload))
    }

    /// 완성된 모델.
    pub fn build(self) -> DocumentModel {
        self.model
    }
}
