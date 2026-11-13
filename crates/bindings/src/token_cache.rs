//! 토큰 값 평가 cache (VIII.5.4).
//!
//! 토큰 값은 요청 시(pull) 계산하고 결과를 cache합니다. 각 cache 항목은 계산 당시의
//!
//! - 토큰 값 revision([`TokenRegistry::value_revision`]),
//! - 참조한 토큰마다 그 토큰 값이 마지막으로 **바뀐** 시점(`changed_at`),
//! - 토큰이 아닌 입력(환경 입력 등)마다 읽은 값
//!
//! 을 기록합니다. 다음 요청에서는 dependency를 먼저 확인한 뒤 기록과 모두 같으면 평가하지 않고 재사용합니다.
//! 따라서 무관한 토큰을 바꾸면 그 토큰에 의존하지 않는 토큰은 다시 평가하지 않습니다. 다시 평가한 결과가 이전과
//! 같으면 `changed_at`을 유지해(early cutoff) 그 위의 토큰도 다시 평가하지 않습니다.
//!
//! 확인·평가는 명시적 stack으로 재귀 없이 진행합니다. registry에 cycle이 있으면(저장 전 검증 VIII.5.3을 거치지 않은
//! 메모리 상태) 그 cycle 위의 토큰은 모두 [`TokenEvalError::Cycle`]이 되며 무한 반복하지 않습니다. 실패한 토큰을
//! 참조하는 토큰은 short-circuit(VIII.4.3) 의미대로 실패 토큰을 실제로 읽을 때만 [`TokenEvalError::Dependency`]입니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::reference::TokenRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::ast::ReferenceTarget;
use stellar_expressions::bytecode::{CompileError, Program};
use stellar_expressions::eval::{EvalError, EvalErrorKind, ReferenceValues};
use stellar_expressions::typecheck::ReferenceTypes;
use stellar_expressions::types::TypeSpec;
use stellar_expressions::vm::EvalLimits;

use crate::registry::TokenRegistry;
use crate::token::TokenReferenceTypes;
use crate::token_graph::TokenCycle;

/// 토큰이 아닌 참조 입력(환경 입력·속성 등)의 타입과 현재 값.
pub trait TokenInputs {
    /// 입력 타입. 모르는 대상이면 `None`.
    fn input_type(&self, target: &ReferenceTarget) -> Option<TypeSpec>;
    /// 입력 값. `None` = 모르는 대상, `Some(None)` = 값 없음.
    fn input_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>>;
}

impl TokenInputs for BTreeMap<ReferenceTarget, (TypeSpec, Option<TypedValue>)> {
    fn input_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        self.get(target).map(|(spec, _)| *spec)
    }

    fn input_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>> {
        self.get(target).map(|(_, value)| value.clone())
    }
}

/// 토큰 평가 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenEvalError {
    /// registry에 없는 토큰.
    UnknownToken(TokenRef),
    /// 토큰 참조 cycle.
    Cycle(TokenCycle),
    /// default expression 컴파일 실패(type 오류).
    Compile(CompileError),
    /// 평가 실패.
    Eval(EvalError),
    /// 참조한 토큰의 평가가 실패했습니다.
    Dependency(TokenRef),
}

impl fmt::Display for TokenEvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenEvalError::UnknownToken(token) => write!(f, "토큰이 없습니다: {}", token.id()),
            TokenEvalError::Cycle(cycle) => write!(f, "{cycle}"),
            TokenEvalError::Compile(error) => write!(f, "{error}"),
            TokenEvalError::Eval(error) => write!(f, "{error}"),
            TokenEvalError::Dependency(token) => {
                write!(f, "참조한 토큰 {}의 평가가 실패했습니다", token.id())
            }
        }
    }
}

impl std::error::Error for TokenEvalError {}

/// 토큰 평가 결과.
pub type TokenValue = Result<Option<TypedValue>, TokenEvalError>;

/// cache 동작 통계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheStats {
    /// default expression을 다시 평가한 횟수.
    pub evaluations: u64,
    /// dependency 확인만으로 재사용한 횟수.
    pub reuses: u64,
}

/// 없는 토큰 dependency의 표식(`changed_at` 자리). 토큰이 생기면 실제 시점과 달라 무효화됩니다.
const MISSING: u64 = u64::MAX;

#[derive(Debug, Clone)]
struct Entry {
    revision: u64,
    value: TokenValue,
    changed_at: u64,
    token_deps: Vec<(TokenRef, u64)>,
    input_deps: Vec<(ReferenceTarget, Option<Option<TypedValue>>)>,
    verified_at: u64,
}

/// 토큰 값 평가 cache (VIII.5.4).
#[derive(Debug, Clone)]
pub struct TokenValueCache {
    entries: BTreeMap<TokenRef, Entry>,
    clock: u64,
    session: u64,
    stats: CacheStats,
    limits: EvalLimits,
}

impl Default for TokenValueCache {
    fn default() -> TokenValueCache {
        TokenValueCache::new()
    }
}

/// 토큰 참조는 이번 session에서 확인한 cache 값, 그 밖은 입력으로 푸는 값 제공자. 실패했거나 확인되지 않은
/// 토큰은 "모르는 대상"으로 보여 읽는 순간 평가 오류가 됩니다.
struct CachedValues<'a> {
    entries: &'a BTreeMap<TokenRef, Entry>,
    session: u64,
    inputs: &'a dyn TokenInputs,
}

impl ReferenceValues for CachedValues<'_> {
    fn reference_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>> {
        match target {
            ReferenceTarget::Token(token) => self
                .entries
                .get(token)
                .filter(|entry| entry.verified_at == self.session)
                .and_then(|entry| entry.value.as_ref().ok().cloned()),
            other => self.inputs.input_value(other),
        }
    }
}

/// 입력 타입을 `ReferenceTypes`로 보는 adapter.
struct InputTypes<'a>(&'a dyn TokenInputs);

impl ReferenceTypes for InputTypes<'_> {
    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        self.0.input_type(target)
    }
}

enum Step {
    Enter(TokenRef),
    Exit(TokenRef),
}

impl TokenValueCache {
    /// 빈 cache(기본 평가 한도).
    pub fn new() -> TokenValueCache {
        TokenValueCache::with_limits(EvalLimits::DEFAULT)
    }

    /// 평가 한도를 지정한 빈 cache.
    pub fn with_limits(limits: EvalLimits) -> TokenValueCache {
        TokenValueCache {
            entries: BTreeMap::new(),
            clock: 0,
            session: 0,
            stats: CacheStats::default(),
            limits,
        }
    }

    /// 누적 통계.
    pub const fn stats(&self) -> CacheStats {
        self.stats
    }

    /// cache 항목 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// cache가 비어 있으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 모든 항목을 버립니다.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// 토큰 값을 구합니다. dependency가 바뀐 토큰만 다시 평가합니다.
    pub fn get(
        &mut self,
        token: TokenRef,
        registry: &TokenRegistry,
        inputs: &dyn TokenInputs,
    ) -> TokenValue {
        self.session += 1;
        self.refresh(token, registry, inputs);
        self.current(token)
    }

    /// registry의 모든 토큰 값을 한 번의 확인으로 구합니다(ID 순).
    pub fn get_all(
        &mut self,
        registry: &TokenRegistry,
        inputs: &dyn TokenInputs,
    ) -> BTreeMap<TokenRef, TokenValue> {
        self.session += 1;
        let tokens: Vec<TokenRef> = registry.tokens().map(|(_, d)| d.id()).collect();
        for token in &tokens {
            self.refresh(*token, registry, inputs);
        }
        let mut out = BTreeMap::new();
        for token in tokens {
            out.insert(token, self.current(token));
        }
        out
    }

    fn current(&self, token: TokenRef) -> TokenValue {
        match self.entries.get(&token) {
            Some(entry) if entry.verified_at == self.session => entry.value.clone(),
            _ => Err(TokenEvalError::UnknownToken(token)),
        }
    }

    fn verified(&self, token: TokenRef) -> bool {
        self.entries
            .get(&token)
            .is_some_and(|entry| entry.verified_at == self.session)
    }

    /// `root`와 그 dependency를 이번 session에서 확인·평가합니다(명시적 stack).
    fn refresh(&mut self, root: TokenRef, registry: &TokenRegistry, inputs: &dyn TokenInputs) {
        let mut stack = vec![Step::Enter(root)];
        let mut open: BTreeSet<TokenRef> = BTreeSet::new();
        let mut path: Vec<TokenRef> = Vec::new();
        let mut cycle: Option<TokenCycle> = None;
        while let Some(step) = stack.pop() {
            match step {
                Step::Enter(token) => {
                    if self.verified(token) {
                        continue;
                    }
                    if open.contains(&token) {
                        let start = path.iter().position(|t| *t == token).unwrap_or(0);
                        let mut cycle_path = path[start..].to_vec();
                        cycle_path.push(token);
                        cycle.get_or_insert(TokenCycle { path: cycle_path });
                        continue;
                    }
                    open.insert(token);
                    path.push(token);
                    stack.push(Step::Exit(token));
                    if let Some(resolved) = registry.resolve(token) {
                        let deps: Vec<TokenRef> = resolved
                            .definition
                            .referenced_tokens()
                            .into_iter()
                            .collect();
                        stack.extend(deps.into_iter().rev().map(Step::Enter));
                    }
                }
                Step::Exit(token) => {
                    open.remove(&token);
                    path.pop();
                    self.settle(token, registry, inputs, cycle.as_ref());
                }
            }
        }
    }

    /// dependency 확인이 끝난 `token`을 재사용하거나 다시 평가합니다.
    fn settle(
        &mut self,
        token: TokenRef,
        registry: &TokenRegistry,
        inputs: &dyn TokenInputs,
        cycle: Option<&TokenCycle>,
    ) {
        let (Some(resolved), Some(revision)) =
            (registry.resolve(token), registry.value_revision(token))
        else {
            self.entries.remove(&token);
            return;
        };
        let definition = resolved.definition;
        let mut token_deps = Vec::new();
        let mut cyclic = false;
        for dep in definition.referenced_tokens() {
            let stamp = match self.entries.get(&dep) {
                Some(entry) if entry.verified_at == self.session => entry.changed_at,
                _ if registry.scope_of(dep).is_none() => MISSING,
                // registry에 있는데 확인되지 않았다면 아직 열린(진행 중) 토큰 = cycle.
                _ => {
                    cyclic = true;
                    MISSING
                }
            };
            token_deps.push((dep, stamp));
        }
        let input_deps: Vec<(ReferenceTarget, Option<Option<TypedValue>>)> = definition
            .references()
            .into_iter()
            .filter(|target| !matches!(target, ReferenceTarget::Token(_)))
            .map(|target| {
                let value = inputs.input_value(&target);
                (target, value)
            })
            .collect();
        if !cyclic {
            if let Some(entry) = self.entries.get_mut(&token) {
                if entry.revision == revision
                    && entry.token_deps == token_deps
                    && entry.input_deps == input_deps
                {
                    entry.verified_at = self.session;
                    self.stats.reuses += 1;
                    return;
                }
            }
        }
        let value = if cyclic {
            Err(TokenEvalError::Cycle(cycle.cloned().unwrap_or(
                TokenCycle {
                    path: vec![token, token],
                },
            )))
        } else if let Some(on_cycle) = token_deps.iter().find_map(|(dep, _)| {
            match self.entries.get(dep).map(|entry| &entry.value) {
                Some(Err(TokenEvalError::Cycle(cycle))) if cycle.path.contains(&token) => {
                    Some(cycle.clone())
                }
                _ => None,
            }
        }) {
            // 같은 cycle 위의 토큰은 선택 branch와 무관하게 모두 cycle 오류입니다(요청 순서와 무관한 결과).
            Err(TokenEvalError::Cycle(on_cycle))
        } else {
            self.evaluate(definition.default_expression(), registry, inputs)
        };
        self.stats.evaluations += 1;
        let changed_at = match self.entries.get(&token) {
            Some(previous) if previous.value == value => previous.changed_at,
            _ => {
                self.clock += 1;
                self.clock
            }
        };
        self.entries.insert(
            token,
            Entry {
                revision,
                value,
                changed_at,
                token_deps,
                input_deps,
                verified_at: self.session,
            },
        );
    }

    fn evaluate(
        &self,
        expression: &stellar_expressions::ast::ExpressionAst,
        registry: &TokenRegistry,
        inputs: &dyn TokenInputs,
    ) -> TokenValue {
        let others = InputTypes(inputs);
        let types = TokenReferenceTypes {
            tokens: registry,
            others: &others,
        };
        let program = Program::compile(expression, &types).map_err(TokenEvalError::Compile)?;
        let values = CachedValues {
            entries: &self.entries,
            session: self.session,
            inputs,
        };
        program
            .evaluate_with_limits(&values, self.limits)
            .map(|(value, _)| value)
            .map_err(|error| match &error.kind {
                // 실제로 읽은 토큰이 실패한 경우(선택되지 않은 branch의 실패 토큰은 읽지 않습니다).
                EvalErrorKind::MissingReference(ReferenceTarget::Token(dep))
                    if self.entries.get(dep).is_some_and(|entry| {
                        entry.verified_at == self.session && entry.value.is_err()
                    }) =>
                {
                    TokenEvalError::Dependency(*dep)
                }
                _ => TokenEvalError::Eval(error),
            })
    }
}

#[cfg(test)]
mod tests {
    use stellar_core_types::ids::StableId;
    use stellar_core_types::value::Length;
    use stellar_expressions::ast::{EnvironmentKey, ExprNode, ExprNodeId, ExpressionAst};
    use stellar_expressions::ops::BinaryOp;
    use stellar_expressions::types::ExpressionType;

    use super::*;
    use crate::registry::TokenScope;
    use crate::token::{TokenDefinition, TokenName, TokenSet, TokenSetId, TokenType};

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

    fn dark() -> ReferenceTarget {
        ReferenceTarget::Environment(EnvironmentKey::new(4))
    }

    fn literal(base: u64, value: TypedValue) -> ExpressionAst {
        ExpressionAst::new(node(base), vec![ExprNode::literal(node(base), value)]).expect("유효")
    }

    /// `token(a) + token(b)`.
    fn sum(base: u64, a: TokenRef, b: TokenRef) -> ExpressionAst {
        ExpressionAst::new(
            node(base),
            vec![
                ExprNode::binary(node(base), BinaryOp::Add, node(base + 1), node(base + 2)),
                ExprNode::reference(node(base + 1), ReferenceTarget::Token(a)),
                ExprNode::reference(node(base + 2), ReferenceTarget::Token(b)),
            ],
        )
        .expect("유효")
    }

    /// `if dark then token(a) else value`.
    fn themed(base: u64, a: TokenRef, value: TypedValue) -> ExpressionAst {
        ExpressionAst::new(
            node(base),
            vec![
                ExprNode::if_else(node(base), node(base + 1), node(base + 2), node(base + 3)),
                ExprNode::reference(node(base + 1), dark()),
                ExprNode::reference(node(base + 2), ReferenceTarget::Token(a)),
                ExprNode::literal(node(base + 3), value),
            ],
        )
        .expect("유효")
    }

    fn define(n: u64, expression: ExpressionAst) -> TokenDefinition {
        TokenDefinition::new(
            token(n),
            TokenName::new(&format!("t{n}")).expect("이름"),
            TokenType::new(ExpressionType::Length),
            expression,
        )
    }

    /// A=8, B=A+A, C=3, D=C+C, E=if dark then A else 1.
    fn fixture() -> TokenRegistry {
        let mut set = TokenSet::new(TokenSetId::new(sid(1)));
        set.insert(define(1, literal(100, len(8.0)))).expect("A");
        set.insert(define(2, sum(200, token(1), token(1))))
            .expect("B");
        set.insert(define(3, literal(300, len(3.0)))).expect("C");
        set.insert(define(4, sum(400, token(3), token(3))))
            .expect("D");
        set.insert(define(5, themed(500, token(1), len(1.0))))
            .expect("E");
        let mut registry = TokenRegistry::new();
        registry.add_scope(TokenScope::Project, set).expect("scope");
        registry
    }

    type Inputs = BTreeMap<ReferenceTarget, (TypeSpec, Option<TypedValue>)>;

    fn inputs(is_dark: bool) -> Inputs {
        BTreeMap::from([(
            dark(),
            (
                TypeSpec::required(ExpressionType::Boolean),
                Some(TypedValue::Boolean(is_dark)),
            ),
        )])
    }

    #[test]
    fn unrelated_token_change_evaluates_nothing() {
        let mut registry = fixture();
        let mut cache = TokenValueCache::new();
        let light = inputs(false);
        assert_eq!(cache.get(token(2), &registry, &light), Ok(Some(len(16.0))));
        assert_eq!(cache.get(token(4), &registry, &light), Ok(Some(len(6.0))));
        assert_eq!(cache.stats().evaluations, 4, "A, B, C, D");

        // C를 바꿔도 B(A에만 의존)는 평가하지 않습니다.
        registry
            .replace_expression(token(3), literal(301, len(5.0)))
            .expect("교체");
        let before = cache.stats().evaluations;
        assert_eq!(cache.get(token(2), &registry, &light), Ok(Some(len(16.0))));
        assert_eq!(
            cache.stats().evaluations - before,
            0,
            "무관 token 변경 시 평가 0"
        );
        // D는 C와 D만 다시 평가합니다.
        assert_eq!(cache.get(token(4), &registry, &light), Ok(Some(len(10.0))));
        assert_eq!(cache.stats().evaluations - before, 2);
        // 변경 없이 다시 요청하면 평가 0.
        let before = cache.stats().evaluations;
        let all = cache.get_all(&registry, &light);
        assert_eq!(all.len(), 5);
        assert_eq!(cache.stats().evaluations - before, 1, "처음 요청한 E만");
        let before = cache.stats().evaluations;
        cache.get_all(&registry, &light);
        assert_eq!(cache.stats().evaluations, before);
    }

    #[test]
    fn rename_keeps_and_equal_result_cuts_off() {
        let mut registry = fixture();
        let mut cache = TokenValueCache::new();
        let light = inputs(false);
        cache.get_all(&registry, &light);
        // rename은 값 revision을 바꾸지 않습니다.
        let revision = registry.value_revision(token(1));
        registry
            .rename(token(1), TokenName::new("spacing").expect("이름"))
            .expect("rename");
        assert_eq!(registry.value_revision(token(1)), revision);
        let before = cache.stats().evaluations;
        cache.get_all(&registry, &light);
        assert_eq!(cache.stats().evaluations, before, "rename 후 평가 0");
        // 같은 값(8px)이 나오는 새 표현식: A만 다시 평가하고 B·E는 재사용(early cutoff).
        registry
            .replace_expression(token(1), sum(110, token(3), token(3)))
            .expect("교체");
        registry
            .replace_expression(token(3), literal(310, len(4.0)))
            .expect("교체");
        let before = cache.stats().evaluations;
        assert_eq!(cache.get(token(2), &registry, &light), Ok(Some(len(16.0))));
        assert_eq!(cache.stats().evaluations - before, 2, "C, A만");
    }

    #[test]
    fn environment_input_change_invalidates_only_readers() {
        let registry = fixture();
        let mut cache = TokenValueCache::new();
        assert_eq!(
            cache.get(token(5), &registry, &inputs(false)),
            Ok(Some(len(1.0)))
        );
        cache.get_all(&registry, &inputs(false));
        let before = cache.stats().evaluations;
        let all = cache.get_all(&registry, &inputs(true));
        assert_eq!(all.get(&token(5)), Some(&Ok(Some(len(8.0)))));
        assert_eq!(cache.stats().evaluations - before, 1, "dark를 읽는 E만");
    }

    #[test]
    fn errors_cycles_and_missing_tokens_are_reported() {
        let mut registry = fixture();
        let mut cache = TokenValueCache::new();
        let light = inputs(false);
        // 메모리 상태의 cycle(저장 전 검증을 거치지 않음): A = B + B, B = A + A.
        registry
            .replace_expression(token(1), sum(120, token(2), token(2)))
            .expect("교체");
        let cycle = |path: Vec<TokenRef>| Err(TokenEvalError::Cycle(TokenCycle { path }));
        assert_eq!(
            cache.get(token(2), &registry, &light),
            cycle(vec![token(2), token(1), token(2)])
        );
        assert_eq!(
            cache.get(token(1), &registry, &light),
            cycle(vec![token(1), token(2), token(1)])
        );
        // cycle 밖에서 cycle 토큰을 참조하는 E: 선택되지 않은 branch면 정상, 읽으면 dependency 실패.
        assert_eq!(cache.get(token(5), &registry, &light), Ok(Some(len(1.0))));
        assert_eq!(
            cache.get(token(5), &registry, &inputs(true)),
            Err(TokenEvalError::Dependency(token(1)))
        );
        // cycle을 풀면 다시 정상.
        registry
            .replace_expression(token(1), literal(130, len(2.0)))
            .expect("교체");
        assert_eq!(cache.get(token(2), &registry, &light), Ok(Some(len(4.0))));
        // 없는 토큰과 dependency 실패.
        assert_eq!(
            cache.get(token(99), &registry, &light),
            Err(TokenEvalError::UnknownToken(token(99)))
        );
        registry.remove(token(3)).expect("제거");
        assert!(matches!(
            cache.get(token(4), &registry, &light),
            Err(TokenEvalError::Compile(_))
        ));
        // 입력이 없으면 E는 평가 오류.
        assert!(matches!(
            cache.get(token(5), &registry, &Inputs::new()),
            Err(TokenEvalError::Compile(_))
        ));
        // 토큰을 다시 추가하면 D가 무효화되어 다시 계산됩니다.
        registry
            .insert(TokenScope::Project, define(3, literal(320, len(7.0))))
            .expect("추가");
        assert_eq!(cache.get(token(4), &registry, &light), Ok(Some(len(14.0))));
    }
}
