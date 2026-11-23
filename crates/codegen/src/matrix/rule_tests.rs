//! XXII.1.4 검증: Apple·Android 규칙별 입력·출력·진단 fixture, Web 항목 불변, 표 결정론.

use std::fmt::Write as _;

use stellar_target_profile::capability::CapabilityStatus;
use stellar_target_profile::css_matrix::{web_capability, MappingInput, RegistryInput};
use stellar_target_profile::profile::TargetProfileKind;

use super::css::{apply, platform, Rule, CSS_LOWERINGS};
use super::{css_entry, css_matrix_text};
use crate::fixture::check_golden;

/// 규칙마다 쓰는 대표 입력 값.
fn sample(rule: Rule) -> &'static str {
    match rule {
        Rule::FrameSize
        | Rule::CornerShape
        | Rule::AbsoluteOffset
        | Rule::LineSpacing
        | Rule::BorderOverlay
        | Rule::PathBake => "12px",
        Rule::Modifier | Rule::SrgbComposite => "0.5",
        Rule::FlexItem => "1",
        Rule::Visibility => "false",
        Rule::StackLayout | Rule::GridLayout | Rule::ExplicitDefault => "center",
        Rule::AspectRatio => "1.5",
        Rule::ClipShape | Rule::MaskLayer => "url(#m)",
        Rule::FontAttribute => "16px",
        Rule::ImageScaling => "cover",
        Rule::TransformEffect => "matrix(1, 0, 0, 1, 4, 8)",
        Rule::Structural => "\"작게\"",
        Rule::GroupKey | Rule::ModifierChain | Rule::Rasterize | Rule::NoOutput => "-",
    }
}

#[test]
fn every_rule_has_input_output_and_diagnostic_fixture() {
    // 검증 기준(XXII.1.4): 규칙별 입력·출력·진단 fixture. 규칙마다 그 규칙을 쓰는 첫 항목(Apple, 없으면 Android)에
    // 대표 입력을 넣은 출력과 짝 진단을 golden과 비교합니다.
    let mut out = String::new();
    for rule in Rule::ALL {
        let mut used = false;
        for row in CSS_LOWERINGS {
            for (kind, lowering) in [("apple", &row.apple), ("android", &row.android)] {
                if lowering.rule != rule || used {
                    continue;
                }
                used = true;
                let input = sample(rule);
                let output = apply(lowering, input);
                let _ = writeln!(
                    out,
                    "{} {kind} {}/{} input={input:?} output={:?} diagnostic={}",
                    rule.id(),
                    row.scope,
                    row.name,
                    output.code,
                    output.diagnostic.unwrap_or("-")
                );
            }
        }
        assert!(used, "규칙 {}을 쓰는 항목이 없습니다", rule.id());
    }
    check_golden("matrix_rules.txt", &out);
}

#[test]
fn rule_application_examples() {
    let row = |scope: &str, name: &str| {
        CSS_LOWERINGS
            .iter()
            .find(|r| r.scope == scope && r.name == name)
            .expect("row")
    };
    let width = row("node", "width");
    assert_eq!(apply(&width.apple, "120px").code, ".frame(width: 120)");
    assert_eq!(
        apply(&width.android, "120px").code,
        "Modifier.width(120.dp)"
    );
    assert_eq!(apply(&width.android, "120px").diagnostic, None);
    // 자유 배치는 근사이고 진단이 짝으로 붙습니다.
    let left = row("node", "left");
    let out = apply(&left.apple, "8px");
    assert_eq!(out.code, ".offset(x: 8)");
    assert_eq!(out.diagnostic, Some("absolute-offset"));
    assert_eq!(left.apple.status, CapabilityStatus::Approximated);
    // 기본값 차이: stack spacing은 항상 명시합니다.
    let gap = row("frame", "row-gap");
    assert_eq!(apply(&gap.apple, "0px").code, "VStack(spacing: 0)");
    assert_eq!(
        apply(&gap.apple, "0px").diagnostic,
        Some("explicit-default")
    );
    // 블렌드 합성 색공간 차이.
    let blend = row("modifier", "mix-blend-mode");
    assert_eq!(
        apply(&blend.android, "multiply").diagnostic,
        Some("srgb-composite")
    );
    // modifier 적용 순서는 chain 선언 순서로 보존합니다(Native, 정보 진단).
    let order = row("modifier", "modifier-order");
    assert_eq!(order.apple.status, CapabilityStatus::Native);
    assert_eq!(
        apply(&order.apple, "-").diagnostic,
        Some("modifier-chain-order")
    );
    // 표시 여부는 숨김일 때만 냅니다.
    let visible = row("node", "visible");
    assert_eq!(apply(&visible.apple, "true").code, "");
    assert_eq!(apply(&visible.apple, "false").code, ".hidden()");
    // 출력 없음.
    let locked = row("node", "locked");
    assert_eq!(apply(&locked.android, "true").code, "");
    assert_eq!(locked.android.status, CapabilityStatus::Unsupported);
}

#[test]
fn web_entries_are_the_ix_11_19_matrix_and_platform_entries_come_from_rules() {
    // 검증 기준(XXII.1.4): Web 항목 상태가 IX.11.19 matrix와 같음. Apple·Android 항목은 이 표의 규칙에서 나옵니다.
    for row in CSS_LOWERINGS {
        for status in [
            MappingInput::Exact,
            MappingInput::Converted,
            MappingInput::Approximated,
            MappingInput::StellarOnly,
        ] {
            let input = RegistryInput {
                scope: row.scope,
                name: row.name,
                status,
            };
            assert_eq!(
                css_entry(input, TargetProfileKind::Web).status,
                Some(web_capability(input).status)
            );
            for kind in [TargetProfileKind::Apple, TargetProfileKind::Android] {
                let entry = css_entry(input, kind);
                let lowering = platform(row, kind).expect("platform");
                // CSS 매핑 상태와 무관하게 규칙이 정합니다(ADR-0025 결정 1.3).
                assert_eq!(entry.status, Some(lowering.status));
                assert_eq!(entry.rule, lowering.rule.id());
                assert_eq!(entry.diagnostic, lowering.rule.diagnostic());
            }
        }
    }
}

#[test]
fn table_has_one_row_per_registry_item_and_is_stable() {
    let mut keys: Vec<(&str, &str)> = CSS_LOWERINGS.iter().map(|r| (r.scope, r.name)).collect();
    let total = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), total, "같은 항목이 두 번 있습니다");
    // 1:1이 아닌 규칙은 진단이 짝으로 있습니다(Native가 아닌 항목).
    for row in CSS_LOWERINGS {
        for lowering in [&row.apple, &row.android] {
            if lowering.status != CapabilityStatus::Native {
                assert!(
                    lowering.rule.diagnostic().is_some(),
                    "{}/{}: 근사·미지원 규칙에 진단이 없습니다",
                    row.scope,
                    row.name
                );
            }
        }
    }
    check_golden("matrix_css.txt", &css_matrix_text());
}

#[test]
fn every_context_and_motion_item_has_a_rule_for_every_profile_kind() {
    // 검증 기준(XXII.1.7): Context·MotionSpec 항목에 profile kind별 상태·규칙이 있는지 완전성 검사. Native가 아닌
    // 항목은 진단이 짝으로 있습니다.
    use super::behavior::{behavior_entry, BehaviorSubject, ContextFeature, MotionFeature};
    let subjects: Vec<BehaviorSubject> = MotionFeature::ALL
        .into_iter()
        .map(BehaviorSubject::Motion)
        .chain(
            ContextFeature::ALL
                .into_iter()
                .map(BehaviorSubject::Context),
        )
        .collect();
    for subject in &subjects {
        for kind in TargetProfileKind::ALL {
            let entry = behavior_entry(*subject, kind)
                .unwrap_or_else(|| panic!("{} {} 규칙 없음", subject.name(), kind.name()));
            if entry.status != CapabilityStatus::Native {
                assert!(
                    entry.diagnostic.is_some(),
                    "{} {}: 진단 없음",
                    subject.name(),
                    kind.name()
                );
            }
        }
    }
    assert_eq!(
        super::behavior::BEHAVIOR_LOWERINGS.len(),
        subjects.len(),
        "항목마다 행 하나"
    );
    // dialect 열(SwiftUI·AppKit·Compose)도 모든 항목에 있고, SwiftUI·Compose는 Apple·Android 열 그대로입니다.
    use super::behavior::dialect_entry;
    use super::dialect::Dialect;
    for subject in &subjects {
        for dialect in Dialect::ALL {
            let entry = dialect_entry(*subject, dialect).expect("dialect 열");
            if entry.status != CapabilityStatus::Native {
                assert!(
                    entry.diagnostic.is_some(),
                    "{} {}",
                    subject.name(),
                    dialect.name()
                );
            }
        }
        assert_eq!(
            dialect_entry(*subject, Dialect::SwiftUi),
            behavior_entry(*subject, TargetProfileKind::Apple)
        );
        assert_eq!(
            dialect_entry(*subject, Dialect::Compose),
            behavior_entry(*subject, TargetProfileKind::Android)
        );
    }
}

#[test]
fn adapters_follow_the_context_rules_of_the_matrix() {
    // ADR-0121 결정 4: Compose local 선택 규칙과 AppKit Environment 부재는 matrix의 Context 규칙과 같습니다.
    use super::behavior::{dialect_entry, BehaviorSubject, ContextFeature};
    use super::dialect::Dialect;
    use crate::android::context::{local_kind, LocalKind, ValueChange};
    let rule =
        |feature, dialect| dialect_entry(BehaviorSubject::Context(feature), dialect).expect("규칙");
    let fixed = rule(ContextFeature::ValueFixed, Dialect::Compose);
    let changing = rule(ContextFeature::ValueChanging, Dialect::Compose);
    assert_eq!(local_kind([ValueChange::Fixed]), LocalKind::Static);
    assert!(fixed.template.starts_with(LocalKind::Static.function()));
    assert_eq!(
        local_kind([ValueChange::Fixed, ValueChange::Changing]),
        LocalKind::Tracked
    );
    assert!(changing.template.starts_with(LocalKind::Tracked.function()));
    // provision이 없으면 key default만 쓰여 바뀌지 않습니다.
    assert_eq!(local_kind([]), LocalKind::Static);
    // AppKit은 Environment가 없어 IR이 해석한 값을 주입합니다.
    use crate::apple::context::{environment_key, environment_names, ContextLoweringError};
    use crate::apple::AppleDialect;
    let names = environment_names("Stellar", "theme");
    assert_eq!(
        environment_key(AppleDialect::AppKit, &names, &crate::fixture::text("a")),
        Err(ContextLoweringError::NoEnvironment(AppleDialect::AppKit))
    );
    for feature in ContextFeature::ALL {
        let appkit = rule(feature, Dialect::AppKit);
        assert!(appkit.rule.starts_with("appkit-"), "{}", feature.name());
        let swiftui = rule(feature, Dialect::SwiftUi);
        assert!(!swiftui.rule.starts_with("appkit-"));
    }
}

#[test]
fn context_and_motion_rules_have_input_output_and_diagnostic_fixture() {
    // 검증 기준(XXII.1.7): 규칙별 입력·출력·진단 fixture. spring과 Context 전달 방식처럼 1:1이 아닌 항목은 진단이
    // 함께 나옵니다.
    use super::behavior::{
        apply, behavior_entry, BehaviorSubject, MotionFeature, BEHAVIOR_LOWERINGS,
    };
    let sample = |subject: BehaviorSubject| match subject {
        BehaviorSubject::Motion(MotionFeature::TimingCubicBezier) => "0.25, 0.1, 0.25, 1",
        BehaviorSubject::Motion(MotionFeature::TimingSteps) => "4, jump-end",
        BehaviorSubject::Motion(MotionFeature::TimingSpring) => "0, 0.52, 0.94, 1.03, 1",
        BehaviorSubject::Motion(MotionFeature::Delay) => "120ms",
        BehaviorSubject::Motion(_) => "opacity",
        BehaviorSubject::Context(_) => "--context-accent",
    };
    let mut out = String::new();
    for row in BEHAVIOR_LOWERINGS {
        for (kind, lowering) in [
            ("web", &row.web),
            ("apple", &row.apple),
            ("appkit", &row.appkit),
            ("android", &row.android),
        ] {
            let input = sample(row.subject);
            let _ = writeln!(
                out,
                "{} {kind} {} rule={} input={input:?} output={:?} diagnostic={}",
                row.subject.name(),
                lowering.status.name(),
                lowering.rule,
                apply(lowering, input),
                lowering.diagnostic.unwrap_or("-")
            );
        }
    }
    check_golden("matrix_behavior.txt", &out);
    let spring = behavior_entry(
        BehaviorSubject::Motion(MotionFeature::TimingSpring),
        TargetProfileKind::Web,
    )
    .expect("spring");
    assert_eq!(spring.status, CapabilityStatus::Approximated);
    assert_eq!(spring.diagnostic, Some("motion-spring-approximated"));
}

#[test]
fn context_and_motion_items_leave_css_items_unchanged() {
    // 검증 기준(XXII.1.7): XXII.1.4 항목 불변. CSS 기반 속성 표 텍스트는 XXII.1.4 커밋의 golden(`matrix_css.txt`)과
    // 같고, Context·MotionSpec 이름은 CSS 표에 없습니다.
    use super::behavior::BEHAVIOR_LOWERINGS;
    let css = css_matrix_text();
    let committed = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/matrix_css.txt"),
    )
    .expect("golden");
    assert_eq!(css, committed);
    for row in BEHAVIOR_LOWERINGS {
        assert!(!css.contains(row.subject.name()));
    }
}
