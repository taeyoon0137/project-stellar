//! 읽기 전용 `PrototypeGraphView` (XXX.2.1, ADR-0025 결정 2.1, ADR-0094 결정 3).
//!
//! 소유자의 binding(속성 출력·expression block), 상태 머신(상태 변수·이벤트 원천·Transition·semantic emit), MotionSpec에서
//! node·edge 목록을 만듭니다. 새 원본을 만들지 않는 projection이며, 같은 입력은 같은 bytes([`PrototypeGraphView::to_bytes`])를
//! 냅니다. node는 (StableId, 종류 tag) 순, edge는 (출력 socket, 입력 socket) 순입니다.
//!
//! node 원본과 StableId:
//!
//! | node | StableId |
//! |---|---|
//! | PropertyOutput | binding id |
//! | ExpressionBlock | Expression AST node id |
//! | StateVariable | 상태 변수 id |
//! | EventSource | 이벤트를 내는 정의 tree node(nested action이면 nested instance node) |
//! | Transition | transition id |
//! | SemanticEmit | semantic action id |
//! | MotionSpec | MotionSpec id |
//!
//! Context 읽기 node는 Context 원본(XXX.7.4)이 생기기 전에는 만들지 않습니다.
//! [`lower_bindings`]·[`lower_transitions`]는 projection에서 원본을 다시 만들어, projection이 원본 의미를 잃지 않는지
//! 확인하는 데 씁니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_bindings::binding::{BindingId, BindingSet, ValueBinding};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::event::EventKind;
use stellar_document::component::state_machine::{
    event_payload_path, state_reference, ActionSpec, SemanticActionId, StateVariableId,
    TransitionAction, TransitionId, TransitionSpec, Trigger,
};
use stellar_document::motion::{MotionOwner, MotionSet, MotionSpecId};
use stellar_document::prototype::node_kind::PrototypeNodeKind;
use stellar_document::prototype::socket::SocketKind;
use stellar_expressions::ast::{
    ChildRef, ChildSlot, ExprNode, ExprNodeId, ExpressionAst, NodePayload, ReferenceTarget,
};
use stellar_expressions::graph_view::ExpressionGraphView;
use stellar_expressions::types::TypeSpec;

/// 그래프 node 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphNodeId {
    /// 원본 StableId.
    pub id: StableId,
    /// node 종류.
    pub kind: PrototypeNodeKind,
}

impl GraphNodeId {
    /// 만듭니다.
    pub const fn new(id: StableId, kind: PrototypeNodeKind) -> GraphNodeId {
        GraphNodeId { id, kind }
    }
}

/// 반복 socket의 구분 key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SocketKey {
    /// 반복이 아닌 socket.
    Single,
    /// expression child slot.
    Slot(ChildSlot),
    /// 이벤트 종류(EventSource의 event·payload).
    Event(EventKind),
    /// nested instance가 낸 semantic action(EventSource의 event).
    Action(SemanticActionId),
    /// action 순번(Transition의 set-value·set·emit).
    Index(u16),
    /// MotionSpec 대상 순번(MotionSpec의 drive).
    Target(u16),
    /// 상태 변수에 쓰는 transition(StateVariable의 write).
    Writer(TransitionId),
    /// emit하는 transition(SemanticEmit의 emit).
    Emitter(TransitionId),
}

/// socket 주소.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphSocket {
    /// node.
    pub node: GraphNodeId,
    /// socket 이름(node 종류 registry의 이름).
    pub name: &'static str,
    /// 반복 구분.
    pub key: SocketKey,
}

impl GraphSocket {
    /// 만듭니다.
    pub const fn new(node: GraphNodeId, name: &'static str, key: SocketKey) -> GraphSocket {
        GraphSocket { node, name, key }
    }

    /// registry가 정한 socket 종류.
    pub fn kind(&self) -> Option<SocketKind> {
        self.node.kind.socket(self.name).map(|s| s.kind)
    }
}

/// node 원본 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeDetail {
    /// binding target 속성.
    Property {
        /// binding.
        binding: BindingId,
        /// 대상.
        target: PropertyPath,
        /// binding 기대 타입.
        expected: TypeSpec,
    },
    /// expression block.
    Expression {
        /// 어느 원본 식의 block인지.
        origin: ExpressionOrigin,
        /// block payload.
        payload: NodePayload,
    },
    /// 상태 변수.
    State {
        /// 변수.
        variable: StateVariableId,
        /// 타입.
        value_type: TypeSpec,
    },
    /// 이벤트 원천.
    EventSource {
        /// node.
        source: StableId,
    },
    /// transition.
    Transition {
        /// transition.
        transition: TransitionId,
        /// 상태 머신 목록 순번(같은 trigger 안 우선순위).
        priority: u16,
    },
    /// semantic action.
    Emit {
        /// action.
        action: SemanticActionId,
    },
    /// MotionSpec.
    Motion {
        /// spec.
        motion: MotionSpecId,
    },
}

/// expression block이 속한 원본 식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExpressionOrigin {
    /// binding source.
    Binding(BindingId),
    /// transition 조건.
    Condition(TransitionId),
    /// transition의 `index`번째 SetState 값.
    ActionValue(TransitionId, u16),
}

/// 그래프 node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    /// 식별자.
    pub id: GraphNodeId,
    /// 표시 이름.
    pub label: String,
    /// 원본 정보.
    pub detail: NodeDetail,
}

/// 그래프 edge(출력 → 입력).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphEdge {
    /// 출력 socket.
    pub from: GraphSocket,
    /// 입력 socket.
    pub to: GraphSocket,
}

/// projection 범위.
#[derive(Debug, Clone)]
pub enum ProjectionScope {
    /// 컴포넌트 정의(대상은 정의 tree node).
    Component(stellar_core_types::reference::ComponentRef),
    /// View(대상은 `nodes`에 든 문서 node).
    View {
        /// View id.
        view: StableId,
        /// View subtree node.
        nodes: BTreeSet<StableId>,
    },
}

impl ProjectionScope {
    /// MotionSpec 소유자.
    pub fn owner(&self) -> MotionOwner {
        match self {
            ProjectionScope::Component(c) => MotionOwner::Component(*c),
            ProjectionScope::View { view, .. } => MotionOwner::View(*view),
        }
    }
}

/// projection 입력.
#[derive(Clone, Copy)]
pub struct ProjectionInput<'a> {
    /// 컴포넌트 library.
    pub library: &'a ComponentLibrary,
    /// binding 모음.
    pub bindings: &'a BindingSet,
    /// MotionSpec 모음.
    pub motions: &'a MotionSet,
}

/// 읽기 전용 프로토타입 그래프.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrototypeGraphView {
    owner: MotionOwner,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

const fn node(id: StableId, kind: PrototypeNodeKind) -> GraphNodeId {
    GraphNodeId::new(id, kind)
}

struct Builder {
    nodes: BTreeMap<GraphNodeId, GraphNode>,
    edges: BTreeSet<GraphEdge>,
}

impl Builder {
    fn add(&mut self, id: GraphNodeId, label: String, detail: NodeDetail) {
        self.nodes
            .entry(id)
            .or_insert(GraphNode { id, label, detail });
    }

    fn edge(&mut self, from: GraphSocket, to: GraphSocket) {
        self.edges.insert(GraphEdge { from, to });
    }

    /// 식 하나의 block과 내부 edge를 더하고 root block의 result socket을 돌려줍니다. 상태 변수·이벤트 payload 참조
    /// block에는 원천에서 `reference` edge를 잇습니다.
    fn expression(
        &mut self,
        ast: &ExpressionAst,
        origin: ExpressionOrigin,
        payload_source: &BTreeMap<TransitionId, (StableId, EventKind)>,
    ) -> GraphSocket {
        let view = ExpressionGraphView::new(ast);
        for block in view.blocks() {
            let id = node(
                block.id().node().stable_id(),
                PrototypeNodeKind::ExpressionBlock,
            );
            self.add(
                id,
                block.label().to_string(),
                NodeDetail::Expression {
                    origin,
                    payload: block.payload().clone(),
                },
            );
            if let NodePayload::Reference(target) = block.payload() {
                let reference = GraphSocket::new(id, "reference", SocketKey::Single);
                if let Some(variable) = state_reference(target) {
                    let from = node(variable.stable_id(), PrototypeNodeKind::StateVariable);
                    self.edge(
                        GraphSocket::new(from, "value", SocketKey::Single),
                        reference,
                    );
                } else if let ReferenceTarget::Property(path) = target {
                    if let Some((transition, (source, kind))) = payload_source
                        .iter()
                        .find(|(t, _)| event_payload_path(**t) == *path)
                    {
                        let _ = transition;
                        let from = node(*source, PrototypeNodeKind::EventSource);
                        self.edge(
                            GraphSocket::new(from, "payload", SocketKey::Event(*kind)),
                            reference,
                        );
                    }
                }
            }
        }
        for edge in view.edges() {
            let from = node(
                edge.source.node().stable_id(),
                PrototypeNodeKind::ExpressionBlock,
            );
            let to = node(
                edge.target.block.node().stable_id(),
                PrototypeNodeKind::ExpressionBlock,
            );
            self.edge(
                GraphSocket::new(from, "result", SocketKey::Single),
                GraphSocket::new(to, "operand", SocketKey::Slot(edge.target.slot)),
            );
        }
        GraphSocket::new(
            node(
                view.root().node().stable_id(),
                PrototypeNodeKind::ExpressionBlock,
            ),
            "result",
            SocketKey::Single,
        )
    }
}

fn in_scope(scope: &ProjectionScope, library: &ComponentLibrary, root: StableId) -> bool {
    match scope {
        ProjectionScope::Component(component) => library
            .get(*component)
            .is_some_and(|entry| entry.tree().contains(root)),
        ProjectionScope::View { nodes, .. } => nodes.contains(&root),
    }
}

/// 속성 출력 node 표시 이름: CSS longhand 이름(registry), 모르면 key 번호.
fn property_label(target: &PropertyPath) -> String {
    match target.keys() {
        [key] => stellar_document::css::registry::name_of(*key)
            .or_else(|| {
                stellar_document::layout_record::LayoutLonghand::ALL
                    .iter()
                    .find(|l| l.record_key() == *key)
                    .map(|l| l.name())
            })
            .map(str::to_string)
            .unwrap_or_else(|| format!("0x{:04x}", key.0)),
        keys => format!("{keys:?}"),
    }
}

/// 소유자 범위의 binding(binding id 순).
pub fn scoped_bindings<'a>(
    scope: &'a ProjectionScope,
    input: ProjectionInput<'a>,
) -> impl Iterator<Item = &'a ValueBinding> + 'a {
    let mut list: Vec<&ValueBinding> = input
        .bindings
        .iter()
        .filter(|b| in_scope(scope, input.library, b.target().root()))
        .collect();
    list.sort_by_key(|b| b.id());
    list.into_iter()
}

impl PrototypeGraphView {
    /// projection을 만듭니다.
    pub fn project(scope: &ProjectionScope, input: ProjectionInput<'_>) -> PrototypeGraphView {
        let mut b = Builder {
            nodes: BTreeMap::new(),
            edges: BTreeSet::new(),
        };
        let mut payload_source: BTreeMap<TransitionId, (StableId, EventKind)> = BTreeMap::new();

        // 상태 머신(컴포넌트 소유자만).
        if let ProjectionScope::Component(component) = scope {
            if let Some(entry) = input.library.get(*component) {
                let machine = entry.state_machine();
                for variable in machine.variables() {
                    b.add(
                        node(variable.id().stable_id(), PrototypeNodeKind::StateVariable),
                        variable.name().to_string(),
                        NodeDetail::State {
                            variable: variable.id(),
                            value_type: variable.value_type(),
                        },
                    );
                }
                for action in machine.actions() {
                    let id = node(action.id().stable_id(), PrototypeNodeKind::SemanticEmit);
                    b.add(
                        id,
                        action.name().to_string(),
                        NodeDetail::Emit {
                            action: action.id(),
                        },
                    );
                }
                for transition in machine.transitions() {
                    if let Trigger::Event { kind, source } = transition.trigger() {
                        if kind.payload_type().is_some() {
                            payload_source.insert(transition.id(), (source, kind));
                        }
                    }
                }
                for (priority, transition) in machine.transitions().iter().enumerate() {
                    let t = node(transition.id().stable_id(), PrototypeNodeKind::Transition);
                    b.add(
                        t,
                        "transition".to_string(),
                        NodeDetail::Transition {
                            transition: transition.id(),
                            priority: priority as u16,
                        },
                    );
                    let (source, key) = match transition.trigger() {
                        Trigger::Event { kind, source } => (source, SocketKey::Event(kind)),
                        Trigger::NestedAction { instance, action } => {
                            (instance, SocketKey::Action(action))
                        }
                    };
                    let s = node(source, PrototypeNodeKind::EventSource);
                    let label = entry
                        .tree()
                        .node(source)
                        .map(|n| n.display_name.clone())
                        .unwrap_or_default();
                    b.add(s, label, NodeDetail::EventSource { source });
                    b.edge(
                        GraphSocket::new(s, "event", key),
                        GraphSocket::new(t, "trigger", SocketKey::Single),
                    );
                    if let Some(condition) = transition.condition() {
                        let root = b.expression(
                            condition.ast(),
                            ExpressionOrigin::Condition(transition.id()),
                            &payload_source,
                        );
                        b.edge(root, GraphSocket::new(t, "condition", SocketKey::Single));
                    }
                    for (index, action) in transition.actions().iter().enumerate() {
                        let index = index as u16;
                        match action {
                            TransitionAction::SetState { variable, value } => {
                                let root = b.expression(
                                    value.ast(),
                                    ExpressionOrigin::ActionValue(transition.id(), index),
                                    &payload_source,
                                );
                                b.edge(
                                    root,
                                    GraphSocket::new(t, "set-value", SocketKey::Index(index)),
                                );
                                let v =
                                    node(variable.stable_id(), PrototypeNodeKind::StateVariable);
                                b.edge(
                                    GraphSocket::new(t, "set", SocketKey::Index(index)),
                                    GraphSocket::new(
                                        v,
                                        "write",
                                        SocketKey::Writer(transition.id()),
                                    ),
                                );
                            }
                            TransitionAction::Emit { action } => {
                                let e = node(action.stable_id(), PrototypeNodeKind::SemanticEmit);
                                b.edge(
                                    GraphSocket::new(t, "emit", SocketKey::Index(index)),
                                    GraphSocket::new(
                                        e,
                                        "emit",
                                        SocketKey::Emitter(transition.id()),
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }

        // binding.
        let mut output_of: BTreeMap<PropertyPath, GraphNodeId> = BTreeMap::new();
        for binding in scoped_bindings(scope, input) {
            let out = node(binding.id().stable_id(), PrototypeNodeKind::PropertyOutput);
            b.add(
                out,
                property_label(binding.target()),
                NodeDetail::Property {
                    binding: binding.id(),
                    target: binding.target().clone(),
                    expected: binding.expected_type(),
                },
            );
            output_of.insert(binding.target().clone(), out);
            let root = b.expression(
                binding.source(),
                ExpressionOrigin::Binding(binding.id()),
                &payload_source,
            );
            b.edge(root, GraphSocket::new(out, "value", SocketKey::Single));
        }

        // MotionSpec.
        let owner = scope.owner();
        for spec in input.motions.owned_by(owner) {
            let m = node(spec.id().stable_id(), PrototypeNodeKind::MotionSpec);
            b.add(
                m,
                "motion".to_string(),
                NodeDetail::Motion { motion: spec.id() },
            );
            let variable = spec.start().variable();
            let v = node(variable.stable_id(), PrototypeNodeKind::StateVariable);
            if b.nodes.contains_key(&v) {
                b.edge(
                    GraphSocket::new(v, "value", SocketKey::Single),
                    GraphSocket::new(m, "start", SocketKey::Single),
                );
            }
            for (index, target) in spec.targets().iter().enumerate() {
                if let Some(out) = output_of.get(target) {
                    b.edge(
                        GraphSocket::new(m, "drive", SocketKey::Target(index as u16)),
                        GraphSocket::new(*out, "motion", SocketKey::Single),
                    );
                }
            }
        }

        // 원천이 없는 edge(예: 상태 변수가 없는 참조)는 남기지 않습니다.
        let nodes = b.nodes;
        let edges = b
            .edges
            .into_iter()
            .filter(|e| nodes.contains_key(&e.from.node) && nodes.contains_key(&e.to.node))
            .collect();
        PrototypeGraphView {
            owner,
            nodes: nodes.into_values().collect(),
            edges,
        }
    }

    /// 소유자.
    pub fn owner(&self) -> MotionOwner {
        self.owner
    }

    /// node((StableId, 종류) 순).
    pub fn nodes(&self) -> &[GraphNode] {
        &self.nodes
    }

    /// edge(출력·입력 순).
    pub fn edges(&self) -> &[GraphEdge] {
        &self.edges
    }

    /// node 하나.
    pub fn node(&self, id: GraphNodeId) -> Option<&GraphNode> {
        self.nodes
            .binary_search_by(|n| n.id.cmp(&id))
            .ok()
            .map(|i| &self.nodes[i])
    }

    /// `to` 입력 socket에 들어오는 edge의 출력 socket.
    pub fn source_of(&self, to: GraphSocket) -> Option<GraphSocket> {
        self.edges.iter().find(|e| e.to == to).map(|e| e.from)
    }

    /// 결정론 확인용 canonical bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        fn socket(out: &mut Vec<u8>, s: &GraphSocket) {
            node_id(out, &s.node);
            out.extend_from_slice(s.name.as_bytes());
            out.push(0);
            out.extend_from_slice(format!("{:?}", s.key).as_bytes());
            out.push(0);
        }
        fn node_id(out: &mut Vec<u8>, id: &GraphNodeId) {
            out.extend_from_slice(&id.id.to_bytes());
            let (tag, sub) = id.kind.tag();
            out.push(tag);
            out.push(sub);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&self.owner.id().to_bytes());
        out.extend_from_slice(&(self.nodes.len() as u32).to_le_bytes());
        for n in &self.nodes {
            node_id(&mut out, &n.id);
            out.extend_from_slice(n.label.as_bytes());
            out.push(0);
        }
        out.extend_from_slice(&(self.edges.len() as u32).to_le_bytes());
        for e in &self.edges {
            socket(&mut out, &e.from);
            socket(&mut out, &e.to);
        }
        out
    }
}

/// `socket`에 들어오는 식을 다시 만듭니다(block과 operand edge를 따라감).
fn rebuild_expression(view: &PrototypeGraphView, root: GraphSocket) -> Option<ExpressionAst> {
    let root_block = view.source_of(root)?.node;
    let mut nodes = Vec::new();
    let mut stack = vec![root_block];
    let mut seen = BTreeSet::new();
    while let Some(block) = stack.pop() {
        if !seen.insert(block) {
            continue;
        }
        let NodeDetail::Expression { payload, .. } = &view.node(block)?.detail else {
            return None;
        };
        let mut children = Vec::new();
        for edge in view
            .edges()
            .iter()
            .filter(|e| e.to.node == block && e.to.name == "operand")
        {
            let SocketKey::Slot(slot) = edge.to.key else {
                return None;
            };
            children.push(ChildRef::new(slot, ExprNodeId::new(edge.from.node.id)));
            stack.push(edge.from.node);
        }
        nodes.push(ExprNode::new(
            ExprNodeId::new(block.id),
            payload.clone(),
            children,
        ));
    }
    ExpressionAst::new(ExprNodeId::new(root_block.id), nodes).ok()
}

/// projection에서 binding(id, target, source)을 다시 만듭니다(binding id 순).
pub fn lower_bindings(view: &PrototypeGraphView) -> Vec<(BindingId, PropertyPath, ExpressionAst)> {
    view.nodes()
        .iter()
        .filter_map(|n| match &n.detail {
            NodeDetail::Property {
                binding, target, ..
            } => {
                let ast =
                    rebuild_expression(view, GraphSocket::new(n.id, "value", SocketKey::Single))?;
                Some((*binding, target.clone(), ast))
            }
            _ => None,
        })
        .collect()
}

/// projection에서 transition 입력을 다시 만듭니다(상태 머신 목록 순서).
pub fn lower_transitions(view: &PrototypeGraphView) -> Vec<TransitionSpec> {
    let mut out = Vec::new();
    for n in view.nodes() {
        let NodeDetail::Transition {
            transition,
            priority,
        } = n.detail
        else {
            continue;
        };
        let Some(trigger_from) =
            view.source_of(GraphSocket::new(n.id, "trigger", SocketKey::Single))
        else {
            continue;
        };
        let trigger = match trigger_from.key {
            SocketKey::Event(kind) => Trigger::Event {
                kind,
                source: trigger_from.node.id,
            },
            SocketKey::Action(action) => Trigger::NestedAction {
                instance: trigger_from.node.id,
                action,
            },
            _ => continue,
        };
        let condition =
            rebuild_expression(view, GraphSocket::new(n.id, "condition", SocketKey::Single));
        let mut actions: BTreeMap<u16, ActionSpec> = BTreeMap::new();
        for edge in view.edges().iter().filter(|e| e.from.node == n.id) {
            let SocketKey::Index(index) = edge.from.key else {
                continue;
            };
            match edge.from.name {
                "set" => {
                    let Some(value) = rebuild_expression(
                        view,
                        GraphSocket::new(n.id, "set-value", SocketKey::Index(index)),
                    ) else {
                        continue;
                    };
                    actions.insert(
                        index,
                        ActionSpec::SetState {
                            variable: StateVariableId::new(edge.to.node.id),
                            value,
                        },
                    );
                }
                "emit" => {
                    actions.insert(
                        index,
                        ActionSpec::Emit {
                            action: SemanticActionId::new(edge.to.node.id),
                        },
                    );
                }
                _ => {}
            }
        }
        out.push((
            priority,
            TransitionSpec {
                id: transition,
                trigger,
                condition,
                actions: actions.into_values().collect(),
            },
        ));
    }
    out.sort_by_key(|(priority, _)| *priority);
    out.into_iter().map(|(_, spec)| spec).collect()
}
