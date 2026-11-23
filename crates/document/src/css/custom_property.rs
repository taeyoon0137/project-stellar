//! TokenName·ContextKey 이름에서 CSS custom property 이름 파생과 토큰 선언 텍스트 (IX.11.16, 속성 모델 10장,
//! ADR-0075 결정 7).
//!
//! 파생은 이름·종류·StableId만 입력으로 받는 순수 함수입니다([`derive_names`]).
//!
//! 1. 이름의 공백·`/`·`.`을 `-`로 바꿉니다.
//! 2. CSS identifier 직렬화 규칙으로 escape하고 접두사를 붙입니다. 토큰은 `--`, ContextKey는 `--context-`입니다.
//!    대소문자는 보존합니다.
//! 3. 충돌 판정은 토큰 이름과 ContextKey 이름을 합친 한 집합으로 합니다.
//! 4. 겹치면 `name-collision` 진단을 내고, StableId 오름차순 첫 항목을 뺀 나머지에 `-` + StableId 32자리 소문자
//!    hex를 붙입니다(앞 8자리만 쓰면 약 65초 안에 만든 UUIDv7끼리 같을 수 있음).
//! 5. rename하면 CSS 이름만 바뀌고 문서 안 참조(StableId)는 그대로입니다.
//!
//! 토큰 선언은 등록 custom property 정책을 따릅니다. `@property`는 모두 `inherits: true`이고 `initial-value`는
//! default expression을 기본 환경에서 평가한 literal입니다([`property_rule`]). 다른 토큰을 참조하는 값(alias)은
//! `:root`의 `var()` 선언으로만 냅니다([`root_block`]).

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::types::ExpressionType;

use super::value::{
    format_angle, format_color, format_duration, format_length, format_number, format_percentage,
    TokenScope,
};

/// 이름 충돌 진단 코드.
pub const NAME_COLLISION: &str = "name-collision";

/// 이름의 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CustomPropertyKind {
    /// 디자인 토큰(`--`).
    Token,
    /// Component Context key(`--context-`, ADR-0025 결정 3).
    ContextKey,
}

impl CustomPropertyKind {
    /// 접두사.
    pub fn prefix(self) -> &'static str {
        match self {
            CustomPropertyKind::Token => "--",
            CustomPropertyKind::ContextKey => "--context-",
        }
    }
}

/// 파생 입력 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameInput<'a> {
    /// TokenName 또는 ContextKey 이름.
    pub name: &'a str,
    /// 종류.
    pub kind: CustomPropertyKind,
    /// 대상 StableId.
    pub id: StableId,
}

/// 파생 결과 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedName {
    /// 대상 StableId.
    pub id: StableId,
    /// 종류.
    pub kind: CustomPropertyKind,
    /// `--`로 시작하는 custom property 이름.
    pub css_name: String,
}

/// 같은 CSS 이름으로 파생된 항목들(`name-collision`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameCollision {
    /// 겹친 기본 이름.
    pub css_name: String,
    /// 겹친 항목의 StableId(오름차순). 첫 항목만 기본 이름을 갖습니다.
    pub ids: Vec<StableId>,
}

impl NameCollision {
    /// 진단 코드.
    pub fn code(&self) -> &'static str {
        NAME_COLLISION
    }
}

impl fmt::Display for NameCollision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{NAME_COLLISION}: {}개 항목이 {}로 파생됩니다. StableId 순 첫 항목 뒤에는 StableId 접미사를 붙입니다",
            self.ids.len(),
            self.css_name
        )
    }
}

/// export scope 하나의 파생 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DerivedNames {
    /// 입력 순서의 결과.
    pub names: Vec<DerivedName>,
    /// 충돌 진단(기본 이름 순).
    pub collisions: Vec<NameCollision>,
}

impl DerivedNames {
    /// `id`의 CSS 이름.
    pub fn name_of(&self, id: StableId) -> Option<&str> {
        self.names
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.css_name.as_str())
    }
}

impl TokenScope for DerivedNames {
    /// `var(--name)`은 토큰만 풉니다(ContextKey 값은 문서 값 문법에서 참조하지 않음).
    fn resolve(&self, name: &str) -> Option<StableId> {
        self.names
            .iter()
            .find(|n| n.kind == CustomPropertyKind::Token && n.css_name == name)
            .map(|n| n.id)
    }
}

/// CSS identifier 직렬화(CSSOM "serialize an identifier")로 이름 본문을 escape합니다. 본문 앞에는 항상
/// 접두사(`--`)가 붙으므로 첫 글자 숫자 규칙은 적용되지 않습니다.
pub fn escape_identifier(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    for c in body.chars() {
        match c {
            '\0' => out.push('\u{FFFD}'),
            '\u{1}'..='\u{1F}' | '\u{7F}' => out.push_str(&format!("\\{:x} ", c as u32)),
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii() => out.push(c),
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
    out
}

/// 충돌을 보기 전의 기본 CSS 이름.
pub fn base_name(name: &str, kind: CustomPropertyKind) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            ' ' | '/' | '.' => '-',
            c if c.is_whitespace() => '-',
            c => c,
        })
        .collect();
    format!("{}{}", kind.prefix(), escape_identifier(&replaced))
}

/// StableId 32자리 소문자 hex.
pub fn id_hex(id: StableId) -> String {
    id.to_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// 토큰·ContextKey를 합친 집합에서 CSS 이름을 파생합니다(순수 함수, 입력 순서와 무관한 결과).
pub fn derive_names(inputs: &[NameInput<'_>]) -> DerivedNames {
    let bases: Vec<String> = inputs.iter().map(|i| base_name(i.name, i.kind)).collect();
    let mut groups: BTreeMap<&str, Vec<StableId>> = BTreeMap::new();
    for (input, base) in inputs.iter().zip(&bases) {
        groups.entry(base.as_str()).or_default().push(input.id);
    }
    let mut collisions = Vec::new();
    for (css_name, ids) in &mut groups {
        ids.sort_unstable();
        ids.dedup();
        if ids.len() > 1 {
            collisions.push(NameCollision {
                css_name: (*css_name).to_string(),
                ids: ids.clone(),
            });
        }
    }
    let names = inputs
        .iter()
        .zip(&bases)
        .map(|(input, base)| {
            let first = groups[base.as_str()][0];
            let css_name = if first == input.id {
                base.clone()
            } else {
                format!("{base}-{}", id_hex(input.id))
            };
            DerivedName {
                id: input.id,
                kind: input.kind,
                css_name,
            }
        })
        .collect();
    DerivedNames { names, collisions }
}

/// 등록 custom property의 `syntax`(속성 모델 10장). 등록하지 않는 타입(String·Gradient·Shadow·Typography·
/// Boolean·Enum·ResourceRef)은 `None`입니다.
pub fn registered_syntax(value_type: &ExpressionType) -> Option<&'static str> {
    Some(match value_type {
        ExpressionType::Length => "<length>",
        ExpressionType::Percentage => "<percentage>",
        ExpressionType::Angle => "<angle>",
        ExpressionType::Duration => "<time>",
        ExpressionType::Number => "<number>",
        ExpressionType::Color => "<color>",
        _ => return None,
    })
}

/// literal 값의 CSS 텍스트(정규형). CSS 값 텍스트가 없는 타입은 `None`입니다.
pub fn literal_text(value: &TypedValue) -> Option<String> {
    Some(match value {
        TypedValue::Length(v) => format_length(v.get()),
        TypedValue::Percentage(v) => format_percentage(v.get()),
        TypedValue::Angle(v) => format_angle(v.get()),
        TypedValue::Duration(v) => format_duration(v.get()),
        TypedValue::Number(v) => format_number(v.get()),
        TypedValue::Color(c) => format_color(*c),
        _ => return None,
    })
}

/// 토큰의 `@property` 규칙. `initial-value`는 default expression을 기본 환경에서 평가한 literal이어야 하며
/// `var()`를 담지 않습니다. 등록하지 않는 타입이거나 literal 타입이 맞지 않으면 `None`입니다.
pub fn property_rule(
    css_name: &str,
    value_type: &ExpressionType,
    initial: &TypedValue,
) -> Option<String> {
    let syntax = registered_syntax(value_type)?;
    let matches = matches!(
        (value_type, initial),
        (ExpressionType::Length, TypedValue::Length(_))
            | (ExpressionType::Percentage, TypedValue::Percentage(_))
            | (ExpressionType::Angle, TypedValue::Angle(_))
            | (ExpressionType::Duration, TypedValue::Duration(_))
            | (ExpressionType::Number, TypedValue::Number(_))
            | (ExpressionType::Color, TypedValue::Color(_))
    );
    if !matches {
        return None;
    }
    let initial = literal_text(initial)?;
    Some(format!(
        "@property {css_name} {{\n  syntax: \"{syntax}\";\n  inherits: true;\n  initial-value: {initial};\n}}\n"
    ))
}

/// `:root` 선언 값.
#[derive(Debug, Clone, PartialEq)]
pub enum RootValue {
    /// literal 값.
    Literal(TypedValue),
    /// 다른 토큰 참조(alias). 값은 참조 대상의 CSS 이름입니다.
    Alias(String),
}

/// `:root { … }` 블록. 선언은 입력 순서이고, alias는 `var(--other)`로만 냅니다. CSS 값 텍스트가 없는 literal은
/// 건너뜁니다.
pub fn root_block(declarations: &[(String, RootValue)]) -> String {
    let mut out = String::from(":root {\n");
    for (name, value) in declarations {
        let text = match value {
            RootValue::Literal(literal) => match literal_text(literal) {
                Some(text) => text,
                None => continue,
            },
            RootValue::Alias(other) => format!("var({other})"),
        };
        out.push_str(&format!("  {name}: {text};\n"));
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::value::{parse_with, CssGrammar, CssValue};
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::value::Length;

    fn token<'a>(name: &'a str, id: StableId) -> NameInput<'a> {
        NameInput {
            name,
            kind: CustomPropertyKind::Token,
            id,
        }
    }

    fn context<'a>(name: &'a str, id: StableId) -> NameInput<'a> {
        NameInput {
            name,
            kind: CustomPropertyKind::ContextKey,
            id,
        }
    }

    #[test]
    fn name_derivation_golden() {
        // 검증 기준(IX.11.16): 이름 파생 golden.
        let cases = [
            (
                "color/brand.primary",
                CustomPropertyKind::Token,
                "--color-brand-primary",
            ),
            ("Space 2", CustomPropertyKind::Token, "--Space-2"),
            ("radius.lg", CustomPropertyKind::Token, "--radius-lg"),
            ("1x", CustomPropertyKind::Token, "--1x"),
            ("a+b", CustomPropertyKind::Token, "--a\\+b"),
            ("50%", CustomPropertyKind::Token, "--50\\%"),
            ("한글 토큰", CustomPropertyKind::Token, "--한글-토큰"),
            ("tab\tname", CustomPropertyKind::Token, "--tab-name"),
            ("bell\u{7}", CustomPropertyKind::Token, "--bell\\7 "),
            ("accent", CustomPropertyKind::ContextKey, "--context-accent"),
            (
                "theme/mode",
                CustomPropertyKind::ContextKey,
                "--context-theme-mode",
            ),
        ];
        for (name, kind, expected) in cases {
            assert_eq!(base_name(name, kind), expected, "{name}");
        }
    }

    #[test]
    fn collisions_between_tokens_and_with_context_keys_get_id_suffixes() {
        // 검증 기준(IX.11.16): 토큰끼리와 토큰·ContextKey 사이 충돌 진단 fixture.
        let a = StableId::new_v7(1_000, 1, 1);
        let b = StableId::new_v7(1_000, 1, 2);
        let c = StableId::new_v7(2_000, 1, 3);
        let d = StableId::new_v7(3_000, 1, 4);
        let derived = derive_names(&[
            token("brand.primary", b),
            token("brand/primary", a),
            token("context/accent", c),
            context("accent", d),
        ]);
        assert_eq!(derived.collisions.len(), 2);
        assert_eq!(derived.collisions[0].css_name, "--brand-primary");
        assert_eq!(derived.collisions[0].ids, vec![a, b]);
        assert_eq!(derived.collisions[0].code(), "name-collision");
        assert_eq!(derived.collisions[1].css_name, "--context-accent");
        assert_eq!(derived.collisions[1].ids, vec![c, d]);
        assert_eq!(
            derived.name_of(a),
            Some("--brand-primary"),
            "StableId 순 첫 항목"
        );
        assert_eq!(
            derived.name_of(b).unwrap(),
            format!("--brand-primary-{}", id_hex(b))
        );
        assert_eq!(derived.name_of(c), Some("--context-accent"));
        assert_eq!(
            derived.name_of(d).unwrap(),
            format!("--context-accent-{}", id_hex(d))
        );
        // 입력 순서가 달라도 같은 결과입니다.
        let reversed = derive_names(&[
            context("accent", d),
            token("context/accent", c),
            token("brand/primary", a),
            token("brand.primary", b),
        ]);
        for id in [a, b, c, d] {
            assert_eq!(reversed.name_of(id), derived.name_of(id));
        }
    }

    #[test]
    fn ids_created_within_65_seconds_get_different_suffixes() {
        // 검증 기준(IX.11.16): 65초 안에 만든 StableId 두 개의 접미사가 다름(앞 8자리는 같음).
        let first = StableId::new_v7(1_700_000_000_000, 0, 7);
        let second = StableId::new_v7(1_700_000_000_000 + 30_000, 0, 8);
        assert_eq!(
            id_hex(first)[..8],
            id_hex(second)[..8],
            "앞 8자리는 같을 수 있음"
        );
        let other = StableId::new_v7(1, 0, 1);
        let derived = derive_names(&[token("x", other), token("x", first), token("x", second)]);
        let a = derived.name_of(first).unwrap();
        let b = derived.name_of(second).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), "--x-".len() + 32);
    }

    #[test]
    fn rename_changes_only_the_css_name_and_var_resolves_by_id() {
        // 검증 기준(IX.11.16): rename 후 StableId 참조 유지, parser var() 연결.
        let id = StableId::new_v7(5, 0, 5);
        let before = derive_names(&[token("brand", id)]);
        let after = derive_names(&[token("brand/main", id)]);
        assert_eq!(before.name_of(id), Some("--brand"));
        assert_eq!(after.name_of(id), Some("--brand-main"));
        let color = CssGrammar::Color { allow_none: false };
        let CssValue::Var(var) = parse_with(color, "var(--brand-main)", &after).unwrap() else {
            panic!("var");
        };
        assert_eq!(var.token, id, "참조는 StableId");
        assert_eq!(
            parse_with(color, "var(--brand)", &after).unwrap_err().code,
            "unknown-token"
        );
        // ContextKey 이름은 문서 값의 var()로 풀지 않습니다.
        let with_context = derive_names(&[context("accent", id)]);
        assert!(with_context.resolve("--context-accent").is_none());
    }

    #[test]
    fn property_rules_inherit_and_use_literal_initial_values() {
        // 검증 기준(IX.11.16): `@property` 선언 golden(`inherits: true`).
        let red = Color::new(ColorSpace::Srgb, 1., 0., 0., 1.).unwrap();
        assert_eq!(
            property_rule("--brand", &ExpressionType::Color, &TypedValue::Color(red)).unwrap(),
            "@property --brand {\n  syntax: \"<color>\";\n  inherits: true;\n  initial-value: #ff0000;\n}\n"
        );
        assert_eq!(
            property_rule(
                "--space-2",
                &ExpressionType::Length,
                &TypedValue::Length(Length::new(8.).unwrap())
            )
            .unwrap(),
            "@property --space-2 {\n  syntax: \"<length>\";\n  inherits: true;\n  initial-value: 8px;\n}\n"
        );
        assert!(property_rule(
            "--x",
            &ExpressionType::String,
            &TypedValue::String("a".into())
        )
        .is_none());
        assert!(property_rule("--x", &ExpressionType::Length, &TypedValue::Color(red)).is_none());
    }

    #[test]
    fn alias_tokens_use_var_only_in_root_declarations() {
        // 검증 기준(IX.11.16): alias 토큰 golden(`initial-value`는 literal이고 `var()`는 `:root` 선언에만).
        let red = Color::new(ColorSpace::Srgb, 1., 0., 0., 1.).unwrap();
        // alias `--accent`의 default expression은 `--brand`를 참조하고, 기본 환경 평가 결과는 #ff0000입니다.
        let rule =
            property_rule("--accent", &ExpressionType::Color, &TypedValue::Color(red)).unwrap();
        assert!(!rule.contains("var("), "{rule}");
        let root = root_block(&[
            ("--brand".into(), RootValue::Literal(TypedValue::Color(red))),
            ("--accent".into(), RootValue::Alias("--brand".into())),
        ]);
        assert_eq!(
            root,
            ":root {\n  --brand: #ff0000;\n  --accent: var(--brand);\n}\n"
        );
    }
}
