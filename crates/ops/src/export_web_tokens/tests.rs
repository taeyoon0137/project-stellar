//! XXII.2.3 검증: deterministic CSS, IX.11.16 이름, 등록 토큰의 브라우저 계산값.
//!
//! 브라우저 기대값(`fixture/expected.txt`)은 `fixture/generate.py`가 고정 버전 Chrome으로 `fixture/tokens.css`를 읽어
//! 만들어 커밋합니다. test는 브라우저를 실행하지 않습니다.

use std::collections::BTreeMap;

use stellar_codegen::ir::{
    IrColorScheme, IrContrast, IrEnvironmentValue, IrInteractionState, IrNode, IrToken,
    IrTokenVariant, IrTree,
};
use stellar_target_profile::capability::ElementKind;

use super::*;
use crate::core_types::color::{Color, ColorSpace};
use crate::core_types::value::{FiniteF32, Length};

pub(crate) fn id(n: u8) -> StableId {
    StableId::from_bytes([0, 0, 0, 0, 0, 0, 0x70, 0, 0x80, 0, 0, 0, 0, 0, 0, n])
}

pub(crate) fn node(n: u8, parent: Option<u32>, depth: u32, element: ElementKind) -> IrNode {
    IrNode {
        id: id(n),
        parent,
        depth,
        children: Vec::new(),
        element,
        contracts: Vec::new(),
        layout: None,
        text: None,
        properties: Vec::new(),
        semantics: None,
        capabilities: Vec::new(),
        mappings: Vec::new(),
    }
}

/// root Frame 하나인 IR.
pub(crate) fn single_root() -> IrTree {
    IrTree::from_nodes(vec![node(1, None, 0, ElementKind::FRAME)]).expect("tree")
}

fn rgb(r: u8, g: u8, b: u8) -> TypedValue {
    let c = |v: u8| f32::from(v) / 255.0;
    TypedValue::Color(Color::new(ColorSpace::Srgb, c(r), c(g), c(b), 1.0).expect("color"))
}

fn px(v: f32) -> TypedValue {
    TypedValue::Length(Length::new(v).expect("length"))
}

fn number(v: f32) -> TypedValue {
    TypedValue::Number(FiniteF32::new(v).expect("number"))
}

fn token(n: u8, name: &str, value: TypedValue) -> IrToken {
    IrToken {
        id: id(n),
        name: name.to_string(),
        value,
        alias: None,
        variants: Vec::new(),
    }
}

fn variant(when: &[IrEnvironmentValue], value: TypedValue) -> IrTokenVariant {
    IrTokenVariant {
        when: when.to_vec(),
        value,
    }
}

const DARK: IrEnvironmentValue = IrEnvironmentValue::ColorScheme(IrColorScheme::Dark);
const HIGH: IrEnvironmentValue = IrEnvironmentValue::Contrast(IrContrast::High);
const REDUCED: IrEnvironmentValue = IrEnvironmentValue::ReducedMotion(true);

/// 브라우저 fixture 토큰.
pub(crate) fn fixture_tokens() -> Vec<IrToken> {
    let mut radius = token(1, "radius.lg", px(8.0));
    radius.variants = vec![variant(&[DARK], px(12.0))];
    let mut accent = token(2, "accent", rgb(10, 132, 255));
    accent.variants = vec![
        variant(&[DARK], rgb(100, 210, 255)),
        variant(&[HIGH], rgb(0, 64, 221)),
        variant(&[DARK, HIGH], rgb(64, 156, 255)),
    ];
    let mut alias = token(3, "accent alias", rgb(10, 132, 255));
    alias.alias = Some(id(2));
    let mut scale = token(4, "motion-scale", number(1.0));
    scale.variants = vec![variant(&[REDUCED], number(0.0))];
    let space = token(5, "Space 2", px(4.0));
    let label = token(6, "label", TypedValue::String("저장".into()));
    let mut hover = token(7, "hover-fill", rgb(255, 255, 255));
    hover.variants = vec![variant(
        &[IrEnvironmentValue::InteractionState(
            IrInteractionState::Hover,
        )],
        rgb(238, 238, 238),
    )];
    let mut viewport = token(8, "viewport-gap", px(10.0));
    viewport.variants = vec![variant(&[IrEnvironmentValue::Other(7)], px(20.0))];
    vec![radius, accent, alias, scale, space, label, hover, viewport]
}

fn fixture_ir() -> IrTree {
    let mut ir = single_root();
    ir.tokens = fixture_tokens();
    ir
}

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/export_web_tokens/fixture")
        .join(name)
}

#[test]
fn token_css_is_deterministic_and_matches_the_committed_browser_input() {
    // 검증 기준(XXII.2.3): deterministic CSS. 브라우저 fixture가 읽은 CSS와 byte 단위로 같습니다.
    let out = export_tokens(&fixture_ir());
    let again = export_tokens(&fixture_ir());
    assert_eq!(out, again);
    let mut reordered = fixture_ir();
    reordered.tokens.reverse();
    reordered.tokens.sort_by_key(|t| t.id);
    assert_eq!(export_tokens(&reordered).css, out.css);
    let path = fixture_path("tokens.css");
    if std::env::var_os("STELLAR_UPDATE_SNAPSHOT_FIXTURES").is_some() {
        std::fs::write(&path, &out.css).expect("갱신");
    }
    let committed = std::fs::read_to_string(&path).expect("tokens.css");
    assert_eq!(
        out.css, committed,
        "tokens.css가 바뀌면 generate.py로 기대값을 다시 만드세요"
    );
    // 등록 정책: 모든 @property는 inherits: true, :root와 :root를 감싼 @media에서만 값을 선언합니다.
    assert_eq!(out.css.matches("@property").count(), 6);
    assert_eq!(out.css.matches("inherits: true;").count(), 6);
    assert!(!out.css.contains("inherits: false"));
    for line in out.css.lines().filter(|l| l.trim_start().starts_with("--")) {
        assert!(
            line.starts_with("  --") || line.starts_with("    --"),
            "{line}"
        );
    }
    // interaction_state 조건 토큰은 토큰 선언에 없습니다(XXII.2.6이 Control selector로 냄).
    assert!(!out.css.contains("--hover-fill"));
    assert_eq!(out.plan.interaction.len(), 1);
    // alias는 :root의 var()이고 initial-value는 literal입니다.
    assert!(out.css.contains("  --accent-alias: var(--accent);\n"));
    assert!(out.css.contains("@property --accent-alias {\n  syntax: \"<color>\";\n  inherits: true;\n  initial-value: #0a84ff;\n}"));
    // 상수와 진단.
    assert_eq!(
        out.js,
        "export const tokens = Object.freeze({\n  \"--label\": \"저장\",\n});\n"
    );
    assert_eq!(
        out.diagnostics,
        vec![WebTokenDiagnostic {
            subject: id(8),
            code: "token-environment-unsupported"
        }]
    );
}

#[test]
fn output_names_follow_the_ix_11_16_derivation() {
    // 검증 기준(XXII.2.3): 출력 이름이 IX.11.16 golden과 같음(문서 custom_property test의 기대값).
    let out = export_tokens(&fixture_ir());
    assert_eq!(out.names.name_of(id(1)), Some("--radius-lg"));
    assert_eq!(out.names.name_of(id(5)), Some("--Space-2"));
    assert!(out.css.contains("@property --radius-lg {"));
    assert!(out.css.contains("  --Space-2: 4px;\n"));
    // 토큰과 ContextKey 이름 공간 충돌.
    let mut ir = single_root();
    ir.tokens = vec![token(9, "context/accent", px(1.0))];
    ir.contexts.keys = vec![stellar_codegen::ir::IrContextKey {
        id: id(10),
        name: "accent".into(),
        default: stellar_codegen::ir::IrExpr::Literal(px(1.0)),
    }];
    let out = export_tokens(&ir);
    assert_eq!(
        out.diagnostics,
        vec![
            WebTokenDiagnostic {
                subject: id(9),
                code: "name-collision"
            },
            WebTokenDiagnostic {
                subject: id(10),
                code: "name-collision"
            },
        ]
    );
}

#[test]
fn property_rule_template_matches_the_document_function() {
    // 문서 `registered_syntax`·`property_rule`과 같은 표·template인지 문서 source와 대조합니다(ADR-0106 결정 5).
    let source = include_str!("../../../document/src/css/custom_property.rs");
    for (variant, syntax) in [
        ("Length", "<length>"),
        ("Percentage", "<percentage>"),
        ("Angle", "<angle>"),
        ("Duration", "<time>"),
        ("Number", "<number>"),
        ("Color", "<color>"),
    ] {
        assert!(
            source.contains(&format!("ExpressionType::{variant} => \"{syntax}\"")),
            "{variant}"
        );
    }
    assert!(source.contains(
        "\"@property {css_name} {{\\n  syntax: \\\"{syntax}\\\";\\n  inherits: true;\\n  initial-value: {initial};\\n}}\\n\""
    ));
    for value in [px(1.0), number(2.0), rgb(1, 2, 3)] {
        assert!(registered_syntax_of(&value).is_some());
    }
    assert!(registered_syntax_of(&TypedValue::Boolean(true)).is_none());
}

/// 조합(dark=1, high=2, reduced=4)에서 IR이 정한 토큰 값(조건 key 밖은 무시, 값이 없으면 기본).
fn engine_value(token: &IrToken, combo: u8) -> TypedValue {
    let bit = |e: &IrEnvironmentValue| match e {
        IrEnvironmentValue::ColorScheme(IrColorScheme::Dark) => 1u8,
        IrEnvironmentValue::Contrast(IrContrast::High) => 2,
        IrEnvironmentValue::ReducedMotion(true) => 4,
        _ => 0,
    };
    let relevant = token.variants.iter().fold(0u8, |acc, v| {
        acc | v.when.iter().map(bit).fold(0, |a, b| a | b)
    });
    let wanted = combo & relevant;
    token
        .variants
        .iter()
        .find(|v| v.when.iter().map(bit).fold(0, |a, b| a | b) == wanted && wanted != 0)
        .map_or(token.value.clone(), |v| v.value.clone())
}

/// CSSOM 계산값 직렬화(test 전용): 길이 `Npx`, sRGB 색 `rgb(r, g, b)`, 수 `N`.
fn computed_text(value: &TypedValue) -> String {
    match value {
        TypedValue::Length(v) => format!("{}px", v.get()),
        TypedValue::Number(v) => format!("{}", v.get()),
        TypedValue::Color(c) => format!(
            "rgb({}, {}, {})",
            (c.r() * 255.0).round(),
            (c.g() * 255.0).round(),
            (c.b() * 255.0).round()
        ),
        other => panic!("fixture에 없는 타입: {other:?}"),
    }
}

#[test]
fn registered_tokens_compute_to_root_and_media_values_in_the_browser() {
    // 검증 기준(XXII.2.3): 등록 토큰을 var()로 쓰는 자손 요소의 계산값이 :root 값과 같고, @media 조건 on·off에서 각
    // 조건부 값과 같음. 기대값은 고정 버전 브라우저로 생성해 커밋했습니다.
    let expected = std::fs::read_to_string(fixture_path("expected.txt")).expect("expected.txt");
    let tokens = fixture_tokens();
    let by_name: BTreeMap<&str, &IrToken> = tokens.iter().map(|t| (t.name.as_str(), t)).collect();
    // probe 속성 → 그 속성이 var()로 읽는 토큰.
    let probes = [
        ("width", "radius.lg"),
        ("color", "accent"),
        ("outline-color", "accent"),
        ("opacity", "motion-scale"),
        ("padding-top", "Space 2"),
    ];
    let mut checked = 0;
    let mut combos = std::collections::BTreeSet::new();
    for line in expected
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let mut parts = line.splitn(3, ' ');
        let combo: u8 = parts.next().and_then(|c| c.parse().ok()).expect("combo");
        let property = parts.next().expect("property");
        let actual = parts.next().unwrap_or("");
        combos.insert(combo);
        if property == "--hover-fill" {
            assert_eq!(actual, "", "interaction 토큰이 :root에 선언되었습니다");
            checked += 1;
            continue;
        }
        let (_, name) = probes
            .iter()
            .find(|(p, _)| *p == property)
            .unwrap_or_else(|| panic!("모르는 probe {property}"));
        let token = by_name[name];
        assert_eq!(
            actual,
            computed_text(&engine_value(token, combo)),
            "combo {combo} {property}"
        );
        checked += 1;
    }
    assert_eq!(combos.len(), 8, "media 조합 8개 모두");
    assert_eq!(checked, 8 * (probes.len() + 1));
}
