//! Expression AST type checker (VIII.2.4).
//!
//! [`type_check`]는 검증된 AST의 모든 node에 대해 추론 타입을 계산하고, 규칙 위반을 **node StableId**가
//! 포함된 [`TypeError`]로 모읍니다. root→node 오류 경로는 보고서의 [`TypeCheckReport::path_to`]가 돌려줍니다.
//! 규칙은 산술(VIII.1.2), 비교·논리·조건식(VIII.1.3), 함수 signature(VIII.1.4) 표를 그대로 사용합니다.
//!
//! - 순회는 root에서 시작하는 반복(비재귀) 후위 순회이며 child를 slot 순서로 방문합니다. 공유(DAG)
//!   node는 한 번만 검사합니다. 오류 목록 순서는 이 후위 순서라 결정적입니다.
//! - 오류가 난 node는 타입을 갖지 않습니다. 그 부모는 child 타입이 없으므로 검사하지 않아, 하나의
//!   원인이 조상 전체로 번지는 연쇄 오류를 만들지 않습니다.
//! - 오류 경로는 root에서 slot 순서 전위 순회로 처음 발견한 경로입니다. 경로는 오류마다 복제하지 않고 순회의
//!   부모 map 하나만 보고서에 보관한 뒤 요청할 때 재구성합니다(EXPR-1). 깊이 `d` 사슬 곳곳에 오류가 있어도
//!   검사 비용과 보고서·오류 목록 메모리는 AST 크기에 선형이며, 경로 하나의 재구성은 O(d)입니다.
//! - 참조 타입은 호출자가 [`ReferenceTypes`]로 제공합니다(토큰·속성 타입은 이 crate 밖의 원본).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::ast::{
    ChildSlot, ExprNode, ExprNodeId, ExpressionAst, Literal, NodePayload, ReferenceTarget,
};
use crate::functions::{check_call, CallTypeError};
use crate::logic::{check_if, check_match, MatchArmType};
use crate::rules::{OperandPosition, TypeRuleError};
use crate::types::{ExpressionType, TypeSpec};

/// 참조 대상의 타입 제공자. 모르는 대상이면 `None`.
pub trait ReferenceTypes {
    /// 참조 대상의 타입.
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec>;
}

impl ReferenceTypes for BTreeMap<ReferenceTarget, TypeSpec> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        self.get(target).copied()
    }
}

impl OperandPosition {
    /// 규칙 위반 위치에 대응하는 AST child slot. pattern 위반은 그 arm slot으로 대응합니다.
    pub const fn child_slot(&self) -> ChildSlot {
        match self {
            OperandPosition::Operand => ChildSlot::Operand,
            OperandPosition::Lhs => ChildSlot::Lhs,
            OperandPosition::Rhs => ChildSlot::Rhs,
            OperandPosition::Condition => ChildSlot::Condition,
            OperandPosition::Then => ChildSlot::Then,
            OperandPosition::Else => ChildSlot::Else,
            OperandPosition::Scrutinee => ChildSlot::Scrutinee,
            OperandPosition::Pattern(index) | OperandPosition::Arm(index) => ChildSlot::Arm(*index),
            OperandPosition::Default => ChildSlot::Default,
            OperandPosition::Argument(index) => ChildSlot::Argument(*index),
        }
    }
}

/// type 오류 종류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeErrorKind {
    /// 연산·조건식 규칙 위반.
    Rule(TypeRuleError),
    /// 함수 호출 위반.
    Call(CallTypeError),
    /// 타입을 알 수 없는 참조 대상.
    UnknownReference(ReferenceTarget),
}

impl TypeErrorKind {
    /// 위반한 피연산자 위치(있으면).
    pub fn position(&self) -> Option<OperandPosition> {
        match self {
            TypeErrorKind::Rule(error) => Some(error.position),
            TypeErrorKind::Call(CallTypeError::Argument { error, .. }) => Some(error.position),
            TypeErrorKind::Call(CallTypeError::Arity { .. })
            | TypeErrorKind::UnknownReference(_) => None,
        }
    }
}

impl fmt::Display for TypeErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeErrorKind::Rule(error) => write!(f, "{error}"),
            TypeErrorKind::Call(error) => write!(f, "{error}"),
            TypeErrorKind::UnknownReference(target) => {
                write!(f, "타입을 알 수 없는 참조: {target}")
            }
        }
    }
}

/// node 하나의 type 오류 (VIII.2.4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeError {
    /// 규칙을 위반한 node. root에서 이 node까지의 경로는 [`TypeCheckReport::path_to`]로 얻습니다.
    pub node: ExprNodeId,
    /// 위반 위치의 child slot(피연산자 위치가 있는 오류만).
    pub slot: Option<ChildSlot>,
    /// 위반 위치에 연결된 child node(있으면).
    pub child: Option<ExprNodeId>,
    /// 오류 내용.
    pub kind: TypeErrorKind,
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "node {}: {}", self.node, self.kind)
    }
}

impl std::error::Error for TypeError {}

/// type 검사 결과: node별 추론 타입과 오류 목록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCheckReport {
    types: BTreeMap<ExprNodeId, TypeSpec>,
    errors: Vec<TypeError>,
    /// root에서 slot 순서 전위 순회로 처음 발견한 부모(root 제외). 오류 경로 재구성용입니다.
    parents: BTreeMap<ExprNodeId, ExprNodeId>,
    root: ExprNodeId,
}

impl TypeCheckReport {
    /// node의 추론 타입. 오류 node와 그 조상은 `None`.
    pub fn type_of(&self, id: ExprNodeId) -> Option<TypeSpec> {
        self.types.get(&id).copied()
    }

    /// root의 추론 타입.
    pub fn root_type(&self) -> Option<TypeSpec> {
        self.type_of(self.root)
    }

    /// 모든 추론 타입(ID 순).
    pub fn types(&self) -> &BTreeMap<ExprNodeId, TypeSpec> {
        &self.types
    }

    /// 오류 목록(후위 순회 순).
    pub fn errors(&self) -> &[TypeError] {
        &self.errors
    }

    /// 오류가 없으면 `true`.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// root에서 `node`까지의 경로(양 끝 포함). root에서 slot 순서 전위 순회로 처음 발견한 경로이며, 검사한
    /// AST에 없는 node면 `None`입니다. 비용은 경로 길이에 비례합니다.
    pub fn path_to(&self, node: ExprNodeId) -> Option<Vec<ExprNodeId>> {
        if node != self.root && !self.parents.contains_key(&node) {
            return None;
        }
        let mut path = vec![node];
        let mut current = node;
        while current != self.root {
            // 부모 map은 순회 tree라 cycle이 없고, root가 아닌 모든 node에 부모가 있습니다.
            current = *self.parents.get(&current)?;
            path.push(current);
        }
        path.reverse();
        Some(path)
    }
}

/// node 하나의 타입을 child 타입으로부터 추론합니다.
///
/// `child_type`이 필요한 slot에 `None`을 주면 결과도 `None`(아직 판단 불가)입니다.
pub fn infer_node(
    node: &ExprNode,
    child_type: &dyn Fn(ChildSlot) -> Option<TypeSpec>,
    references: &dyn ReferenceTypes,
) -> Option<Result<TypeSpec, TypeErrorKind>> {
    let rule = |result: Result<TypeSpec, TypeRuleError>| result.map_err(TypeErrorKind::Rule);
    Some(match node.payload() {
        NodePayload::Literal(Literal::Value(value)) => {
            Ok(TypeSpec::required(ExpressionType::of(value)))
        }
        NodePayload::Literal(Literal::Null(value_type)) => Ok(TypeSpec::optional(*value_type)),
        NodePayload::Reference(target) => references
            .reference_type(target)
            .ok_or_else(|| TypeErrorKind::UnknownReference(target.clone())),
        NodePayload::Unary(op) => rule(op.check(child_type(ChildSlot::Operand)?)),
        NodePayload::Binary(op) => {
            rule(op.check(child_type(ChildSlot::Lhs)?, child_type(ChildSlot::Rhs)?))
        }
        NodePayload::Call(function) => {
            let mut args = Vec::with_capacity(node.children().len());
            for index in 0..function.signature().arity() {
                let slot = ChildSlot::Argument(u16::try_from(index).unwrap_or(u16::MAX));
                args.push(child_type(slot)?);
            }
            check_call(*function, &args).map_err(TypeErrorKind::Call)
        }
        NodePayload::If => rule(check_if(
            child_type(ChildSlot::Condition)?,
            child_type(ChildSlot::Then)?,
            child_type(ChildSlot::Else)?,
        )),
        NodePayload::Match(patterns) => {
            let scrutinee = child_type(ChildSlot::Scrutinee)?;
            let mut arms = Vec::with_capacity(patterns.len());
            for (index, pattern) in patterns.iter().enumerate() {
                let value = child_type(ChildSlot::Arm(u16::try_from(index).unwrap_or(u16::MAX)))?;
                arms.push(MatchArmType { pattern, value });
            }
            rule(check_match(
                scrutinee,
                &arms,
                child_type(ChildSlot::Default)?,
            ))
        }
    })
}

/// AST 전체를 type 검사합니다(VIII.2.4).
pub fn type_check(ast: &ExpressionAst, references: &dyn ReferenceTypes) -> TypeCheckReport {
    let (order, parents) = post_order(ast);
    let mut types: BTreeMap<ExprNodeId, TypeSpec> = BTreeMap::new();
    let mut errors = Vec::new();
    for id in order {
        let Some(node) = ast.node(id) else {
            continue;
        };
        let lookup = |slot: ChildSlot| {
            node.child(slot)
                .and_then(|child| types.get(&child).copied())
        };
        match infer_node(node, &lookup, references) {
            None => {}
            Some(Ok(spec)) => {
                types.insert(id, spec);
            }
            Some(Err(kind)) => {
                let slot = kind.position().map(|position| position.child_slot());
                errors.push(TypeError {
                    node: id,
                    slot,
                    child: slot.and_then(|slot| node.child(slot)),
                    kind,
                });
            }
        }
    }
    TypeCheckReport {
        types,
        errors,
        parents,
        root: ast.root(),
    }
}

/// root에서의 후위 순서와, 전위(slot 순서)로 처음 발견한 부모 map.
fn post_order(ast: &ExpressionAst) -> (Vec<ExprNodeId>, BTreeMap<ExprNodeId, ExprNodeId>) {
    let mut order = Vec::with_capacity(ast.len());
    let mut parents = BTreeMap::new();
    let mut seen = BTreeSet::new();
    // (node, 발견한 부모, 확장 완료 여부)
    let mut stack: Vec<(ExprNodeId, Option<ExprNodeId>, bool)> = vec![(ast.root(), None, false)];
    while let Some((id, parent, expanded)) = stack.pop() {
        if expanded {
            order.push(id);
            continue;
        }
        if !seen.insert(id) {
            continue;
        }
        if let Some(parent) = parent {
            parents.insert(id, parent);
        }
        stack.push((id, None, true));
        if let Some(node) = ast.node(id) {
            for child in node.children().iter().rev() {
                if !seen.contains(&child.target) {
                    stack.push((child.target, Some(id), false));
                }
            }
        }
    }
    (order, parents)
}

#[cfg(test)]
mod tests {
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::StableId;
    use stellar_core_types::reference::TokenRef;
    use stellar_core_types::typed_value::{EnumValue, TypedValue};
    use stellar_core_types::value::{FiniteF32, Length};

    use super::*;
    use crate::ast::EnvironmentKey;
    use crate::functions::FunctionId;
    use crate::ops::{BinaryOp, UnaryOp};
    use crate::rules::Expectation;
    use crate::types::EnumTypeId;

    fn id(n: u64) -> ExprNodeId {
        ExprNodeId::new(StableId::new_v7(1_700_000_000_000 + n, 0, n))
    }

    fn length(n: u64, value: f32) -> ExprNode {
        ExprNode::literal(id(n), TypedValue::Length(Length::new(value).expect("유한")))
    }

    fn number(n: u64, value: f32) -> ExprNode {
        ExprNode::literal(
            id(n),
            TypedValue::Number(FiniteF32::new(value).expect("유한")),
        )
    }

    fn color(n: u64) -> ExprNode {
        ExprNode::literal(
            id(n),
            TypedValue::Color(Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).expect("색")),
        )
    }

    fn token(n: u64) -> ReferenceTarget {
        ReferenceTarget::Token(TokenRef::new(StableId::new_v7(9, 9, n)))
    }

    fn refs() -> BTreeMap<ReferenceTarget, TypeSpec> {
        BTreeMap::from([
            (token(1), TypeSpec::required(ExpressionType::Length)),
            (
                token(2),
                TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(3))),
            ),
            (
                ReferenceTarget::Environment(EnvironmentKey::new(1)),
                TypeSpec::required(ExpressionType::Boolean),
            ),
        ])
    }

    #[test]
    fn valid_expression_infers_every_node_type() {
        // if env.dark then clamp(token1 * 2, 0, 100) else -token1
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::if_else(id(1), id(2), id(3), id(4)),
                ExprNode::reference(id(2), ReferenceTarget::Environment(EnvironmentKey::new(1))),
                ExprNode::call(id(3), FunctionId::Clamp, &[id(5), id(6), id(7)]),
                ExprNode::unary(id(4), UnaryOp::Negate, id(8)),
                ExprNode::binary(id(5), BinaryOp::Multiply, id(8), id(9)),
                length(6, 0.0),
                length(7, 100.0),
                ExprNode::reference(id(8), token(1)),
                number(9, 2.0),
            ],
        )
        .expect("구조 유효");
        let report = type_check(&ast, &refs());
        assert!(report.is_ok(), "{:?}", report.errors());
        let length_t = TypeSpec::required(ExpressionType::Length);
        assert_eq!(report.root_type(), Some(length_t));
        assert_eq!(report.types().len(), ast.len(), "모든 node 타입 추론");
        assert_eq!(
            report.type_of(id(2)),
            Some(TypeSpec::required(ExpressionType::Boolean))
        );
        assert_eq!(report.type_of(id(5)), Some(length_t));
        assert_eq!(
            report.type_of(id(9)),
            Some(TypeSpec::required(ExpressionType::Number))
        );
    }

    #[test]
    fn nested_error_contains_exact_node_stable_id() {
        // root: if (a < b) then (x + color) else z — 오류는 깊이 2의 Add node.
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::if_else(id(1), id(2), id(3), id(4)),
                ExprNode::binary(id(2), BinaryOp::Less, id(5), id(6)),
                ExprNode::binary(id(3), BinaryOp::Add, id(7), id(8)),
                length(4, 1.0),
                length(5, 1.0),
                length(6, 2.0),
                length(7, 3.0),
                color(8),
            ],
        )
        .expect("구조 유효");
        let report = type_check(&ast, &refs());
        assert_eq!(report.errors().len(), 1, "연쇄 오류 없음");
        let error = &report.errors()[0];
        assert_eq!(error.node, id(3), "오류 node StableId");
        assert_eq!(report.path_to(error.node), Some(vec![id(1), id(3)]));
        assert_eq!(error.slot, Some(ChildSlot::Rhs));
        assert_eq!(error.child, Some(id(8)));
        match &error.kind {
            TypeErrorKind::Rule(rule) => {
                assert_eq!(rule.position, OperandPosition::Rhs);
                assert_eq!(
                    rule.expected,
                    Expectation::OneOf(vec![ExpressionType::Length])
                );
            }
            other => panic!("규칙 오류 기대: {other:?}"),
        }
        // 오류 node와 조상은 타입이 없고, 무관한 node는 타입이 있음.
        assert_eq!(report.type_of(id(3)), None);
        assert_eq!(report.root_type(), None);
        assert_eq!(
            report.type_of(id(2)),
            Some(TypeSpec::required(ExpressionType::Boolean))
        );
    }

    #[test]
    fn deeply_nested_errors_report_full_path() {
        // 1: neg(2), 2: neg(3), 3: max(4, 5), 4: Length, 5: abs(6), 6: String → 오류는 node 5(abs의 인자 0).
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::unary(id(1), UnaryOp::Negate, id(2)),
                ExprNode::unary(id(2), UnaryOp::Negate, id(3)),
                ExprNode::call(id(3), FunctionId::Max, &[id(4), id(5)]),
                length(4, 1.0),
                ExprNode::call(id(5), FunctionId::Abs, &[id(6)]),
                ExprNode::literal(id(6), TypedValue::String("x".to_string())),
            ],
        )
        .expect("구조 유효");
        let report = type_check(&ast, &refs());
        assert_eq!(report.errors().len(), 1);
        let error = &report.errors()[0];
        assert_eq!(error.node, id(5));
        assert_eq!(
            report.path_to(error.node),
            Some(vec![id(1), id(2), id(3), id(5)])
        );
        assert_eq!(error.slot, Some(ChildSlot::Argument(0)));
        assert_eq!(error.child, Some(id(6)));
        assert!(matches!(
            error.kind,
            TypeErrorKind::Call(CallTypeError::Argument {
                function: FunctionId::Abs,
                ..
            })
        ));
    }

    #[test]
    fn independent_errors_are_all_reported_in_deterministic_order() {
        // root = (len + color) == (not number) — 두 독립 오류, 후위 순서(Lhs 쪽 먼저).
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::Equal, id(2), id(3)),
                ExprNode::binary(id(2), BinaryOp::Add, id(4), id(5)),
                ExprNode::unary(id(3), UnaryOp::Not, id(6)),
                length(4, 1.0),
                color(5),
                number(6, 1.0),
            ],
        )
        .expect("구조 유효");
        let report = type_check(&ast, &refs());
        let nodes: Vec<ExprNodeId> = report.errors().iter().map(|e| e.node).collect();
        assert_eq!(nodes, vec![id(2), id(3)]);
        assert_eq!(report.errors()[1].slot, Some(ChildSlot::Operand));
        assert_eq!(
            report.path_to(report.errors()[1].node),
            Some(vec![id(1), id(3)])
        );
    }

    #[test]
    fn unknown_reference_and_match_errors() {
        let unknown = token(99);
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::Add, id(2), id(3)),
                ExprNode::reference(id(2), unknown.clone()),
                length(3, 1.0),
            ],
        )
        .expect("구조 유효");
        let report = type_check(&ast, &refs());
        assert_eq!(report.errors().len(), 1);
        assert_eq!(report.errors()[0].node, id(2));
        assert_eq!(
            report.errors()[0].kind,
            TypeErrorKind::UnknownReference(unknown)
        );
        assert_eq!(report.errors()[0].slot, None);

        // match token2 { Enum(3,0) => len, Enum(4,0) => len } else len — pattern 1의 enum 타입 불일치.
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::match_on(
                    id(1),
                    id(2),
                    vec![
                        (TypedValue::Enum(EnumValue::new(3, 0)), id(3)),
                        (TypedValue::Enum(EnumValue::new(4, 0)), id(4)),
                    ],
                    id(5),
                ),
                ExprNode::reference(id(2), token(2)),
                length(3, 1.0),
                length(4, 2.0),
                length(5, 3.0),
            ],
        )
        .expect("구조 유효");
        let report = type_check(&ast, &refs());
        assert_eq!(report.errors().len(), 1);
        let error = &report.errors()[0];
        assert_eq!(error.node, id(1));
        assert_eq!(
            error.slot,
            Some(ChildSlot::Arm(1)),
            "pattern 1 → arm 1 slot"
        );
        assert_eq!(error.kind.position(), Some(OperandPosition::Pattern(1)));
    }

    #[test]
    fn shared_error_node_is_reported_once_with_first_path() {
        // 4(오류: len + color)를 2와 3이 공유. 경로는 slot 순서 전위로 처음 발견한 1→2→4.
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::Add, id(2), id(3)),
                ExprNode::unary(id(2), UnaryOp::Negate, id(4)),
                ExprNode::unary(id(3), UnaryOp::Negate, id(4)),
                ExprNode::binary(id(4), BinaryOp::Add, id(5), id(6)),
                length(5, 1.0),
                color(6),
            ],
        )
        .expect("구조 유효");
        let report = type_check(&ast, &refs());
        assert_eq!(report.errors().len(), 1);
        assert_eq!(
            report.path_to(report.errors()[0].node),
            Some(vec![id(1), id(2), id(4)])
        );
    }

    #[test]
    fn path_to_covers_root_and_rejects_foreign_node() {
        let ast = ExpressionAst::new(
            id(1),
            vec![
                ExprNode::binary(id(1), BinaryOp::Add, id(2), id(3)),
                length(2, 1.0),
                color(3),
            ],
        )
        .expect("구조 유효");
        let report = type_check(&ast, &refs());
        assert_eq!(report.errors().len(), 1);
        assert_eq!(report.path_to(id(1)), Some(vec![id(1)]), "root 오류 경로");
        assert_eq!(
            report.path_to(id(3)),
            Some(vec![id(1), id(3)]),
            "오류 없는 node도 경로"
        );
        assert_eq!(report.path_to(id(99)), None, "검사한 AST 밖 node");
    }

    #[test]
    fn deep_ast_is_checked_without_recursion() {
        let depth = 100_000u64;
        let mut nodes: Vec<ExprNode> = (1..depth)
            .map(|n| ExprNode::unary(id(n), UnaryOp::Negate, id(n + 1)))
            .collect();
        nodes.push(ExprNode::literal(
            id(depth),
            TypedValue::String("x".to_string()),
        ));
        let ast = ExpressionAst::new(id(1), nodes).expect("구조 유효");
        let report = type_check(&ast, &refs());
        assert_eq!(report.errors().len(), 1);
        assert_eq!(report.errors()[0].node, id(depth - 1));
        let path = report.path_to(report.errors()[0].node).expect("경로");
        assert_eq!(path.len(), depth as usize - 1);
        assert_eq!(path.first(), Some(&id(1)));
        assert_eq!(report.types().len(), 1, "leaf만 타입 보유");
    }
}
