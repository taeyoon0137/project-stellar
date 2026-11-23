//! XXII.4.1 검증: Compose golden source(static fixture), 정적 구조 검사, 옵션 오류.
//!
//! Kotlin 컴파일러가 DEV-MAC에 없어 compile fixture 대신 정적 검사를 씁니다. 괄호 짝, Kotlin 문법 검사기
//! ([`super::kotlin_check`]), import 표 검사(import가 표의 정규 경로이고 표의 package가 Compose artifact에 있음), import한
//! 이름이 모두 본문에 쓰였는지, 본문이 쓴 Compose 이름(import 표에 있는 이름)이 모두 import되었는지 확인합니다.

use std::collections::BTreeSet;

use super::*;
use crate::apple::ui_tests::check_golden;
use crate::fixture::{self as fx, sid, Fixture};

pub(crate) const GOLDEN: &str = "src/android/golden";

pub(crate) fn compose(fixture: &Fixture) -> ComposeExport {
    export_compose(
        &fixture.ir(&fx::android_profile()),
        &ComposeOptions::default(),
    )
    .expect("export")
}

/// 문자열 literal 밖에서 괄호 짝이 맞는지.
fn balanced(source: &str) -> bool {
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in source.chars() {
        if in_string {
            match (escaped, ch) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' | '{' | '[' => stack.push(ch),
            ')' | '}' | ']' => {
                let open = match ch {
                    ')' => '(',
                    '}' => '{',
                    _ => '[',
                };
                if stack.pop() != Some(open) {
                    return false;
                }
            }
            _ => {}
        }
    }
    stack.is_empty() && !in_string
}

/// 문자열 literal을 지운 본문의 식별자 집합.
fn identifiers_outside_strings(source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in source.chars() {
        if in_string {
            match (escaped, ch) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else {
            if !current.is_empty() {
                out.insert(std::mem::take(&mut current));
            }
            if ch == '"' {
                in_string = true;
            }
        }
    }
    if !current.is_empty() {
        out.insert(current);
    }
    out
}

/// 정적 검사: 괄호 짝, import 이름 사용, 사용 이름 import.
pub(crate) fn static_check(source: &str) {
    assert!(balanced(source), "괄호 짝이 맞지 않습니다:\n{source}");
    // compile fixture 대체(ADR-0121 결정 6): 문법 검사기와 import 표 검사.
    if let Err(error) = super::kotlin_check::check_syntax(source) {
        panic!("Kotlin 문법 오류 {error:?}:\n{source}");
    }
    if let Err(error) = super::kotlin_check::check_imports(source) {
        panic!("import 표 검사 실패 {error}:\n{source}");
    }
    let imports: BTreeSet<&str> = source
        .lines()
        .filter_map(|line| line.strip_prefix("import "))
        .map(|path| path.rsplit('.').next().expect("이름"))
        .collect();
    let body: String = source
        .lines()
        .filter(|line| !line.starts_with("import ") && !line.starts_with("package "))
        .collect::<Vec<_>>()
        .join("\n");
    let mut used = identifiers_outside_strings(&body);
    // `by` 위임 속성은 getValue·setValue import를 이름 없이 씁니다.
    if used.contains("by") {
        used.insert("getValue".to_string());
        used.insert("setValue".to_string());
    }
    for name in &imports {
        assert!(
            used.contains(*name),
            "import한 {name}을 쓰지 않습니다:\n{source}"
        );
    }
    for (name, _) in compose::IMPORTS {
        // scope 멤버(weight 등)는 import 표에 없습니다. 표에 있는 이름을 쓰면 import가 있어야 합니다. `role`·`selected`는
        // `clickable`·`selectable`의 이름 있는 인자와 철자가 같아 이 방향 검사에서 뺍니다.
        if matches!(*name, "role" | "selected") {
            continue;
        }
        if used.contains(*name) {
            assert!(
                imports.contains(name),
                "{name}을 import하지 않았습니다:\n{source}"
            );
        }
    }
}

#[test]
fn compose_source_matches_golden_static_fixture() {
    // 검증 기준(XXII.4.1) golden source: 같은 IR·옵션의 Compose 소스가 golden과 byte 단위로 같습니다.
    let fixture = Fixture::new();
    let first = compose(&fixture);
    assert_eq!(first, compose(&fixture), "결정론");
    check_golden(GOLDEN, "android_compose.kt", &first.source);
    static_check(&first.source);
}

#[test]
fn compose_output_hoists_state_and_maps_elements() {
    let export = compose(&Fixture::new());
    let source = &export.source;
    assert!(source.starts_with("package stellar.generated\n"));
    // 상태 holder와 root composable.
    assert!(source.contains(
        "class StellarState {\n    var selectedSize by mutableStateOf(\"00000000-0000-0000-0000-000000000008\")\n    var memo by mutableStateOf(\"\")\n    var isSaving by mutableStateOf(false)\n}"
    ));
    assert!(source.contains("fun StellarView(\n    state: StellarState = remember { StellarState() },\n    onAction: (String) -> Unit = {},\n)"));
    // node 함수는 부모 container scope를 receiver로 받습니다.
    assert!(source.contains(
        "private fun ColumnScope.Node00000000000000000000000000000002(state: StellarState, onAction: (String) -> Unit)"
    ));
    assert!(source.contains("    BasicText(\n        text = \"설정\","));
    // TextInput은 상태를 읽고 쓰는 BasicTextField와 placeholder.
    assert!(
        source.contains("    value = state.memo,\n        onValueChange = { state.memo = it },")
    );
    assert!(source.contains("if (state.memo.isEmpty()) BasicText(text = \"메모\")"));
    // 색: token은 P3, token에 묶인 속성은 token 상수.
    assert!(source.contains("    val surface = Color(red = 0.96f, green = 0.96f, blue = 0.98f, alpha = 1f, colorSpace = ColorSpaces.DisplayP3)"));
    assert!(source.contains("    val n0000000000000000000000000000000c_4 = accent"));
    // android profile에서는 node 13(확장 범위 sRGB)도 나옵니다.
    assert!(source.contains("ColorSpaces.ExtendedSrgb"));
    assert!(export.notes.contains(&AndroidNote {
        subject: AndroidNoteSubject::Node(sid(13)),
        kind: AndroidNoteKind::Color(ColorNote::ExtendedRange),
    }));
    // SF Symbol은 Android에 없어 이미지 원본 노트, 도형·Modal 노트.
    for (id, note) in [
        (4, ControlNote::ImageSourceUnavailable),
        (5, ControlNote::ShapeGeometryUnavailable),
        (16, ControlNote::ModalInPlace),
    ] {
        assert!(export.notes.contains(&AndroidNote {
            subject: AndroidNoteSubject::Node(sid(id)),
            kind: AndroidNoteKind::Control(note),
        }));
    }
}

#[test]
fn invalid_targets_and_options_are_rejected() {
    let fixture = Fixture::new();
    assert_eq!(
        export_compose(&fixture.ir(&fx::ios_profile()), &ComposeOptions::default()),
        Err(ComposeError::NotAndroidProfile)
    );
    let ir = fixture.ir(&fx::android_profile());
    for package in ["", "Stellar", "a..b", "a.val", "1a", "a-b"] {
        let options = ComposeOptions {
            package: package.to_string(),
            ..ComposeOptions::default()
        };
        assert_eq!(
            export_compose(&ir, &options),
            Err(ComposeError::InvalidPackage),
            "{package}"
        );
    }
    let options = ComposeOptions {
        type_prefix: "1A".to_string(),
        ..ComposeOptions::default()
    };
    assert_eq!(
        export_compose(&ir, &options),
        Err(ComposeError::InvalidTypePrefix)
    );
}

#[test]
fn static_check_detects_broken_sources() {
    assert!(balanced("f(\"(\") { g() }"));
    assert!(!balanced("f({)}"));
    let missing = std::panic::catch_unwind(|| static_check("package a\n\nfun f() { Row() }\n"));
    assert!(missing.is_err());
    let unused =
        std::panic::catch_unwind(|| static_check("package a\n\nimport x.Row\n\nfun f() {}\n"));
    assert!(unused.is_err());
}

#[test]
fn kotlin_syntax_checker_catches_mutations_of_real_output() {
    // 문법 검사기가 실제 출력을 읽고, 실제 출력에 넣은 문법 오류를 잡는지 확인합니다(ADR-0121 결정 6).
    let source = compose(&Fixture::new()).source;
    super::kotlin_check::check_syntax(&source).expect("실제 출력");
    let mutations: [(&str, &str); 6] = [
        (
            "horizontalAlignment = Alignment.Start,",
            "horizontalAlignment = ,",
        ),
        (".width(390.dp)", ".width(390.dp"),
        (
            "state: StellarState, onAction",
            "state: StellarState onAction",
        ),
        ("var memo by mutableStateOf(\"\")", "var memo by"),
        (
            "@Composable\nfun StellarView(",
            "@Composable\nfun (StellarView(",
        ),
        ("Modifier.weight(1f)", "Modifier.weight(1f) 2f"),
    ];
    for (from, to) in mutations {
        assert!(source.contains(from), "{from}");
        let broken = source.replacen(from, to, 1);
        assert!(
            super::kotlin_check::check_syntax(&broken).is_err(),
            "{from} → {to}"
        );
    }
}
