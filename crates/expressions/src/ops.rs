//! 닫힌(closed) 표현식 연산 집합 (VIII.1.5).
//!
//! 표현식 모델에 들어갈 수 있는 연산은 이 모듈의 enum 값뿐입니다. 각 enum은 `#[non_exhaustive]`가
//! 아니고 문자열·사용자 정의 payload variant가 없으므로, 아래 능력은 **표현 자체가 불가능**합니다
//! (아키텍처 §14.1 금지 목록).
//!
//! - 파일·네트워크·Git·터미널 접근, 현재 시각, 난수: 그런 연산 kind·함수 ID가 없습니다. 함수 호출은
//!   [`FunctionId`] registry(VIII.1.4)의 순수 함수만 가리킵니다.
//! - 반복: 반복 node kind가 없습니다(7개 kind: Literal·Reference·Unary·Binary·Call·If·Match).
//! - 재귀: 사용자 정의 함수가 없고, AST 참조 cycle은 cycle detector(VIII.2.3)가 거부합니다.
//!
//! 직렬화된 연산은 [`ExpressionOp::encode`]의 고정 3바이트(`node kind u8` + `operand u16 LE`)이며,
//! [`ExpressionOp::decode`]는 목록에 없는 kind·operator·함수 ID, operand가 없어야 하는 kind의 0이 아닌
//! operand, 길이 불일치를 모두 [`ClosedOpError`]로 거부합니다. 유효한 3바이트 조합은 정확히
//! [`ExpressionOp::all`]의 개수만큼만 존재합니다.

use std::fmt;

use crate::arithmetic::{check_arithmetic, check_negation, ArithmeticOp};
use crate::functions::FunctionId;
use crate::logic::{check_comparison, check_logical, check_not, ComparisonOp, LogicalOp};
use crate::rules::TypeRuleError;
use crate::types::TypeSpec;

/// AST node 종류 (VIII.1.5, VIII.2.1). numeric tag는 직렬화 포맷 계약입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeKind {
    /// 리터럴 값.
    Literal,
    /// 토큰·속성·component property·환경 입력 참조.
    Reference,
    /// 단항 연산.
    Unary,
    /// 이항 연산.
    Binary,
    /// 등록 함수 호출.
    Call,
    /// `if` 조건식.
    If,
    /// `match` 조건식.
    Match,
}

impl NodeKind {
    /// 모든 node 종류(tag 오름차순).
    pub const ALL: [NodeKind; 7] = [
        NodeKind::Literal,
        NodeKind::Reference,
        NodeKind::Unary,
        NodeKind::Binary,
        NodeKind::Call,
        NodeKind::If,
        NodeKind::Match,
    ];

    /// 안정 numeric tag.
    pub const fn tag(&self) -> u8 {
        match self {
            NodeKind::Literal => 0x01,
            NodeKind::Reference => 0x02,
            NodeKind::Unary => 0x03,
            NodeKind::Binary => 0x04,
            NodeKind::Call => 0x05,
            NodeKind::If => 0x06,
            NodeKind::Match => 0x07,
        }
    }

    /// tag로 복원합니다. 미등록 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<NodeKind> {
        match tag {
            0x01 => Some(NodeKind::Literal),
            0x02 => Some(NodeKind::Reference),
            0x03 => Some(NodeKind::Unary),
            0x04 => Some(NodeKind::Binary),
            0x05 => Some(NodeKind::Call),
            0x06 => Some(NodeKind::If),
            0x07 => Some(NodeKind::Match),
            _ => None,
        }
    }

    /// 진단용 이름.
    pub const fn name(&self) -> &'static str {
        match self {
            NodeKind::Literal => "literal",
            NodeKind::Reference => "reference",
            NodeKind::Unary => "unary",
            NodeKind::Binary => "binary",
            NodeKind::Call => "call",
            NodeKind::If => "if",
            NodeKind::Match => "match",
        }
    }
}

impl fmt::Display for NodeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 단항 연산자 (VIII.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnaryOp {
    /// 부호 반전 `-x`(VIII.1.2 규칙).
    Negate,
    /// 논리 부정 `not x`(VIII.1.3 규칙).
    Not,
}

impl UnaryOp {
    /// 모든 단항 연산자(tag 오름차순).
    pub const ALL: [UnaryOp; 2] = [UnaryOp::Negate, UnaryOp::Not];

    /// 안정 numeric tag.
    pub const fn tag(&self) -> u16 {
        match self {
            UnaryOp::Negate => 0x01,
            UnaryOp::Not => 0x02,
        }
    }

    /// tag로 복원합니다. 미등록 tag는 `None`.
    pub const fn from_tag(tag: u16) -> Option<UnaryOp> {
        match tag {
            0x01 => Some(UnaryOp::Negate),
            0x02 => Some(UnaryOp::Not),
            _ => None,
        }
    }

    /// 진단용 기호.
    pub const fn symbol(&self) -> &'static str {
        match self {
            UnaryOp::Negate => "neg",
            UnaryOp::Not => "not",
        }
    }

    /// 결과 타입을 검사합니다(VIII.1.2·VIII.1.3 규칙).
    pub fn check(&self, operand: TypeSpec) -> Result<TypeSpec, TypeRuleError> {
        match self {
            UnaryOp::Negate => check_negation(operand),
            UnaryOp::Not => check_not(operand),
        }
    }
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// 이항 연산자의 규칙 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BinaryOpClass {
    /// 산술(VIII.1.2).
    Arithmetic(ArithmeticOp),
    /// 비교(VIII.1.3).
    Comparison(ComparisonOp),
    /// 논리(VIII.1.3).
    Logical(LogicalOp),
}

/// 이항 연산자 (VIII.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BinaryOp {
    /// `+`.
    Add,
    /// `-`.
    Subtract,
    /// `×`.
    Multiply,
    /// `÷`.
    Divide,
    /// 나머지 `%`.
    Remainder,
    /// `==`.
    Equal,
    /// `!=`.
    NotEqual,
    /// `<`.
    Less,
    /// `<=`.
    LessEqual,
    /// `>`.
    Greater,
    /// `>=`.
    GreaterEqual,
    /// `and`.
    And,
    /// `or`.
    Or,
}

impl BinaryOp {
    /// 모든 이항 연산자(tag 오름차순).
    pub const ALL: [BinaryOp; 13] = [
        BinaryOp::Add,
        BinaryOp::Subtract,
        BinaryOp::Multiply,
        BinaryOp::Divide,
        BinaryOp::Remainder,
        BinaryOp::Equal,
        BinaryOp::NotEqual,
        BinaryOp::Less,
        BinaryOp::LessEqual,
        BinaryOp::Greater,
        BinaryOp::GreaterEqual,
        BinaryOp::And,
        BinaryOp::Or,
    ];

    /// 안정 numeric tag. 산술 `0x01~0x05`, 비교 `0x10~0x15`, 논리 `0x20~0x21`.
    pub const fn tag(&self) -> u16 {
        match self {
            BinaryOp::Add => 0x01,
            BinaryOp::Subtract => 0x02,
            BinaryOp::Multiply => 0x03,
            BinaryOp::Divide => 0x04,
            BinaryOp::Remainder => 0x05,
            BinaryOp::Equal => 0x10,
            BinaryOp::NotEqual => 0x11,
            BinaryOp::Less => 0x12,
            BinaryOp::LessEqual => 0x13,
            BinaryOp::Greater => 0x14,
            BinaryOp::GreaterEqual => 0x15,
            BinaryOp::And => 0x20,
            BinaryOp::Or => 0x21,
        }
    }

    /// tag로 복원합니다. 미등록 tag는 `None`.
    pub fn from_tag(tag: u16) -> Option<BinaryOp> {
        BinaryOp::ALL.into_iter().find(|op| op.tag() == tag)
    }

    /// 규칙 분류.
    pub const fn class(&self) -> BinaryOpClass {
        match self {
            BinaryOp::Add => BinaryOpClass::Arithmetic(ArithmeticOp::Add),
            BinaryOp::Subtract => BinaryOpClass::Arithmetic(ArithmeticOp::Subtract),
            BinaryOp::Multiply => BinaryOpClass::Arithmetic(ArithmeticOp::Multiply),
            BinaryOp::Divide => BinaryOpClass::Arithmetic(ArithmeticOp::Divide),
            BinaryOp::Remainder => BinaryOpClass::Arithmetic(ArithmeticOp::Remainder),
            BinaryOp::Equal => BinaryOpClass::Comparison(ComparisonOp::Equal),
            BinaryOp::NotEqual => BinaryOpClass::Comparison(ComparisonOp::NotEqual),
            BinaryOp::Less => BinaryOpClass::Comparison(ComparisonOp::Less),
            BinaryOp::LessEqual => BinaryOpClass::Comparison(ComparisonOp::LessEqual),
            BinaryOp::Greater => BinaryOpClass::Comparison(ComparisonOp::Greater),
            BinaryOp::GreaterEqual => BinaryOpClass::Comparison(ComparisonOp::GreaterEqual),
            BinaryOp::And => BinaryOpClass::Logical(LogicalOp::And),
            BinaryOp::Or => BinaryOpClass::Logical(LogicalOp::Or),
        }
    }

    /// 진단용 기호.
    pub const fn symbol(&self) -> &'static str {
        match self.class() {
            BinaryOpClass::Arithmetic(op) => op.symbol(),
            BinaryOpClass::Comparison(op) => op.symbol(),
            BinaryOpClass::Logical(op) => op.symbol(),
        }
    }

    /// 결과 타입을 검사합니다(VIII.1.2·VIII.1.3 규칙).
    pub fn check(&self, lhs: TypeSpec, rhs: TypeSpec) -> Result<TypeSpec, TypeRuleError> {
        match self.class() {
            BinaryOpClass::Arithmetic(op) => check_arithmetic(op, lhs, rhs),
            BinaryOpClass::Comparison(op) => check_comparison(op, lhs, rhs),
            BinaryOpClass::Logical(op) => check_logical(op, lhs, rhs),
        }
    }
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// node 하나의 연산(kind + kind별 operator·함수 ID) (VIII.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExpressionOp {
    /// 리터럴.
    Literal,
    /// 참조.
    Reference,
    /// 단항 연산.
    Unary(UnaryOp),
    /// 이항 연산.
    Binary(BinaryOp),
    /// 등록 함수 호출.
    Call(FunctionId),
    /// `if`.
    If,
    /// `match`.
    Match,
}

/// 직렬화된 연산 디코드 실패 (VIII.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClosedOpError {
    /// 입력 길이가 고정 길이(3바이트)보다 짧습니다.
    Truncated,
    /// 고정 길이 뒤에 잉여 bytes가 있습니다.
    TrailingBytes,
    /// 등록되지 않은 node kind tag.
    UnknownNodeKind(u8),
    /// 등록되지 않은 단항 연산자 tag.
    UnknownUnaryOp(u16),
    /// 등록되지 않은 이항 연산자 tag.
    UnknownBinaryOp(u16),
    /// 등록되지 않은 함수 ID.
    UnknownFunction(u16),
    /// operand가 없어야 하는 kind에 0이 아닌 operand가 있습니다.
    UnexpectedOperand {
        /// node kind.
        kind: NodeKind,
        /// 실제 operand.
        operand: u16,
    },
}

impl fmt::Display for ClosedOpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClosedOpError::Truncated => f.write_str("연산 bytes가 3바이트보다 짧습니다"),
            ClosedOpError::TrailingBytes => f.write_str("연산 bytes 뒤에 잉여 bytes가 있습니다"),
            ClosedOpError::UnknownNodeKind(tag) => {
                write!(f, "등록되지 않은 node kind: 0x{tag:02X}")
            }
            ClosedOpError::UnknownUnaryOp(tag) => {
                write!(f, "등록되지 않은 단항 연산자: 0x{tag:04X}")
            }
            ClosedOpError::UnknownBinaryOp(tag) => {
                write!(f, "등록되지 않은 이항 연산자: 0x{tag:04X}")
            }
            ClosedOpError::UnknownFunction(tag) => {
                write!(f, "등록되지 않은 함수 ID: 0x{tag:04X}")
            }
            ClosedOpError::UnexpectedOperand { kind, operand } => {
                write!(
                    f,
                    "{kind} 연산에는 operand가 없어야 합니다: 0x{operand:04X}"
                )
            }
        }
    }
}

impl std::error::Error for ClosedOpError {}

impl ExpressionOp {
    /// 직렬화 길이(바이트).
    pub const ENCODED_LEN: usize = 3;

    /// node 종류.
    pub const fn kind(&self) -> NodeKind {
        match self {
            ExpressionOp::Literal => NodeKind::Literal,
            ExpressionOp::Reference => NodeKind::Reference,
            ExpressionOp::Unary(_) => NodeKind::Unary,
            ExpressionOp::Binary(_) => NodeKind::Binary,
            ExpressionOp::Call(_) => NodeKind::Call,
            ExpressionOp::If => NodeKind::If,
            ExpressionOp::Match => NodeKind::Match,
        }
    }

    /// kind별 operand(operator tag·함수 ID). operand가 없는 kind는 0.
    pub const fn operand(&self) -> u16 {
        match self {
            ExpressionOp::Unary(op) => op.tag(),
            ExpressionOp::Binary(op) => op.tag(),
            ExpressionOp::Call(function) => function.tag(),
            ExpressionOp::Literal
            | ExpressionOp::Reference
            | ExpressionOp::If
            | ExpressionOp::Match => 0,
        }
    }

    /// 고정 3바이트 직렬화: `kind tag u8` ++ `operand u16 LE`.
    pub fn encode(&self) -> [u8; ExpressionOp::ENCODED_LEN] {
        let operand = self.operand().to_le_bytes();
        [self.kind().tag(), operand[0], operand[1]]
    }

    /// [`ExpressionOp::encode`]의 역. 정확히 3바이트여야 하며 닫힌 집합 밖의 값은 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<ExpressionOp, ClosedOpError> {
        match bytes.len() {
            len if len < ExpressionOp::ENCODED_LEN => return Err(ClosedOpError::Truncated),
            len if len > ExpressionOp::ENCODED_LEN => return Err(ClosedOpError::TrailingBytes),
            _ => {}
        }
        let kind = NodeKind::from_tag(bytes[0]).ok_or(ClosedOpError::UnknownNodeKind(bytes[0]))?;
        let operand = u16::from_le_bytes([bytes[1], bytes[2]]);
        let no_operand = |op: ExpressionOp| {
            if operand == 0 {
                Ok(op)
            } else {
                Err(ClosedOpError::UnexpectedOperand { kind, operand })
            }
        };
        match kind {
            NodeKind::Literal => no_operand(ExpressionOp::Literal),
            NodeKind::Reference => no_operand(ExpressionOp::Reference),
            NodeKind::If => no_operand(ExpressionOp::If),
            NodeKind::Match => no_operand(ExpressionOp::Match),
            NodeKind::Unary => UnaryOp::from_tag(operand)
                .map(ExpressionOp::Unary)
                .ok_or(ClosedOpError::UnknownUnaryOp(operand)),
            NodeKind::Binary => BinaryOp::from_tag(operand)
                .map(ExpressionOp::Binary)
                .ok_or(ClosedOpError::UnknownBinaryOp(operand)),
            NodeKind::Call => FunctionId::from_tag(operand)
                .map(ExpressionOp::Call)
                .map_err(|_| ClosedOpError::UnknownFunction(operand)),
        }
    }

    /// 닫힌 집합의 모든 연산(kind 순, kind 안에서는 operand 순).
    pub fn all() -> Vec<ExpressionOp> {
        let mut ops = vec![ExpressionOp::Literal, ExpressionOp::Reference];
        ops.extend(UnaryOp::ALL.into_iter().map(ExpressionOp::Unary));
        ops.extend(BinaryOp::ALL.into_iter().map(ExpressionOp::Binary));
        ops.extend(FunctionId::ALL.into_iter().map(ExpressionOp::Call));
        ops.push(ExpressionOp::If);
        ops.push(ExpressionOp::Match);
        ops
    }

    /// 진단용 이름(kind 이름, operator 기호, 함수 이름).
    pub fn name(&self) -> &'static str {
        match self {
            ExpressionOp::Unary(op) => op.symbol(),
            ExpressionOp::Binary(op) => op.symbol(),
            ExpressionOp::Call(function) => function.name(),
            other => other.kind().name(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::types::ExpressionType;

    #[test]
    fn every_op_round_trips() {
        let ops = ExpressionOp::all();
        assert_eq!(ops.len(), 2 + 2 + 13 + 16 + 2);
        let encodings: BTreeSet<[u8; 3]> = ops.iter().map(ExpressionOp::encode).collect();
        assert_eq!(encodings.len(), ops.len(), "인코딩 유일");
        for op in ops {
            assert_eq!(ExpressionOp::decode(&op.encode()), Ok(op), "{}", op.name());
        }
    }

    #[test]
    fn tags_are_stable_golden() {
        let kinds: Vec<(u8, &str)> = NodeKind::ALL.iter().map(|k| (k.tag(), k.name())).collect();
        assert_eq!(
            kinds,
            vec![
                (0x01, "literal"),
                (0x02, "reference"),
                (0x03, "unary"),
                (0x04, "binary"),
                (0x05, "call"),
                (0x06, "if"),
                (0x07, "match"),
            ]
        );
        let unary: Vec<(u16, &str)> = UnaryOp::ALL.iter().map(|o| (o.tag(), o.symbol())).collect();
        assert_eq!(unary, vec![(0x01, "neg"), (0x02, "not")]);
        let binary: Vec<(u16, &str)> = BinaryOp::ALL
            .iter()
            .map(|o| (o.tag(), o.symbol()))
            .collect();
        assert_eq!(
            binary,
            vec![
                (0x01, "+"),
                (0x02, "-"),
                (0x03, "*"),
                (0x04, "/"),
                (0x05, "%"),
                (0x10, "=="),
                (0x11, "!="),
                (0x12, "<"),
                (0x13, "<="),
                (0x14, ">"),
                (0x15, ">="),
                (0x20, "and"),
                (0x21, "or"),
            ]
        );
        assert_eq!(
            ExpressionOp::Call(FunctionId::Mix).encode(),
            [0x05, 0x01, 0x01]
        );
        assert_eq!(
            ExpressionOp::Binary(BinaryOp::Less).encode(),
            [0x04, 0x12, 0x00]
        );
    }

    #[test]
    fn serialized_malicious_unknown_ops_are_rejected() {
        let corpus: Vec<(&str, Vec<u8>, ClosedOpError)> = vec![
            ("빈 입력", vec![], ClosedOpError::Truncated),
            ("kind만", vec![0x05], ClosedOpError::Truncated),
            ("operand 절반", vec![0x05, 0x01], ClosedOpError::Truncated),
            (
                "잉여 byte",
                vec![0x01, 0x00, 0x00, 0x00],
                ClosedOpError::TrailingBytes,
            ),
            (
                "kind 0",
                vec![0x00, 0x00, 0x00],
                ClosedOpError::UnknownNodeKind(0x00),
            ),
            (
                "가상 반복(loop) kind",
                vec![0x08, 0x00, 0x00],
                ClosedOpError::UnknownNodeKind(0x08),
            ),
            (
                "가상 외부 호출 kind",
                vec![0xFF, 0x01, 0x00],
                ClosedOpError::UnknownNodeKind(0xFF),
            ),
            (
                "가상 파일 읽기 함수",
                vec![0x05, 0x00, 0x0F],
                ClosedOpError::UnknownFunction(0x0F00),
            ),
            (
                "가상 시각·난수 함수",
                vec![0x05, 0xFF, 0xFF],
                ClosedOpError::UnknownFunction(0xFFFF),
            ),
            (
                "함수 ID 0",
                vec![0x05, 0x00, 0x00],
                ClosedOpError::UnknownFunction(0x0000),
            ),
            (
                "단항 tag 3",
                vec![0x03, 0x03, 0x00],
                ClosedOpError::UnknownUnaryOp(0x0003),
            ),
            (
                "이항 tag 0x22",
                vec![0x04, 0x22, 0x00],
                ClosedOpError::UnknownBinaryOp(0x0022),
            ),
            (
                "리터럴에 숨긴 operand",
                vec![0x01, 0x01, 0x00],
                ClosedOpError::UnexpectedOperand {
                    kind: NodeKind::Literal,
                    operand: 0x0001,
                },
            ),
            (
                "if에 숨긴 operand",
                vec![0x06, 0x00, 0x80],
                ClosedOpError::UnexpectedOperand {
                    kind: NodeKind::If,
                    operand: 0x8000,
                },
            ),
            (
                "참조에 숨긴 operand",
                vec![0x02, 0x05, 0x02],
                ClosedOpError::UnexpectedOperand {
                    kind: NodeKind::Reference,
                    operand: 0x0205,
                },
            ),
        ];
        for (name, bytes, expected) in corpus {
            assert_eq!(ExpressionOp::decode(&bytes), Err(expected), "{name}");
        }
    }

    #[test]
    fn only_registered_encodings_decode() {
        // 3바이트 공간 전체(256 × 65536)에서 디코드 성공은 닫힌 집합 개수와 정확히 같습니다.
        let mut accepted = BTreeSet::new();
        for kind in 0..=u8::MAX {
            for operand in 0..=u16::MAX {
                let le = operand.to_le_bytes();
                if let Ok(op) = ExpressionOp::decode(&[kind, le[0], le[1]]) {
                    accepted.insert(op);
                }
            }
        }
        let expected: BTreeSet<ExpressionOp> = ExpressionOp::all().into_iter().collect();
        assert_eq!(accepted, expected);
    }

    #[test]
    fn no_forbidden_capability_is_representable() {
        let forbidden = [
            "file",
            "read",
            "write",
            "net",
            "http",
            "fetch",
            "url",
            "socket",
            "git",
            "terminal",
            "shell",
            "exec",
            "eval",
            "time",
            "now",
            "clock",
            "date",
            "random",
            "rand",
            "loop",
            "while",
            "for",
            "repeat",
            "recur",
            "recursive",
            "global",
            "define",
            "lambda",
            "fn",
            "import",
            "env",
            "sleep",
            "wait",
        ];
        for op in ExpressionOp::all() {
            let name = op.name();
            // 이름을 `_` 단위 단어로 나눠 비교합니다(`format_number`의 "for" 같은 부분 일치 제외).
            for word in name.split('_') {
                assert!(
                    !forbidden.contains(&word),
                    "금지 능력 이름이 연산 집합에 있음: {name}"
                );
            }
        }
        // node kind는 7개로 닫혀 있고 반복 kind가 없습니다.
        assert_eq!(NodeKind::ALL.len(), 7);
    }

    #[test]
    fn op_type_checks_dispatch_to_rule_tables() {
        let length = TypeSpec::required(ExpressionType::Length);
        let boolean = TypeSpec::required(ExpressionType::Boolean);
        assert_eq!(BinaryOp::Add.check(length, length), Ok(length));
        assert_eq!(BinaryOp::Less.check(length, length), Ok(boolean));
        assert_eq!(BinaryOp::And.check(boolean, boolean), Ok(boolean));
        assert!(BinaryOp::And.check(length, boolean).is_err());
        assert_eq!(UnaryOp::Negate.check(length), Ok(length));
        assert_eq!(UnaryOp::Not.check(boolean), Ok(boolean));
        assert!(UnaryOp::Not.check(length).is_err());
    }
}
