//! AST 검증 오류의 GUI block·socket 매핑 (VIII.3.4).
//!
//! 구조 검증 오류([`AstError`], VIII.2.1~VIII.2.3)와 type 오류([`TypeError`], VIII.2.4)를 GUI가 표시할 위치인
//! [`BlockId`]와 [`SocketId`](VIII.3.1)로 바꿉니다.
//!
//! | 오류 | block | socket |
//! |---|---|---|
//! | 중복 ID·root 없음·도달 불가 | 그 node | 없음 |
//! | dangling child·중복/불필요/누락 slot | 부모 node | 그 slot |
//! | cycle `[a, b, …, a]` | `a` | `a`에서 `b`로 가는 slot |
//! | type 오류 | 위반 node | 위반 피연산자 slot(pattern 위반은 그 arm slot), 없으면 없음 |

use std::fmt;

use crate::ast::{AstError, ExprNode, ExprNodeId, ExpressionAst};
use crate::graph_view::{BlockId, SocketId};
use crate::typecheck::{type_check, ReferenceTypes, TypeCheckReport, TypeError};

/// 진단 출처.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSource {
    /// AST 구조 검증.
    Structure,
    /// type 검사.
    Type,
}

/// GUI 위치가 붙은 진단 하나 (VIII.3.4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GuiDiagnostic {
    /// 표시할 block.
    pub block: BlockId,
    /// 표시할 입력 socket(있으면).
    pub socket: Option<SocketId>,
    /// 진단 출처.
    pub source: DiagnosticSource,
    /// 사용자 표시용 메시지.
    pub message: String,
}

impl fmt::Display for GuiDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.socket {
            Some(socket) => write!(f, "{socket}: {}", self.message),
            None => write!(f, "{}: {}", self.block, self.message),
        }
    }
}

/// 구조 검증 오류를 GUI 위치로 바꿉니다. `nodes`는 검증에 넘긴 node 목록입니다(cycle slot 계산용).
pub fn map_ast_error(error: &AstError, nodes: &[ExprNode]) -> GuiDiagnostic {
    let at = |node: ExprNodeId, socket: Option<SocketId>| GuiDiagnostic {
        block: BlockId::new(node),
        socket,
        source: DiagnosticSource::Structure,
        message: error.to_string(),
    };
    let socket = |node: ExprNodeId, slot| Some(SocketId::new(BlockId::new(node), slot));
    match error {
        AstError::DuplicateNodeId(id)
        | AstError::MissingRoot(id)
        | AstError::UnreachableNode(id) => at(*id, None),
        AstError::DanglingChild { parent, slot, .. } => at(*parent, socket(*parent, *slot)),
        AstError::DuplicateSlot { node, slot }
        | AstError::UnexpectedSlot { node, slot }
        | AstError::MissingSlot { node, slot } => at(*node, socket(*node, *slot)),
        AstError::Cycle { path } => {
            let start = path.first().copied();
            let next = path.get(1).copied();
            match (start, next) {
                (Some(start), Some(next)) => {
                    let slot = nodes
                        .iter()
                        .find(|node| node.id() == start)
                        .and_then(|node| node.children().iter().find(|c| c.target == next))
                        .map(|child| child.slot);
                    at(start, slot.and_then(|slot| socket(start, slot)))
                }
                (Some(start), None) => at(start, None),
                // 빈 경로는 만들어지지 않지만, 위치 없이 첫 node로 보고할 대상이 없으므로 nil block을 씁니다.
                (None, _) => at(
                    ExprNodeId::new(stellar_core_types::ids::StableId::NIL),
                    None,
                ),
            }
        }
    }
}

/// type 오류 하나를 GUI 위치로 바꿉니다.
pub fn map_type_error(error: &TypeError) -> GuiDiagnostic {
    let block = BlockId::new(error.node);
    GuiDiagnostic {
        block,
        socket: error.slot.map(|slot| SocketId::new(block, slot)),
        source: DiagnosticSource::Type,
        message: error.kind.to_string(),
    }
}

/// type 검사 결과의 모든 오류를 GUI 위치로 바꿉니다(오류 순서 유지).
pub fn map_type_errors(report: &TypeCheckReport) -> Vec<GuiDiagnostic> {
    report.errors().iter().map(map_type_error).collect()
}

/// GUI 편집 중인 node 목록을 검증하고 진단을 돌려줍니다. 구조 오류가 있으면 그 하나, 없으면 type 오류 전부입니다.
pub fn diagnose(
    root: ExprNodeId,
    nodes: Vec<ExprNode>,
    references: &dyn ReferenceTypes,
) -> Vec<GuiDiagnostic> {
    match ExpressionAst::new(root, nodes.clone()) {
        Err(error) => vec![map_ast_error(&error, &nodes)],
        Ok(ast) => map_type_errors(&type_check(&ast, references)),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::StableId;
    use stellar_core_types::typed_value::{EnumValue, TypedValue};
    use stellar_core_types::value::{FiniteF32, Length};

    use super::*;
    use crate::ast::{ChildRef, ChildSlot, NodePayload, ReferenceTarget};
    use crate::functions::FunctionId;
    use crate::graph_view::ExpressionGraphView;
    use crate::ops::{BinaryOp, UnaryOp};
    use crate::types::TypeSpec;

    fn id(n: u8) -> ExprNodeId {
        ExprNodeId::new(StableId::from_bytes([n; 16]))
    }

    fn block(n: u8) -> BlockId {
        BlockId::new(id(n))
    }

    fn socket(n: u8, slot: ChildSlot) -> Option<SocketId> {
        Some(SocketId::new(block(n), slot))
    }

    fn length(n: u8) -> ExprNode {
        ExprNode::literal(id(n), TypedValue::Length(Length::new(1.0).expect("유한")))
    }

    fn number(n: u8) -> ExprNode {
        ExprNode::literal(id(n), TypedValue::Number(FiniteF32::ONE))
    }

    fn color(n: u8) -> ExprNode {
        ExprNode::literal(
            id(n),
            TypedValue::Color(Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).expect("색")),
        )
    }

    fn refs() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::new()
    }

    /// (설명, root, node 목록, 기대 출처, 기대 block, 기대 socket)
    type Fixture = (
        &'static str,
        ExprNodeId,
        Vec<ExprNode>,
        DiagnosticSource,
        BlockId,
        Option<SocketId>,
    );

    fn fixtures() -> Vec<Fixture> {
        use DiagnosticSource::{Structure, Type};
        vec![
            (
                "중복 ID",
                id(1),
                vec![
                    ExprNode::binary(id(1), BinaryOp::Add, id(2), id(3)),
                    length(2),
                    length(2),
                ],
                Structure,
                block(2),
                None,
            ),
            (
                "root 없음",
                id(9),
                vec![length(1)],
                Structure,
                block(9),
                None,
            ),
            (
                "dangling child",
                id(1),
                vec![
                    ExprNode::binary(id(1), BinaryOp::Add, id(2), id(7)),
                    length(2),
                ],
                Structure,
                block(1),
                socket(1, ChildSlot::Rhs),
            ),
            (
                "누락 slot",
                id(1),
                vec![
                    ExprNode::new(
                        id(1),
                        NodePayload::Binary(BinaryOp::Add),
                        vec![ChildRef::new(ChildSlot::Lhs, id(2))],
                    ),
                    length(2),
                ],
                Structure,
                block(1),
                socket(1, ChildSlot::Rhs),
            ),
            (
                "불필요 slot",
                id(1),
                vec![
                    ExprNode::new(
                        id(1),
                        NodePayload::Unary(UnaryOp::Negate),
                        vec![
                            ChildRef::new(ChildSlot::Operand, id(2)),
                            ChildRef::new(ChildSlot::Then, id(2)),
                        ],
                    ),
                    length(2),
                ],
                Structure,
                block(1),
                socket(1, ChildSlot::Then),
            ),
            (
                "도달 불가",
                id(1),
                vec![
                    ExprNode::unary(id(1), UnaryOp::Negate, id(2)),
                    length(2),
                    length(3),
                ],
                Structure,
                block(3),
                None,
            ),
            (
                "cycle",
                id(1),
                vec![
                    ExprNode::binary(id(1), BinaryOp::Add, id(2), id(3)),
                    length(2),
                    ExprNode::unary(id(3), UnaryOp::Negate, id(4)),
                    ExprNode::unary(id(4), UnaryOp::Negate, id(3)),
                ],
                Structure,
                block(3),
                socket(3, ChildSlot::Operand),
            ),
            (
                "type: Length + Color",
                id(1),
                vec![
                    ExprNode::binary(id(1), BinaryOp::Add, id(2), id(3)),
                    length(2),
                    color(3),
                ],
                Type,
                block(1),
                socket(1, ChildSlot::Rhs),
            ),
            (
                "type: 중첩 if 조건",
                id(1),
                vec![
                    ExprNode::unary(id(1), UnaryOp::Negate, id(2)),
                    ExprNode::if_else(id(2), id(3), id(4), id(4)),
                    number(3),
                    length(4),
                ],
                Type,
                block(2),
                socket(2, ChildSlot::Condition),
            ),
            (
                "type: 함수 인자 2",
                id(1),
                vec![
                    ExprNode::call(id(1), FunctionId::Clamp, &[id(2), id(2), id(3)]),
                    length(2),
                    number(3),
                ],
                Type,
                block(1),
                socket(1, ChildSlot::Argument(2)),
            ),
            (
                "type: match pattern 1",
                id(1),
                vec![
                    ExprNode::match_on(
                        id(1),
                        id(2),
                        vec![
                            (TypedValue::Number(FiniteF32::ONE), id(3)),
                            (TypedValue::Enum(EnumValue::new(1, 0)), id(3)),
                        ],
                        id(3),
                    ),
                    number(2),
                    length(3),
                ],
                Type,
                block(1),
                socket(1, ChildSlot::Arm(1)),
            ),
            (
                "type: 미지 참조",
                id(1),
                vec![
                    ExprNode::unary(id(1), UnaryOp::Negate, id(2)),
                    ExprNode::reference(
                        id(2),
                        ReferenceTarget::ComponentProperty(StableId::from_bytes([0xCC; 16])),
                    ),
                ],
                Type,
                block(2),
                None,
            ),
        ]
    }

    #[test]
    fn error_location_fixtures_are_exact() {
        for (name, root, nodes, source, block, socket) in fixtures() {
            let diagnostics = diagnose(root, nodes, &refs());
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:?}");
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.source, source, "{name}");
            assert_eq!(diagnostic.block, block, "{name}");
            assert_eq!(diagnostic.socket, socket, "{name}");
            assert!(!diagnostic.message.is_empty(), "{name}");
        }
    }

    #[test]
    fn type_error_sockets_exist_in_graph_view() {
        // type 오류 socket은 graph view의 실제 입력 socket이어야 합니다.
        for (name, root, nodes, source, _, socket) in fixtures() {
            if source != DiagnosticSource::Type {
                continue;
            }
            let ast = ExpressionAst::new(root, nodes).expect("구조 유효");
            let view = ExpressionGraphView::new(&ast);
            if let Some(socket) = socket {
                let block = view.block(socket.block).expect("block 존재");
                assert!(block.inputs().contains(&socket), "{name}");
            }
        }
    }

    #[test]
    fn multiple_type_errors_map_in_order_and_valid_ast_has_none() {
        let nodes = vec![
            ExprNode::binary(id(1), BinaryOp::Equal, id(2), id(3)),
            ExprNode::binary(id(2), BinaryOp::Add, id(4), id(5)),
            ExprNode::unary(id(3), UnaryOp::Not, id(6)),
            length(4),
            color(5),
            number(6),
        ];
        let diagnostics = diagnose(id(1), nodes, &refs());
        let locations: Vec<(BlockId, Option<SocketId>)> =
            diagnostics.iter().map(|d| (d.block, d.socket)).collect();
        assert_eq!(
            locations,
            vec![
                (block(2), socket(2, ChildSlot::Rhs)),
                (block(3), socket(3, ChildSlot::Operand))
            ]
        );
        let valid = vec![
            ExprNode::binary(id(1), BinaryOp::Add, id(2), id(2)),
            length(2),
        ];
        assert!(diagnose(id(1), valid, &refs()).is_empty());
        assert!(diagnostics[0].to_string().contains("#rhs"));
    }
}
