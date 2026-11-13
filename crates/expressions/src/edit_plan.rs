//! GUI block 추가·연결·삭제 command plan (VIII.3.3).
//!
//! GUI 표현식 편집기의 동작을 AST 원시 변경 목록([`EditPlan`] = [`EditOp`] 순서열)으로 만듭니다. 실제 UI 연결과
//! Phase VII command(창별 history)로의 포장은 이후 작업이 담당하고, 이 모듈은 다음을 보장합니다.
//!
//! - plan 생성 함수는 plan을 기준 AST에 **실제로 적용해 검증한 뒤에만** 반환합니다. 검증은 AST 구조 검증
//!   (ID 유일성·slot 구성·도달성·cycle 금지)과 type 검사이며, 연결 전보다 type 오류 node가 늘면 거부합니다.
//! - 변경 결과 root에서 닿지 않게 된 node는 plan에 [`EditOp::RemoveNode`]로 명시해 제거합니다.
//! - 새 block의 빈 입력은 [`default_literal`] 초기값 literal block으로 채워 항상 완전한 AST를 유지합니다.
//! - 새 node ID는 호출자가 주는 생성기로 받아 plan이 결정적입니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::gradient::Gradient;
use stellar_core_types::ids::StableId;
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::typed_value::{EnumValue, ResourceRef, TypedValue, TypographyValue};
use stellar_core_types::value::{Angle, Duration, FiniteF32, Length, Percentage, Vec2};

use crate::ast::{
    AstError, ChildRef, ChildSlot, ExprNode, ExprNodeId, ExpressionAst, Literal, NodePayload,
};
use crate::graph_view::{BlockId, SocketId};
use crate::sockets::{can_connect, ConnectError};
use crate::typecheck::{type_check, ReferenceTypes, TypeError};
use crate::types::{ExpressionType, TypeSpec};

/// AST 원시 변경 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOp {
    /// 새 node를 추가합니다.
    InsertNode(ExprNode),
    /// socket의 child를 `target`으로 바꿉니다.
    SetChild {
        /// 대상 socket.
        socket: SocketId,
        /// 새 child.
        target: ExprNodeId,
    },
    /// node를 제거합니다(도달 불가가 된 node).
    RemoveNode(ExprNodeId),
    /// root를 바꿉니다.
    SetRoot(ExprNodeId),
}

/// plan 생성·적용 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// 없는 block.
    UnknownBlock(BlockId),
    /// block에 없는 socket.
    UnknownSocket(SocketId),
    /// 새 block 입력 지정이 slot 구성과 맞지 않습니다(누락·중복·불필요 slot).
    InvalidInputs {
        /// 문제 slot.
        slot: ChildSlot,
    },
    /// 삽입하려는 node ID가 이미 있습니다.
    DuplicateNodeId(ExprNodeId),
    /// 삭제 대상 block의 타입을 알 수 없어 대체 literal을 만들 수 없습니다.
    UnresolvedType(BlockId),
    /// 기존 block 연결 거부(VIII.3.2).
    Connect(ConnectError),
    /// 적용 결과가 AST 구조 검증을 통과하지 못했습니다.
    Ast(AstError),
    /// 적용 결과에 새 type 오류가 생깁니다.
    IntroducesErrors(Vec<TypeError>),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::UnknownBlock(block) => write!(f, "없는 block: {block}"),
            PlanError::UnknownSocket(socket) => write!(f, "없는 socket: {socket}"),
            PlanError::InvalidInputs { slot } => write!(f, "새 block 입력 지정 오류: {slot}"),
            PlanError::DuplicateNodeId(id) => write!(f, "이미 있는 node ID: {id}"),
            PlanError::UnresolvedType(block) => write!(f, "block {block}의 타입을 알 수 없습니다"),
            PlanError::Connect(error) => write!(f, "연결 거부: {error}"),
            PlanError::Ast(error) => write!(f, "AST 구조 오류: {error}"),
            PlanError::IntroducesErrors(errors) => {
                write!(f, "적용 후 type 오류 {}개가 새로 생깁니다", errors.len())
            }
        }
    }
}

impl std::error::Error for PlanError {}

/// 검증된 AST 변경 plan (VIII.3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditPlan {
    ops: Vec<EditOp>,
}

impl EditPlan {
    /// 원시 변경 목록(적용 순서).
    pub fn ops(&self) -> &[EditOp] {
        &self.ops
    }

    /// plan을 AST에 적용해 새 AST를 만듭니다. 결과는 구조 검증을 거칩니다.
    pub fn apply(&self, ast: &ExpressionAst) -> Result<ExpressionAst, PlanError> {
        apply_ops(ast, &self.ops)
    }
}

fn apply_ops(ast: &ExpressionAst, ops: &[EditOp]) -> Result<ExpressionAst, PlanError> {
    let mut root = ast.root();
    let mut nodes: Vec<ExprNode> = ast.nodes().to_vec();
    for op in ops {
        match op {
            EditOp::InsertNode(node) => {
                if nodes.iter().any(|existing| existing.id() == node.id()) {
                    return Err(PlanError::DuplicateNodeId(node.id()));
                }
                nodes.push(node.clone());
            }
            EditOp::SetChild { socket, target } => {
                let position = nodes
                    .iter()
                    .position(|node| node.id() == socket.block.node())
                    .ok_or(PlanError::UnknownBlock(socket.block))?;
                let node = &nodes[position];
                if node.child(socket.slot).is_none() {
                    return Err(PlanError::UnknownSocket(*socket));
                }
                let children = node
                    .children()
                    .iter()
                    .map(|child| {
                        if child.slot == socket.slot {
                            ChildRef::new(child.slot, *target)
                        } else {
                            *child
                        }
                    })
                    .collect();
                nodes[position] = ExprNode::new(node.id(), node.payload().clone(), children);
            }
            EditOp::RemoveNode(id) => {
                let position = nodes
                    .iter()
                    .position(|node| node.id() == *id)
                    .ok_or(PlanError::UnknownBlock(BlockId::new(*id)))?;
                nodes.remove(position);
            }
            EditOp::SetRoot(id) => root = *id,
        }
    }
    ExpressionAst::new(root, nodes).map_err(PlanError::Ast)
}

/// 변경 op 뒤에 도달 불가 node 제거 op를 붙이고, 적용·type 검사로 검증한 plan을 만듭니다.
fn finish_plan(
    ast: &ExpressionAst,
    references: &dyn ReferenceTypes,
    mut ops: Vec<EditOp>,
) -> Result<EditPlan, PlanError> {
    // 변경 후 그래프에서 root 도달성을 계산합니다(제거 전 단계는 구조 검증 없이 계산).
    let mut root = ast.root();
    let mut order: Vec<ExprNodeId> = ast.nodes().iter().map(ExprNode::id).collect();
    let mut slots: BTreeMap<ExprNodeId, Vec<ChildRef>> = ast
        .nodes()
        .iter()
        .map(|node| (node.id(), node.children().to_vec()))
        .collect();
    for op in &ops {
        match op {
            EditOp::InsertNode(node) => {
                order.push(node.id());
                slots.insert(node.id(), node.children().to_vec());
            }
            EditOp::SetChild { socket, target } => {
                if let Some(refs) = slots.get_mut(&socket.block.node()) {
                    for child in refs.iter_mut() {
                        if child.slot == socket.slot {
                            child.target = *target;
                        }
                    }
                }
            }
            EditOp::RemoveNode(id) => {
                slots.remove(id);
            }
            EditOp::SetRoot(id) => root = *id,
        }
    }
    let mut reachable = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if reachable.insert(id) {
            if let Some(refs) = slots.get(&id) {
                stack.extend(refs.iter().map(|child| child.target));
            }
        }
    }
    for id in order {
        if slots.contains_key(&id) && !reachable.contains(&id) {
            ops.push(EditOp::RemoveNode(id));
        }
    }
    let plan = EditPlan { ops };
    let result = plan.apply(ast)?;
    let before: BTreeSet<ExprNodeId> = type_check(ast, references)
        .errors()
        .iter()
        .map(|error| error.node)
        .collect();
    let introduced: Vec<TypeError> = type_check(&result, references)
        .errors()
        .iter()
        .filter(|error| !before.contains(&error.node))
        .cloned()
        .collect();
    if introduced.is_empty() {
        Ok(plan)
    } else {
        Err(PlanError::IntroducesErrors(introduced))
    }
}

/// 타입별 초기값 literal. GUI가 새 block의 빈 입력을 채울 때 씁니다(nullable은 null).
pub fn default_literal(spec: TypeSpec) -> Literal {
    let value_type = match spec {
        TypeSpec::Optional(optional) => return Literal::Null(optional.inner()),
        TypeSpec::Required(value_type) => value_type,
    };
    let black = Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0);
    let value = match value_type {
        ExpressionType::Boolean => TypedValue::Boolean(false),
        ExpressionType::Number => TypedValue::Number(FiniteF32::ZERO),
        ExpressionType::String => TypedValue::String(String::new()),
        ExpressionType::Enum(id) => TypedValue::Enum(EnumValue::new(id.raw(), 0)),
        ExpressionType::Length => TypedValue::Length(Length::from_raw(FiniteF32::ZERO)),
        ExpressionType::Percentage => TypedValue::Percentage(Percentage::from_raw(FiniteF32::ZERO)),
        ExpressionType::Angle => TypedValue::Angle(Angle::from_raw(FiniteF32::ZERO)),
        ExpressionType::Duration => TypedValue::Duration(Duration::from_raw(FiniteF32::ZERO)),
        ExpressionType::Color => match black {
            Ok(color) => TypedValue::Color(color),
            Err(_) => TypedValue::Number(FiniteF32::ZERO),
        },
        ExpressionType::Gradient => TypedValue::Gradient(Gradient::default()),
        ExpressionType::Shadow => match black {
            Ok(color) => TypedValue::Shadow(Shadow::new(Vec2::ZERO, BlurRadius::ZERO, color)),
            Err(_) => TypedValue::Number(FiniteF32::ZERO),
        },
        ExpressionType::Typography => TypedValue::Typography(TypographyValue::default()),
        ExpressionType::ResourceRef => TypedValue::ResourceRef(ResourceRef::new(StableId::NIL)),
    };
    Literal::Value(value)
}

/// 새 block 입력 하나의 지정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSpec {
    /// 기존 block을 연결(공유 가능, 자리를 내준 기존 block 포함).
    Existing(BlockId),
    /// 새 literal block.
    Literal(Literal),
    /// 그 타입의 초기값 literal block([`default_literal`]).
    DefaultOf(TypeSpec),
}

/// 추가할 block: payload와 slot별 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBlock {
    /// block payload.
    pub payload: NodePayload,
    /// slot별 입력. block 종류의 slot 구성과 정확히 같아야 합니다.
    pub inputs: Vec<(ChildSlot, InputSpec)>,
}

/// 새 block을 놓을 위치.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// 기존 socket 자리(기존 child를 대체).
    Socket(SocketId),
    /// 표현식 결과(root) 자리.
    Root,
}

/// block 추가 plan (VIII.3.3).
pub fn plan_add_block(
    ast: &ExpressionAst,
    references: &dyn ReferenceTypes,
    placement: Placement,
    block: NewBlock,
    new_id: &mut dyn FnMut() -> ExprNodeId,
) -> Result<EditPlan, PlanError> {
    if let Placement::Socket(socket) = placement {
        let node = ast
            .node(socket.block.node())
            .ok_or(PlanError::UnknownBlock(socket.block))?;
        if node.child(socket.slot).is_none() {
            return Err(PlanError::UnknownSocket(socket));
        }
    }
    let mut seen_slots = BTreeSet::new();
    let mut ops = Vec::new();
    let mut children = Vec::with_capacity(block.inputs.len());
    for (slot, input) in block.inputs {
        if !seen_slots.insert(slot) {
            return Err(PlanError::InvalidInputs { slot });
        }
        let target = match input {
            InputSpec::Existing(existing) => {
                if ast.node(existing.node()).is_none() {
                    return Err(PlanError::UnknownBlock(existing));
                }
                existing.node()
            }
            InputSpec::Literal(literal) => {
                let id = new_id();
                ops.push(EditOp::InsertNode(ExprNode::new(
                    id,
                    NodePayload::Literal(literal),
                    Vec::new(),
                )));
                id
            }
            InputSpec::DefaultOf(spec) => {
                let id = new_id();
                ops.push(EditOp::InsertNode(ExprNode::new(
                    id,
                    NodePayload::Literal(default_literal(spec)),
                    Vec::new(),
                )));
                id
            }
        };
        children.push(ChildRef::new(slot, target));
    }
    let id = new_id();
    ops.push(EditOp::InsertNode(ExprNode::new(
        id,
        block.payload,
        children,
    )));
    match placement {
        Placement::Socket(socket) => ops.push(EditOp::SetChild { socket, target: id }),
        Placement::Root => ops.push(EditOp::SetRoot(id)),
    }
    match finish_plan(ast, references, ops) {
        Err(PlanError::Ast(AstError::MissingSlot { slot, .. }))
        | Err(PlanError::Ast(AstError::UnexpectedSlot { slot, .. })) => {
            Err(PlanError::InvalidInputs { slot })
        }
        other => other,
    }
}

/// 기존 block 연결 plan (VIII.3.3). 연결 가능 여부는 VIII.3.2 규칙으로 먼저 확인합니다.
pub fn plan_connect(
    ast: &ExpressionAst,
    references: &dyn ReferenceTypes,
    socket: SocketId,
    source: BlockId,
) -> Result<EditPlan, PlanError> {
    can_connect(ast, references, socket, source).map_err(PlanError::Connect)?;
    finish_plan(
        ast,
        references,
        vec![EditOp::SetChild {
            socket,
            target: source.node(),
        }],
    )
}

/// 삭제한 block 자리를 채우는 방법.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteReplacement {
    /// 삭제 block의 추론 타입 초기값 literal block.
    DefaultLiteral,
    /// 삭제 block의 이 입력 slot에 연결된 block을 끌어올림.
    PromoteInput(ChildSlot),
}

/// block 삭제 plan (VIII.3.3). 삭제 block을 참조하던 모든 socket(공유 포함)과 root를 대체 block으로 바꿉니다.
pub fn plan_delete_block(
    ast: &ExpressionAst,
    references: &dyn ReferenceTypes,
    block: BlockId,
    replacement: DeleteReplacement,
    new_id: &mut dyn FnMut() -> ExprNodeId,
) -> Result<EditPlan, PlanError> {
    let node = ast
        .node(block.node())
        .ok_or(PlanError::UnknownBlock(block))?;
    let mut ops = Vec::new();
    let substitute = match replacement {
        DeleteReplacement::DefaultLiteral => {
            let spec = type_check(ast, references)
                .type_of(block.node())
                .ok_or(PlanError::UnresolvedType(block))?;
            let id = new_id();
            ops.push(EditOp::InsertNode(ExprNode::new(
                id,
                NodePayload::Literal(default_literal(spec)),
                Vec::new(),
            )));
            id
        }
        DeleteReplacement::PromoteInput(slot) => node
            .child(slot)
            .ok_or(PlanError::UnknownSocket(SocketId::new(block, slot)))?,
    };
    for parent in ast.nodes() {
        for child in parent.children() {
            if child.target == block.node() {
                ops.push(EditOp::SetChild {
                    socket: SocketId::new(BlockId::new(parent.id()), child.slot),
                    target: substitute,
                });
            }
        }
    }
    if ast.root() == block.node() {
        ops.push(EditOp::SetRoot(substitute));
    }
    finish_plan(ast, references, ops)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::typed_value::TypedValue;

    use super::*;
    use crate::ast::ReferenceTarget;
    use crate::functions::FunctionId;
    use crate::ops::{BinaryOp, UnaryOp};
    use crate::types::EnumTypeId;

    fn id(n: u64) -> ExprNodeId {
        ExprNodeId::new(StableId::new_v7(1_000 + n, 0, n))
    }

    fn block(n: u64) -> BlockId {
        BlockId::new(id(n))
    }

    fn length_value(v: f32) -> TypedValue {
        TypedValue::Length(Length::new(v).expect("유한"))
    }

    fn refs() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::new()
    }

    /// 결정적 ID 생성기(1000번부터).
    fn ids() -> impl FnMut() -> ExprNodeId {
        let mut next = 1_000u64;
        move || {
            next += 1;
            id(next)
        }
    }

    /// `if (len1 < len2) then len1 + len2 else -len3`.
    fn base() -> ExpressionAst {
        ExpressionAst::new(
            id(10),
            vec![
                ExprNode::if_else(id(10), id(11), id(12), id(13)),
                ExprNode::binary(id(11), BinaryOp::Less, id(1), id(2)),
                ExprNode::binary(id(12), BinaryOp::Add, id(1), id(2)),
                ExprNode::unary(id(13), UnaryOp::Negate, id(3)),
                ExprNode::literal(id(1), length_value(1.0)),
                ExprNode::literal(id(2), length_value(2.0)),
                ExprNode::literal(id(3), length_value(3.0)),
            ],
        )
        .expect("유효")
    }

    fn assert_valid(ast: &ExpressionAst) {
        let report = type_check(ast, &refs());
        assert!(report.is_ok(), "type 오류: {:?}", report.errors());
        // 구조 검증은 ExpressionAst 생성에서 이미 통과. canonical round-trip도 확인.
        let bytes = ast.to_canonical_bytes().expect("직렬화");
        assert_eq!(
            &ExpressionAst::from_canonical_bytes(&bytes).expect("역직렬화"),
            ast
        );
    }

    #[test]
    fn add_block_wrapping_existing_input_yields_valid_ast() {
        let ast = base();
        let mut new_id = ids();
        // Add의 Rhs 자리에 `len2 * 0`을 넣습니다(기존 len2를 감쌈).
        let plan = plan_add_block(
            &ast,
            &refs(),
            Placement::Socket(SocketId::new(block(12), ChildSlot::Rhs)),
            NewBlock {
                payload: NodePayload::Binary(BinaryOp::Multiply),
                inputs: vec![
                    (ChildSlot::Lhs, InputSpec::Existing(block(2))),
                    (
                        ChildSlot::Rhs,
                        InputSpec::DefaultOf(TypeSpec::required(ExpressionType::Number)),
                    ),
                ],
            },
            &mut new_id,
        )
        .expect("plan");
        let after = plan.apply(&ast).expect("적용");
        assert_valid(&after);
        let mul = after
            .node(id(12))
            .and_then(|n| n.child(ChildSlot::Rhs))
            .expect("연결");
        assert_eq!(
            after.node(mul).map(ExprNode::op),
            Some(crate::ops::ExpressionOp::Binary(BinaryOp::Multiply))
        );
        assert_eq!(after.len(), ast.len() + 2);
        // 같은 입력·같은 ID 생성기 → 같은 plan(결정적).
        let mut again_ids = ids();
        let again = plan_add_block(
            &ast,
            &refs(),
            Placement::Socket(SocketId::new(block(12), ChildSlot::Rhs)),
            NewBlock {
                payload: NodePayload::Binary(BinaryOp::Multiply),
                inputs: vec![
                    (ChildSlot::Lhs, InputSpec::Existing(block(2))),
                    (
                        ChildSlot::Rhs,
                        InputSpec::DefaultOf(TypeSpec::required(ExpressionType::Number)),
                    ),
                ],
            },
            &mut again_ids,
        )
        .expect("plan");
        assert_eq!(again, plan);
    }

    #[test]
    fn add_block_at_root_and_with_defaults_yields_valid_ast() {
        let ast = base();
        let mut new_id = ids();
        // root를 neg로 감쌈.
        let plan = plan_add_block(
            &ast,
            &refs(),
            Placement::Root,
            NewBlock {
                payload: NodePayload::Unary(UnaryOp::Negate),
                inputs: vec![(ChildSlot::Operand, InputSpec::Existing(block(10)))],
            },
            &mut new_id,
        )
        .expect("plan");
        let after = plan.apply(&ast).expect("적용");
        assert_valid(&after);
        assert_ne!(after.root(), ast.root());

        // else 자리의 -len3을 clamp(default×3)로 대체 → -len3·len3 제거 op 포함.
        let plan = plan_add_block(
            &ast,
            &refs(),
            Placement::Socket(SocketId::new(block(10), ChildSlot::Else)),
            NewBlock {
                payload: NodePayload::Call(FunctionId::Clamp),
                inputs: (0..3)
                    .map(|i| {
                        (
                            ChildSlot::Argument(i),
                            InputSpec::DefaultOf(TypeSpec::required(ExpressionType::Length)),
                        )
                    })
                    .collect(),
            },
            &mut new_id,
        )
        .expect("plan");
        let removed: Vec<&EditOp> = plan
            .ops()
            .iter()
            .filter(|op| matches!(op, EditOp::RemoveNode(_)))
            .collect();
        assert_eq!(
            removed,
            vec![&EditOp::RemoveNode(id(13)), &EditOp::RemoveNode(id(3))]
        );
        let after = plan.apply(&ast).expect("적용");
        assert_valid(&after);
        assert!(after.node(id(13)).is_none());
    }

    #[test]
    fn connect_plan_yields_valid_ast_and_prunes() {
        let ast = base();
        // Add의 Rhs(len2)를 len1로: len2는 Less에서 계속 쓰므로 남음.
        let plan = plan_connect(
            &ast,
            &refs(),
            SocketId::new(block(12), ChildSlot::Rhs),
            block(1),
        )
        .expect("plan");
        assert_eq!(plan.ops().len(), 1);
        assert_valid(&plan.apply(&ast).expect("적용"));
        // neg의 operand를 len1로: len3 제거.
        let plan = plan_connect(
            &ast,
            &refs(),
            SocketId::new(block(13), ChildSlot::Operand),
            block(1),
        )
        .expect("plan");
        assert_eq!(plan.ops().last(), Some(&EditOp::RemoveNode(id(3))));
        let after = plan.apply(&ast).expect("적용");
        assert_valid(&after);
        assert_eq!(after.len(), ast.len() - 1);
        // cycle·타입 불일치 연결은 plan을 만들지 않음.
        assert!(matches!(
            plan_connect(
                &ast,
                &refs(),
                SocketId::new(block(12), ChildSlot::Lhs),
                block(10)
            ),
            Err(PlanError::Connect(ConnectError::WouldCreateCycle { .. }))
        ));
        assert!(matches!(
            plan_connect(
                &ast,
                &refs(),
                SocketId::new(block(12), ChildSlot::Lhs),
                block(11)
            ),
            Err(PlanError::Connect(ConnectError::TypeMismatch { .. }))
        ));
    }

    #[test]
    fn delete_plans_yield_valid_ast() {
        let ast = base();
        let mut new_id = ids();
        // Add 삭제 → Length 초기값 literal로 대체. len1·len2는 Less가 계속 사용.
        let plan = plan_delete_block(
            &ast,
            &refs(),
            block(12),
            DeleteReplacement::DefaultLiteral,
            &mut new_id,
        )
        .expect("plan");
        let after = plan.apply(&ast).expect("적용");
        assert_valid(&after);
        assert!(after.node(id(12)).is_none());
        // neg 삭제 → operand(len3) 끌어올림.
        let plan = plan_delete_block(
            &ast,
            &refs(),
            block(13),
            DeleteReplacement::PromoteInput(ChildSlot::Operand),
            &mut new_id,
        )
        .expect("plan");
        let after = plan.apply(&ast).expect("적용");
        assert_valid(&after);
        assert_eq!(
            after.node(id(10)).and_then(|n| n.child(ChildSlot::Else)),
            Some(id(3))
        );
        // 공유 block(len1) 삭제 → 두 부모 socket 모두 대체.
        let plan = plan_delete_block(
            &ast,
            &refs(),
            block(1),
            DeleteReplacement::DefaultLiteral,
            &mut new_id,
        )
        .expect("plan");
        let set_children = plan
            .ops()
            .iter()
            .filter(|op| matches!(op, EditOp::SetChild { .. }))
            .count();
        assert_eq!(set_children, 2);
        assert_valid(&plan.apply(&ast).expect("적용"));
        // root 삭제 → root가 초기값 literal 하나로.
        let plan = plan_delete_block(
            &ast,
            &refs(),
            block(10),
            DeleteReplacement::DefaultLiteral,
            &mut new_id,
        )
        .expect("plan");
        let after = plan.apply(&ast).expect("적용");
        assert_valid(&after);
        assert_eq!(after.len(), 1);
        // 타입이 다른 입력 끌어올림(Less 삭제 시 Lhs Length를 Boolean 조건 자리에)은 거부.
        assert!(matches!(
            plan_delete_block(
                &ast,
                &refs(),
                block(11),
                DeleteReplacement::PromoteInput(ChildSlot::Lhs),
                &mut new_id
            ),
            Err(PlanError::IntroducesErrors(_))
        ));
    }

    #[test]
    fn invalid_add_requests_are_rejected() {
        let ast = base();
        let mut new_id = ids();
        let color = Literal::Value(TypedValue::Color(
            Color::new(ColorSpace::Srgb, 1.0, 1.0, 1.0, 1.0).expect("색"),
        ));
        // Length + Color 블록을 Add Rhs 자리에.
        assert!(matches!(
            plan_add_block(
                &ast,
                &refs(),
                Placement::Socket(SocketId::new(block(12), ChildSlot::Rhs)),
                NewBlock {
                    payload: NodePayload::Binary(BinaryOp::Add),
                    inputs: vec![
                        (ChildSlot::Lhs, InputSpec::Existing(block(2))),
                        (ChildSlot::Rhs, InputSpec::Literal(color)),
                    ],
                },
                &mut new_id,
            ),
            Err(PlanError::IntroducesErrors(_))
        ));
        // 입력 slot 누락·중복.
        let missing = plan_add_block(
            &ast,
            &refs(),
            Placement::Root,
            NewBlock {
                payload: NodePayload::Binary(BinaryOp::Add),
                inputs: vec![(ChildSlot::Lhs, InputSpec::Existing(block(1)))],
            },
            &mut new_id,
        );
        assert_eq!(
            missing,
            Err(PlanError::InvalidInputs {
                slot: ChildSlot::Rhs
            })
        );
        let duplicate = plan_add_block(
            &ast,
            &refs(),
            Placement::Root,
            NewBlock {
                payload: NodePayload::Unary(UnaryOp::Negate),
                inputs: vec![
                    (ChildSlot::Operand, InputSpec::Existing(block(1))),
                    (ChildSlot::Operand, InputSpec::Existing(block(2))),
                ],
            },
            &mut new_id,
        );
        assert_eq!(
            duplicate,
            Err(PlanError::InvalidInputs {
                slot: ChildSlot::Operand
            })
        );
        // 없는 socket.
        assert_eq!(
            plan_add_block(
                &ast,
                &refs(),
                Placement::Socket(SocketId::new(block(12), ChildSlot::Then)),
                NewBlock {
                    payload: NodePayload::Literal(Literal::Value(length_value(1.0))),
                    inputs: vec![],
                },
                &mut new_id,
            ),
            Err(PlanError::UnknownSocket(SocketId::new(
                block(12),
                ChildSlot::Then
            )))
        );
    }

    #[test]
    fn random_edit_sequences_keep_ast_valid() {
        // 결정적 무작위 편집 300회: 생성된 plan은 항상 유효 AST를 만듭니다.
        let mut ast = base();
        let mut new_id = ids();
        let mut state: u64 = 42;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        let mut applied = 0;
        for _ in 0..300 {
            let nodes = ast.nodes().to_vec();
            let pick = &nodes[next() % nodes.len()];
            let plan = match next() % 3 {
                0 => {
                    let Some(child) = pick.children().get(next() % pick.children().len().max(1))
                    else {
                        continue;
                    };
                    let source = &nodes[next() % nodes.len()];
                    plan_connect(
                        &ast,
                        &refs(),
                        SocketId::new(BlockId::new(pick.id()), child.slot),
                        BlockId::new(source.id()),
                    )
                }
                1 => {
                    let Some(child) = pick.children().first() else {
                        continue;
                    };
                    plan_add_block(
                        &ast,
                        &refs(),
                        Placement::Socket(SocketId::new(BlockId::new(pick.id()), child.slot)),
                        NewBlock {
                            payload: NodePayload::Unary(UnaryOp::Negate),
                            inputs: vec![(
                                ChildSlot::Operand,
                                InputSpec::Existing(BlockId::new(child.target)),
                            )],
                        },
                        &mut new_id,
                    )
                }
                _ => plan_delete_block(
                    &ast,
                    &refs(),
                    BlockId::new(pick.id()),
                    DeleteReplacement::DefaultLiteral,
                    &mut new_id,
                ),
            };
            if let Ok(plan) = plan {
                ast = plan.apply(&ast).expect("검증된 plan은 적용 성공");
                assert_valid(&ast);
                applied += 1;
            }
        }
        assert!(applied > 100, "충분한 plan이 적용됨: {applied}");
    }

    #[test]
    fn default_literals_match_their_type() {
        let types = [
            ExpressionType::Boolean,
            ExpressionType::Number,
            ExpressionType::String,
            ExpressionType::Enum(EnumTypeId::new(4)),
            ExpressionType::Length,
            ExpressionType::Percentage,
            ExpressionType::Angle,
            ExpressionType::Duration,
            ExpressionType::Color,
            ExpressionType::Gradient,
            ExpressionType::Shadow,
            ExpressionType::Typography,
            ExpressionType::ResourceRef,
        ];
        for value_type in types {
            match default_literal(TypeSpec::required(value_type)) {
                Literal::Value(value) => assert_eq!(ExpressionType::of(&value), value_type),
                Literal::Null(_) => panic!("Required는 값"),
            }
            assert_eq!(
                default_literal(TypeSpec::optional(value_type)),
                Literal::Null(value_type)
            );
        }
    }
}
