//! XXII.4.2 검증: control semantics·접근성 Compose 매핑, layout 규칙 완전성과 규칙별 진단(static fixture).

use std::collections::BTreeSet;

use super::tests::{compose, static_check, GOLDEN};
use super::*;
use crate::apple::layout_tests::layout_fixture;
use crate::apple::ui_tests::check_golden;
use crate::fixture::{sid, Fixture};

fn layout_codes(export: &ComposeExport) -> BTreeSet<(u8, &'static str, &'static str)> {
    export
        .notes
        .iter()
        .filter_map(|note| match (&note.subject, note.kind) {
            (AndroidNoteSubject::Node(id), AndroidNoteKind::Layout(finding)) => {
                Some((id.to_bytes()[15], finding.longhand, finding.code))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn controls_and_accessibility_map_to_compose_semantics() {
    // 검증 기준(XXII.4.2) static fixture: control semantics와 접근성 의미가 Compose API로 나옵니다.
    let export = compose(&Fixture::new());
    let source = &export.source;
    // Button: clickable, 비활성 상태는 enabled, 눌림 표시 없음.
    assert!(source.contains(
        ".clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, enabled = !(state.isSaving), role = Role.Button) { onAction(\"00000000-0000-0000-0000-00000000003c\") }"
    ));
    // radio: selectable(선택 식) + 선택 상태 쓰기, 그룹은 selectableGroup.
    assert!(source.contains(
        ".selectable(selected = state.selectedSize == \"00000000-0000-0000-0000-000000000008\", interactionSource = remember { MutableInteractionSource() }, indication = null, role = Role.RadioButton) { onAction(\"00000000-0000-0000-0000-00000000003d\"); state.selectedSize = \"00000000-0000-0000-0000-000000000008\" }"
    ));
    assert!(source.contains(".selectableGroup()"));
    // 이름·역할·dismiss·숨김·읽기 순서.
    assert!(source.contains(
        ".semantics { role = Role.Image; contentDescription = \"프로필 사진\"; traversalIndex = 0f }"
    ));
    assert!(
        source.contains(".semantics(mergeDescendants = true) { contentDescription = \"크게\" }")
    );
    assert!(source.contains(
        "paneTitle = \"알림\"; dismiss { onAction(\"00000000-0000-0000-0000-00000000003f\"); true }"
    ));
    assert!(source.contains(".clearAndSetSemantics {}"));
    assert!(source.contains(".semantics { isTraversalGroup = true }"));
    // dialog 역할과 modal 상태는 paneTitle 근사.
    assert!(export.notes.contains(&AndroidNote {
        subject: AndroidNoteSubject::Node(sid(16)),
        kind: AndroidNoteKind::Approximated(Facet::Role),
    }));
    static_check(source);
}

#[test]
fn reading_order_needs_compose_ui_1_6() {
    let fixture = Fixture::new();
    let mut profile = crate::fixture::android_profile();
    if let stellar_target_profile::spec::ProfileSpec::Android(android) = &mut profile.spec {
        android.ui_system_version = stellar_target_profile::spec::DottedVersion::new(1, 5, 0);
    }
    let export = export_compose(&fixture.ir(&profile), &ComposeOptions::default()).expect("export");
    assert!(!export.source.contains("traversalIndex"));
    assert!(export.notes.contains(&AndroidNote {
        subject: AndroidNoteSubject::Node(sid(1)),
        kind: AndroidNoteKind::Unavailable(Facet::ReadingOrder),
    }));
    static_check(&export.source);
}

#[test]
fn compose_rules_cover_every_longhand() {
    // XXII.1.4 Android 열 중 CSS layout longhand 부분: 모든 longhand에 Compose 규칙이 하나씩 있습니다.
    let expected: BTreeSet<&str> = stellar_layout::declared::LONGHANDS
        .iter()
        .copied()
        .collect();
    let names: BTreeSet<&str> = layout::COMPOSE_RULES
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(names, expected);
    assert_eq!(names.len(), layout::COMPOSE_RULES.len(), "중복 규칙");
}

#[test]
fn layout_findings_follow_compose_rules() {
    // 검증 기준(XXII.4.2): 1:1이 아닌 layout 값은 Compose 규칙의 상태로 진단합니다.
    let export = compose(&layout_fixture());
    let codes = layout_codes(&export);
    let expected: BTreeSet<(u8, &str, &str)> = [
        (1, "flex-shrink", "platform-compression"),
        (1, "justify-content", "spread-gap"),
        (3, "width", "percent-size"),
        (4, "flex-shrink", "shrink-ratio"),
        (4, "min-width", "intrinsic-limit"),
        (5, "flex-direction", "reverse-reading-order"),
        (8, "grid-column-start", "grid-placement"),
        (8, "grid-template-columns", "fr-auto-minimum"),
        (11, "aspect-ratio", "aspect-ratio"),
        (11, "transform", "draw-transform"),
        (16, "position", "static-position"),
    ]
    .into_iter()
    .collect();
    assert_eq!(codes, expected);
    let source = &export.source;
    check_golden(GOLDEN, "layout_compose.kt", source);
    // wrap은 FlowRow(Native), fr 비율은 weight 그대로, reverse는 출력 순서 반전.
    assert!(source.starts_with("@file:OptIn(ExperimentalLayoutApi::class)\n"));
    assert!(source.contains("    FlowRow(\n"));
    assert!(source.contains(".weight(2f)"));
    assert!(source.contains(
        "        Node00000000000000000000000000000007(state, onAction)\n        Node00000000000000000000000000000006(state, onAction)"
    ));
    // content-box는 padding 뒤 크기, absolute 오른쪽·아래 inset은 BottomEnd 정렬과 음수 offset.
    assert!(source.contains(
        "            .padding(start = 16.dp, top = 16.dp, end = 16.dp, bottom = 16.dp)\n            .width(360.dp)\n            .height(640.dp)"
    ));
    assert!(source
        .contains(".align(Alignment.BottomEnd)\n            .offset(x = (-8).dp, y = (-8).dp)"));
    // relative inset offset과 회전 그리기 변환.
    assert!(source.contains(".offset(x = 8.dp, y = 4.dp)"));
    assert!(source.contains(
        "transform(Matrix(floatArrayOf(0f, 1f, 0f, 0f, -1f, 0f, 0f, 0f, 0f, 0f, 1f, 0f, 0f, 0f, 0f, 1f)))"
    ));
    // grid: 고정 열 폭, 행 높이, display none 제외.
    assert!(source.contains(".width(100.dp)"));
    assert!(source.contains("                .height(40.dp)\n                .fillMaxWidth(),"));
    assert!(source.contains("Spacer(Modifier.weight(2f))"));
    assert!(!source.contains("Node0000000000000000000000000000000c"));
    // baseline row는 alignByBaseline.
    assert!(source.contains(".alignByBaseline()"));
    static_check(source);
}
