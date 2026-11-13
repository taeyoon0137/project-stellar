//! GUI 편집용 block·edge projection (VIII.3.1).
//!
//! [`ExpressionGraphView`]는 AST를 GUI 표현식 편집기의 **block 목록**과 **edge 목록**으로 읽는 읽기 전용
//! 투영입니다. 별도 원본 상태를 만들지 않습니다.
//!
//! - block 하나 = AST node 하나. block ID는 node StableId([`BlockId`])이고 payload는 AST를 빌려 씁니다.
//! - block의 입력 socket([`SocketId`]) = node의 child slot. edge는 "child block의 출력 → 부모 block의 입력
//!   socket"입니다. 공유(DAG) node는 block 하나에 출력 edge가 여러 개입니다.
//! - 편집은 view를 고치지 않고 AST에 edit plan(VIII.3.3)을 적용한 뒤 view를 다시 만듭니다.
//!
//! block·edge 순서는 canonical 순서(VIII.2.5)를 따르므로 같은 AST는 항상 같은 view가 됩니다.
//! [`ExpressionGraphView::to_ast`]는 block과 edge만으로 AST를 다시 만들어 AST→view→AST 의미 동일성을 보장합니다.

use std::collections::BTreeMap;
use std::fmt;

use crate::ast::{AstError, ChildRef, ChildSlot, ExprNode, ExprNodeId, ExpressionAst, NodePayload};
use crate::ops::{ExpressionOp, NodeKind};

/// GUI block 식별자. AST node의 StableId와 같습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(ExprNodeId);

impl BlockId {
    /// node ID로부터 만듭니다.
    pub const fn new(node: ExprNodeId) -> BlockId {
        BlockId(node)
    }

    /// 대응하는 AST node ID.
    pub const fn node(&self) -> ExprNodeId {
        self.0
    }
}

impl From<ExprNodeId> for BlockId {
    fn from(node: ExprNodeId) -> BlockId {
        BlockId(node)
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// block의 입력 socket 식별자: block + child slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SocketId {
    /// socket을 가진 block.
    pub block: BlockId,
    /// 대응 child slot.
    pub slot: ChildSlot,
}

impl SocketId {
    /// socket ID를 만듭니다.
    pub const fn new(block: BlockId, slot: ChildSlot) -> SocketId {
        SocketId { block, slot }
    }
}

impl fmt::Display for SocketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.block, self.slot)
    }
}

/// block 하나의 projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockView<'a> {
    id: BlockId,
    payload: &'a NodePayload,
    inputs: Vec<SocketId>,
    is_root: bool,
}

impl<'a> BlockView<'a> {
    /// block ID.
    pub const fn id(&self) -> BlockId {
        self.id
    }

    /// AST에서 빌린 node payload.
    pub const fn payload(&self) -> &'a NodePayload {
        self.payload
    }

    /// block 종류.
    pub const fn kind(&self) -> NodeKind {
        self.payload.kind()
    }

    /// GUI 표시용 이름(연산 기호·함수 이름·kind 이름).
    pub fn label(&self) -> &'static str {
        self.payload.op().name()
    }

    /// node 연산.
    pub const fn op(&self) -> ExpressionOp {
        self.payload.op()
    }

    /// 입력 socket(slot 순서).
    pub fn inputs(&self) -> &[SocketId] {
        &self.inputs
    }

    /// 표현식 결과 block(root)이면 `true`.
    pub const fn is_root(&self) -> bool {
        self.is_root
    }
}

/// edge: `source` block의 출력이 `target` socket에 연결됩니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdgeView {
    /// 값을 내는 child block.
    pub source: BlockId,
    /// 값을 받는 부모 block의 입력 socket.
    pub target: SocketId,
}

/// AST의 GUI block·edge projection (VIII.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionGraphView<'a> {
    root: BlockId,
    blocks: Vec<BlockView<'a>>,
    edges: Vec<EdgeView>,
    index: BTreeMap<BlockId, usize>,
}

impl<'a> ExpressionGraphView<'a> {
    /// AST를 block·edge로 읽습니다. block은 canonical 순서, edge는 (부모 block 순서, slot 순서)입니다.
    pub fn new(ast: &'a ExpressionAst) -> ExpressionGraphView<'a> {
        let root = BlockId::new(ast.root());
        let mut blocks = Vec::with_capacity(ast.len());
        let mut edges = Vec::new();
        let mut index = BTreeMap::new();
        for id in ast.canonical_order() {
            let Some(node) = ast.node(id) else {
                continue;
            };
            let block = BlockId::new(id);
            let inputs: Vec<SocketId> = node
                .children()
                .iter()
                .map(|child| SocketId::new(block, child.slot))
                .collect();
            edges.extend(node.children().iter().map(|child| EdgeView {
                source: BlockId::new(child.target),
                target: SocketId::new(block, child.slot),
            }));
            index.insert(block, blocks.len());
            blocks.push(BlockView {
                id: block,
                payload: node.payload(),
                inputs,
                is_root: block == root,
            });
        }
        ExpressionGraphView {
            root,
            blocks,
            edges,
            index,
        }
    }

    /// 결과 block.
    pub const fn root(&self) -> BlockId {
        self.root
    }

    /// 모든 block(canonical 순서).
    pub fn blocks(&self) -> &[BlockView<'a>] {
        &self.blocks
    }

    /// 모든 edge.
    pub fn edges(&self) -> &[EdgeView] {
        &self.edges
    }

    /// ID로 block을 찾습니다.
    pub fn block(&self, id: BlockId) -> Option<&BlockView<'a>> {
        self.index
            .get(&id)
            .and_then(|&position| self.blocks.get(position))
    }

    /// socket에 연결된 source block.
    pub fn source_of(&self, socket: SocketId) -> Option<BlockId> {
        self.edges
            .iter()
            .find(|edge| edge.target == socket)
            .map(|edge| edge.source)
    }

    /// block 출력이 연결된 socket 목록(edge 순서).
    pub fn consumers_of(&self, block: BlockId) -> Vec<SocketId> {
        self.edges
            .iter()
            .filter(|edge| edge.source == block)
            .map(|edge| edge.target)
            .collect()
    }

    /// block·edge만으로 AST를 다시 만듭니다. 원본과 의미(동등성·canonical bytes)가 같습니다.
    pub fn to_ast(&self) -> Result<ExpressionAst, AstError> {
        let mut children: BTreeMap<BlockId, Vec<ChildRef>> = BTreeMap::new();
        for edge in &self.edges {
            children
                .entry(edge.target.block)
                .or_default()
                .push(ChildRef::new(edge.target.slot, edge.source.node()));
        }
        let nodes = self
            .blocks
            .iter()
            .map(|block| {
                ExprNode::new(
                    block.id.node(),
                    block.payload.clone(),
                    children.remove(&block.id).unwrap_or_default(),
                )
            })
            .collect();
        ExpressionAst::new(self.root.node(), nodes)
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::ids::StableId;
    use stellar_core_types::reference::TokenRef;
    use stellar_core_types::typed_value::{EnumValue, TypedValue};
    use stellar_core_types::value::{FiniteF32, Length};

    use super::*;
    use crate::ast::ReferenceTarget;
    use crate::functions::FunctionId;
    use crate::ops::{BinaryOp, UnaryOp};

    fn id(n: u8) -> ExprNodeId {
        ExprNodeId::new(StableId::from_bytes([n; 16]))
    }

    fn number(value: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(value).expect("유한"))
    }

    fn sample() -> ExpressionAst {
        // match token { E0 => clamp(-x, 0, x), E1 => x } else 0  (x = 공유 length 리터럴 4)
        let token = ReferenceTarget::Token(TokenRef::new(StableId::from_bytes([0xEE; 16])));
        ExpressionAst::new(
            id(1),
            vec![
                ExprNode::literal(id(4), TypedValue::Length(Length::new(8.0).expect("유한"))),
                ExprNode::match_on(
                    id(1),
                    id(2),
                    vec![
                        (TypedValue::Enum(EnumValue::new(1, 0)), id(3)),
                        (TypedValue::Enum(EnumValue::new(1, 1)), id(4)),
                    ],
                    id(5),
                ),
                ExprNode::reference(id(2), token),
                ExprNode::call(id(3), FunctionId::Clamp, &[id(6), id(5), id(4)]),
                ExprNode::literal(id(5), TypedValue::Length(Length::new(0.0).expect("유한"))),
                ExprNode::unary(id(6), UnaryOp::Negate, id(4)),
            ],
        )
        .expect("유효")
    }

    #[test]
    fn ast_to_view_to_ast_is_semantically_identical() {
        let ast = sample();
        let view = ExpressionGraphView::new(&ast);
        let rebuilt = view.to_ast().expect("재구성");
        assert_eq!(rebuilt, ast);
        assert_eq!(
            rebuilt.to_canonical_bytes().expect("직렬화"),
            ast.to_canonical_bytes().expect("직렬화")
        );
        // 다시 투영해도 같은 view.
        assert_eq!(ExpressionGraphView::new(&rebuilt), view);
    }

    #[test]
    fn blocks_and_edges_mirror_nodes_and_child_refs() {
        let ast = sample();
        let view = ExpressionGraphView::new(&ast);
        assert_eq!(view.blocks().len(), ast.len());
        let child_count: usize = ast.nodes().iter().map(|n| n.children().len()).sum();
        assert_eq!(view.edges().len(), child_count);
        assert_eq!(view.root(), BlockId::new(id(1)));
        let block_ids: Vec<BlockId> = view.blocks().iter().map(BlockView::id).collect();
        let canonical: Vec<BlockId> = ast
            .canonical_order()
            .into_iter()
            .map(BlockId::new)
            .collect();
        assert_eq!(block_ids, canonical);
        assert!(view.blocks()[0].is_root());
        assert_eq!(view.blocks().iter().filter(|b| b.is_root()).count(), 1);

        // 공유 block 4는 출력 edge 3개(arm1, clamp max, neg operand).
        let consumers = view.consumers_of(BlockId::new(id(4)));
        assert_eq!(consumers.len(), 3);
        assert!(consumers.contains(&SocketId::new(BlockId::new(id(1)), ChildSlot::Arm(1))));
        assert!(consumers.contains(&SocketId::new(BlockId::new(id(3)), ChildSlot::Argument(2))));
        assert!(consumers.contains(&SocketId::new(BlockId::new(id(6)), ChildSlot::Operand)));

        let clamp = view.block(BlockId::new(id(3))).expect("block");
        assert_eq!(clamp.label(), "clamp");
        assert_eq!(clamp.kind(), NodeKind::Call);
        assert_eq!(
            clamp.inputs().iter().map(|s| s.slot).collect::<Vec<_>>(),
            vec![
                ChildSlot::Argument(0),
                ChildSlot::Argument(1),
                ChildSlot::Argument(2)
            ]
        );
        assert_eq!(
            view.source_of(SocketId::new(BlockId::new(id(3)), ChildSlot::Argument(0))),
            Some(BlockId::new(id(6)))
        );
        // payload는 AST를 빌려 씀(별도 원본 없음).
        assert!(std::ptr::eq(
            clamp.payload(),
            ast.node(id(3)).map(ExprNode::payload).expect("node")
        ));
    }

    #[test]
    fn view_is_independent_of_insertion_order() {
        let ast = sample();
        let mut nodes = ast.nodes().to_vec();
        nodes.reverse();
        let reversed = ExpressionAst::new(id(1), nodes).expect("유효");
        let a = ExpressionGraphView::new(&ast);
        let b = ExpressionGraphView::new(&reversed);
        assert_eq!(a.edges(), b.edges());
        let ids = |view: &ExpressionGraphView<'_>| -> Vec<BlockId> {
            view.blocks().iter().map(BlockView::id).collect()
        };
        assert_eq!(ids(&a), ids(&b));
    }

    #[test]
    fn single_literal_and_binary_round_trip() {
        let literal =
            ExpressionAst::new(id(1), vec![ExprNode::literal(id(1), number(3.0))]).expect("유효");
        let view = ExpressionGraphView::new(&literal);
        assert!(view.edges().is_empty());
        assert_eq!(view.to_ast().expect("재구성"), literal);

        let binary = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::Divide, id(2), id(3)),
                ExprNode::literal(id(2), number(1.0)),
                ExprNode::literal(id(3), number(2.0)),
            ],
        )
        .expect("유효");
        let view = ExpressionGraphView::new(&binary);
        assert_eq!(view.to_ast().expect("재구성"), binary);
        assert_eq!(view.blocks()[0].label(), "/");
    }
}
