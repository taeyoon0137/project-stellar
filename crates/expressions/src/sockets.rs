//! GUI socket 타입과 연결 가능 여부 (VIII.3.2).
//!
//! - [`accepted_types`] / [`socket_accepts`]: 한 입력 socket에 올 수 있는 타입. 같은 block의 **다른 socket에
//!   이미 연결된 값의 타입**을 고려한 국소 판단입니다(예: `Length + ?`의 `?`는 `Length`만, `Length × ?`는
//!   `Number`만).
//! - [`can_connect`]: 기존 block 출력을 socket에 연결해도 되는지 전체 판단합니다. 거부 사유는
//!   [`ConnectError`]로 돌려줍니다: 없는 block·socket·source, cycle 생성, 국소 타입 불일치, source 타입
//!   미확정, 연결 후 다른 곳에 새 type 오류 발생(예: 결과 타입이 바뀌어 상위 `if` branch가 어긋남).
//! - [`connect`]: 연결을 적용한 새 AST를 만듭니다. 더 이상 root에서 닿지 않는 node는 제거합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::ast::{
    AstError, ChildRef, ChildSlot, ExprNode, ExprNodeId, ExpressionAst, Literal, NodePayload,
};
use crate::graph_view::{BlockId, SocketId};
use crate::typecheck::{infer_node, type_check, ReferenceTypes, TypeError, TypeErrorKind};
use crate::types::{EnumTypeId, ExpressionType, TypeSpec};

/// socket 연결 거부 사유 (VIII.3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// socket을 가진 block이 AST에 없습니다.
    UnknownBlock(BlockId),
    /// block에 그 socket(slot)이 없습니다.
    UnknownSocket(SocketId),
    /// 연결하려는 source block이 AST에 없습니다.
    UnknownSource(BlockId),
    /// 연결하면 child 참조 cycle이 생깁니다. 경로는 `[block, …, block]`입니다.
    WouldCreateCycle {
        /// 생길 cycle 경로.
        path: Vec<ExprNodeId>,
    },
    /// source 타입이 socket 규칙에 맞지 않습니다.
    TypeMismatch {
        /// 대상 socket.
        socket: SocketId,
        /// source 타입.
        found: TypeSpec,
        /// 위반 내용.
        error: TypeErrorKind,
    },
    /// source block 자체에 type 오류가 있어 타입을 알 수 없습니다.
    UnresolvedSource(BlockId),
    /// 연결 후 다른 node에 새 type 오류가 생깁니다.
    IntroducesErrors(Vec<TypeError>),
    /// 연결 결과가 AST 구조 검증을 통과하지 못했습니다.
    Ast(AstError),
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectError::UnknownBlock(block) => write!(f, "없는 block: {block}"),
            ConnectError::UnknownSocket(socket) => write!(f, "없는 socket: {socket}"),
            ConnectError::UnknownSource(block) => write!(f, "없는 source block: {block}"),
            ConnectError::WouldCreateCycle { path } => {
                write!(f, "연결하면 cycle이 생깁니다(길이 {})", path.len())
            }
            ConnectError::TypeMismatch {
                socket,
                found,
                error,
            } => write!(f, "{socket}에 {found} 타입을 연결할 수 없습니다: {error}"),
            ConnectError::UnresolvedSource(block) => {
                write!(f, "source block {block}의 타입을 알 수 없습니다")
            }
            ConnectError::IntroducesErrors(errors) => {
                write!(f, "연결 후 type 오류 {}개가 새로 생깁니다", errors.len())
            }
            ConnectError::Ast(error) => write!(f, "AST 구조 오류: {error}"),
        }
    }
}

impl std::error::Error for ConnectError {}

/// socket이 있는 node를 찾습니다.
fn socket_node(ast: &ExpressionAst, socket: SocketId) -> Result<&ExprNode, ConnectError> {
    let node = ast
        .node(socket.block.node())
        .ok_or(ConnectError::UnknownBlock(socket.block))?;
    if node.child(socket.slot).is_none() {
        return Err(ConnectError::UnknownSocket(socket));
    }
    Ok(node)
}

/// socket이 `candidate` 타입 값을 받을 수 있는지(같은 block 다른 socket 고려) 판단합니다.
///
/// 형제 socket 중 타입을 알 수 없는 값(오류 node)이 있으면 제약할 수 없으므로 `true`입니다.
pub fn socket_accepts(
    ast: &ExpressionAst,
    references: &dyn ReferenceTypes,
    socket: SocketId,
    candidate: TypeSpec,
) -> Result<bool, ConnectError> {
    let node = socket_node(ast, socket)?;
    let report = type_check(ast, references);
    Ok(local_check(
        node,
        socket.slot,
        candidate,
        &|id| report.type_of(id),
        references,
    )
    .is_ok())
}

fn local_check(
    node: &ExprNode,
    slot: ChildSlot,
    candidate: TypeSpec,
    type_of: &dyn Fn(ExprNodeId) -> Option<TypeSpec>,
    references: &dyn ReferenceTypes,
) -> Result<(), TypeErrorKind> {
    let child_type = |other: ChildSlot| {
        if other == slot {
            Some(candidate)
        } else {
            node.child(other).and_then(type_of)
        }
    };
    match infer_node(node, &child_type, references) {
        None | Some(Ok(_)) => Ok(()),
        Some(Err(kind)) => Err(kind),
    }
}

/// socket이 받을 수 있는 타입 목록(오름차순).
///
/// 후보는 enum이 아닌 12개 값 타입과 AST에 등장하는 enum 타입 각각의 `Required`/`Optional` 형태입니다.
pub fn accepted_types(
    ast: &ExpressionAst,
    references: &dyn ReferenceTypes,
    socket: SocketId,
) -> Result<Vec<TypeSpec>, ConnectError> {
    let node = socket_node(ast, socket)?;
    let report = type_check(ast, references);
    let mut candidates: BTreeSet<ExpressionType> = [
        ExpressionType::Boolean,
        ExpressionType::Number,
        ExpressionType::String,
        ExpressionType::Length,
        ExpressionType::Percentage,
        ExpressionType::Angle,
        ExpressionType::Duration,
        ExpressionType::Color,
        ExpressionType::Gradient,
        ExpressionType::Shadow,
        ExpressionType::Typography,
        ExpressionType::ResourceRef,
    ]
    .into_iter()
    .collect();
    candidates.extend(
        report
            .types()
            .values()
            .map(TypeSpec::value_type)
            .filter(|t| matches!(t, ExpressionType::Enum(_))),
    );
    for each in ast.nodes() {
        match each.payload() {
            NodePayload::Match(patterns) => candidates.extend(
                patterns
                    .iter()
                    .map(ExpressionType::of)
                    .filter(|t| matches!(t, ExpressionType::Enum(_))),
            ),
            NodePayload::Literal(Literal::Null(ExpressionType::Enum(id))) => {
                candidates.insert(ExpressionType::Enum(EnumTypeId::new(id.raw())));
            }
            _ => {}
        }
    }
    let type_of = |id| report.type_of(id);
    let mut accepted = Vec::new();
    for value_type in candidates {
        for spec in [
            TypeSpec::required(value_type),
            TypeSpec::optional(value_type),
        ] {
            if local_check(node, socket.slot, spec, &type_of, references).is_ok() {
                accepted.push(spec);
            }
        }
    }
    accepted.sort();
    Ok(accepted)
}

/// `from`에서 child 참조를 따라 `to`에 닿는 경로(`[from, …, to]`). 없으면 `None`.
fn path_between(ast: &ExpressionAst, from: ExprNodeId, to: ExprNodeId) -> Option<Vec<ExprNodeId>> {
    let mut parent: BTreeMap<ExprNodeId, ExprNodeId> = BTreeMap::new();
    let mut seen = BTreeSet::from([from]);
    let mut stack = vec![from];
    while let Some(id) = stack.pop() {
        if id == to {
            let mut path = vec![to];
            let mut current = to;
            while let Some(&p) = parent.get(&current) {
                path.push(p);
                current = p;
            }
            path.reverse();
            return Some(path);
        }
        if let Some(node) = ast.node(id) {
            for child in node.children().iter().rev() {
                if seen.insert(child.target) {
                    parent.insert(child.target, id);
                    stack.push(child.target);
                }
            }
        }
    }
    None
}

/// socket의 child를 `source`로 바꾼 새 AST를 만듭니다. root에서 닿지 않게 된 node는 제거합니다.
pub fn connect(
    ast: &ExpressionAst,
    socket: SocketId,
    source: ExprNodeId,
) -> Result<ExpressionAst, AstError> {
    let nodes: Vec<ExprNode> = ast
        .nodes()
        .iter()
        .map(|node| {
            if node.id() != socket.block.node() {
                return node.clone();
            }
            let children = node
                .children()
                .iter()
                .map(|child| {
                    if child.slot == socket.slot {
                        ChildRef::new(child.slot, source)
                    } else {
                        *child
                    }
                })
                .collect();
            ExprNode::new(node.id(), node.payload().clone(), children)
        })
        .collect();
    rebuild_reachable(ast.root(), nodes)
}

/// root에서 닿는 node만 남겨(삽입 순서 유지) AST를 만듭니다. cycle이 있어도 도달성 계산은 종료합니다.
pub(crate) fn rebuild_reachable(
    root: ExprNodeId,
    nodes: Vec<ExprNode>,
) -> Result<ExpressionAst, AstError> {
    let by_id: BTreeMap<ExprNodeId, &ExprNode> = nodes.iter().map(|n| (n.id(), n)).collect();
    let mut reachable = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !reachable.insert(id) {
            continue;
        }
        if let Some(node) = by_id.get(&id) {
            stack.extend(node.children().iter().map(|child| child.target));
        }
    }
    let kept = nodes
        .iter()
        .filter(|node| reachable.contains(&node.id()))
        .cloned()
        .collect();
    ExpressionAst::new(root, kept)
}

/// 기존 block `source`의 출력을 `socket`에 연결해도 되는지 판단합니다(VIII.3.2).
pub fn can_connect(
    ast: &ExpressionAst,
    references: &dyn ReferenceTypes,
    socket: SocketId,
    source: BlockId,
) -> Result<(), ConnectError> {
    let node = socket_node(ast, socket)?;
    if ast.node(source.node()).is_none() {
        return Err(ConnectError::UnknownSource(source));
    }
    if let Some(mut path) = path_between(ast, source.node(), socket.block.node()) {
        // block → source(새 간선) → … → block.
        path.insert(0, socket.block.node());
        return Err(ConnectError::WouldCreateCycle { path });
    }
    let before = type_check(ast, references);
    let found = before
        .type_of(source.node())
        .ok_or(ConnectError::UnresolvedSource(source))?;
    local_check(
        node,
        socket.slot,
        found,
        &|id| before.type_of(id),
        references,
    )
    .map_err(|error| ConnectError::TypeMismatch {
        socket,
        found,
        error,
    })?;
    let after_ast = connect(ast, socket, source.node()).map_err(ConnectError::Ast)?;
    let after = type_check(&after_ast, references);
    let old: BTreeSet<ExprNodeId> = before.errors().iter().map(|e| e.node).collect();
    let introduced: Vec<TypeError> = after
        .errors()
        .iter()
        .filter(|error| !old.contains(&error.node))
        .cloned()
        .collect();
    if introduced.is_empty() {
        Ok(())
    } else {
        Err(ConnectError::IntroducesErrors(introduced))
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::StableId;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::{Angle, FiniteF32, Length};

    use super::*;
    use crate::ast::ReferenceTarget;
    use crate::functions::FunctionId;
    use crate::ops::BinaryOp;

    fn id(n: u8) -> ExprNodeId {
        ExprNodeId::new(StableId::from_bytes([n; 16]))
    }

    fn block(n: u8) -> BlockId {
        BlockId::new(id(n))
    }

    fn socket(n: u8, slot: ChildSlot) -> SocketId {
        SocketId::new(block(n), slot)
    }

    fn length(n: u8, v: f32) -> ExprNode {
        ExprNode::literal(id(n), TypedValue::Length(Length::new(v).expect("유한")))
    }

    fn number(n: u8, v: f32) -> ExprNode {
        ExprNode::literal(id(n), TypedValue::Number(FiniteF32::new(v).expect("유한")))
    }

    fn color(n: u8) -> ExprNode {
        ExprNode::literal(
            id(n),
            TypedValue::Color(Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).expect("색")),
        )
    }

    fn no_refs() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::new()
    }

    const LEN: TypeSpec = TypeSpec::required(ExpressionType::Length);

    /// `if (len1 < len3) then (len1 + len2) else len3` — len1·len3은 공유 node.
    fn add_ast() -> ExpressionAst {
        ExpressionAst::new(
            id(10),
            vec![
                ExprNode::if_else(id(10), id(11), id(12), id(3)),
                ExprNode::binary(id(11), BinaryOp::Less, id(1), id(3)),
                ExprNode::binary(id(12), BinaryOp::Add, id(1), id(2)),
                length(1, 1.0),
                length(2, 2.0),
                length(3, 3.0),
            ],
        )
        .expect("유효")
    }

    #[test]
    fn accepted_types_reflect_sibling_sockets() {
        let ast = add_ast();
        let refs = no_refs();
        // len1 + ? → Length만.
        assert_eq!(
            accepted_types(&ast, &refs, socket(12, ChildSlot::Rhs)).expect("socket"),
            vec![LEN]
        );
        // if 조건 → Boolean만.
        assert_eq!(
            accepted_types(&ast, &refs, socket(10, ChildSlot::Condition)).expect("socket"),
            vec![TypeSpec::required(ExpressionType::Boolean)]
        );
        // if else(then이 Length) → Length 또는 Optional<Length>.
        assert_eq!(
            accepted_types(&ast, &refs, socket(10, ChildSlot::Else)).expect("socket"),
            vec![LEN, TypeSpec::optional(ExpressionType::Length)]
        );

        let mul = ExpressionAst::new(
            id(10),
            vec![
                ExprNode::binary(id(10), BinaryOp::Multiply, id(1), id(2)),
                length(1, 1.0),
                number(2, 2.0),
            ],
        )
        .expect("유효");
        // Length × ? → Number만.
        assert_eq!(
            accepted_types(&mul, &refs, socket(10, ChildSlot::Rhs)).expect("socket"),
            vec![TypeSpec::required(ExpressionType::Number)]
        );
        // ? × Number → Number·4개 단위.
        assert_eq!(
            accepted_types(&mul, &refs, socket(10, ChildSlot::Lhs)).expect("socket"),
            vec![
                TypeSpec::required(ExpressionType::Number),
                LEN,
                TypeSpec::required(ExpressionType::Percentage),
                TypeSpec::required(ExpressionType::Angle),
                TypeSpec::required(ExpressionType::Duration),
            ]
        );

        let angle =
            |n: u8| ExprNode::literal(id(n), TypedValue::Angle(Angle::new(1.0).expect("유한")));
        let clamp = ExpressionAst::new(
            id(10),
            vec![
                ExprNode::call(id(10), FunctionId::Clamp, &[id(1), id(2), id(3)]),
                angle(1),
                angle(2),
                angle(3),
            ],
        )
        .expect("유효");
        assert_eq!(
            accepted_types(&clamp, &refs, socket(10, ChildSlot::Argument(1))).expect("socket"),
            vec![TypeSpec::required(ExpressionType::Angle)]
        );
        assert!(socket_accepts(
            &clamp,
            &refs,
            socket(10, ChildSlot::Argument(2)),
            TypeSpec::required(ExpressionType::Angle)
        )
        .expect("socket"));
        assert!(
            !socket_accepts(&clamp, &refs, socket(10, ChildSlot::Argument(2)), LEN)
                .expect("socket")
        );
    }

    #[test]
    fn invalid_socket_connections_are_rejected() {
        let refs = no_refs();
        // if (color6 == color6) then (len1 + len2) else max(len2, len3).
        // 연결 후보(색·길이)가 모두 root에서 닿도록 한 AST입니다.
        let ast = ExpressionAst::new(
            id(10),
            vec![
                ExprNode::if_else(id(10), id(11), id(12), id(13)),
                ExprNode::binary(id(11), BinaryOp::Equal, id(6), id(6)),
                ExprNode::binary(id(12), BinaryOp::Add, id(1), id(2)),
                ExprNode::call(id(13), FunctionId::Max, &[id(2), id(3)]),
                length(1, 1.0),
                length(2, 2.0),
                length(3, 3.0),
                color(6),
            ],
        )
        .expect("유효");
        assert!(type_check(&ast, &refs).is_ok());

        // 1. Length + Color: 국소 타입 불일치.
        let err = can_connect(&ast, &refs, socket(12, ChildSlot::Rhs), block(6)).expect_err("거부");
        match err {
            ConnectError::TypeMismatch { found, error, .. } => {
                assert_eq!(found, TypeSpec::required(ExpressionType::Color));
                assert_eq!(
                    error.position().map(|p| p.child_slot()),
                    Some(ChildSlot::Rhs)
                );
            }
            other => panic!("TypeMismatch 기대: {other:?}"),
        }
        // 2. if 조건에 Length.
        assert!(matches!(
            can_connect(&ast, &refs, socket(10, ChildSlot::Condition), block(1)),
            Err(ConnectError::TypeMismatch { .. })
        ));
        // 3. 자기 자신 연결과 조상을 자손 socket에 연결 → cycle.
        assert_eq!(
            can_connect(&ast, &refs, socket(12, ChildSlot::Lhs), block(12)),
            Err(ConnectError::WouldCreateCycle {
                path: vec![id(12), id(12)]
            })
        );
        assert_eq!(
            can_connect(&ast, &refs, socket(13, ChildSlot::Argument(0)), block(10)),
            Err(ConnectError::WouldCreateCycle {
                path: vec![id(13), id(10), id(13)]
            })
        );
        // 4. 없는 socket·block·source.
        assert_eq!(
            can_connect(&ast, &refs, socket(12, ChildSlot::Then), block(1)),
            Err(ConnectError::UnknownSocket(socket(12, ChildSlot::Then)))
        );
        assert_eq!(
            can_connect(&ast, &refs, socket(99, ChildSlot::Lhs), block(1)),
            Err(ConnectError::UnknownBlock(block(99)))
        );
        assert_eq!(
            can_connect(&ast, &refs, socket(12, ChildSlot::Lhs), block(98)),
            Err(ConnectError::UnknownSource(block(98)))
        );
        // 5. 정상 연결(공유 DAG)은 통과하고, 적용 결과는 유효하며 type 오류가 없음.
        assert_eq!(
            can_connect(&ast, &refs, socket(12, ChildSlot::Rhs), block(3)),
            Ok(())
        );
        let connected = connect(&ast, socket(12, ChildSlot::Rhs), id(3)).expect("유효");
        assert!(type_check(&connected, &refs).is_ok());
        assert_eq!(
            connected.node(id(12)).and_then(|n| n.child(ChildSlot::Rhs)),
            Some(id(3))
        );
    }

    #[test]
    fn connection_that_breaks_downstream_types_is_rejected() {
        let refs = no_refs();
        // if (c == c) then (len1 / num2) else len3 — Div 결과 Length.
        let ast = ExpressionAst::new(
            id(10),
            vec![
                ExprNode::if_else(id(10), id(11), id(12), id(3)),
                ExprNode::binary(id(11), BinaryOp::Equal, id(3), id(3)),
                ExprNode::binary(id(12), BinaryOp::Divide, id(1), id(2)),
                length(1, 1.0),
                number(2, 2.0),
                length(3, 3.0),
            ],
        )
        .expect("유효");
        assert!(type_check(&ast, &refs).is_ok());
        // Div.Rhs에 Length: 국소적으로 Length ÷ Length = Number로 유효하지만 if branch가 어긋남.
        assert!(socket_accepts(&ast, &refs, socket(12, ChildSlot::Rhs), LEN).expect("socket"));
        match can_connect(&ast, &refs, socket(12, ChildSlot::Rhs), block(3)) {
            Err(ConnectError::IntroducesErrors(errors)) => {
                assert_eq!(errors.len(), 1);
                assert_eq!(errors[0].node, id(10));
                assert_eq!(errors[0].slot, Some(ChildSlot::Else));
            }
            other => panic!("IntroducesErrors 기대: {other:?}"),
        }
    }

    #[test]
    fn connect_prunes_unreachable_nodes() {
        let ast = add_ast();
        // len1 + len2의 Rhs를 len3로 바꾸면 len2는 더 이상 닿지 않아 제거.
        let connected = connect(&ast, socket(12, ChildSlot::Rhs), id(3)).expect("유효");
        assert_eq!(connected.len(), ast.len() - 1);
        assert!(connected.node(id(2)).is_none());
    }
}
