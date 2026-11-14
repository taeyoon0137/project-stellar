//! compact/error/dark nullable Variant 조합 fixture (IX.9.3, 테스트 전용).
//!
//! 결제 화면(checkout) View 하나에 변수 세 개를 둡니다:
//!
//! - `device`: compact | regular (default regular)
//! - `paymentState`: idle | processing | error | completed, **nullable**(default idle). 실제 null 값은
//!   `ExplicitNull`이며 조건의 `Any`(제한 없음)와 다릅니다.
//! - `dark`: Boolean (default false)
//!
//! Variant(이름: 조건 → patch):
//!
//! | 이름 | 조건 | patch |
//! |---|---|---|
//! | compact | device=compact | card.padding=8 |
//! | error | payment=error | banner.visible=true, banner.height=48, card.fill=danger-subtle |
//! | dark | dark=true | card.fill=surface-dark, title.color=on-dark |
//! | noPayment | payment=ExplicitNull | banner.visible=true, banner.text="결제 수단 없음" |
//! | compactError | device=compact, payment=error | banner.height=32 |
//! | compactErrorDark | device=compact, payment=error, dark=true | card.fill=danger-dark |
//!
//! 조합 규칙(기술 아키텍처 16장): 일반 Variant 위에 더 구체적인 조합 Variant가 덮어쓰고, 정의되지 않은 조합
//! (compact+dark, regular+error+dark)은 추가 override 없이 일반 Variant만 적용합니다. **error+dark**(regular)는 같은
//! 구체성 1의 error·dark가 `card.fill`을 다르게 지정해 **ambiguity**이며 그 속성은 base 값을 유지합니다.
//! compact+error+dark에서는 구체성 3의 compactErrorDark가 같은 속성을 지정해 모호함이 해소됩니다.
//!
//! [`resolution_snapshot`]은 모든 case의 resolve 결과를 결정론적 JSON(ASCII, key 정렬)으로 출력합니다. 저장 fixture
//! `fixtures/variant_combinations.json`과 byte 단위로 비교합니다(IX.9.4 플랫폼 간 비교의 입력).

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::typed_value::{EnumValue, TypedValue};
use stellar_core_types::value::FiniteF32;
use stellar_expressions::types::{EnumTypeId, ExpressionType, TypeSpec};

use super::json::Json;
use crate::component::variant_override::VariantOverrideSet;
use crate::view_variant::commands::{ViewVariantCommand, ViewVariantModel};
use crate::view_variant::condition::{Assignment, VariantCondition};
use crate::view_variant::resolver::{
    resolve_properties, resolve_variants, PatchKey, ValueSource, ViewVariant, ViewVariantId,
};
use crate::view_variant::variable::{VariableValue, ViewVariable, ViewVariableId};

/// fixture StableId.
pub fn sid(n: u64) -> StableId {
    StableId::new_v7(1_700_000_900_000 + n, 0x0f19, 0x0093_0000_0000 ^ n)
}

const DEVICE_ENUM: u32 = 0x0E01;
const PAYMENT_ENUM: u32 = 0x0E02;

/// device 변수.
pub fn device_var() -> ViewVariableId {
    ViewVariableId::new(sid(11))
}

/// paymentState 변수.
pub fn payment_var() -> ViewVariableId {
    ViewVariableId::new(sid(12))
}

/// dark 변수.
pub fn dark_var() -> ViewVariableId {
    ViewVariableId::new(sid(13))
}

/// device 값: compact(0)·regular(1).
pub fn device(compact: bool) -> VariableValue {
    VariableValue::Value(TypedValue::Enum(EnumValue::new(
        DEVICE_ENUM,
        if compact { 0 } else { 1 },
    )))
}

/// paymentState 값: idle(0)·processing(1)·error(2)·completed(3).
pub fn payment(state: u32) -> VariableValue {
    VariableValue::Value(TypedValue::Enum(EnumValue::new(PAYMENT_ENUM, state)))
}

/// dark 값.
pub fn dark(on: bool) -> VariableValue {
    VariableValue::Value(TypedValue::Boolean(on))
}

/// 고정 Variant 목록: (id 번호, 이름).
const VARIANTS: [(u64, &str); 6] = [
    (21, "compact"),
    (22, "error"),
    (23, "dark"),
    (24, "noPayment"),
    (25, "compactError"),
    (26, "compactErrorDark"),
];

/// 이름의 Variant id.
pub fn variant(name: &str) -> ViewVariantId {
    let n = VARIANTS
        .iter()
        .find(|(_, v)| *v == name)
        .map(|(n, _)| *n)
        .unwrap_or(0);
    ViewVariantId::new(sid(n))
}

fn variant_name(id: ViewVariantId) -> &'static str {
    VARIANTS
        .iter()
        .find(|(n, _)| ViewVariantId::new(sid(*n)) == id)
        .map(|(_, name)| *name)
        .unwrap_or("?")
}

/// 속성 경로: (node 이름, key 이름).
const PATHS: [(&str, u64, &str, u32); 7] = [
    ("card", 31, "padding", 0x0110),
    ("card", 31, "fill", 0x0121),
    ("banner", 32, "visible", 0x0001),
    ("banner", 32, "height", 0x0112),
    ("banner", 32, "text", 0x0701),
    ("title", 33, "color", 0x0122),
    ("title", 33, "size", 0x0702),
];

/// `node.key` 이름의 속성 경로.
pub fn path(name: &str) -> PropertyPath {
    let (node, key) = name.split_once('.').unwrap_or((name, ""));
    let (_, id, _, raw) = PATHS
        .iter()
        .find(|(n, _, k, _)| *n == node && *k == key)
        .copied()
        .unwrap_or(("?", 0, "?", 0));
    PropertyPath::with_keys(sid(id), vec![PropertyKey(raw)])
}

fn path_name(path: &PropertyPath) -> String {
    PATHS
        .iter()
        .find(|(_, id, _, raw)| *path == PropertyPath::with_keys(sid(*id), vec![PropertyKey(*raw)]))
        .map(|(node, _, key, _)| format!("{node}.{key}"))
        .unwrap_or_else(|| "?".to_string())
}

fn number(value: f32) -> TypedValue {
    TypedValue::Number(FiniteF32::new(value).unwrap_or(FiniteF32::ZERO))
}

fn token(name: &str) -> TypedValue {
    TypedValue::String(name.to_string())
}

/// 조합 case 하나.
#[derive(Debug, Clone)]
pub struct VariantCase {
    /// case 이름.
    pub name: &'static str,
    /// 변수 할당.
    pub assignment: Assignment,
}

/// fixture: View 정의(변수 + Variant), base 속성 값, case 목록.
#[derive(Debug, Clone)]
pub struct VariantFixture {
    /// View 정의.
    pub model: ViewVariantModel,
    /// base 속성 값.
    pub base: BTreeMap<PropertyPath, TypedValue>,
    /// 조합 case(고정 순서).
    pub cases: Vec<VariantCase>,
}

fn patched(name: &str, condition: VariantCondition, patches: &[(&str, TypedValue)]) -> ViewVariant {
    let mut overrides = VariantOverrideSet::new();
    for (target, value) in patches {
        overrides.set_property(path(target), value.clone());
    }
    ViewVariant::new(variant(name), condition, overrides)
}

/// checkout fixture.
pub fn checkout_fixture() -> VariantFixture {
    let mut model = ViewVariantModel::new(sid(1));
    let variables = [
        ViewVariable::new(
            device_var(),
            "device".into(),
            TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(DEVICE_ENUM))),
            device(false),
            [
                EnumValue::new(DEVICE_ENUM, 0),
                EnumValue::new(DEVICE_ENUM, 1),
            ],
        ),
        ViewVariable::new(
            payment_var(),
            "paymentState".into(),
            TypeSpec::optional(ExpressionType::Enum(EnumTypeId::new(PAYMENT_ENUM))),
            payment(0),
            (0..4).map(|v| EnumValue::new(PAYMENT_ENUM, v)),
        ),
        ViewVariable::new(
            dark_var(),
            "dark".into(),
            TypeSpec::required(ExpressionType::Boolean),
            dark(false),
            [],
        ),
    ];
    for variable in variables.into_iter().flatten() {
        let _ = ViewVariantCommand::CreateVariable(variable).apply(&mut model);
    }
    let compact = || VariantCondition::new().equals(device_var(), device(true));
    let error = || VariantCondition::new().equals(payment_var(), payment(2));
    let variants = [
        patched("compact", compact(), &[("card.padding", number(8.0))]),
        patched(
            "error",
            error(),
            &[
                ("banner.visible", TypedValue::Boolean(true)),
                ("banner.height", number(48.0)),
                ("card.fill", token("danger-subtle")),
            ],
        ),
        patched(
            "dark",
            VariantCondition::new().equals(dark_var(), dark(true)),
            &[
                ("card.fill", token("surface-dark")),
                ("title.color", token("on-dark")),
            ],
        ),
        patched(
            "noPayment",
            VariantCondition::new().equals(payment_var(), VariableValue::ExplicitNull),
            &[
                ("banner.visible", TypedValue::Boolean(true)),
                ("banner.text", token("결제 수단 없음")),
            ],
        ),
        patched(
            "compactError",
            compact().equals(payment_var(), payment(2)),
            &[("banner.height", number(32.0))],
        ),
        patched(
            "compactErrorDark",
            compact()
                .equals(payment_var(), payment(2))
                .equals(dark_var(), dark(true)),
            &[("card.fill", token("danger-dark"))],
        ),
    ];
    for variant in variants {
        let _ = ViewVariantCommand::CreateVariant(variant).apply(&mut model);
    }
    let base = [
        ("card.padding", number(16.0)),
        ("card.fill", token("surface")),
        ("banner.visible", TypedValue::Boolean(false)),
        ("banner.height", number(0.0)),
        ("banner.text", token("")),
        ("title.color", token("on-surface")),
        ("title.size", number(20.0)),
    ]
    .into_iter()
    .map(|(name, value)| (path(name), value))
    .collect();
    let a = Assignment::new;
    let cases = vec![
        VariantCase {
            name: "default",
            assignment: a(),
        },
        VariantCase {
            name: "compact",
            assignment: a().with(device_var(), device(true)),
        },
        VariantCase {
            name: "error",
            assignment: a().with(payment_var(), payment(2)),
        },
        VariantCase {
            name: "dark",
            assignment: a().with(dark_var(), dark(true)),
        },
        VariantCase {
            name: "compact+error",
            assignment: a()
                .with(device_var(), device(true))
                .with(payment_var(), payment(2)),
        },
        VariantCase {
            name: "compact+dark",
            assignment: a()
                .with(device_var(), device(true))
                .with(dark_var(), dark(true)),
        },
        VariantCase {
            name: "error+dark",
            assignment: a()
                .with(payment_var(), payment(2))
                .with(dark_var(), dark(true)),
        },
        VariantCase {
            name: "compact+error+dark",
            assignment: a()
                .with(device_var(), device(true))
                .with(payment_var(), payment(2))
                .with(dark_var(), dark(true)),
        },
        VariantCase {
            name: "compact+processing",
            assignment: a()
                .with(device_var(), device(true))
                .with(payment_var(), payment(1)),
        },
        VariantCase {
            name: "payment=null",
            assignment: a().with(payment_var(), VariableValue::ExplicitNull),
        },
    ];
    VariantFixture { model, base, cases }
}

fn value_json(value: &TypedValue) -> Json {
    match value {
        TypedValue::Boolean(b) => Json::Bool(*b),
        TypedValue::Number(n) => Json::float(n.get()),
        TypedValue::String(s) => Json::str(s),
        other => Json::str(&format!("{other:?}")),
    }
}

/// 모든 case의 resolve 결과 JSON(일치 Variant 순서, 속성별 값·출처, 충돌).
pub fn resolution_snapshot(fixture: &VariantFixture) -> String {
    let cases = fixture
        .cases
        .iter()
        .map(|case| {
            let resolution = resolve_variants(
                fixture.model.variants(),
                &case.assignment,
                fixture.model.variables(),
            );
            let values = resolve_properties(&fixture.base, &resolution);
            let mut properties = BTreeMap::new();
            for (target, resolved) in &values {
                let source = match resolved.source {
                    ValueSource::Base => "base".to_string(),
                    ValueSource::Variant(id) => variant_name(id).to_string(),
                };
                properties.insert(
                    path_name(target),
                    Json::object([
                        ("source", Json::String(source)),
                        ("value", value_json(&resolved.value)),
                    ]),
                );
            }
            let conflicts = resolution
                .conflicts
                .iter()
                .map(|conflict| {
                    let target = match &conflict.key {
                        PatchKey::Property(p) => path_name(p),
                        other => format!("{other:?}"),
                    };
                    Json::object([
                        ("property", Json::String(target)),
                        ("specificity", Json::uint(conflict.specificity as u64)),
                        (
                            "variants",
                            Json::Array(
                                conflict
                                    .variants
                                    .iter()
                                    .map(|v| Json::str(variant_name(*v)))
                                    .collect(),
                            ),
                        ),
                    ])
                })
                .collect();
            Json::object([
                ("ambiguous", Json::Bool(resolution.is_ambiguous())),
                ("case", Json::str(case.name)),
                ("conflicts", Json::Array(conflicts)),
                (
                    "matched",
                    Json::Array(
                        resolution
                            .matched
                            .iter()
                            .map(|v| Json::str(variant_name(*v)))
                            .collect(),
                    ),
                ),
                ("properties", Json::Object(properties)),
            ])
        })
        .collect();
    Json::object([
        ("fixture", Json::str("checkout compact/error/dark")),
        ("cases", Json::Array(cases)),
    ])
    .to_pretty_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_variant::resolver::VariantResolution;

    const FIXTURE: &str = "src/test_support/fixtures/variant_combinations.json";

    fn resolve(fixture: &VariantFixture, name: &str) -> VariantResolution {
        let case = fixture.cases.iter().find(|c| c.name == name).unwrap();
        resolve_variants(
            fixture.model.variants(),
            &case.assignment,
            fixture.model.variables(),
        )
    }

    fn names(resolution: &VariantResolution) -> Vec<&'static str> {
        resolution
            .matched
            .iter()
            .map(|v| variant_name(*v))
            .collect()
    }

    fn value(fixture: &VariantFixture, name: &str, target: &str) -> (TypedValue, &'static str) {
        let resolution = resolve(fixture, name);
        let values = resolve_properties(&fixture.base, &resolution);
        let resolved = &values[&path(target)];
        let source = match resolved.source {
            ValueSource::Base => "base",
            ValueSource::Variant(id) => variant_name(id),
        };
        (resolved.value.clone(), source)
    }

    #[test]
    fn fixture_model_is_valid_and_complete() {
        let fixture = checkout_fixture();
        assert_eq!(fixture.model.variables().len(), 3);
        assert_eq!(fixture.model.variants().count(), VARIANTS.len());
        assert!(fixture.model.diagnostics().is_empty(), "orphan 조건 없음");
        for case in &fixture.cases {
            case.assignment
                .validate(fixture.model.variables())
                .unwrap_or_else(|e| panic!("{}: {e}", case.name));
        }
        // model round-trip.
        let bytes = fixture.model.encode();
        assert_eq!(ViewVariantModel::decode(&bytes).unwrap().encode(), bytes);
    }

    #[test]
    fn expected_override_resolution_per_combination() {
        // 검증 기준(IX.9.3): 예상 override resolution.
        let f = checkout_fixture();
        assert!(names(&resolve(&f, "default")).is_empty());
        assert_eq!(
            value(&f, "default", "card.fill"),
            (token("surface"), "base")
        );
        assert_eq!(names(&resolve(&f, "compact")), ["compact"]);
        assert_eq!(
            value(&f, "compact", "card.padding"),
            (number(8.0), "compact")
        );
        // 구체 조합이 일반 값을 덮어씀: compact+error의 banner.height는 compactError(32).
        assert_eq!(
            names(&resolve(&f, "compact+error")),
            ["compact", "error", "compactError"]
        );
        assert_eq!(
            value(&f, "compact+error", "banner.height"),
            (number(32.0), "compactError")
        );
        assert_eq!(
            value(&f, "compact+error", "card.fill"),
            (token("danger-subtle"), "error")
        );
        // 정의되지 않은 조합 compact+dark: 일반 Variant만, 충돌 없음(서로 다른 속성).
        let compact_dark = resolve(&f, "compact+dark");
        assert_eq!(names(&compact_dark), ["compact", "dark"]);
        assert!(!compact_dark.is_ambiguous());
        assert_eq!(
            value(&f, "compact+dark", "card.fill"),
            (token("surface-dark"), "dark")
        );
        // compact+error+dark: 구체성 3이 card.fill 모호함을 해소.
        let all = resolve(&f, "compact+error+dark");
        assert_eq!(
            names(&all),
            [
                "compact",
                "error",
                "dark",
                "compactError",
                "compactErrorDark"
            ]
        );
        assert!(!all.is_ambiguous());
        assert_eq!(
            value(&f, "compact+error+dark", "card.fill"),
            (token("danger-dark"), "compactErrorDark")
        );
        // 일치 Variant 없는 조합(compact+processing)은 compact만.
        assert_eq!(names(&resolve(&f, "compact+processing")), ["compact"]);
        // ExplicitNull은 실제 null 값: noPayment만 일치(error·Any와 구분).
        assert_eq!(names(&resolve(&f, "payment=null")), ["noPayment"]);
        assert_eq!(
            value(&f, "payment=null", "banner.text"),
            (token("결제 수단 없음"), "noPayment")
        );
    }

    #[test]
    fn ambiguity_diagnostic_for_same_specificity_conflict() {
        // 검증 기준(IX.9.3): ambiguity diagnostic.
        let f = checkout_fixture();
        let resolution = resolve(&f, "error+dark");
        assert_eq!(names(&resolution), ["error", "dark"]);
        assert!(resolution.is_ambiguous());
        assert_eq!(resolution.conflicts.len(), 1);
        let conflict = &resolution.conflicts[0];
        assert_eq!(conflict.key, PatchKey::Property(path("card.fill")));
        assert_eq!(conflict.specificity, 1);
        assert_eq!(conflict.variants, vec![variant("error"), variant("dark")]);
        // 충돌 속성은 base 유지, 나머지는 각 Variant 값 적용.
        assert_eq!(
            value(&f, "error+dark", "card.fill"),
            (token("surface"), "base")
        );
        assert_eq!(
            value(&f, "error+dark", "title.color"),
            (token("on-dark"), "dark")
        );
        assert_eq!(
            value(&f, "error+dark", "banner.visible"),
            (TypedValue::Boolean(true), "error")
        );
        // 다른 case에는 충돌이 없습니다.
        for case in &f.cases {
            if case.name != "error+dark" {
                assert!(!resolve(&f, case.name).is_ambiguous(), "{}", case.name);
            }
        }
    }

    #[test]
    fn resolution_snapshot_matches_fixture_bytes() {
        let snapshot = resolution_snapshot(&checkout_fixture());
        assert!(snapshot.is_ascii());
        assert_eq!(snapshot, resolution_snapshot(&checkout_fixture()), "결정론");
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        if std::env::var_os("STELLAR_UPDATE_SNAPSHOT_FIXTURES").is_some() {
            std::fs::write(&fixture_path, &snapshot).unwrap();
        }
        let expected = std::fs::read(&fixture_path).expect("fixture 파일");
        assert!(
            snapshot.as_bytes() == expected.as_slice(),
            "variant 조합 snapshot이 fixture와 다릅니다. 의도한 변경이면 \
             STELLAR_UPDATE_SNAPSHOT_FIXTURES=1로 갱신하세요.\n{snapshot}"
        );
    }
}
