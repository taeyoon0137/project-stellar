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

    /// host Frame에 Slot을 선언합니다. default content는 등록 시점의 host 자식입니다.
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
        let slots: Vec<ComponentSlot> = self
            .slots
            .into_iter()
            .map(|slot| {
                let content = self
                    .tree
                    .children(slot.host.id())
                    .iter()
                    .map(|child| NodeRef::new(*child))
                    .collect();
                ComponentSlot::new(
                    slot.id,
                    slot.host,
                    slot.allowed_kinds().collect::<Vec<_>>(),
                    slot.cardinality,
                    content,
                )
            })
            .collect();
        library
            .insert(self.definition, self.tree)
            .expect("컴포넌트 등록");
        for slot in slots {
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

/// 숫자 리터럴 AST.
pub fn number(seed: u64, value: f32) -> ExpressionAst {
    literal(
        seed,
        TypedValue::Number(stellar_core_types::value::FiniteF32::new(value).expect("유한")),
    )
}

/// `Context(key) + add` AST.
pub fn context_plus(seed: u64, key: StableId, add: f32) -> ExpressionAst {
    use stellar_expressions::ops::BinaryOp;
    let read = expr_id(seed, 1);
    let lit = expr_id(seed, 2);
    let root = expr_id(seed, 3);
    ExpressionAst::new(
        root,
        vec![
            ExprNode::reference(read, ReferenceTarget::Context(key)),
            ExprNode::literal(
                lit,
                TypedValue::Number(stellar_core_types::value::FiniteF32::new(add).expect("유한")),
            ),
            ExprNode::binary(root, BinaryOp::Add, read, lit),
        ],
    )
    .expect("덧셈 AST")
}

/// Number 필수 타입.
pub fn number_type() -> TypeSpec {
    TypeSpec::required(ExpressionType::Number)
}

/// [`sample_world`]의 id.
pub mod world {
    /// 문서.
    pub const DOC: u64 = 1;
    /// 페이지.
    pub const PAGE: u64 = 2;
    /// View.
    pub const VIEW: u64 = 3;
    /// View root Frame.
    pub const ROOT: u64 = 10;
    /// scene Card 인스턴스.
    pub const CARD_INSTANCE: u64 = 11;
    /// Card body slot 콘텐츠(scene, View 최상위에서 작성).
    pub const CONTENT: u64 = 12;
    /// scene Frame.
    pub const FRAME: u64 = 13;
    /// slot 콘텐츠의 자식.
    pub const CONTENT_CHILD: u64 = 14;
    /// Card 컴포넌트.
    pub const CARD: u64 = 100;
    /// Card root.
    pub const CARD_ROOT: u64 = 101;
    /// Card body slot host.
    pub const CARD_HOST: u64 = 102;
    /// Card 안 Button 인스턴스.
    pub const CARD_BUTTON: u64 = 103;
    /// Card body slot.
    pub const BODY: u64 = 105;
    /// Card가 Button slot에 작성한 콘텐츠.
    pub const CARD_LABEL: u64 = 106;
    /// Card 안 Frame.
    pub const CARD_FRAME: u64 = 107;
    /// Button 컴포넌트.
    pub const BUTTON: u64 = 200;
    /// Button root(= label slot host).
    pub const BUTTON_ROOT: u64 = 201;
    /// Button 안 Frame.
    pub const BUTTON_FRAME: u64 = 202;
    /// Button label slot.
    pub const LABEL: u64 = 205;
    /// Number key N(default 1).
    pub const N: u64 = 900;
    /// Number key D(default 0).
    pub const D: u64 = 901;
    /// Number key E(default 0).
    pub const E: u64 = 902;
    /// View root가 N = 10 공급.
    pub const P_ROOT_N: u64 = 1000;
    /// View root가 E = 3 공급.
    pub const P_ROOT_E: u64 = 1001;
    /// Card root가 N = Context(N) + 5 공급(같은 key 재읽기).
    pub const P_CARD_N: u64 = 1002;
    /// Card body host가 D = 2 공급.
    pub const P_HOST_D: u64 = 1003;
    /// Button root가 D = 7 공급.
    pub const P_BUTTON_D: u64 = 1004;
}

/// 해석 시험 공용 문서: View root ← Card 인스턴스(body slot ← 12 ← 14), Frame 13. Card: root ← body host, Button 인스턴스
/// (label slot ← 106), Frame 107. Button: root(label host) ← Frame 202. body는 D·E를, label은 D를 공개합니다. Card는 N을
/// requirement로 선언합니다.
pub fn sample_world() -> (DocumentModel, ComponentLibrary) {
    use self::world::*;
    use crate::context::key::{ContextKey, ContextScope};
    use crate::context::provision::{ContextOwner, ContextProvision};
    use crate::context::set::ContextSet;

    let mut library = ComponentLibrary::new();
    ComponentBuilder::new(sid(BUTTON), "Button", sid(BUTTON_ROOT))
        .frame(sid(BUTTON_FRAME), sid(BUTTON_ROOT))
        .slot(sid(LABEL), sid(BUTTON_ROOT))
        .register(&mut library);
    ComponentBuilder::new(sid(CARD), "Card", sid(CARD_ROOT))
        .frame(sid(CARD_HOST), sid(CARD_ROOT))
        .instance(
            sid(CARD_BUTTON),
            sid(CARD_ROOT),
            instance_of(sid(BUTTON), &[(sid(LABEL), vec![sid(CARD_LABEL)])]),
        )
        .frame(sid(CARD_LABEL), sid(CARD_BUTTON))
        .frame(sid(CARD_FRAME), sid(CARD_ROOT))
        .slot(sid(BODY), sid(CARD_HOST))
        .register(&mut library);
    let mut model = SceneBuilder::new(sid(DOC), sid(PAGE), sid(VIEW), sid(ROOT))
        .instance(
            sid(CARD_INSTANCE),
            sid(ROOT),
            instance_of(sid(CARD), &[(sid(BODY), vec![sid(CONTENT)])]),
        )
        .frame(sid(CONTENT), sid(CARD_INSTANCE))
        .frame(sid(CONTENT_CHILD), sid(CONTENT))
        .frame(sid(FRAME), sid(ROOT))
        .build();
    let mut set = ContextSet::new();
    for (id, name, default) in [(N, "n", 1.0), (D, "d", 0.0), (E, "e", 0.0)] {
        set.insert_key(
            ContextKey::new(
                sid(id),
                name,
                ContextScope::Document(sid(DOC)),
                number_type(),
                number(id, default),
            )
            .expect("key"),
        )
        .expect("key 등록");
    }
    let provisions = [
        (P_ROOT_N, ContextOwner::View(sid(VIEW)), ROOT, N, number(P_ROOT_N, 10.0)),
        (P_ROOT_E, ContextOwner::View(sid(VIEW)), ROOT, E, number(P_ROOT_E, 3.0)),
        (
            P_CARD_N,
            ContextOwner::Component(sid(CARD)),
            CARD_ROOT,
            N,
            context_plus(P_CARD_N, sid(N), 5.0),
        ),
        (P_HOST_D, ContextOwner::Component(sid(CARD)), CARD_HOST, D, number(P_HOST_D, 2.0)),
        (
            P_BUTTON_D,
            ContextOwner::Component(sid(BUTTON)),
            BUTTON_ROOT,
            D,
            number(P_BUTTON_D, 7.0),
        ),
    ];
    for (id, owner, provider, key, value) in provisions {
        set.insert_provision(
            ContextProvision::new(sid(id), owner, sid(provider), sid(key), value)
                .expect("provision"),
        )
        .expect("provision 등록");
    }
    set.add_slot_key(sid(BODY), sid(D));
    set.add_slot_key(sid(BODY), sid(E));
    set.add_slot_key(sid(LABEL), sid(D));
    set.add_requirement(sid(CARD), sid(N));
    model.replace_context(set);
    (model, library)
}
