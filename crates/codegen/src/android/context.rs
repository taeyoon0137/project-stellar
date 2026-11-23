//! Context → CompositionLocal 출력 형태와 추적형·비추적형 선택 규칙 (XXII.4.3 일부, ADR-0107 결정 7).
//!
//! ContextKey 하나는 top-level `val Local<접두어><이름>`이 됩니다. provision은
//! `CompositionLocalProvider(Local… provides 값) { … }`, 소비는 `Local….current`입니다. CompositionLocal은 렌더 트리로
//! 전달되므로 slot 삽입 위치의 다시 공급(ADR-0025 결정 3.11)도 같은 provider로 냅니다.
//!
//! # local 선택 규칙
//!
//! key의 export scope 안 provision 값이 모두 [`ValueChange::Fixed`]이면 `staticCompositionLocalOf`(비추적형), 하나라도
//! [`ValueChange::Changing`]이면 `compositionLocalOf`(추적형)입니다. provision이 없으면(key default만 쓰임) 값이 바뀌지
//! 않으므로 비추적형입니다.
//!
//! - Fixed: View가 보이는 동안 바뀌지 않는 값입니다. literal, 선택 profile로 해석한 token, key default가 여기에 듭니다.
//! - Changing: 상태 변수를 읽거나 실행 중 바뀌는 환경 값(예: `interaction_state`)을 읽는 값입니다.
//!
//! 비추적형은 읽기 추적 비용이 없고 값이 바뀌면 공급 subtree 전체를 다시 구성합니다. 추적형은 읽은 곳만 다시
//! 구성합니다. 바뀌지 않는 값에 추적형을 쓰면 추적 비용만 들고, 자주 바뀌는 값에 비추적형을 쓰면 subtree 전체를 다시
//! 구성하므로 위 규칙이 두 경우를 모두 피합니다.
//!
//! 이 module은 key 선언·공급·읽기의 Kotlin 형태와 선택 규칙만 정합니다. provision 값이 Fixed인지 Changing인지와 소비
//! 지점의 출처는 CodegenIR의 Context 정보(XXII.1.6, XXX.7.4 해석 결과)가 정합니다.

use stellar_core_types::typed_value::TypedValue;

use crate::android::color::color_expression;
use crate::android::kotlin::{dp, string_literal};
use crate::text::{identifier, num};

/// provision 값의 변경 빈도.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueChange {
    /// View가 보이는 동안 바뀌지 않음.
    Fixed,
    /// 상태·실행 중 환경 값을 읽어 바뀜.
    Changing,
}

/// CompositionLocal 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LocalKind {
    /// `staticCompositionLocalOf`(비추적형).
    Static,
    /// `compositionLocalOf`(추적형).
    Tracked,
}

impl LocalKind {
    /// 생성 함수 이름.
    pub const fn function(self) -> &'static str {
        match self {
            LocalKind::Static => "staticCompositionLocalOf",
            LocalKind::Tracked => "compositionLocalOf",
        }
    }
}

/// key의 provision 값 변경 빈도로 local 종류를 고릅니다.
pub fn local_kind(provisions: impl IntoIterator<Item = ValueChange>) -> LocalKind {
    if provisions
        .into_iter()
        .any(|change| change == ValueChange::Changing)
    {
        LocalKind::Tracked
    } else {
        LocalKind::Static
    }
}

/// 값의 Kotlin 타입과 literal(Kotlin으로 낼 수 없는 타입은 `None`).
pub fn kotlin_value(value: &TypedValue) -> Option<(&'static str, String)> {
    match value {
        TypedValue::String(text) => Some(("String", string_literal(text))),
        TypedValue::Boolean(flag) => Some(("Boolean", flag.to_string())),
        TypedValue::Number(n) => {
            let text = num(n.get());
            Some((
                "Double",
                if text.contains('.') {
                    text
                } else {
                    format!("{text}.0")
                },
            ))
        }
        TypedValue::Length(length) => Some(("Dp", dp(length.get()))),
        TypedValue::Color(color) => Some(("Color", color_expression(color).0)),
        _ => None,
    }
}

/// key의 local 이름.
pub fn local_name(prefix: &str, key_name: &str) -> String {
    let mut base = identifier(key_name, "context");
    if let Some(first) = base.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    format!("Local{prefix}{base}")
}

/// local 선언.
pub fn composition_local(name: &str, kind: LocalKind, default: &TypedValue) -> Option<String> {
    let (ty, literal) = kotlin_value(default)?;
    Some(format!(
        "val {name} = {}<{ty}> {{ {literal} }}",
        kind.function()
    ))
}

/// provision(또는 slot 다시 공급)의 provider 머리 줄.
pub fn provide(name: &str, value_expression: &str) -> String {
    format!("CompositionLocalProvider({name} provides {value_expression}) {{")
}

/// 소비 식.
pub fn read(name: &str) -> String {
    format!("{name}.current")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{srgb, text};

    #[test]
    fn local_selection_rule_follows_value_change() {
        // 검증 기준(XXII.4.3) local 선택 규칙 fixture.
        assert_eq!(local_kind([]), LocalKind::Static);
        assert_eq!(
            local_kind([ValueChange::Fixed, ValueChange::Fixed]),
            LocalKind::Static
        );
        assert_eq!(
            local_kind([ValueChange::Fixed, ValueChange::Changing]),
            LocalKind::Tracked
        );
        assert_eq!(local_kind([ValueChange::Changing]), LocalKind::Tracked);
    }

    #[test]
    fn context_keys_lower_to_composition_locals() {
        let name = local_name("Stellar", "theme");
        assert_eq!(name, "LocalStellarTheme");
        assert_eq!(
            composition_local(&name, LocalKind::Static, &text("light")).as_deref(),
            Some("val LocalStellarTheme = staticCompositionLocalOf<String> { \"light\" }")
        );
        assert_eq!(
            composition_local(&name, LocalKind::Tracked, &srgb(1.0, 0.0, 0.0, 1.0)).as_deref(),
            Some("val LocalStellarTheme = compositionLocalOf<Color> { Color(red = 1f, green = 0f, blue = 0f, alpha = 1f, colorSpace = ColorSpaces.Srgb) }")
        );
        assert_eq!(
            provide(&name, "state.theme"),
            "CompositionLocalProvider(LocalStellarTheme provides state.theme) {"
        );
        assert_eq!(read(&name), "LocalStellarTheme.current");
        assert_eq!(
            kotlin_value(&crate::fixture::number(2.0)),
            Some(("Double", "2.0".to_string()))
        );
    }
}
