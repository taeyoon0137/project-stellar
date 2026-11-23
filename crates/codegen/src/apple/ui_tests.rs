//! XXII.3.1 검증: view tree golden(static fixture), Control contract 매핑과 노트, `swiftc -typecheck` compile fixture.

use stellar_target_profile::capability::CapabilityStatus;
use stellar_target_profile::spec::{DottedVersion, ProfileSpec, TargetProfile};

use super::*;
use crate::fixture::{self as fx, sid, Fixture};

pub(crate) fn ui(fixture: &Fixture, profile: &TargetProfile, dialect: AppleDialect) -> AppleExport {
    export_apple_ui(&fixture.ir(profile), &AppleOptions::new(dialect)).expect("export")
}

/// `src/apple/golden/<name>` golden과 비교합니다. `STELLAR_UPDATE_SNAPSHOT_FIXTURES`가 있으면 먼저 갱신합니다.
pub(crate) fn check_golden(dir: &str, name: &str, actual: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(dir)
        .join(name);
    if std::env::var_os("STELLAR_UPDATE_SNAPSHOT_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().expect("부모")).expect("golden 디렉터리");
        std::fs::write(&path, actual).expect("golden 갱신");
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "{name}이 golden과 다릅니다. 의도한 변경이면 STELLAR_UPDATE_SNAPSHOT_FIXTURES=1로 갱신하세요.\n{actual}"
    );
}

pub(crate) fn with_minimum_os(mut profile: TargetProfile, major: u16, minor: u16) -> TargetProfile {
    if let ProfileSpec::Apple(apple) = &mut profile.spec {
        apple.minimum_os = DottedVersion::new(major, minor, 0);
    }
    profile
}

pub(crate) const GOLDEN: &str = "src/apple/golden";

#[test]
fn view_sources_match_golden_static_fixtures() {
    // 검증 기준(XXII.3.1) golden source: 같은 IR·옵션의 view tree가 golden과 byte 단위로 같습니다.
    let fixture = Fixture::new();
    for (name, input, profile, dialect) in [
        (
            "ios_swiftui_view.swift",
            &fixture,
            fx::ios_profile(),
            AppleDialect::SwiftUi,
        ),
        (
            "mac_swiftui_view.swift",
            &fixture,
            fx::mac_profile(),
            AppleDialect::SwiftUi,
        ),
        (
            "mac_appkit_view.swift",
            &fixture,
            fx::mac_profile(),
            AppleDialect::AppKit,
        ),
    ] {
        let first = ui(input, &profile, dialect);
        assert_eq!(first, ui(input, &profile, dialect), "{name} 결정론");
        check_golden(GOLDEN, name, &first.source);
    }
}

#[test]
fn ui_source_extends_accessibility_export() {
    // view tree 출력은 XXII.3.3 색·접근성 출력을 그대로 앞에 둡니다(같은 modifier를 다시 만들지 않음).
    let fixture = Fixture::new();
    for (profile, dialect) in [
        (fx::ios_profile(), AppleDialect::SwiftUi),
        (fx::mac_profile(), AppleDialect::AppKit),
    ] {
        let ir = fixture.ir(&profile);
        let base = export_apple(&ir, &AppleOptions::new(dialect)).expect("base");
        let full = export_apple_ui(&ir, &AppleOptions::new(dialect)).expect("ui");
        assert!(full.source.starts_with(&base.source));
        assert!(base.notes.iter().all(|note| full.notes.contains(note)));
    }
}

#[test]
fn controls_map_to_swiftui_and_appkit() {
    let fixture = Fixture::new();
    let ios = ui(&fixture, &fx::ios_profile(), AppleDialect::SwiftUi);
    let source = &ios.source;
    // 상태는 @State, 접근성 modifier는 같은 상태의 binding을 받습니다.
    assert!(source.contains("@State private var isSaving: Bool = false"));
    assert!(source.contains(
        ".modifier(StellarAccessibility0000000000000000000000000000000c(isSaving: $isSaving, onAction: onAction))"
    ));
    // Button: activate는 onAction, radio 선택은 선택 상태 쓰기. 디자인 모양은 label 안 layout이 그립니다.
    assert!(source.contains(
        "        Button {\n            onAction(\"00000000-0000-0000-0000-00000000003d\")\n            selectedSize = \"00000000-0000-0000-0000-000000000008\"\n        } label: {"
    ));
    assert!(source.contains(".buttonStyle(.plain)"));
    // TextInput: 상태 binding과 placeholder.
    assert!(source.contains(
        "TextField(text: $memo, prompt: Text(verbatim: \"메모\")) {\n            Text(verbatim: \"메모\")\n        }\n        .textFieldStyle(.plain)"
    ));
    // Image: SF Symbol 속성.
    assert!(source.contains("Image(systemName: \"person.crop.circle\")"));
    let notes: Vec<_> = ios
        .notes
        .iter()
        .filter(|note| matches!(note.kind, AppleNoteKind::Control(_)))
        .map(|note| (note.subject.clone(), note.kind))
        .collect();
    assert_eq!(
        notes,
        vec![
            (
                AppleNoteSubject::Node(sid(5)),
                AppleNoteKind::Control(ControlNote::ShapeGeometryUnavailable)
            ),
            (
                AppleNoteSubject::Node(sid(16)),
                AppleNoteKind::Control(ControlNote::ModalInPlace)
            ),
        ]
    );
    // ios profile이 제외한 node 13은 view가 없습니다.
    assert!(!source.contains("node0000000000000000000000000000000d"));

    let appkit = ui(&fixture, &fx::mac_profile(), AppleDialect::AppKit);
    let source = &appkit.source;
    assert!(source.contains("final class StellarView: NSView"));
    // 비활성 상태면 버튼 동작을 막고, 입력 확정 시 상태를 씁니다.
    assert!(source.contains("guard !(self.isSaving) else { return }"));
    assert!(source.contains("self.memo = view.stringValue"));
    assert!(source.contains(
        "NSClickGestureRecognizer(target: target, action: #selector(StellarActionTarget.fire))"
    ));
    // 선택 상태를 쓰는 접근성 동작은 setState로 연결됩니다.
    assert!(source.contains(
        "case \"00000000-0000-0000-0000-000000000032\":\n            selectedSize = value"
    ));
    // mac에는 node 13 grid가 NSGridView로 나옵니다.
    assert!(source.contains("let grid = NSGridView(views: [[cell1, cell2]])"));
}

#[test]
fn unsupported_contract_falls_back_to_container() {
    // profile이 contract를 Unsupported로 판정하면 contract 없는 container로 내고 노트를 남깁니다.
    let fixture = Fixture::new();
    let mut ir = fixture.ir(&fx::ios_profile());
    let index = ir.node_index(sid(12)).expect("button") as usize;
    for finding in &mut ir.nodes[index].capabilities {
        if finding.subject
            == stellar_target_profile::capability::CapabilitySubject::Contract(
                stellar_target_profile::capability::ContractCode::BUTTON,
            )
        {
            finding.status = Some(CapabilityStatus::Unsupported);
        }
    }
    let export = export_apple_ui(&ir, &AppleOptions::new(AppleDialect::SwiftUi)).expect("export");
    assert!(export.notes.contains(&AppleNote {
        subject: AppleNoteSubject::Node(sid(12)),
        kind: AppleNoteKind::Control(ControlNote::ContractUnsupported(Control::Button)),
    }));
    assert!(!export
        .source
        .contains("onAction(\"00000000-0000-0000-0000-00000000003c\")\n        } label:"));
}

#[cfg(target_os = "macos")]
pub(crate) fn typecheck(source: &str, name: &str, sdk: &str, target: &str) {
    let dir =
        std::env::temp_dir().join(format!("stellar-codegen-ui-{}-{name}", std::process::id()));
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

#[cfg(target_os = "macos")]
pub(crate) fn sdk_target(profile: &TargetProfile) -> (&'static str, String) {
    let ProfileSpec::Apple(apple) = profile.spec else {
        panic!("Apple profile");
    };
    let version = format!("{}.{}", apple.minimum_os.major(), apple.minimum_os.minor());
    match apple.platform {
        stellar_target_profile::spec::ApplePlatform::MacOs => {
            ("macosx", format!("arm64-apple-macos{version}"))
        }
        _ => (
            "iphonesimulator",
            format!("arm64-apple-ios{version}-simulator"),
        ),
    }
}

#[test]
#[cfg(target_os = "macos")]
fn generated_views_typecheck_against_apple_sdks() {
    // 검증 기준(XXII.3.1 golden source): view tree를 포함한 소스를 profile 최소 OS를 deployment target으로
    // typecheck합니다. 낮은 최소 OS도 같이 검사합니다.
    let fixture = Fixture::new();
    let cases = [
        ("ios", &fixture, fx::ios_profile(), AppleDialect::SwiftUi),
        ("mac", &fixture, fx::mac_profile(), AppleDialect::SwiftUi),
        (
            "mac-appkit",
            &fixture,
            fx::mac_profile(),
            AppleDialect::AppKit,
        ),
        (
            "ios16",
            &fixture,
            with_minimum_os(fx::ios_profile(), 16, 4),
            AppleDialect::SwiftUi,
        ),
        (
            "mac13-appkit",
            &fixture,
            with_minimum_os(fx::mac_profile(), 13, 0),
            AppleDialect::AppKit,
        ),
    ];
    for (name, input, profile, dialect) in cases {
        let (sdk, target) = sdk_target(&profile);
        typecheck(&ui(input, &profile, dialect).source, name, sdk, &target);
    }
}

#[test]
fn apple_output_is_a_generated_file_set() {
    // XXII.5.1: Swift 출력은 생성 파일 집합의 `<접두어>View.swift` 하나이고 marker·LF·끝 줄바꿈 규칙을 따릅니다.
    let fixture = Fixture::new();
    for (profile, dialect) in [
        (fx::ios_profile(), AppleDialect::SwiftUi),
        (fx::mac_profile(), AppleDialect::AppKit),
    ] {
        let ir = fixture.ir(&profile);
        let options = AppleOptions::new(dialect);
        let (files, notes) = export_apple_files(&ir, &options).expect("files");
        let (again, _) = export_apple_files(&ir, &options).expect("files");
        assert_eq!(files.manifest_text(), again.manifest_text(), "byte 동일");
        let paths: Vec<&str> = files.iter().map(|(p, _)| p).collect();
        assert_eq!(paths, ["StellarView.swift"]);
        let export = export_apple_ui(&ir, &options).expect("export");
        assert_eq!(notes, export.notes);
        let content = files.get(paths[0]).expect("내용");
        let marker = crate::output::FileKind::Swift.marker();
        assert_eq!(content, format!("{marker}\n{}", export.source));
        assert!(!content.contains('\r') && !content.ends_with("\n\n"));
        #[cfg(target_os = "macos")]
        {
            let (sdk, target) = sdk_target(&profile);
            typecheck(content, &format!("files-{dialect:?}"), sdk, &target);
        }
    }
    let ir = fixture.ir(&fx::ios_profile());
    let bad = AppleOptions {
        dialect: AppleDialect::SwiftUi,
        type_prefix: "x/y".to_string(),
    };
    assert_eq!(
        export_apple_files(&ir, &bad).map(|_| ()),
        Err(AppleError::InvalidTypePrefix)
    );
}
