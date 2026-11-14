//! orphan 기본값·안전값 evaluator golden corpus (VIII.9.3, VIII.8.2·VIII.8.3 경로).
//!
//! 모든 값 타입의 안전값과, 저장된 기본값의 네 상태(정상·없음·bytes 손상·타입 불일치)로 연 orphan target 값,
//! orphan target을 읽는 live binding 값, 손상 진단을 고정 golden 파일
//! `tests/fixtures/fallback_golden_corpus_v1.golden`과 **파일 bytes 단위로** 비교합니다. 다른 플랫폼에서 같은
//! 테스트를 실행하면 모든 결과 bytes가 golden과 같아야 합니다. 같은 입력으로 두 번 열어 결과가 같은지도 확인합니다.
//!
//! 각 줄은 `<이름> <결과> <진단>`입니다. 결과: `value:<TypedValue::to_bytes hex>` | `null` | `error:<종류>`.
//! 진단: `none` | `missing` | `malformed` | `invalid-type`.
//!
//! golden 갱신(의도적 변경만): `STELLAR_UPDATE_FALLBACK_CORPUS=1 cargo test -p stellar-bindings --test
//! fallback_golden_corpus`.

use std::collections::BTreeMap;

use stellar_bindings::binding::{BindingId, BindingSet, ScopeOwner, ValueBinding};
use stellar_bindings::fallback::{
    safe_value, CorruptionDiagnostic, DefaultCorruption, StoredDefault,
};
use stellar_bindings::graph::DependencyNode;
use stellar_bindings::orphan::{BindingStore, BindingWorld};
use stellar_bindings::registry::{TokenRegistry, TokenScope};
use stellar_bindings::scheduler::{BindingEvaluator, NodeValue, StaticSources};
use stellar_bindings::token::{TokenDefinition, TokenName, TokenSet, TokenSetId, TokenType};
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::gradient::{Gradient, GradientStop};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reference::{NodeRef, TokenRef};
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::typed_value::{EnumValue, ResourceRef, TypedValue, TypographyValue};
use stellar_core_types::value::{Angle, Duration, FiniteF32, Length, Percentage, Vec2};
use stellar_expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_expressions::ops::BinaryOp;
use stellar_expressions::types::{EnumTypeId, ExpressionType, TypeSpec};

const GOLDEN_PATH: &str = "tests/fixtures/fallback_golden_corpus_v1.golden";
const GOLDEN: &str = include_str!("fixtures/fallback_golden_corpus_v1.golden");

const HEADER: &str = "\
# orphan 기본값·안전값 evaluator golden corpus (VIII.9.3, 버전 1).
#
# 각 줄: <case 이름> <결과> <진단>
# - 결과: value:<TypedValue::to_bytes 소문자 hex> | null | error:<종류>
# - 진단: none | missing | malformed | invalid-type (VIII.8.3 CorruptionDiagnostic)
# 모든 값은 상수 구성·IEEE 기본 연산만 써서 플랫폼과 무관하게 같은 bytes여야 합니다(ADR-0006 규칙 4).
#
# case 정의와 검사: crates/bindings/tests/fallback_golden_corpus.rs
# 갱신: STELLAR_UPDATE_FALLBACK_CORPUS=1 cargo test -p stellar-bindings --test fallback_golden_corpus
";

const ENUM_TYPE: u32 = 7;

/// 모든 비-nullable 값 타입(TypedValue variant 순).
const TYPES: [ExpressionType; 13] = [
    ExpressionType::Boolean,
    ExpressionType::Number,
    ExpressionType::String,
    ExpressionType::Enum(EnumTypeId::new(ENUM_TYPE)),
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

fn short(t: ExpressionType) -> &'static str {
    match t {
        ExpressionType::Boolean => "bool",
        ExpressionType::Number => "num",
        ExpressionType::String => "str",
        ExpressionType::Enum(_) => "enum",
        ExpressionType::Length => "len",
        ExpressionType::Percentage => "pct",
        ExpressionType::Angle => "deg",
        ExpressionType::Duration => "dur",
        ExpressionType::Color => "color",
        ExpressionType::Gradient => "gradient",
        ExpressionType::Shadow => "shadow",
        ExpressionType::Typography => "typo",
        ExpressionType::ResourceRef => "res",
    }
}

fn finite(v: f32) -> FiniteF32 {
    FiniteF32::new(v).expect("유한")
}

fn color(space: ColorSpace, r: f32, g: f32, b: f32, a: f32) -> Color {
    Color::new(space, r, g, b, a).expect("유효한 색")
}

/// 타입별 정상 기본값(안전값과 다른 값).
fn sample(t: ExpressionType) -> TypedValue {
    match t {
        ExpressionType::Boolean => TypedValue::Boolean(true),
        ExpressionType::Number => TypedValue::Number(finite(0.75)),
        ExpressionType::String => TypedValue::String("기본값".to_string()),
        ExpressionType::Enum(_) => TypedValue::Enum(EnumValue::new(ENUM_TYPE, 3)),
        ExpressionType::Length => TypedValue::Length(Length::from_raw(finite(12.5))),
        ExpressionType::Percentage => TypedValue::Percentage(Percentage::from_raw(finite(50.0))),
        ExpressionType::Angle => TypedValue::Angle(Angle::from_raw(finite(-90.0))),
        ExpressionType::Duration => TypedValue::Duration(Duration::from_raw(finite(250.0))),
        ExpressionType::Color => {
            TypedValue::Color(color(ColorSpace::DisplayP3, 1.0, 0.5, 0.0, 0.8))
        }
        ExpressionType::Gradient => TypedValue::Gradient(Gradient::new(vec![
            GradientStop::new(
                StableId::from_bytes([0x61; 16]),
                0.0,
                color(ColorSpace::Srgb, 1.0, 1.0, 1.0, 1.0),
            )
            .expect("stop"),
            GradientStop::new(
                StableId::from_bytes([0x62; 16]),
                1.0,
                color(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0),
            )
            .expect("stop"),
        ])),
        ExpressionType::Shadow => TypedValue::Shadow(Shadow::new(
            Vec2::new(1.0, 2.0).expect("유한"),
            BlurRadius::new(4.0).expect("blur"),
            color(ColorSpace::Srgb, 0.0, 0.0, 0.0, 0.25),
        )),
        ExpressionType::Typography => {
            TypedValue::Typography(TypographyValue::new(vec![0x01, 0x02, 0x03]))
        }
        ExpressionType::ResourceRef => {
            TypedValue::ResourceRef(ResourceRef::new(StableId::from_bytes([0x52; 16])))
        }
    }
}

/// `t`가 아닌 타입의 값(타입 불일치 기본값).
fn wrong_value(t: ExpressionType) -> TypedValue {
    if t == ExpressionType::Boolean {
        TypedValue::Number(finite(1.0))
    } else {
        TypedValue::Boolean(false)
    }
}

fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

fn prop(n: u64) -> PropertyPath {
    PropertyPath::new(sid(2_000 + n)).push(PropertyKey(1))
}

fn x(n: u64) -> ExprNodeId {
    ExprNodeId::new(sid(10_000 + n))
}

fn missing_token() -> ReferenceTarget {
    ReferenceTarget::Token(TokenRef::new(sid(999)))
}

fn base_token() -> TokenRef {
    TokenRef::new(sid(1_001))
}

fn literal(node: u64, value: TypedValue) -> ExpressionAst {
    ExpressionAst::new(x(node), vec![ExprNode::literal(x(node), value)]).expect("유효")
}

/// 기본값 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stored {
    Valid,
    Missing,
    Truncated,
    WrongType,
}

impl Stored {
    const ALL: [Stored; 4] = [
        Stored::Valid,
        Stored::Missing,
        Stored::Truncated,
        Stored::WrongType,
    ];

    const fn name(self) -> &'static str {
        match self {
            Stored::Valid => "valid",
            Stored::Missing => "missing",
            Stored::Truncated => "truncated",
            Stored::WrongType => "wrong_type",
        }
    }
}

/// orphan 한 개: 번호, 이름, 선언 타입, 기본값 상태.
struct OrphanCase {
    n: u64,
    name: String,
    declared: TypeSpec,
    stored: Stored,
}

fn orphan_cases() -> Vec<OrphanCase> {
    let mut out = Vec::new();
    let mut n = 100;
    for t in TYPES {
        for stored in Stored::ALL {
            n += 1;
            out.push(OrphanCase {
                n,
                name: format!("open.{}.{}", short(t), stored.name()),
                declared: TypeSpec::required(t),
                stored,
            });
        }
    }
    for t in [
        ExpressionType::Length,
        ExpressionType::String,
        ExpressionType::Color,
    ] {
        for stored in Stored::ALL {
            n += 1;
            out.push(OrphanCase {
                n,
                name: format!("open.optional_{}.{}", short(t), stored.name()),
                declared: TypeSpec::optional(t),
                stored,
            });
        }
    }
    out
}

/// 토큰 registry: `base`(Length 10px).
fn registry() -> TokenRegistry {
    let mut set = TokenSet::new(TokenSetId::new(sid(1)));
    set.insert(TokenDefinition::new(
        base_token(),
        TokenName::new("base").expect("이름"),
        TokenType::new(ExpressionType::Length),
        literal(1, TypedValue::Length(Length::from_raw(finite(10.0)))),
    ))
    .expect("토큰");
    let mut registry = TokenRegistry::new();
    registry.add_scope(TokenScope::Project, set).expect("scope");
    registry
}

/// 속성 선언 타입과 토큰 존재를 알려 주는 현재 상태. 없는 토큰(999)을 읽는 binding은 orphan이 됩니다.
struct World {
    targets: BTreeMap<PropertyPath, TypeSpec>,
}

impl BindingWorld for World {
    fn target_type(&self, target: &PropertyPath) -> Option<TypeSpec> {
        self.targets.get(target).copied()
    }

    fn reference_type(&self, target: &ReferenceTarget) -> Option<TypeSpec> {
        match target {
            ReferenceTarget::Token(token) if *token == base_token() => {
                Some(TypeSpec::required(ExpressionType::Length))
            }
            ReferenceTarget::Property(path) => self.targets.get(path).copied(),
            _ => None,
        }
    }
}

const TOKEN_DEFAULT: u64 = 90;
const LIVE_READER: u64 = 91;

fn binding(n: u64, target: PropertyPath, source: ExpressionAst, spec: TypeSpec) -> ValueBinding {
    ValueBinding::new(
        BindingId::new(sid(4_000 + n)),
        target,
        source,
        spec,
        ScopeOwner::Node(NodeRef::new(sid(5_000 + n))),
    )
}

/// `target + literal` (Length).
fn plus_px(node: u64, target: ReferenceTarget, px: f32) -> ExpressionAst {
    ExpressionAst::new(
        x(node),
        vec![
            ExprNode::binary(x(node), BinaryOp::Add, x(node + 1), x(node + 2)),
            ExprNode::reference(x(node + 1), target),
            ExprNode::literal(
                x(node + 2),
                TypedValue::Length(Length::from_raw(finite(px))),
            ),
        ],
    )
    .expect("유효")
}

/// 저장소·저장된 기본값을 만듭니다.
///
/// - orphan case마다: target `prop(n)`, source = 없는 토큰 참조 → orphan.
/// - `TOKEN_DEFAULT`: 기본값이 토큰을 참조하는 orphan(`base + 2px`).
/// - `LIVE_READER`: live binding `prop(91) ← prop(len.valid) + 1px`(orphan target을 읽음).
fn fixture() -> (
    TokenRegistry,
    BindingStore,
    BTreeMap<PropertyPath, StoredDefault>,
) {
    let cases = orphan_cases();
    let length = TypeSpec::required(ExpressionType::Length);
    let mut live = BindingSet::new();
    let mut targets = BTreeMap::new();
    let mut stored = BTreeMap::new();
    for case in &cases {
        let source = ExpressionAst::new(
            x(case.n * 10),
            vec![ExprNode::reference(x(case.n * 10), missing_token())],
        )
        .expect("유효");
        live.insert(binding(case.n, prop(case.n), source, case.declared))
            .expect("binding");
        targets.insert(prop(case.n), case.declared);
        let valid = literal(case.n * 10 + 1, sample(case.declared.value_type()));
        let bytes = match case.stored {
            Stored::Valid => Some(valid.to_canonical_bytes().expect("직렬화")),
            Stored::Missing => None,
            Stored::Truncated => {
                let mut bytes = valid.to_canonical_bytes().expect("직렬화");
                bytes.truncate(bytes.len() / 2);
                Some(bytes)
            }
            Stored::WrongType => Some(
                literal(case.n * 10 + 2, wrong_value(case.declared.value_type()))
                    .to_canonical_bytes()
                    .expect("직렬화"),
            ),
        };
        if let Some(expression) = bytes {
            stored.insert(
                prop(case.n),
                StoredDefault {
                    declared_type: case.declared,
                    expression,
                },
            );
        }
    }
    // 기본값이 토큰을 참조하는 orphan.
    let source = ExpressionAst::new(
        x(TOKEN_DEFAULT * 10),
        vec![ExprNode::reference(x(TOKEN_DEFAULT * 10), missing_token())],
    )
    .expect("유효");
    live.insert(binding(TOKEN_DEFAULT, prop(TOKEN_DEFAULT), source, length))
        .expect("binding");
    targets.insert(prop(TOKEN_DEFAULT), length);
    stored.insert(
        prop(TOKEN_DEFAULT),
        StoredDefault {
            declared_type: length,
            expression: plus_px(
                TOKEN_DEFAULT * 10 + 1,
                ReferenceTarget::Token(base_token()),
                2.0,
            )
            .to_canonical_bytes()
            .expect("직렬화"),
        },
    );
    // orphan target(len.valid)을 읽는 live binding.
    let len_valid = cases
        .iter()
        .find(|case| case.name == "open.len.valid")
        .map(|case| case.n)
        .expect("len.valid case");
    live.insert(binding(
        LIVE_READER,
        prop(LIVE_READER),
        plus_px(
            LIVE_READER * 10,
            ReferenceTarget::Property(prop(len_valid)),
            1.0,
        ),
        length,
    ))
    .expect("live");
    targets.insert(prop(LIVE_READER), length);
    let mut store = BindingStore::new(live);
    let orphaned = store.orphan_broken(&World { targets });
    assert_eq!(
        orphaned.len(),
        cases.len() + 1,
        "없는 토큰을 읽는 binding만 orphan"
    );
    (registry(), store, stored)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn encode_value(value: Option<&NodeValue>) -> String {
    match value {
        None => "error:UnknownNode".to_string(),
        Some(Ok(Some(value))) => format!("value:{}", hex(&value.to_bytes())),
        Some(Ok(None)) => "null".to_string(),
        Some(Err(error)) => {
            let kind: String = format!("{error:?}")
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            format!("error:{kind}")
        }
    }
}

fn diagnostic_of(diagnostics: &[CorruptionDiagnostic], target: &PropertyPath) -> &'static str {
    match diagnostics.iter().find(|d| d.target == *target) {
        None => "none",
        Some(d) => match d.corruption {
            DefaultCorruption::Missing => "missing",
            DefaultCorruption::Malformed(_) => "malformed",
            DefaultCorruption::InvalidType(_) => "invalid-type",
        },
    }
}

fn render() -> String {
    let mut out = String::from(HEADER);
    for t in TYPES {
        let encoded = encode_value(Some(&Ok(safe_value(t))));
        out.push_str(&format!("safe.{} {encoded} none\n", short(t)));
    }
    let (tokens, store, stored) = fixture();
    let sources = StaticSources::new();
    let before = (store.clone(), stored.clone());
    let (evaluator, diagnostics) =
        BindingEvaluator::open_with_stored_defaults(&tokens, &store, &stored, &sources)
            .expect("손상 기본값이 있어도 열림");
    assert_eq!((store.clone(), stored.clone()), before, "입력 수정 없음");
    let (again, again_diagnostics) =
        BindingEvaluator::open_with_stored_defaults(&tokens, &store, &stored, &sources)
            .expect("재열기");
    assert_eq!(again_diagnostics, diagnostics, "진단 결정적");
    let mut named: Vec<(String, u64)> = orphan_cases()
        .into_iter()
        .map(|case| (case.name, case.n))
        .collect();
    named.push(("open.token_default".to_string(), TOKEN_DEFAULT));
    named.push(("open.live_reads_orphan".to_string(), LIVE_READER));
    for (name, n) in named {
        let node = DependencyNode::Property(prop(n));
        let value = encode_value(evaluator.value(&node));
        assert_eq!(
            encode_value(again.value(&node)),
            value,
            "{name}: 재열기 같은 값"
        );
        let diagnostic = diagnostic_of(&diagnostics, &prop(n));
        out.push_str(&format!("{name} {value} {diagnostic}\n"));
    }
    out.push_str(&format!(
        "open.diagnostics count:{} none\n",
        diagnostics.len()
    ));
    out
}

#[test]
fn fallback_results_match_golden_bytes() {
    let rendered = render();
    if std::env::var_os("STELLAR_UPDATE_FALLBACK_CORPUS").is_some() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_PATH);
        std::fs::write(&path, &rendered).expect("golden 쓰기");
        return;
    }
    if rendered != GOLDEN {
        let diff: Vec<String> = rendered
            .lines()
            .zip(GOLDEN.lines())
            .filter(|(a, e)| a != e)
            .take(20)
            .map(|(a, e)| format!("- {e}\n+ {a}"))
            .collect();
        panic!(
            "golden 불일치(줄 수 실제 {} / golden {}). 처음 차이:\n{}",
            rendered.lines().count(),
            GOLDEN.lines().count(),
            diff.join("\n")
        );
    }
}

#[test]
fn golden_semantics_are_hand_checked() {
    let line = |name: &str| -> Vec<String> {
        GOLDEN
            .lines()
            .find(|line| line.split(' ').next() == Some(name))
            .unwrap_or_else(|| panic!("{name} 없음"))
            .split(' ')
            .skip(1)
            .map(str::to_string)
            .collect()
    };
    let value = |v: &TypedValue| format!("value:{}", hex(&v.to_bytes()));
    for t in TYPES {
        let s = short(t);
        let safe = safe_value(t).expect("안전값");
        assert_eq!(
            line(&format!("safe.{s}")),
            vec![value(&safe), "none".into()]
        );
        // 정상 기본값은 그대로, 손상된 기본값은 안전값과 진단.
        assert_eq!(
            line(&format!("open.{s}.valid")),
            vec![value(&sample(t)), "none".into()]
        );
        assert_eq!(
            line(&format!("open.{s}.missing")),
            vec![value(&safe), "missing".into()]
        );
        assert_eq!(
            line(&format!("open.{s}.truncated")),
            vec![value(&safe), "malformed".into()]
        );
        assert_eq!(
            line(&format!("open.{s}.wrong_type")),
            vec![value(&safe), "invalid-type".into()]
        );
    }
    // Optional 선언 타입의 손상 기본값은 null.
    assert_eq!(
        line("open.optional_color.missing"),
        vec!["null".to_string(), "missing".into()]
    );
    // base(10px) + 2px = 12px, len.valid(12.5px) + 1px = 13.5px.
    let px = |v: f32| value(&TypedValue::Length(Length::from_raw(finite(v))));
    assert_eq!(line("open.token_default"), vec![px(12.0), "none".into()]);
    assert_eq!(
        line("open.live_reads_orphan"),
        vec![px(13.5), "none".into()]
    );
    // 손상 진단 수 = 타입 16개(필수 13 + optional 3) × 손상 3종.
    assert_eq!(
        line("open.diagnostics"),
        vec![format!("count:{}", 16 * 3), "none".into()]
    );
}
