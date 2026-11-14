//! XXII.4.2 검증: Compose golden(static fixture)과 Kotlin 정적 검사(통과·위반별).

use stellar_layout::input::ContainerLayout;
use stellar_layout::stack::StackWrap;
use stellar_target_profile::spec::{DottedVersion, ProfileSpec};

use super::check::{check_kotlin, ComposeIssue};
use super::*;
use crate::fixture::{self as fx, sid, Fixture};

fn options() -> ComposeOptions {
    ComposeOptions {
        package: "com.example.generated".to_string(),
        function_name: "SettingsView".to_string(),
    }
}

fn export(
    fixture: &Fixture,
    profile: &stellar_target_profile::spec::TargetProfile,
) -> ComposeExport {
    export_compose(&fixture.ir(profile), &options()).expect("export")
}

#[test]
fn android_profile_maps_to_golden_compose_and_passes_static_check() {
    // 검증 기준(XXII.4.2) static fixture: layout·control semantics·접근성을 매핑한 Compose 소스가 golden과 같고
    // 정적 검사(import·사용 형태·괄호·Compose UI 버전·opt-in) issue가 없습니다.
    let fixture = Fixture::new();
    let first = export(&fixture, &fx::android_profile());
    assert_eq!(first, export(&fixture, &fx::android_profile()), "결정론");
    fx::check_golden("compose_android.kt", &first.source);
    assert_eq!(check_kotlin(&first.source, first.ui_version), Vec::new());
}

#[test]
fn layout_control_and_accessibility_use_compose_api() {
    let fixture = Fixture::new();
    let export = export(&fixture, &fx::android_profile());
    let source = &export.source;
    // layout: root Column(gap 16, cross stretch → 자식 fillMaxWidth), radio 줄 Row(gap 8), Overlay Box, Grid.
    assert!(source
        .contains("    Column(\n        modifier = Modifier\n            .size(390.dp, 844.dp)"));
    assert!(source.contains("verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.Top),"));
    assert!(source.contains("horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.Start),"));
    // 자식 없는 Overlay Box는 content 없는 overload라 contentAlignment를 쓰지 않습니다.
    assert!(!source.contains("contentAlignment"));
    assert!(source.contains("columns = GridCells.Fixed(2),"));
    assert!(source.contains(".widthIn(min = 120.dp)"));
    assert!(source
        .contains(".absolutePadding(left = 16.dp, top = 12.dp, right = 16.dp, bottom = 12.dp)"));
    assert!(source.contains(".offset(x = 16.dp, y = 16.dp)"));
    // control semantics: radio selectable, 버튼 clickable(enabled = !isSaving), radio group, text field, dialog.
    assert!(source.contains(
        ".selectable(selected = selectedSize == \"00000000-0000-0000-0000-000000000008\", role = Role.RadioButton, onClick = { onAction(\"00000000-0000-0000-0000-00000000003d\"); selectedSize = \"00000000-0000-0000-0000-000000000008\" })"
    ));
    assert!(source.contains(".clickable(enabled = !(isSaving), role = Role.Button, onClick = { onAction(\"00000000-0000-0000-0000-00000000003c\") })"));
    assert!(source.contains(".selectableGroup()"));
    assert!(source.contains("BasicTextField(\n"));
    assert!(source.contains("onValueChange = { memo = it },"));
    assert!(source.contains("Dialog(\n"));
    assert!(source.contains("paneTitle = \"알림\""));
    // 접근성: 이름, 이미지 role, 숨김, 읽기 순서(이미지가 먼저).
    assert!(source.contains("contentDescription = \"프로필 사진\""));
    assert!(source.contains("role = Role.Image"));
    assert!(source.contains(".clearAndSetSemantics { }"));
    assert!(source.contains("isTraversalGroup = true"));
    assert!(source.contains("traversalIndex = 0f"));
    assert!(source.contains(
        "var selectedSize by remember { mutableStateOf(\"00000000-0000-0000-0000-000000000008\") }"
    ));
    // 노트: flex grow + hug 근사, 이미지 리소스 없음.
    assert!(export.notes.contains(&ComposeNote {
        node: sid(8),
        kind: ComposeNoteKind::Approximated(ComposeFacet::Layout(LayoutFacet::Flex)),
    }));
    assert!(export.notes.contains(&ComposeNote {
        node: sid(4),
        kind: ComposeNoteKind::NoEquivalent(ComposeFacet::Layout(LayoutFacet::Content)),
    }));
}

#[test]
fn compose_ui_version_gates_traversal_semantics() {
    let fixture = Fixture::new();
    let mut old = fx::android_profile();
    if let ProfileSpec::Android(android) = &mut old.spec {
        android.ui_system_version = DottedVersion::new(1, 4, 0);
    }
    let export = export(&fixture, &old);
    assert!(!export.source.contains("traversalIndex"));
    assert!(!export.source.contains("isTraversalGroup"));
    assert!(export.notes.contains(&ComposeNote {
        node: sid(1),
        kind: ComposeNoteKind::Unavailable(ComposeFacet::Semantics(Facet::ReadingOrder)),
    }));
    assert_eq!(check_kotlin(&export.source, export.ui_version), Vec::new());
    // 1.7 소스를 1.4 기준으로 검사하면 새 API가 걸립니다.
    let new =
        super::export_compose(&fixture.ir(&fx::android_profile()), &options()).expect("export");
    let issues = check_kotlin(&new.source, (1, 4));
    assert!(issues.contains(&ComposeIssue::RequiresNewerCompose {
        symbol: "traversalIndex",
        since: (1, 5)
    }));
    assert!(issues.contains(&ComposeIssue::RequiresNewerCompose {
        symbol: "isTraversalGroup",
        since: (1, 5)
    }));
}

#[test]
fn wrapping_stack_uses_flow_layout_with_opt_in() {
    let mut fixture = Fixture::new();
    for node in &mut fixture.layout.nodes {
        if node.id == sid(7) {
            if let ContainerLayout::Stack(stack) = &mut node.container {
                stack.wrap = StackWrap::Wrap;
                stack.cross_gap = fx::len(4.0);
            }
        }
    }
    let export = export(&fixture, &fx::android_profile());
    assert!(export
        .source
        .contains("@OptIn(ExperimentalLayoutApi::class)\n@Composable"));
    assert!(export.source.contains("FlowRow(\n"));
    assert!(export
        .source
        .contains("import androidx.compose.foundation.layout.FlowRow\n"));
    assert_eq!(check_kotlin(&export.source, export.ui_version), Vec::new());
    // opt-in을 지우면 정적 검사가 잡습니다.
    let without = export
        .source
        .replace("@OptIn(ExperimentalLayoutApi::class)\n", "");
    assert!(
        check_kotlin(&without, export.ui_version).contains(&ComposeIssue::MissingOptIn("FlowRow"))
    );
}

#[test]
fn static_check_detects_each_violation() {
    let fixture = Fixture::new();
    let source = export(&fixture, &fx::android_profile()).source;
    let ui = (1, 7);
    let cases: Vec<(&str, String, ComposeIssue)> = vec![
        (
            "package 없음",
            source.replacen("package com.example.generated\n", "", 1),
            ComposeIssue::MissingPackage,
        ),
        (
            "import 누락",
            source.replacen("import androidx.compose.foundation.selection.selectableGroup\n", "", 1),
            ComposeIssue::MissingImport("selectableGroup"),
        ),
        (
            "표에 없는 import",
            source.replacen(
                "import androidx.compose.ui.Alignment\n",
                "import androidx.compose.ui.Alignment\nimport androidx.compose.material3.Text\n",
                1,
            ),
            ComposeIssue::UnknownImport("androidx.compose.material3.Text".to_string()),
        ),
        (
            "중복 import",
            source.replacen(
                "import androidx.compose.ui.Alignment\n",
                "import androidx.compose.ui.Alignment\nimport androidx.compose.ui.Alignment\n",
                1,
            ),
            ComposeIssue::DuplicateImport("androidx.compose.ui.Alignment".to_string()),
        ),
        (
            "쓰지 않는 import",
            source.replacen(
                "import androidx.compose.ui.Alignment\n",
                "import androidx.compose.ui.Alignment\nimport androidx.compose.ui.semantics.focused\n",
                1,
            ),
            ComposeIssue::UnusedImport("androidx.compose.ui.semantics.focused".to_string()),
        ),
        (
            "닫는 괄호 누락",
            {
                let index = source.rfind('}').expect("}");
                let mut broken = source.clone();
                broken.remove(index);
                broken
            },
            ComposeIssue::Unbalanced {
                line: source.lines().count(),
                delimiter: '{',
            },
        ),
        (
            "닫히지 않은 문자열",
            source.replacen("paneTitle = \"알림\"", "paneTitle = \"알림", 1),
            ComposeIssue::UnterminatedString {
                line: source
                    .lines()
                    .position(|line| line.contains("paneTitle = \"알림\""))
                    .map(|i| i + 1)
                    .unwrap_or_default(),
            },
        ),
    ];
    for (name, broken, expected) in cases {
        let issues = check_kotlin(&broken, ui);
        assert!(issues.contains(&expected), "{name}: {issues:?}");
    }
    // import 순서를 바꾸면 정렬 위반입니다.
    let mut lines: Vec<&str> = source.lines().collect();
    let first = lines
        .iter()
        .position(|l| l.starts_with("import "))
        .expect("import");
    lines.swap(first, first + 1);
    let unsorted = lines.join("\n");
    assert!(check_kotlin(&unsorted, ui).contains(&ComposeIssue::UnsortedImports));
}

#[test]
fn invalid_targets_and_options_are_rejected() {
    let fixture = Fixture::new();
    let web = fixture.ir(&fx::web_profile());
    assert_eq!(
        export_compose(&web, &options()),
        Err(ComposeError::NotAndroidProfile)
    );
    let android = fixture.ir(&fx::android_profile());
    for package in ["", "com..x", "com.1x", "com.val"] {
        let options = ComposeOptions {
            package: package.to_string(),
            function_name: "View".to_string(),
        };
        assert_eq!(
            export_compose(&android, &options),
            Err(ComposeError::InvalidPackage),
            "{package}"
        );
    }
    for name in ["", "1View", "my-view", "fun"] {
        let options = ComposeOptions {
            package: "a.b".to_string(),
            function_name: name.to_string(),
        };
        assert_eq!(
            export_compose(&android, &options),
            Err(ComposeError::InvalidFunctionName),
            "{name}"
        );
    }
}
