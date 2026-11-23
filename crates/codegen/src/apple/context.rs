//! Context → SwiftUI Environment 값 출력 형태 (XXII.3.4 일부, ADR-0107 결정 7).
//!
//! ContextKey 하나는 SwiftUI `EnvironmentKey`(default는 ContextKey default 값)와 `EnvironmentValues` 속성 하나가 됩니다.
//! provision은 `.environment(\.<속성>, 값)`, 소비는 `@Environment(\.<속성>)`입니다. Environment는 렌더 트리로 전달되므로
//! slot 삽입 위치의 다시 공급(ADR-0025 결정 3.11)도 같은 `.environment` 호출로 냅니다.
//!
//! 이 module은 key 선언·공급·읽기의 Swift 형태만 정합니다. 어느 node가 어떤 값을 공급하고 소비 지점이 어느 provision을
//! 읽는지는 CodegenIR의 Context 정보(XXII.1.6, XXX.7.4 해석 결과)가 정하며, 이 module이 다시 해석하지 않습니다.
//! AppKit에는 Environment가 없어 [`ContextLoweringError::NoEnvironment`]입니다.

use std::fmt;

use stellar_core_types::typed_value::TypedValue;

use crate::apple::color::{color_expression, ColorApi};
use crate::apple::swift::{number, string_literal};
use crate::apple::AppleDialect;
use crate::text::identifier;

/// Context 출력 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextLoweringError {
    /// dialect에 렌더 트리 전달 값(Environment)이 없습니다.
    NoEnvironment(AppleDialect),
    /// 값 타입을 Swift Environment 값으로 낼 수 없습니다.
    UnsupportedValue,
}

impl fmt::Display for ContextLoweringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextLoweringError::NoEnvironment(dialect) => {
                write!(f, "{dialect:?} dialect에는 Environment가 없습니다")
            }
            ContextLoweringError::UnsupportedValue => {
                f.write_str("이 값 타입은 Swift Environment 값으로 낼 수 없습니다")
            }
        }
    }
}

impl std::error::Error for ContextLoweringError {}

/// 값의 Swift 타입과 literal.
pub fn swift_value(value: &TypedValue) -> Result<(&'static str, String), ContextLoweringError> {
    match value {
        TypedValue::String(text) => Ok(("String", string_literal(text))),
        TypedValue::Boolean(flag) => Ok(("Bool", flag.to_string())),
        TypedValue::Number(n) => Ok(("Double", number(n.get()))),
        TypedValue::Length(length) => Ok(("CGFloat", number(length.get()))),
        TypedValue::Color(color) => Ok((
            "Color",
            color_expression(color, ColorApi::SwiftUi).expression,
        )),
        _ => Err(ContextLoweringError::UnsupportedValue),
    }
}

/// ContextKey 하나의 SwiftUI 출력 이름.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentNames {
    /// `EnvironmentKey` 타입 이름.
    pub key_type: String,
    /// `EnvironmentValues` 속성 이름(접두어를 붙여 SwiftUI 기본 속성과 겹치지 않음).
    pub property: String,
}

/// key 이름에서 출력 이름을 만듭니다.
pub fn environment_names(prefix: &str, key_name: &str) -> EnvironmentNames {
    let base = identifier(key_name, "context");
    let mut upper = base.clone();
    if let Some(first) = upper.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    let mut lower_prefix = prefix.to_string();
    if let Some(first) = lower_prefix.get_mut(0..1) {
        first.make_ascii_lowercase();
    }
    EnvironmentNames {
        key_type: format!("{prefix}{upper}ContextKey"),
        property: format!("{lower_prefix}{upper}"),
    }
}

/// `EnvironmentKey`와 `EnvironmentValues` 확장 선언.
pub fn environment_key(
    dialect: AppleDialect,
    names: &EnvironmentNames,
    default: &TypedValue,
) -> Result<String, ContextLoweringError> {
    if dialect != AppleDialect::SwiftUi {
        return Err(ContextLoweringError::NoEnvironment(dialect));
    }
    let (ty, literal) = swift_value(default)?;
    Ok(format!(
        "struct {key}: EnvironmentKey {{\n    static let defaultValue: {ty} = {literal}\n}}\n\nextension EnvironmentValues {{\n    var {property}: {ty} {{\n        get {{ self[{key}.self] }}\n        set {{ self[{key}.self] = newValue }}\n    }}\n}}\n",
        key = names.key_type,
        property = names.property,
    ))
}

/// provision(또는 slot 다시 공급) modifier.
pub fn provide(names: &EnvironmentNames, value_expression: &str) -> String {
    format!(".environment(\\.{}, {value_expression})", names.property)
}

/// 소비 지점의 속성 선언.
pub fn read(names: &EnvironmentNames, ty: &str) -> String {
    format!(
        "@Environment(\\.{p}) private var {p}: {ty}",
        p = names.property
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{p3, text};

    #[test]
    fn context_keys_lower_to_environment_values() {
        let names = environment_names("Stellar", "accent color");
        assert_eq!(names.key_type, "StellarAccent_colorContextKey");
        assert_eq!(names.property, "stellarAccent_color");
        let source =
            environment_key(AppleDialect::SwiftUi, &names, &p3(0.2, 0.4, 0.6, 1.0)).expect("key");
        assert!(source.contains(
            "static let defaultValue: Color = Color(.displayP3, red: 0.2, green: 0.4, blue: 0.6, opacity: 1)"
        ));
        assert_eq!(
            provide(&names, "Color.red"),
            ".environment(\\.stellarAccent_color, Color.red)"
        );
        assert_eq!(
            read(&names, "Color"),
            "@Environment(\\.stellarAccent_color) private var stellarAccent_color: Color"
        );
        assert_eq!(
            environment_key(AppleDialect::AppKit, &names, &text("a")),
            Err(ContextLoweringError::NoEnvironment(AppleDialect::AppKit))
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn environment_source_typechecks() {
        // 검증 기준(XXII.3.4) compile fixture 일부: key 선언·안쪽 공급이 바깥 공급을 가리는 구조·읽기가 typecheck됩니다.
        let theme = environment_names("Stellar", "theme");
        let mut source = String::from("import SwiftUI\n\n");
        source.push_str(
            &environment_key(AppleDialect::SwiftUi, &theme, &text("light")).expect("key"),
        );
        source.push_str(&format!(
            "\nstruct Reader: View {{\n    {}\n\n    var body: some View {{\n        Text(verbatim: {})\n    }}\n}}\n",
            read(&theme, "String"),
            theme.property
        ));
        source.push_str(&format!(
            "\nstruct Root: View {{\n    var body: some View {{\n        VStack {{\n            Reader()\n                {}\n        }}\n        {}\n    }}\n}}\n",
            provide(&theme, "\"dark\""),
            provide(&theme, "\"light\"")
        ));
        crate::apple::ui_tests::typecheck(
            &source,
            "environment",
            "macosx",
            "arm64-apple-macos13.0",
        );
    }
}
