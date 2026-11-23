//! Context·`MotionSpec` 항목의 profile kind별 상태와 lowering 규칙 (XXII.1.7, ADR-0106 결정 3).
//!
//! CSS 기반 속성 항목([`super::css`])은 바꾸지 않습니다. 1:1이 아닌 항목(spring 같은 timing function 종류, Context
//! 전달 방식)은 규칙과 진단을 한 쌍으로 둡니다. template의 `{v}`는 규칙 입력 텍스트입니다.
//!
//! Apple 열은 SwiftUI dialect이고 AppKit dialect는 따로 한 열입니다. AppKit에는 렌더 트리 전달 값이 없어 IR이 해석한
//! 값을 생성자로 주입합니다. Compose의 추적형·비추적형 CompositionLocal 선택은 값 변경 빈도 항목(`ValueFixed`·
//! `ValueChanging`)이 정하고 `android::context::local_kind`가 그 규칙을 따릅니다(ADR-0121 결정 4).

use stellar_target_profile::capability::CapabilityStatus;
use stellar_target_profile::profile::TargetProfileKind;

use crate::ir::{IrReducedMotion, IrTiming};

/// `MotionSpec` 항목(닫힌 목록).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MotionFeature {
    /// `linear`.
    TimingLinear,
    /// `cubic-bezier()`.
    TimingCubicBezier,
    /// `steps()`.
    TimingSteps,
    /// 감쇠 spring.
    TimingSpring,
    /// reduced motion: 즉시 끝값.
    ReducedInstant,
    /// reduced motion: 그대로 재생.
    ReducedPlay,
    /// 시작 조건: 값이 된다.
    StartBecomes,
    /// 시작 조건: 값이 바뀐다.
    StartChanges,
    /// 지연.
    Delay,
    /// Color 보간(encoded premultiplied, ADR-0025 결정 1.2).
    ColorInterpolation,
}

impl MotionFeature {
    /// 모든 항목.
    pub const ALL: [MotionFeature; 10] = [
        MotionFeature::TimingLinear,
        MotionFeature::TimingCubicBezier,
        MotionFeature::TimingSteps,
        MotionFeature::TimingSpring,
        MotionFeature::ReducedInstant,
        MotionFeature::ReducedPlay,
        MotionFeature::StartBecomes,
        MotionFeature::StartChanges,
        MotionFeature::Delay,
        MotionFeature::ColorInterpolation,
    ];

    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            MotionFeature::TimingLinear => "timing-linear",
            MotionFeature::TimingCubicBezier => "timing-cubic-bezier",
            MotionFeature::TimingSteps => "timing-steps",
            MotionFeature::TimingSpring => "timing-spring",
            MotionFeature::ReducedInstant => "reduced-motion-instant",
            MotionFeature::ReducedPlay => "reduced-motion-play",
            MotionFeature::StartBecomes => "start-becomes",
            MotionFeature::StartChanges => "start-changes",
            MotionFeature::Delay => "delay",
            MotionFeature::ColorInterpolation => "color-interpolation",
        }
    }

    /// timing function의 항목.
    pub const fn of_timing(timing: &IrTiming) -> MotionFeature {
        match timing {
            IrTiming::Linear => MotionFeature::TimingLinear,
            IrTiming::CubicBezier { .. } => MotionFeature::TimingCubicBezier,
            IrTiming::Steps { .. } => MotionFeature::TimingSteps,
            IrTiming::Spring { .. } => MotionFeature::TimingSpring,
        }
    }

    /// reduced motion의 항목.
    pub const fn of_reduced(reduced: IrReducedMotion) -> MotionFeature {
        match reduced {
            IrReducedMotion::Instant => MotionFeature::ReducedInstant,
            IrReducedMotion::Play => MotionFeature::ReducedPlay,
        }
    }
}

/// Context 항목(닫힌 목록).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContextFeature {
    /// CSS 값 텍스트가 있는 스타일 값 전달.
    StyleValue,
    /// 그 밖의 값 전달.
    NonStyleValue,
    /// 같은 key를 다시 읽는 provision.
    SelfRead,
    /// provision 깊이가 instance 위치마다 다름.
    DepthVaries,
    /// slot 삽입 위치의 비공개 key 재공급(ADR-0025 결정 3.11).
    SlotResupply,
    /// key의 provision 값이 모두 View가 보이는 동안 바뀌지 않음(literal, 해석한 token, key default).
    ValueFixed,
    /// key의 provision 값 하나라도 상태·실행 중 환경 값을 읽어 바뀜.
    ValueChanging,
}

impl ContextFeature {
    /// 모든 항목.
    pub const ALL: [ContextFeature; 7] = [
        ContextFeature::StyleValue,
        ContextFeature::NonStyleValue,
        ContextFeature::SelfRead,
        ContextFeature::DepthVaries,
        ContextFeature::SlotResupply,
        ContextFeature::ValueFixed,
        ContextFeature::ValueChanging,
    ];

    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ContextFeature::StyleValue => "context-style-value",
            ContextFeature::NonStyleValue => "context-non-style-value",
            ContextFeature::SelfRead => "context-self-read",
            ContextFeature::DepthVaries => "context-depth-varies",
            ContextFeature::SlotResupply => "context-slot-resupply",
            ContextFeature::ValueFixed => "context-value-fixed",
            ContextFeature::ValueChanging => "context-value-changing",
        }
    }
}

/// Context·MotionSpec 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BehaviorSubject {
    /// MotionSpec 항목.
    Motion(MotionFeature),
    /// Context 항목.
    Context(ContextFeature),
}

impl BehaviorSubject {
    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            BehaviorSubject::Motion(feature) => feature.name(),
            BehaviorSubject::Context(feature) => feature.name(),
        }
    }
}

/// profile 하나의 항목 규칙.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BehaviorLowering {
    /// 상태.
    pub status: CapabilityStatus,
    /// 규칙 id.
    pub rule: &'static str,
    /// 대상 API template.
    pub template: &'static str,
    /// 짝 진단 code(1:1이면 `None`).
    pub diagnostic: Option<&'static str>,
}

/// 항목 하나의 Web·Apple·Android 규칙.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BehaviorRow {
    /// 항목.
    pub subject: BehaviorSubject,
    /// Web.
    pub web: BehaviorLowering,
    /// Apple(SwiftUI, Apple profile kind의 기준 dialect).
    pub apple: BehaviorLowering,
    /// AppKit dialect(ADR-0121 결정 4).
    pub appkit: BehaviorLowering,
    /// Android(Compose).
    pub android: BehaviorLowering,
}

const N: CapabilityStatus = CapabilityStatus::Native;
const A: CapabilityStatus = CapabilityStatus::Approximated;

type Cell = (
    CapabilityStatus,
    &'static str,
    &'static str,
    Option<&'static str>,
);

const fn cell(c: Cell) -> BehaviorLowering {
    BehaviorLowering {
        status: c.0,
        rule: c.1,
        template: c.2,
        diagnostic: c.3,
    }
}

const fn row(
    subject: BehaviorSubject,
    web: Cell,
    apple: Cell,
    appkit: Cell,
    android: Cell,
) -> BehaviorRow {
    BehaviorRow {
        subject,
        web: cell(web),
        apple: cell(apple),
        appkit: cell(appkit),
        android: cell(android),
    }
}

use BehaviorSubject::{Context as C, Motion as M};
use ContextFeature as CF;
use MotionFeature as MF;

/// Context·MotionSpec 규칙 표. 열은 Web, Apple(SwiftUI), AppKit, Android(Compose) 순입니다(ADR-0121 결정 4).
pub const BEHAVIOR_LOWERINGS: &[BehaviorRow] = &[
    row(
        M(MF::TimingLinear),
        (N, "css-easing", "transition-timing-function: linear", None),
        (N, "swiftui-animation", ".linear(duration: {v})", None),
        (
            N,
            "appkit-media-timing",
            "CAMediaTimingFunction(name: .linear)",
            None,
        ),
        (N, "compose-tween", "tween(easing = LinearEasing)", None),
    ),
    row(
        M(MF::TimingCubicBezier),
        (
            N,
            "css-easing",
            "transition-timing-function: cubic-bezier({v})",
            None,
        ),
        (N, "swiftui-animation", ".timingCurve({v}, duration:)", None),
        (
            N,
            "appkit-media-timing",
            "CAMediaTimingFunction(controlPoints: {v})",
            None,
        ),
        (
            N,
            "compose-tween",
            "tween(easing = CubicBezierEasing({v}))",
            None,
        ),
    ),
    row(
        M(MF::TimingSteps),
        (
            N,
            "css-easing",
            "transition-timing-function: steps({v})",
            None,
        ),
        (
            A,
            "swiftui-custom-animation",
            "CustomAnimation steps({v})",
            Some("motion-steps-custom-animation"),
        ),
        (
            N,
            "appkit-discrete-keyframes",
            "CAKeyframeAnimation(calculationMode: .discrete) steps({v})",
            None,
        ),
        (
            N,
            "compose-step-easing",
            "tween(easing = Easing {{ steps({v}) }})",
            None,
        ),
    ),
    row(
        M(MF::TimingSpring),
        (
            A,
            "css-linear-samples",
            "transition-timing-function: linear({v})",
            Some("motion-spring-approximated"),
        ),
        (
            A,
            "swiftui-interpolating-spring",
            ".interpolatingSpring({v})",
            Some("motion-spring-integration"),
        ),
        (
            A,
            "appkit-spring-animation",
            "CASpringAnimation({v})",
            Some("motion-spring-integration"),
        ),
        (
            A,
            "compose-spring",
            "spring(dampingRatio, stiffness)({v})",
            Some("motion-spring-normalized"),
        ),
    ),
    row(
        M(MF::ReducedInstant),
        (
            N,
            "reduced-motion-media",
            "@media (prefers-reduced-motion: reduce) {{ transition-property: none }}",
            None,
        ),
        (
            N,
            "swiftui-reduce-motion",
            "@Environment(\\.accessibilityReduceMotion)",
            None,
        ),
        (
            N,
            "appkit-reduce-motion",
            "NSWorkspace.shared.accessibilityDisplayShouldReduceMotion",
            None,
        ),
        (
            A,
            "android-animator-scale",
            "ANIMATOR_DURATION_SCALE == 0",
            Some("reduced-motion-system-scale"),
        ),
    ),
    row(
        M(MF::ReducedPlay),
        (N, "no-change", "", None),
        (N, "no-change", "", None),
        (N, "no-change", "", None),
        (N, "no-change", "", None),
    ),
    row(
        M(MF::StartBecomes),
        (
            N,
            "state-selector-transition",
            "[data-s-{v}] rule",
            Some("motion-start-selector"),
        ),
        (
            N,
            "swiftui-on-change",
            ".onChange(of: {v}) {{ withAnimation }}",
            None,
        ),
        (
            N,
            "appkit-state-setter",
            "setState({v}) + NSAnimationContext.runAnimationGroup",
            None,
        ),
        (
            N,
            "compose-animate-as-state",
            "animate*AsState(target = {v})",
            None,
        ),
    ),
    row(
        M(MF::StartChanges),
        (N, "element-transition", "transition-property: {v}", None),
        (
            N,
            "swiftui-animation-value",
            ".animation(_, value: {v})",
            None,
        ),
        (
            N,
            "appkit-state-setter",
            "didSet {v} + NSAnimationContext.runAnimationGroup",
            None,
        ),
        (
            N,
            "compose-animate-as-state",
            "animate*AsState(target = {v})",
            None,
        ),
    ),
    row(
        M(MF::Delay),
        (N, "css-delay", "transition-delay: {v}", None),
        (N, "swiftui-delay", ".delay({v})", None),
        (N, "appkit-begin-time", "CAAnimation.beginTime = {v}", None),
        (N, "compose-delay", "tween(delayMillis = {v})", None),
    ),
    row(
        M(MF::ColorInterpolation),
        (
            A,
            "encoded-color-interpolation",
            "transition on color",
            Some("motion-color-space-approximated"),
        ),
        (
            A,
            "encoded-color-interpolation",
            "Color animation",
            Some("motion-color-space-approximated"),
        ),
        (
            A,
            "encoded-color-interpolation",
            "CABasicAnimation(keyPath: backgroundColor)",
            Some("motion-color-space-approximated"),
        ),
        (
            A,
            "encoded-color-interpolation",
            "animateColorAsState",
            Some("motion-color-space-approximated"),
        ),
    ),
    row(
        C(CF::StyleValue),
        (N, "registered-custom-property", "{v}: <value>", None),
        (N, "environment-value", "EnvironmentValues.{v}", None),
        (N, "appkit-resolved-value", "init(context {v}:)", None),
        (
            N,
            "composition-local-by-change",
            "staticCompositionLocalOf | compositionLocalOf {{ {v} }}",
            Some("composition-local-by-change"),
        ),
    ),
    row(
        C(CF::NonStyleValue),
        (
            N,
            "context-structure",
            "contexts[{v}]",
            Some("context-structure"),
        ),
        (N, "environment-value", "EnvironmentValues.{v}", None),
        (N, "appkit-resolved-value", "init(context {v}:)", None),
        (
            N,
            "composition-local-by-change",
            "staticCompositionLocalOf | compositionLocalOf {{ {v} }}",
            Some("composition-local-by-change"),
        ),
    ),
    row(
        C(CF::SelfRead),
        (
            N,
            "depth-named-property",
            "{v}-<d>: f(var({v}-<d-1>))",
            Some("context-depth-rename"),
        ),
        (
            N,
            "environment-transform",
            ".transformEnvironment(\\.{v})",
            None,
        ),
        (N, "appkit-resolved-value", "init(context {v}:)", None),
        (
            N,
            "composition-local-outer-read",
            "Local{v} provides f(Local{v}.current)",
            None,
        ),
    ),
    row(
        C(CF::DepthVaries),
        (
            A,
            "context-structure-fallback",
            "contexts[{v}]",
            Some("context-depth-varies"),
        ),
        (N, "environment-value", "EnvironmentValues.{v}", None),
        (N, "appkit-resolved-value", "init(context {v}:)", None),
        (
            N,
            "composition-local-by-change",
            "compositionLocalOf {{ {v} }}",
            None,
        ),
    ),
    row(
        C(CF::SlotResupply),
        (
            N,
            "slot-outer-resupply",
            "{v}-outer at instance, {v} at slot host",
            Some("context-slot-resupply"),
        ),
        (
            N,
            "environment-outer-resupply",
            ".environment(\\.{v}, outer)",
            Some("context-slot-resupply"),
        ),
        (N, "appkit-resolved-value", "init(context {v}:)", None),
        (
            N,
            "composition-local-outer-resupply",
            "CompositionLocalProvider(Local{v} provides outer)",
            Some("context-slot-resupply"),
        ),
    ),
    row(
        C(CF::ValueFixed),
        (N, "no-change", "", None),
        (N, "no-change", "", None),
        (N, "appkit-resolved-value", "init(context {v}:)", None),
        (
            N,
            "composition-local-static",
            "staticCompositionLocalOf {{ {v} }}",
            None,
        ),
    ),
    row(
        C(CF::ValueChanging),
        (N, "no-change", "", None),
        (N, "no-change", "", None),
        (
            A,
            "appkit-property-setter",
            "var {v} {{ didSet {{ needsLayout = true }} }}",
            Some("context-manual-propagation"),
        ),
        (
            N,
            "composition-local-tracked",
            "compositionLocalOf {{ {v} }}",
            None,
        ),
    ),
];

/// 항목의 규칙 행.
pub fn find(subject: BehaviorSubject) -> Option<&'static BehaviorRow> {
    BEHAVIOR_LOWERINGS.iter().find(|row| row.subject == subject)
}

/// 항목·profile kind의 규칙.
pub fn behavior_entry(
    subject: BehaviorSubject,
    kind: TargetProfileKind,
) -> Option<&'static BehaviorLowering> {
    let row = find(subject)?;
    Some(match kind {
        TargetProfileKind::Web => &row.web,
        TargetProfileKind::Apple => &row.apple,
        TargetProfileKind::Android => &row.android,
    })
}

/// 항목·dialect의 규칙. SwiftUI는 Apple 열, Compose는 Android 열, AppKit은 AppKit 열입니다.
pub fn dialect_entry(
    subject: BehaviorSubject,
    dialect: super::dialect::Dialect,
) -> Option<&'static BehaviorLowering> {
    use super::dialect::Dialect;
    let row = find(subject)?;
    Some(match dialect {
        Dialect::SwiftUi => &row.apple,
        Dialect::AppKit => &row.appkit,
        Dialect::Compose => &row.android,
    })
}

/// 규칙을 입력 텍스트 하나에 적용합니다(`{v}` 치환, `{{`·`}}`는 중괄호).
pub fn apply(lowering: &BehaviorLowering, input: &str) -> String {
    lowering
        .template
        .replace("{v}", input)
        .replace("{{", "{")
        .replace("}}", "}")
}
