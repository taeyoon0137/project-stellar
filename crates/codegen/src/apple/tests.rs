//! XXII.3.3 검증: Swift golden(static fixture)과 `swiftc -typecheck` compile fixture.

use stellar_accessibility::node::StateKind;

use super::*;
use crate::fixture::{self as fx, sid, Fixture};

fn export(
    profile: &stellar_target_profile::spec::TargetProfile,
    dialect: AppleDialect,
) -> AppleExport {
    let fixture = Fixture::new();
    let ir = fixture.ir(profile);
    export_apple(&ir, &AppleOptions::new(dialect)).expect("export")
}

#[test]
fn swift_sources_match_golden_static_fixtures() {
    // 검증 기준(XXII.3.3) static fixture: 같은 IR·옵션의 Swift 소스가 golden과 byte 단위로 같습니다.
    for (name, profile, dialect) in [
        (
            "apple_ios_swiftui.swift",
            fx::ios_profile(),
            AppleDialect::SwiftUi,
        ),
        (
            "apple_mac_swiftui.swift",
            fx::mac_profile(),
            AppleDialect::SwiftUi,
        ),
        (
            "apple_mac_appkit.swift",
            fx::mac_profile(),
            AppleDialect::AppKit,
        ),
    ] {
        let first = export(&profile, dialect);
        assert_eq!(first, export(&profile, dialect), "{name} 결정론");
        fx::check_golden(name, &first.source);
    }
}

#[test]
fn color_spaces_and_semantics_use_apple_api() {
    let ios = export(&fx::ios_profile(), AppleDialect::SwiftUi);
    let source = &ios.source;
    // token 색공간: surface는 Display P3, accent는 sRGB. token에 묶인 속성은 token 상수를 가리킵니다.
    assert!(source.contains(
        "static let surface = Color(.displayP3, red: 0.96, green: 0.96, blue: 0.98, opacity: 1)"
    ));
    assert!(source
        .contains("static let accent = Color(.sRGB, red: 0, green: 0.48, blue: 1, opacity: 1)"));
    assert!(source.contains("static let n0000000000000000000000000000000c_4 = accent"));
    // radio 선택은 상태 비교, 저장 버튼 비활성은 상태 binding, dialog는 modal trait와 escape 동작.
    assert!(source.contains(
        ".accessibilityAddTraits(selectedSize == \"00000000-0000-0000-0000-000000000008\" ? .isSelected : [])"
    ));
    assert!(source.contains(".disabled(isSaving)"));
    assert!(source.contains(".accessibilityAddTraits(.isModal)"));
    assert!(source.contains(".accessibilityAction(.escape) {"));
    // 읽기 순서: image(4)가 가장 큰 sort priority.
    assert!(source
        .contains("struct StellarAccessibility00000000000000000000000000000004: ViewModifier"));
    assert!(source.contains(".accessibilitySortPriority(6)"));
    // 장식 shape(5)는 숨김.
    assert!(source.contains(".accessibilityHidden(true)"));
    // text field 역할은 native control 몫, radio group·dialog 역할은 근사.
    let node_notes = |id: u8| -> Vec<AppleNoteKind> {
        ios.notes
            .iter()
            .filter(|n| n.subject == AppleNoteSubject::Node(sid(id)))
            .map(|n| n.kind)
            .collect()
    };
    assert_eq!(
        node_notes(11),
        vec![AppleNoteKind::NativeControlRequired(Facet::Role)]
    );
    assert_eq!(
        node_notes(6),
        vec![AppleNoteKind::Approximated(Facet::Role)]
    );

    // mac에서는 13(확장 범위 sRGB)이 제외되지 않아 gamut 노트가 납니다.
    let mac = export(&fx::mac_profile(), AppleDialect::AppKit);
    assert!(mac.notes.contains(&AppleNote {
        subject: AppleNoteSubject::Node(sid(13)),
        kind: AppleNoteKind::Color(ColorNote::ExtendedRange),
    }));
    assert!(mac.source.contains(
        "NSColor(colorSpace: .extendedSRGB, components: [1.1, 0.5, -0.05, 1], count: 4)"
    ));
    assert!(mac
        .source
        .contains("view.setAccessibilityRole(.radioButton)"));
    assert!(mac
        .source
        .contains("view.setAccessibilityEnabled(!(isSaving))"));
}

#[test]
fn pressed_toggle_respects_minimum_os() {
    // pressed 상태를 가진 버튼: 최소 OS가 낮으면 .isToggle을 쓰지 않고 노트를 남깁니다.
    let mut fixture = Fixture::new();
    let mut tree = stellar_accessibility::tree::AccessibilityTree::new();
    let mut root = stellar_accessibility::node::AccessibilityNode::new(
        sid(1),
        stellar_accessibility::node::AccessibilityRole::Generic,
    );
    root.states.clear();
    let button = stellar_accessibility::node::AccessibilityNode::new(
        sid(12),
        stellar_accessibility::node::AccessibilityRole::Button,
    )
    .with_label(stellar_accessibility::node::AccessibilityBinding::text(
        "굵게",
    ))
    .with_state(
        StateKind::Pressed,
        stellar_accessibility::node::AccessibilityBinding::State(sid(fx::STATE_SAVING)),
    );
    tree.insert(root).expect("root");
    tree.insert(button).expect("button");
    tree.push_root(sid(1));
    tree.push_child(sid(1), sid(12));
    fixture.accessibility =
        stellar_accessibility::snapshot::AccessibilitySnapshot::from_tree(&tree).expect("snapshot");
    let mut old = fx::ios_profile();
    if let stellar_target_profile::spec::ProfileSpec::Apple(apple) = &mut old.spec {
        apple.minimum_os = stellar_target_profile::spec::DottedVersion::new(16, 4, 0);
    }
    let old_export =
        export_apple(&fixture.ir(&old), &AppleOptions::new(AppleDialect::SwiftUi)).expect("export");
    assert!(!old_export.source.contains(".isToggle"));
    assert!(old_export.notes.contains(&AppleNote {
        subject: AppleNoteSubject::Node(sid(12)),
        kind: AppleNoteKind::Unavailable(Facet::State(StateKind::Pressed)),
    }));
    let new_export = export_apple(
        &fixture.ir(&fx::ios_profile()),
        &AppleOptions::new(AppleDialect::SwiftUi),
    )
    .expect("export");
    assert!(new_export
        .source
        .contains(".accessibilityAddTraits(.isToggle)"));
    assert!(new_export
        .source
        .contains(".accessibilityAddTraits(isSaving ? .isSelected : [])"));
}

#[test]
fn invalid_targets_and_options_are_rejected() {
    let fixture = Fixture::new();
    let web = fixture.ir(&fx::web_profile());
    assert_eq!(
        export_apple(&web, &AppleOptions::new(AppleDialect::SwiftUi)),
        Err(AppleError::NotAppleProfile)
    );
    let ios = fixture.ir(&fx::ios_profile());
    assert_eq!(
        export_apple(&ios, &AppleOptions::new(AppleDialect::AppKit)),
        Err(AppleError::DialectUnavailable {
            dialect: AppleDialect::AppKit,
            platform: ApplePlatform::Ios,
        })
    );
    for prefix in ["", "1A", "A-B", "가"] {
        let options = AppleOptions {
            dialect: AppleDialect::SwiftUi,
            type_prefix: prefix.to_string(),
        };
        assert_eq!(
            export_apple(&ios, &options),
            Err(AppleError::InvalidTypePrefix),
            "{prefix}"
        );
    }
}

/// `xcrun swiftc -typecheck`로 생성 소스를 검사합니다(macOS 개발 환경 전용).
#[cfg(target_os = "macos")]
fn typecheck(source: &str, name: &str, sdk: &str, target: &str) {
    let dir = std::env::temp_dir().join(format!("stellar-codegen-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("임시 디렉터리");
    let file = dir.join("Generated.swift");
    std::fs::write(&file, source).expect("소스 쓰기");
    let output = std::process::Command::new("xcrun")
        .args(["--sdk", sdk, "swiftc", "-typecheck", "-target", target])
        .arg(&file)
        .output();
    let _ = std::fs::remove_dir_all(&dir);
    let output = output.expect("xcrun 실행(DEV-MAC에는 Xcode가 필요합니다)");
    assert!(
        output.status.success(),
        "{name} typecheck 실패:\n{}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[cfg(target_os = "macos")]
fn generated_swift_typechecks_against_apple_sdks() {
    // 검증 기준(XXII.3.3) compile fixture: iOS·macOS SDK로 SwiftUI·AppKit 소스를 typecheck합니다.
    let ios = export(&fx::ios_profile(), AppleDialect::SwiftUi);
    typecheck(
        &ios.source,
        "ios-swiftui",
        "iphonesimulator",
        "arm64-apple-ios26.0-simulator",
    );
    let mac = export(&fx::mac_profile(), AppleDialect::SwiftUi);
    typecheck(
        &mac.source,
        "mac-swiftui",
        "macosx",
        "arm64-apple-macos26.0",
    );
    let mut fixture = Fixture::new();
    let property_name = "n0000000000000000000000000000000c_4";
    let accent = fixture
        .tokens
        .iter_mut()
        .find(|token| token.id == sid(fx::TOKEN_ACCENT))
        .unwrap();
    accent.name = property_name.into();
    for state in &mut fixture.states {
        if state.name == "selectedSize" {
            state.name = "onStateChange".into();
        } else if state.name == "memo" {
            state.name = "views".into();
        }
    }
    let appkit = export_apple(
        &fixture.ir(&fx::mac_profile()),
        &AppleOptions::new(AppleDialect::AppKit),
    )
    .unwrap();
    assert!(appkit.source.contains("onStateChange_2"));
    assert!(appkit.source.contains("views_2"));
    assert!(appkit
        .source
        .contains(&format!("static let {property_name}_2 = {property_name}")));
    typecheck(
        &appkit.source,
        "mac-appkit",
        "macosx",
        "arm64-apple-macos26.0",
    );
}
