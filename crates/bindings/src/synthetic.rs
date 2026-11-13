//! 대규모 synthetic token/binding graph 생성기 (VIII.9.1).
//!
//! 성능·결정론 검증용 graph를 고정 seed(SplitMix64)로 만듭니다. 같은 설정·seed는 항상 같은 graph(같은
//! [`SyntheticGraph::digest`])를 만듭니다.
//!
//! - **구조**: 토큰 `i`는 앞선 토큰(`0..i`)만, binding `j`는 토큰·앞선 binding의 target 속성(`0..j`)·환경 입력만
//!   참조하므로 구성상 비순환입니다. 한 producer의 참조 대상은 서로 다릅니다(중복 간선 없음).
//! - **간선 수**: 토큰 참조 수를 먼저 정하고, 남은 간선을 binding에 고르게 나눠 전체 간선 수가 설정값과 정확히
//!   같게 합니다.
//! - **preset**: [`SyntheticConfig::EDGES_1M`](토큰·binding 간선 1,000,000개)과 활성 document 한도 fixture
//!   [`SyntheticConfig::ACTIVE_DOCUMENT_200K`](node 200,000개: 토큰 + binding target 속성 + 환경 입력)를 따로 둡니다.
//!
//! [`SyntheticGraph::materialize`]는 실제 [`TokenRegistry`]·[`BindingSet`]로 만듭니다. 모든 값은 Number이며 표현식은
//! 참조들의 `max`(참조 없으면 상수)라 깊은 graph에서도 유한 범위를 벗어나지 않습니다. 환경 입력은 Number 타입 key
//! (`os_version`, `api_level`, `density`)만 씁니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::{NodeRef, TokenRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;
use stellar_expressions::ast::{
    AstError, EnvironmentKey, ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget,
};
use stellar_expressions::environment::{API_LEVEL, DENSITY, OS_VERSION};
use stellar_expressions::functions::FunctionId;
use stellar_expressions::types::{ExpressionType, TypeSpec};

use crate::binding::{BindingId, BindingSet, BindingSetError, ScopeOwner, ValueBinding};
use crate::registry::{RegistryError, TokenRegistry, TokenScope};
use crate::token::{
    TokenDefinition, TokenName, TokenNameError, TokenSet, TokenSetError, TokenSetId, TokenType,
};

/// synthetic graph에서 쓰는 Number 환경 key.
pub const SYNTHETIC_ENVIRONMENT: [EnvironmentKey; 3] = [OS_VERSION, API_LEVEL, DENSITY];

/// 생성 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyntheticConfig {
    /// 고정 seed.
    pub seed: u64,
    /// 토큰 수.
    pub tokens: u32,
    /// binding 수(= binding target 속성 node 수).
    pub bindings: u32,
    /// 전체 간선 수(토큰 + binding 참조).
    pub edges: u64,
    /// 토큰 하나가 참조하는 앞선 토큰 수 상한.
    pub max_token_refs: u32,
    /// binding 참조 중 환경 입력을 넣을 확률(1/n, 0이면 없음).
    pub environment_one_in: u32,
}

impl SyntheticConfig {
    /// 토큰·binding 간선 1,000,000개(토큰 100,000 + binding 200,000).
    pub const EDGES_1M: SyntheticConfig = SyntheticConfig {
        seed: 0x5354_454C_4C41_5231,
        tokens: 100_000,
        bindings: 200_000,
        edges: 1_000_000,
        max_token_refs: 4,
        environment_one_in: 16,
    };

    /// 활성 document 한도 fixture: node 200,000개(토큰 20,000 + binding 179,997 + 환경 입력 3).
    pub const ACTIVE_DOCUMENT_200K: SyntheticConfig = SyntheticConfig {
        seed: 0x5354_454C_4C41_5232,
        tokens: 20_000,
        bindings: 179_997,
        edges: 500_000,
        max_token_refs: 3,
        environment_one_in: 16,
    };
}

/// 설정 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntheticError {
    /// 남은 간선을 binding에 나누면 binding 하나가 참조할 수 있는 대상 수를 넘습니다.
    TooManyEdges {
        /// binding 하나에 필요한 최대 참조 수.
        per_binding: u64,
    },
    /// binding이 없는데 남은 간선이 있습니다.
    NoBindings,
}

impl fmt::Display for SyntheticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyntheticError::TooManyEdges { per_binding } => {
                write!(
                    f,
                    "binding 하나에 참조 {per_binding}개가 필요해 대상이 모자랍니다"
                )
            }
            SyntheticError::NoBindings => {
                f.write_str("binding 없이 남은 간선을 배치할 수 없습니다")
            }
        }
    }
}

impl std::error::Error for SyntheticError {}

/// 참조 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SyntheticRef {
    /// 토큰 번호.
    Token(u32),
    /// binding 번호의 target 속성.
    Property(u32),
    /// [`SYNTHETIC_ENVIRONMENT`] 번호.
    Environment(u8),
}

/// producer(토큰 또는 binding) 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SyntheticProducer {
    /// 참조 대상(서로 다름, 생성 순서).
    pub refs: Vec<SyntheticRef>,
    /// 참조가 없을 때의 상수(정수 0~999).
    pub constant: u16,
}

/// 생성된 graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SyntheticGraph {
    /// 생성 설정.
    pub config: SyntheticConfig,
    /// 토큰(번호 순).
    pub tokens: Vec<SyntheticProducer>,
    /// binding(번호 순).
    pub bindings: Vec<SyntheticProducer>,
}

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..n`(n > 0).
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// 서로 다른 대상 `count`개를 `pick`으로 뽑습니다(작은 count 전용 거절 표본).
fn distinct(
    count: usize,
    rng: &mut Rng,
    mut pick: impl FnMut(&mut Rng) -> SyntheticRef,
) -> Vec<SyntheticRef> {
    let mut out: Vec<SyntheticRef> = Vec::with_capacity(count);
    while out.len() < count {
        let candidate = pick(rng);
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// 설정으로 graph를 만듭니다.
pub fn generate(config: SyntheticConfig) -> Result<SyntheticGraph, SyntheticError> {
    let mut rng = Rng(config.seed);
    let mut tokens = Vec::with_capacity(config.tokens as usize);
    let mut token_edges: u64 = 0;
    for i in 0..config.tokens {
        let wanted = rng.below(u64::from(config.max_token_refs) + 1) as u32;
        let count = wanted.min(i) as usize;
        let refs = distinct(count, &mut rng, |rng| {
            SyntheticRef::Token(rng.below(u64::from(i)) as u32)
        });
        token_edges += refs.len() as u64;
        tokens.push(SyntheticProducer {
            refs,
            constant: rng.below(1_000) as u16,
        });
    }
    let remaining = config.edges.saturating_sub(token_edges);
    let binding_count = u64::from(config.bindings);
    if binding_count == 0 && remaining > 0 {
        return Err(SyntheticError::NoBindings);
    }
    let base = remaining.checked_div(binding_count).unwrap_or(0);
    let extra = remaining.checked_rem(binding_count).unwrap_or(0);
    // binding 하나가 참조할 수 있는 최소 대상 수(토큰 + 환경 입력)의 절반 이하로 제한해 표본이 빨리 끝나게 합니다.
    let capacity = (u64::from(config.tokens) + SYNTHETIC_ENVIRONMENT.len() as u64) / 2;
    if base + u64::from(extra > 0) > capacity.max(1) && remaining > 0 {
        return Err(SyntheticError::TooManyEdges {
            per_binding: base + u64::from(extra > 0),
        });
    }
    let mut bindings = Vec::with_capacity(config.bindings as usize);
    for j in 0..config.bindings {
        let count = (base + u64::from(u64::from(j) < extra)) as usize;
        let refs = distinct(count, &mut rng, |rng| {
            let roll = rng.below(16);
            if config.environment_one_in > 0 && rng.below(u64::from(config.environment_one_in)) == 0
            {
                SyntheticRef::Environment(rng.below(SYNTHETIC_ENVIRONMENT.len() as u64) as u8)
            } else if j > 0 && roll < 4 {
                SyntheticRef::Property(rng.below(u64::from(j)) as u32)
            } else {
                SyntheticRef::Token(rng.below(u64::from(config.tokens.max(1))) as u32)
            }
        });
        bindings.push(SyntheticProducer {
            refs,
            constant: rng.below(1_000) as u16,
        });
    }
    Ok(SyntheticGraph {
        config,
        tokens,
        bindings,
    })
}

/// FNV-1a 64.
struct Fnv(u64);

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
}

impl SyntheticGraph {
    /// 전체 간선 수.
    pub fn edge_count(&self) -> u64 {
        self.tokens
            .iter()
            .chain(&self.bindings)
            .map(|producer| producer.refs.len() as u64)
            .sum()
    }

    /// node 수: 토큰 + binding target 속성 + 참조된 환경 입력.
    pub fn node_count(&self) -> u64 {
        let mut environment = [false; SYNTHETIC_ENVIRONMENT.len()];
        for producer in &self.bindings {
            for r in &producer.refs {
                if let SyntheticRef::Environment(k) = r {
                    environment[usize::from(*k)] = true;
                }
            }
        }
        u64::from(self.config.tokens)
            + u64::from(self.config.bindings)
            + environment.iter().filter(|used| **used).count() as u64
    }

    /// 내용 digest(FNV-1a 64, little-endian 필드 순서). 같은 설정·seed면 같습니다.
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv(0xCBF2_9CE4_8422_2325);
        hash.write(&self.config.seed.to_le_bytes());
        hash.write(&self.config.tokens.to_le_bytes());
        hash.write(&self.config.bindings.to_le_bytes());
        hash.write(&self.config.edges.to_le_bytes());
        for producer in self.tokens.iter().chain(&self.bindings) {
            hash.write(&(producer.refs.len() as u32).to_le_bytes());
            for r in &producer.refs {
                let (tag, value) = match r {
                    SyntheticRef::Token(i) => (1u8, *i),
                    SyntheticRef::Property(j) => (2u8, *j),
                    SyntheticRef::Environment(k) => (3u8, u32::from(*k)),
                };
                hash.write(&[tag]);
                hash.write(&value.to_le_bytes());
            }
            hash.write(&producer.constant.to_le_bytes());
        }
        hash.0
    }

    /// 토큰 `i`의 ID.
    pub fn token_ref(i: u32) -> TokenRef {
        TokenRef::new(StableId::new_v7(1, 0, u64::from(i)))
    }

    /// binding `j`의 target 속성.
    pub fn property(j: u32) -> PropertyPath {
        PropertyPath::new(StableId::new_v7(3, 0, u64::from(j))).push(PropertyKey(1))
    }

    fn target(r: SyntheticRef) -> ReferenceTarget {
        match r {
            SyntheticRef::Token(i) => ReferenceTarget::Token(SyntheticGraph::token_ref(i)),
            SyntheticRef::Property(j) => ReferenceTarget::Property(SyntheticGraph::property(j)),
            SyntheticRef::Environment(k) => {
                ReferenceTarget::Environment(SYNTHETIC_ENVIRONMENT[usize::from(k)])
            }
        }
    }

    /// producer 표현식: 참조가 없으면 상수, 있으면 `max(r0, max(r1, …))`.
    pub fn expression(producer: &SyntheticProducer) -> Result<ExpressionAst, AstError> {
        let node = |k: u64| ExprNodeId::new(StableId::new_v7(2, 0, k));
        let refs = &producer.refs;
        if refs.is_empty() {
            let value = TypedValue::Number(
                FiniteF32::new(f32::from(producer.constant)).unwrap_or(FiniteF32::ZERO),
            );
            return ExpressionAst::new(node(0), vec![ExprNode::literal(node(0), value)]);
        }
        let n = refs.len() as u64;
        let mut nodes: Vec<ExprNode> = refs
            .iter()
            .enumerate()
            .map(|(i, r)| ExprNode::reference(node(n + i as u64), SyntheticGraph::target(*r)))
            .collect();
        if n == 1 {
            return ExpressionAst::new(node(1), nodes);
        }
        // node(i) = max(ref(i), node(i + 1)), 마지막 max는 ref(n - 1).
        for i in 0..n - 1 {
            let rest = if i == n - 2 {
                node(n + i + 1)
            } else {
                node(i + 1)
            };
            nodes.push(ExprNode::call(
                node(i),
                FunctionId::Max,
                &[node(n + i), rest],
            ));
        }
        ExpressionAst::new(node(0), nodes)
    }

    /// 실제 토큰 registry(프로젝트 scope)와 binding 모음으로 만듭니다.
    pub fn materialize(&self) -> Result<(TokenRegistry, BindingSet), MaterializeError> {
        let mut set = TokenSet::new(TokenSetId::new(StableId::new_v7(4, 0, 0)));
        for (i, producer) in self.tokens.iter().enumerate() {
            let i = i as u32;
            let name = TokenName::new(&format!("synthetic.{i}")).map_err(MaterializeError::Name)?;
            set.insert(TokenDefinition::new(
                SyntheticGraph::token_ref(i),
                name,
                TokenType::new(ExpressionType::Number),
                SyntheticGraph::expression(producer).map_err(MaterializeError::Ast)?,
            ))
            .map_err(MaterializeError::Token)?;
        }
        let mut registry = TokenRegistry::new();
        registry
            .add_scope(TokenScope::Project, set)
            .map_err(MaterializeError::Registry)?;
        let mut bindings = BindingSet::new();
        for (j, producer) in self.bindings.iter().enumerate() {
            let j = j as u32;
            bindings
                .insert(ValueBinding::new(
                    BindingId::new(StableId::new_v7(5, 0, u64::from(j))),
                    SyntheticGraph::property(j),
                    SyntheticGraph::expression(producer).map_err(MaterializeError::Ast)?,
                    TypeSpec::required(ExpressionType::Number),
                    ScopeOwner::Node(NodeRef::new(StableId::new_v7(6, 0, u64::from(j)))),
                ))
                .map_err(MaterializeError::Binding)?;
        }
        Ok((registry, bindings))
    }
}

/// 실체화 오류(생성 규칙상 발생하지 않는 방어 오류).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializeError {
    /// 표현식 구성 실패.
    Ast(AstError),
    /// 토큰 이름 검증 실패.
    Name(TokenNameError),
    /// 토큰 추가 실패.
    Token(TokenSetError),
    /// scope 등록 실패.
    Registry(RegistryError),
    /// binding 추가 실패.
    Binding(BindingSetError),
}

impl fmt::Display for MaterializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MaterializeError::Ast(error) => write!(f, "{error}"),
            MaterializeError::Name(error) => write!(f, "{error}"),
            MaterializeError::Token(error) => write!(f, "{error}"),
            MaterializeError::Registry(error) => write!(f, "{error}"),
            MaterializeError::Binding(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for MaterializeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment_index::EnvironmentSources;
    use crate::graph::DependencyGraph;
    use crate::scheduler::{BindingEvaluator, StaticSources};
    use stellar_expressions::environment::EvaluationEnvironment;

    fn small(seed: u64) -> SyntheticConfig {
        SyntheticConfig {
            seed,
            tokens: 400,
            bindings: 1_200,
            edges: 4_000,
            max_token_refs: 3,
            environment_one_in: 8,
        }
    }

    /// preset golden digest. 생성 규칙(난수·배치·digest)을 바꾸면 의도적으로 갱신해야 합니다.
    const EDGES_1M_DIGEST: u64 = 0x86FA_5952_3FE2_EEB3;
    const ACTIVE_200K_DIGEST: u64 = 0x44AA_C72B_4135_F204;

    #[test]
    fn presets_are_seed_deterministic_with_exact_sizes() {
        let a = generate(SyntheticConfig::EDGES_1M).expect("생성");
        let b = generate(SyntheticConfig::EDGES_1M).expect("생성");
        assert_eq!(a.edge_count(), 1_000_000, "간선 정확히 100만");
        assert_eq!(a.digest(), b.digest());
        assert_eq!(a, b);
        assert_eq!(a.digest(), EDGES_1M_DIGEST, "golden digest");
        let doc = generate(SyntheticConfig::ACTIVE_DOCUMENT_200K).expect("생성");
        assert_eq!(doc.node_count(), 200_000, "node 정확히 20만");
        assert_eq!(doc.edge_count(), 500_000);
        assert_eq!(doc.digest(), ACTIVE_200K_DIGEST, "golden digest");
        assert_ne!(doc.digest(), a.digest());
    }

    #[test]
    fn different_seeds_differ_and_references_point_backwards() {
        let a = generate(small(1)).expect("생성");
        let b = generate(small(2)).expect("생성");
        assert_ne!(a.digest(), b.digest());
        assert_eq!(a.edge_count(), 4_000);
        for (i, token) in a.tokens.iter().enumerate() {
            for r in &token.refs {
                assert!(matches!(r, SyntheticRef::Token(t) if (*t as usize) < i));
            }
            let mut sorted = token.refs.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), token.refs.len(), "중복 참조 없음");
        }
        for (j, binding) in a.bindings.iter().enumerate() {
            for r in &binding.refs {
                if let SyntheticRef::Property(p) = r {
                    assert!((*p as usize) < j);
                }
            }
        }
        assert!(matches!(
            generate(SyntheticConfig {
                edges: 10_000_000,
                ..small(1)
            }),
            Err(SyntheticError::TooManyEdges { .. })
        ));
    }

    #[test]
    fn materialized_graph_matches_spec_and_evaluates() {
        let synthetic = generate(small(7)).expect("생성");
        let (registry, bindings) = synthetic.materialize().expect("실체화");
        assert_eq!(registry.len(), 400);
        assert_eq!(bindings.len(), 1_200);
        let graph = DependencyGraph::build(&registry, &bindings);
        assert_eq!(graph.edge_count() as u64, synthetic.edge_count());
        assert_eq!(graph.node_count() as u64, synthetic.node_count());
        assert!(graph.find_cycle().is_none());
        let environment = EvaluationEnvironment::new();
        let others = StaticSources::new();
        let sources = EnvironmentSources {
            environment: &environment,
            others: &others,
        };
        let evaluator = BindingEvaluator::new(&registry, &bindings, &sources).expect("생성");
        for index in graph.indices() {
            assert!(evaluator.value_at(index).is_ok(), "{:?}", graph.node(index));
        }
        // 같은 spec은 같은 registry·binding.
        assert_eq!(synthetic.materialize(), Ok((registry, bindings)));
    }
}
