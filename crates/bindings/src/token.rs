//! 토큰 모델 (VIII.5.1).
//!
//! 디자인 토큰은 이름 붙은 타입 값이며, 값은 default expression([`ExpressionAst`])으로 정의합니다.
//!
//! - **식별**: 토큰은 [`TokenRef`](`StableId`)로 식별합니다. 다른 토큰·binding의 표현식은 이름이 아니라
//!   `ReferenceTarget::Token(TokenRef)`로 가리키므로, 이름을 바꿔도 참조는 그대로 유지됩니다.
//! - **이름**: [`TokenName`]은 사람이 읽는 표시·검색용 이름입니다. 한 [`TokenSet`] 안에서만 유일하며(같은
//!   이름 거부), scope 사이의 같은 이름 허용·shadowing 규칙은 registry(VIII.5.2)가 정합니다.
//! - **타입**: [`TokenType`]은 비-nullable 값 타입입니다. 토큰은 항상 값을 가지므로 default expression의
//!   결과 타입은 정확히 `Required(token type)`이어야 합니다([`TokenDefinition::type_check`]).
//!
//! default expression이 참조할 수 있는 대상의 종류(토큰·환경 입력 등)는 이 모델에서 제한하지 않습니다.
//! 참조 대상의 존재·scope 가시성은 registry(VIII.5.2), cycle은 dependency graph(VIII.5.3)가 검사합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::TokenRef;
use stellar_expressions::ast::{ExpressionAst, ReferenceTarget};
use stellar_expressions::typecheck::{type_check, ReferenceTypes, TypeError};
use stellar_expressions::types::{ExpressionType, TypeSpec};

/// 토큰 이름 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TokenNameError {
    /// 빈 이름.
    Empty,
    /// UTF-8 길이가 [`TokenName::MAX_BYTES`]를 넘습니다.
    TooLong {
        /// 실제 byte 수.
        len: usize,
    },
    /// 제어 문자를 포함합니다.
    ControlCharacter,
    /// 앞뒤 공백이 있습니다.
    SurroundingWhitespace,
}

impl fmt::Display for TokenNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenNameError::Empty => f.write_str("토큰 이름이 비어 있습니다"),
            TokenNameError::TooLong { len } => write!(
                f,
                "토큰 이름이 {len} bytes로 {} bytes를 넘습니다",
                TokenName::MAX_BYTES
            ),
            TokenNameError::ControlCharacter => f.write_str("토큰 이름에 제어 문자가 있습니다"),
            TokenNameError::SurroundingWhitespace => {
                f.write_str("토큰 이름 앞뒤에 공백이 있습니다")
            }
        }
    }
}

impl std::error::Error for TokenNameError {}

/// 사람이 읽는 토큰 이름. 참조에는 쓰지 않습니다(참조는 [`TokenRef`]).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenName(String);

impl TokenName {
    /// 이름의 최대 UTF-8 byte 수.
    pub const MAX_BYTES: usize = 256;

    /// 이름을 검증해 만듭니다: 비어 있지 않고, [`TokenName::MAX_BYTES`] 이하, 제어 문자 없음, 앞뒤 공백 없음.
    pub fn new(name: &str) -> Result<TokenName, TokenNameError> {
        if name.is_empty() {
            return Err(TokenNameError::Empty);
        }
        if name.len() > TokenName::MAX_BYTES {
            return Err(TokenNameError::TooLong { len: name.len() });
        }
        if name.chars().any(char::is_control) {
            return Err(TokenNameError::ControlCharacter);
        }
        if name.trim() != name {
            return Err(TokenNameError::SurroundingWhitespace);
        }
        Ok(TokenName(name.to_string()))
    }

    /// 이름 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TokenName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 토큰 값 타입. 토큰은 항상 값을 가지므로 비-nullable [`ExpressionType`]입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenType(ExpressionType);

impl TokenType {
    /// 값 타입으로 만듭니다.
    pub const fn new(value_type: ExpressionType) -> TokenType {
        TokenType(value_type)
    }

    /// 값 타입.
    pub const fn value_type(&self) -> ExpressionType {
        self.0
    }

    /// 표현식 위치 타입(`Required`). 이 토큰을 참조하는 표현식이 보는 타입입니다.
    pub const fn type_spec(&self) -> TypeSpec {
        TypeSpec::required(self.0)
    }
}

impl fmt::Display for TokenType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// default expression type 검사 오류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TokenTypeError {
    /// 표현식 안의 type 오류(VIII.2.4, 오류 node StableId 포함).
    Expression(Vec<TypeError>),
    /// 표현식 결과 타입이 토큰 타입과 다릅니다.
    ResultMismatch {
        /// 토큰 타입(`Required`).
        expected: TypeSpec,
        /// 표현식 결과 타입.
        actual: TypeSpec,
    },
}

impl fmt::Display for TokenTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenTypeError::Expression(errors) => {
                write!(f, "default expression type 오류 {}개", errors.len())?;
                if let Some(first) = errors.first() {
                    write!(f, ": {first}")?;
                }
                Ok(())
            }
            TokenTypeError::ResultMismatch { expected, actual } => {
                write!(
                    f,
                    "default expression 결과 {actual}가 토큰 타입 {expected}와 다릅니다"
                )
            }
        }
    }
}

impl std::error::Error for TokenTypeError {}

/// 토큰 정의 하나 (VIII.5.1): ID, 이름, 타입, default expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenDefinition {
    id: TokenRef,
    name: TokenName,
    token_type: TokenType,
    default_expression: ExpressionAst,
}

impl TokenDefinition {
    /// 정의를 만듭니다. 표현식 type 검사는 참조 타입을 아는 쪽([`TokenDefinition::type_check`])에서 합니다.
    pub fn new(
        id: TokenRef,
        name: TokenName,
        token_type: TokenType,
        default_expression: ExpressionAst,
    ) -> TokenDefinition {
        TokenDefinition {
            id,
            name,
            token_type,
            default_expression,
        }
    }

    /// 토큰 ID.
    pub const fn id(&self) -> TokenRef {
        self.id
    }

    /// 표시 이름.
    pub fn name(&self) -> &TokenName {
        &self.name
    }

    /// 값 타입.
    pub const fn token_type(&self) -> TokenType {
        self.token_type
    }

    /// default expression.
    pub fn default_expression(&self) -> &ExpressionAst {
        &self.default_expression
    }

    /// default expression이 참조하는 대상 전체(중복 제거, 정렬).
    pub fn references(&self) -> BTreeSet<ReferenceTarget> {
        self.default_expression
            .referenced_targets()
            .into_iter()
            .cloned()
            .collect()
    }

    /// default expression이 참조하는 토큰(중복 제거, 정렬).
    pub fn referenced_tokens(&self) -> BTreeSet<TokenRef> {
        self.default_expression
            .referenced_targets()
            .into_iter()
            .filter_map(|target| match target {
                ReferenceTarget::Token(token) => Some(*token),
                _ => None,
            })
            .collect()
    }

    /// default expression을 type 검사하고 결과가 정확히 `Required(token type)`인지 확인합니다.
    pub fn type_check(&self, references: &dyn ReferenceTypes) -> Result<(), TokenTypeError> {
        check_expression(self.token_type, &self.default_expression, references)
    }

    pub(crate) fn set_name(&mut self, name: TokenName) -> TokenName {
        std::mem::replace(&mut self.name, name)
    }

    pub(crate) fn set_default_expression(&mut self, expression: ExpressionAst) -> ExpressionAst {
        std::mem::replace(&mut self.default_expression, expression)
    }
}

/// `expression`이 `token_type`의 default expression으로 올바른지 검사합니다.
pub fn check_expression(
    token_type: TokenType,
    expression: &ExpressionAst,
    references: &dyn ReferenceTypes,
) -> Result<(), TokenTypeError> {
    let report = type_check(expression, references);
    if !report.is_ok() {
        return Err(TokenTypeError::Expression(report.errors().to_vec()));
    }
    let expected = token_type.type_spec();
    match report.root_type() {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(TokenTypeError::ResultMismatch { expected, actual }),
        // 오류 없는 report는 root 타입을 가집니다. 방어적으로 표현식 오류로 보고합니다.
        None => Err(TokenTypeError::Expression(Vec::new())),
    }
}

/// TokenSet 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenSetId(StableId);

impl TokenSetId {
    /// `StableId`로 만듭니다.
    pub const fn new(id: StableId) -> TokenSetId {
        TokenSetId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// TokenSet 편집 오류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TokenSetError {
    /// 같은 ID의 토큰이 이미 있습니다.
    DuplicateId(TokenRef),
    /// 같은 이름의 토큰이 이미 있습니다(한 set 안에서 이름은 유일).
    DuplicateName {
        /// 중복 이름.
        name: TokenName,
        /// 이미 그 이름을 쓰는 토큰.
        existing: TokenRef,
    },
    /// 토큰이 없습니다.
    UnknownToken(TokenRef),
}

impl fmt::Display for TokenSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenSetError::DuplicateId(id) => write!(f, "토큰 ID가 중복됩니다: {}", id.id()),
            TokenSetError::DuplicateName { name, existing } => {
                write!(f, "토큰 이름 {name}을 이미 {}가 씁니다", existing.id())
            }
            TokenSetError::UnknownToken(id) => write!(f, "토큰이 없습니다: {}", id.id()),
        }
    }
}

impl std::error::Error for TokenSetError {}

/// 토큰 모음 (VIII.5.1). ID 색인과 이름 색인을 함께 유지합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSet {
    id: TokenSetId,
    tokens: BTreeMap<TokenRef, TokenDefinition>,
    names: BTreeMap<TokenName, TokenRef>,
}

impl TokenSet {
    /// 빈 set.
    pub fn new(id: TokenSetId) -> TokenSet {
        TokenSet {
            id,
            tokens: BTreeMap::new(),
            names: BTreeMap::new(),
        }
    }

    /// set ID.
    pub const fn id(&self) -> TokenSetId {
        self.id
    }

    /// 토큰을 추가합니다. ID·이름이 이미 있으면 거부합니다.
    pub fn insert(&mut self, definition: TokenDefinition) -> Result<(), TokenSetError> {
        if self.tokens.contains_key(&definition.id) {
            return Err(TokenSetError::DuplicateId(definition.id));
        }
        if let Some(existing) = self.names.get(&definition.name) {
            return Err(TokenSetError::DuplicateName {
                name: definition.name.clone(),
                existing: *existing,
            });
        }
        self.names.insert(definition.name.clone(), definition.id);
        self.tokens.insert(definition.id, definition);
        Ok(())
    }

    /// 토큰을 제거하고 정의를 돌려줍니다.
    pub fn remove(&mut self, id: TokenRef) -> Option<TokenDefinition> {
        let definition = self.tokens.remove(&id)?;
        self.names.remove(&definition.name);
        Some(definition)
    }

    /// ID로 찾습니다.
    pub fn get(&self, id: TokenRef) -> Option<&TokenDefinition> {
        self.tokens.get(&id)
    }

    /// 이름으로 찾습니다(이 set 안에서만).
    pub fn by_name(&self, name: &str) -> Option<&TokenDefinition> {
        let name = TokenName::new(name).ok()?;
        self.names.get(&name).and_then(|id| self.tokens.get(id))
    }

    /// 이름을 바꾸고 이전 이름을 돌려줍니다. ID와 다른 토큰의 참조는 바뀌지 않습니다.
    pub fn rename(&mut self, id: TokenRef, name: TokenName) -> Result<TokenName, TokenSetError> {
        if !self.tokens.contains_key(&id) {
            return Err(TokenSetError::UnknownToken(id));
        }
        match self.names.get(&name) {
            Some(existing) if *existing == id => return Ok(name),
            Some(existing) => {
                return Err(TokenSetError::DuplicateName {
                    name,
                    existing: *existing,
                })
            }
            None => {}
        }
        let definition = self
            .tokens
            .get_mut(&id)
            .ok_or(TokenSetError::UnknownToken(id))?;
        let previous = definition.set_name(name.clone());
        self.names.remove(&previous);
        self.names.insert(name, id);
        Ok(previous)
    }

    /// default expression을 바꾸고 이전 표현식을 돌려줍니다. 검사는 호출자(command, VIII.5.5)가 합니다.
    pub fn replace_expression(
        &mut self,
        id: TokenRef,
        expression: ExpressionAst,
    ) -> Result<ExpressionAst, TokenSetError> {
        let definition = self
            .tokens
            .get_mut(&id)
            .ok_or(TokenSetError::UnknownToken(id))?;
        Ok(definition.set_default_expression(expression))
    }

    /// 모든 토큰(ID 순).
    pub fn iter(&self) -> impl Iterator<Item = &TokenDefinition> {
        self.tokens.values()
    }

    /// 토큰 수.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// 토큰이 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// `id`를 default expression에서 참조하는 이 set의 토큰(ID 순).
    pub fn referrers(&self, id: TokenRef) -> BTreeSet<TokenRef> {
        self.tokens
            .values()
            .filter(|definition| definition.referenced_tokens().contains(&id))
            .map(TokenDefinition::id)
            .collect()
    }
}

/// 토큰 타입 + 그 밖의 참조 타입을 함께 제공하는 type 검사 입력.
///
/// 토큰 참조는 `tokens`의 토큰 타입(`Required`)으로, 나머지(환경 입력·속성 등)는 `others`로 해석합니다.
pub struct TokenReferenceTypes<'a, T: TokenTypes + ?Sized> {
    /// 토큰 타입 제공자.
    pub tokens: &'a T,
    /// 토큰이 아닌 참조의 타입 제공자.
    pub others: &'a dyn ReferenceTypes,
}

/// 토큰 ID → 토큰 타입 제공자.
pub trait TokenTypes {
    /// 토큰 타입. 모르는 토큰이면 `None`.
    fn token_type_of(&self, id: TokenRef) -> Option<TokenType>;
}

impl TokenTypes for TokenSet {
    fn token_type_of(&self, id: TokenRef) -> Option<TokenType> {
        self.get(id).map(TokenDefinition::token_type)
    }
}

impl<T: TokenTypes + ?Sized> ReferenceTypes for TokenReferenceTypes<'_, T> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        match target {
            ReferenceTarget::Token(id) => self.tokens.token_type_of(*id).map(|t| t.type_spec()),
            other => self.others.reference_type(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use stellar_core_types::typed_value::TypedValue;
    use stellar_core_types::value::{FiniteF32, Length};
    use stellar_expressions::ast::{ExprNode, ExprNodeId};
    use stellar_expressions::bytecode::Program;
    use stellar_expressions::ops::BinaryOp;

    use super::*;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn token(n: u64) -> TokenRef {
        TokenRef::new(sid(1_000 + n))
    }

    fn node(n: u64) -> ExprNodeId {
        ExprNodeId::new(sid(n))
    }

    fn len(v: f32) -> TypedValue {
        TypedValue::Length(Length::new(v).expect("유한"))
    }

    fn name(s: &str) -> TokenName {
        TokenName::new(s).expect("유효 이름")
    }

    fn literal(n: u64, value: TypedValue) -> ExpressionAst {
        ExpressionAst::new(node(n), vec![ExprNode::literal(node(n), value)]).expect("유효")
    }

    /// `token(base) × 2`.
    fn doubled(base: TokenRef, n: u64) -> ExpressionAst {
        ExpressionAst::new(
            node(n),
            vec![
                ExprNode::binary(node(n), BinaryOp::Multiply, node(n + 1), node(n + 2)),
                ExprNode::reference(node(n + 1), ReferenceTarget::Token(base)),
                ExprNode::literal(
                    node(n + 2),
                    TypedValue::Number(FiniteF32::new(2.0).expect("유한")),
                ),
            ],
        )
        .expect("유효")
    }

    fn length_type() -> TokenType {
        TokenType::new(ExpressionType::Length)
    }

    fn fixture() -> TokenSet {
        let mut set = TokenSet::new(TokenSetId::new(sid(1)));
        set.insert(TokenDefinition::new(
            token(1),
            name("spacing.base"),
            length_type(),
            literal(10, len(8.0)),
        ))
        .expect("추가");
        set.insert(TokenDefinition::new(
            token(2),
            name("spacing.double"),
            length_type(),
            doubled(token(1), 20),
        ))
        .expect("추가");
        set
    }

    fn evaluate(set: &TokenSet, id: TokenRef, base_value: TypedValue) -> Option<TypedValue> {
        let others: BTreeMap<ReferenceTarget, TypeSpec> = BTreeMap::new();
        let types = TokenReferenceTypes {
            tokens: set,
            others: &others,
        };
        let definition = set.get(id).expect("토큰");
        let program = Program::compile(definition.default_expression(), &types).expect("컴파일");
        let values = BTreeMap::from([(ReferenceTarget::Token(token(1)), Some(base_value))]);
        program.evaluate(&values).expect("평가")
    }

    #[test]
    fn references_survive_rename() {
        let mut set = fixture();
        let before = set
            .get(token(2))
            .expect("토큰")
            .default_expression()
            .to_canonical_bytes()
            .expect("직렬화");
        assert_eq!(evaluate(&set, token(2), len(8.0)), Some(len(16.0)));

        let previous = set.rename(token(1), name("space.unit")).expect("rename");
        assert_eq!(previous, name("spacing.base"));
        // 참조하는 토큰의 표현식은 bytes까지 그대로이고, 같은 ID를 계속 가리킵니다.
        let referrer = set.get(token(2)).expect("토큰");
        assert_eq!(
            referrer
                .default_expression()
                .to_canonical_bytes()
                .expect("직렬화"),
            before
        );
        assert_eq!(referrer.referenced_tokens(), BTreeSet::from([token(1)]));
        assert_eq!(set.referrers(token(1)), BTreeSet::from([token(2)]));
        // 새 이름으로 찾고, 옛 이름은 비고, 평가 결과는 같습니다.
        assert_eq!(
            set.by_name("space.unit").map(TokenDefinition::id),
            Some(token(1))
        );
        assert!(set.by_name("spacing.base").is_none());
        assert_eq!(evaluate(&set, token(2), len(8.0)), Some(len(16.0)));
        // type 검사도 이름과 무관합니다.
        let others: BTreeMap<ReferenceTarget, TypeSpec> = BTreeMap::new();
        let types = TokenReferenceTypes {
            tokens: &set,
            others: &others,
        };
        assert_eq!(referrer.type_check(&types), Ok(()));
    }

    #[test]
    fn names_are_validated_and_unique_within_a_set() {
        assert_eq!(TokenName::new(""), Err(TokenNameError::Empty));
        assert_eq!(
            TokenName::new(" padded"),
            Err(TokenNameError::SurroundingWhitespace)
        );
        assert_eq!(
            TokenName::new("tab\there"),
            Err(TokenNameError::ControlCharacter)
        );
        assert_eq!(
            TokenName::new(&"a".repeat(257)),
            Err(TokenNameError::TooLong { len: 257 })
        );
        assert!(TokenName::new("색상.기본 / primary").is_ok());

        let mut set = fixture();
        assert_eq!(
            set.insert(TokenDefinition::new(
                token(3),
                name("spacing.base"),
                length_type(),
                literal(30, len(1.0)),
            )),
            Err(TokenSetError::DuplicateName {
                name: name("spacing.base"),
                existing: token(1)
            })
        );
        assert_eq!(
            set.insert(TokenDefinition::new(
                token(1),
                name("other"),
                length_type(),
                literal(31, len(1.0)),
            )),
            Err(TokenSetError::DuplicateId(token(1)))
        );
        assert!(matches!(
            set.rename(token(2), name("spacing.base")),
            Err(TokenSetError::DuplicateName { .. })
        ));
        assert_eq!(
            set.rename(token(9), name("x")),
            Err(TokenSetError::UnknownToken(token(9)))
        );
        // 같은 이름으로 rename은 변화 없음.
        assert_eq!(
            set.rename(token(1), name("spacing.base")),
            Ok(name("spacing.base"))
        );
        let removed = set.remove(token(1)).expect("제거");
        assert_eq!(removed.name(), &name("spacing.base"));
        assert!(set.by_name("spacing.base").is_none(), "이름 색인도 제거");
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn default_expression_type_must_match_token_type() {
        let set = fixture();
        let others: BTreeMap<ReferenceTarget, TypeSpec> = BTreeMap::new();
        let types = TokenReferenceTypes {
            tokens: &set,
            others: &others,
        };
        // Number 리터럴을 Length 토큰의 default로.
        let wrong = TokenDefinition::new(
            token(3),
            name("wrong"),
            length_type(),
            literal(40, TypedValue::Number(FiniteF32::ONE)),
        );
        assert_eq!(
            wrong.type_check(&types),
            Err(TokenTypeError::ResultMismatch {
                expected: TypeSpec::required(ExpressionType::Length),
                actual: TypeSpec::required(ExpressionType::Number),
            })
        );
        // nullable 결과는 토큰 타입이 될 수 없습니다.
        let nullable = TokenDefinition::new(
            token(4),
            name("nullable"),
            length_type(),
            ExpressionAst::new(
                node(50),
                vec![ExprNode::null(node(50), ExpressionType::Length)],
            )
            .expect("유효"),
        );
        assert!(matches!(
            nullable.type_check(&types),
            Err(TokenTypeError::ResultMismatch { .. })
        ));
        // 모르는 토큰 참조는 표현식 오류(오류 node 포함).
        let dangling = TokenDefinition::new(
            token(5),
            name("dangling"),
            length_type(),
            doubled(token(99), 60),
        );
        match dangling.type_check(&types) {
            Err(TokenTypeError::Expression(errors)) => {
                assert_eq!(errors.len(), 1);
                assert_eq!(errors[0].node, node(61));
            }
            other => panic!("표현식 오류 기대: {other:?}"),
        }
        assert_eq!(set.get(token(2)).expect("토큰").type_check(&types), Ok(()));
    }
}
