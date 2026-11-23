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
