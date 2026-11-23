//! 토큰 내보내기 정보: alias와 환경 조건부 값 (XXII.2.3, ADR-0106 결정 1.5·5).
//!
//! 토큰 평가기가 환경을 바꿔 평가한 결과를 그대로 받습니다. 환경 key·값은 문서 평가 환경(VIII.7)의 지역 거울이고
//! 상위가 옮깁니다. 기본 환경 값(`light`·`standard`·reduced motion 꺼짐·`default`)은 조건으로 쓰지 않습니다.

use stellar_core_types::typed_value::TypedValue;

/// `color_scheme`의 기본이 아닌 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IrColorScheme {
    /// `light`(기본).
    Light,
    /// `dark`.
    Dark,
}

/// `contrast` 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IrContrast {
    /// `standard`(기본).
    Standard,
    /// `high`.
    High,
}

/// `interaction_state` 값(IX.6 Control 상태).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IrInteractionState {
    /// `default`(기본).
    Default,
    /// `hover`.
    Hover,
    /// `pressed`.
    Pressed,
    /// `focused`.
    Focused,
    /// `disabled`.
    Disabled,
}

impl IrInteractionState {
    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            IrInteractionState::Default => "default",
            IrInteractionState::Hover => "hover",
            IrInteractionState::Pressed => "pressed",
            IrInteractionState::Focused => "focused",
            IrInteractionState::Disabled => "disabled",
        }
    }
}

/// 환경 key 하나의 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IrEnvironmentValue {
    /// `color_scheme`.
    ColorScheme(IrColorScheme),
    /// `contrast`.
    Contrast(IrContrast),
    /// `reduced_motion`.
    ReducedMotion(bool),
    /// `interaction_state`.
    InteractionState(IrInteractionState),
    /// Web 토큰 선언으로 표현하지 않는 key(`viewport_*`·`density`·`locale` 등, numeric key).
    Other(u32),
}

impl IrEnvironmentValue {
    /// key 순서(조건 정렬·조합용).
    pub const fn key_rank(self) -> u32 {
        match self {
            IrEnvironmentValue::ColorScheme(_) => 0,
            IrEnvironmentValue::Contrast(_) => 1,
            IrEnvironmentValue::ReducedMotion(_) => 2,
            IrEnvironmentValue::InteractionState(_) => 3,
            IrEnvironmentValue::Other(key) => key.saturating_add(4),
        }
    }

    /// 기본 환경 값인지.
    pub const fn is_default(self) -> bool {
        matches!(
            self,
            IrEnvironmentValue::ColorScheme(IrColorScheme::Light)
                | IrEnvironmentValue::Contrast(IrContrast::Standard)
                | IrEnvironmentValue::ReducedMotion(false)
                | IrEnvironmentValue::InteractionState(IrInteractionState::Default)
        )
    }

    /// 안정 표기.
    pub fn text(self) -> String {
        match self {
            IrEnvironmentValue::ColorScheme(IrColorScheme::Light) => "color_scheme=light".into(),
            IrEnvironmentValue::ColorScheme(IrColorScheme::Dark) => "color_scheme=dark".into(),
            IrEnvironmentValue::Contrast(IrContrast::Standard) => "contrast=standard".into(),
            IrEnvironmentValue::Contrast(IrContrast::High) => "contrast=high".into(),
            IrEnvironmentValue::ReducedMotion(on) => format!("reduced_motion={on}"),
            IrEnvironmentValue::InteractionState(state) => {
                format!("interaction_state={}", state.name())
            }
            IrEnvironmentValue::Other(key) => format!("env({key})"),
        }
    }
}

/// 환경 조건 조합 하나에서의 토큰 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrTokenVariant {
    /// 조건(기본 값 제외, key 순, key마다 하나).
    pub when: Vec<IrEnvironmentValue>,
    /// 그 조합에서 평가한 값.
    pub value: TypedValue,
}

/// 조건을 정규화합니다: 기본 값을 빼고 key 순으로 정렬합니다. key가 두 번 나오면 `None`입니다.
pub fn normalize_when(when: &[IrEnvironmentValue]) -> Option<Vec<IrEnvironmentValue>> {
    let mut keys: Vec<u32> = when.iter().map(|v| v.key_rank()).collect();
    keys.sort_unstable();
    if keys.windows(2).any(|w| w[0] == w[1]) {
        return None;
    }
    let mut out: Vec<IrEnvironmentValue> =
        when.iter().copied().filter(|v| !v.is_default()).collect();
    out.sort_by_key(|v| (v.key_rank(), *v));
    Some(out)
}
