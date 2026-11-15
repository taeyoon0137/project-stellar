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
fn fragment_is_transparent_for_hidden_subtrees() {
    // Fragment(2)는 view가 없어 modifier·함수를 만들지 않고, 그 아래 장식 Text(3)를 숨깁니다.
    let fixture = fx::fragment_fixture();
    let swiftui = export_apple(
        &fixture.ir(&fx::ios_profile()),
        &AppleOptions::new(AppleDialect::SwiftUi),
    )
    .expect("export");
    assert!(!swiftui
        .source
        .contains("StellarAccessibility00000000000000000000000000000002"));
    assert!(swiftui.source.contains(
        "struct StellarAccessibility00000000000000000000000000000003: ViewModifier {\n    func body(content: Content) -> some View {\n        content\n            .accessibilityHidden(true)\n"
    ));
    let appkit = export_apple(
        &fixture.ir(&fx::mac_profile()),
        &AppleOptions::new(AppleDialect::AppKit),
    )
    .expect("export");
    assert!(!appkit
        .source
        .contains("apply00000000000000000000000000000002"));
    assert!(appkit.source.contains(
        "static func apply00000000000000000000000000000003(to view: NSView) {\n        view.setAccessibilityElement(false)\n        view.setAccessibilityChildren([])\n"
    ));
}

/// 상태·token 이름을 생성 코드가 쓰는 type·매개변수 이름으로 바꾼 fixture.
fn colliding_names() -> Fixture {
    let mut fixture = Fixture::new();
    for state in &mut fixture.states {
        state.name = match state.name.as_str() {
            "memo" => "Text",
            "selectedSize" => "Content",
            _ => "views",
        }
        .to_string();
    }
    for token in &mut fixture.tokens {
        if token.id == sid(fx::TOKEN_ACCENT) {
            token.name = "Color".to_string();
        } else if token.id == sid(fx::TOKEN_SURFACE) {
            // node 속성 상수 이름과 같은 token 이름.
            token.name = "n0000000000000000000000000000000c_4".to_string();
        }
    }
    fixture
}

#[test]
fn identifiers_avoid_generated_type_and_parameter_names() {
    let fixture = colliding_names();
    let swiftui = export_apple(
        &fixture.ir(&fx::ios_profile()),
        &AppleOptions::new(AppleDialect::SwiftUi),
    )
    .expect("export");
    assert!(swiftui.source.contains("@Binding var Text_2: String"));
    assert!(swiftui.source.contains("@Binding var Content_2: String"));
    assert!(swiftui.source.contains("@Binding var views_2: Bool"));
    assert!(swiftui.source.contains("static let Color_2 = Color(.sRGB"));
    // token 상수가 먼저 이름을 가지므로 같은 이름의 node 속성 상수는 접미사를 받습니다.
    assert!(swiftui
        .source
        .contains("static let n0000000000000000000000000000000c_4_2 = Color_2"));
    let appkit = export_apple(
        &fixture.ir(&fx::mac_profile()),
        &AppleOptions::new(AppleDialect::AppKit),
    )
    .expect("export");
    assert!(appkit.source.contains("views_2: Bool"));
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

/// profile 최소 OS를 deployment target으로 쓰는 `swiftc` target triple과 SDK.
#[cfg(target_os = "macos")]
fn sdk_target(profile: &stellar_target_profile::spec::TargetProfile) -> (&'static str, String) {
    let stellar_target_profile::spec::ProfileSpec::Apple(apple) = profile.spec else {
        panic!("Apple profile");
    };
    let version = format!("{}.{}", apple.minimum_os.major(), apple.minimum_os.minor());
    match apple.platform {
        ApplePlatform::MacOs => ("macosx", format!("arm64-apple-macos{version}")),
        ApplePlatform::Ios | ApplePlatform::IpadOs => (
            "iphonesimulator",
            format!("arm64-apple-ios{version}-simulator"),
        ),
        other => panic!("compile fixture가 다루지 않는 플랫폼 {}", other.name()),
    }
}

/// 최소 OS를 바꾼 profile.
#[cfg(target_os = "macos")]
fn with_minimum_os(
    mut profile: stellar_target_profile::spec::TargetProfile,
    major: u16,
    minor: u16,
) -> stellar_target_profile::spec::TargetProfile {
    if let stellar_target_profile::spec::ProfileSpec::Apple(apple) = &mut profile.spec {
        apple.minimum_os = stellar_target_profile::spec::DottedVersion::new(major, minor, 0);
    }
    profile
}

#[test]
#[cfg(target_os = "macos")]
fn generated_swift_typechecks_against_apple_sdks() {
    // 검증 기준(XXII.3.3) compile fixture: iOS·macOS SDK로 SwiftUI·AppKit 소스를 profile 최소 OS를 deployment
    // target으로 typecheck합니다. 최소 OS가 낮은 profile(`.isToggle` 이전 iOS 16.4·macOS 13.0)과 생성 코드 이름과
    // 겹치는 상태·token 이름도 같은 방식으로 검사합니다.
    let fixture = Fixture::new();
    let colliding = colliding_names();
    let cases = [
        ("ios-swiftui", fx::ios_profile(), AppleDialect::SwiftUi),
        ("mac-swiftui", fx::mac_profile(), AppleDialect::SwiftUi),
        ("mac-appkit", fx::mac_profile(), AppleDialect::AppKit),
        (
            "ios16-swiftui",
            with_minimum_os(fx::ios_profile(), 16, 4),
            AppleDialect::SwiftUi,
        ),
        (
            "mac13-swiftui",
            with_minimum_os(fx::mac_profile(), 13, 0),
            AppleDialect::SwiftUi,
        ),
        (
            "mac13-appkit",
            with_minimum_os(fx::mac_profile(), 13, 0),
            AppleDialect::AppKit,
        ),
    ];
    for (name, profile, dialect) in cases {
        let (sdk, target) = sdk_target(&profile);
        for (variant, input) in [("", &fixture), ("-names", &colliding)] {
            let export =
                export_apple(&input.ir(&profile), &AppleOptions::new(dialect)).expect("export");
            typecheck(&export.source, &format!("{name}{variant}"), sdk, &target);
        }
    }
}
