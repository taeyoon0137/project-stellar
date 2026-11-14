//! II.1.3 release profile / panic 정책 검증.
//!
//! 루트 `Cargo.toml`의 profile 선언을 검사합니다.

fn root_manifest() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml"))
        .expect("root Cargo.toml")
}

/// 지정한 profile 섹션 본문(다음 `[` 섹션 전까지)을 반환합니다.
fn profile_section<'a>(manifest: &'a str, header: &str) -> &'a str {
    let after = manifest
        .split(header)
        .nth(1)
        .unwrap_or_else(|| panic!("{header} 섹션이 필요합니다"));
    after.split("\n[").next().unwrap_or(after)
}

#[test]
fn release_profile_declares_panic_abort() {
    let manifest = root_manifest();
    let section = profile_section(&manifest, "[profile.release]");
    assert!(
        section.contains("panic = \"abort\""),
        "release profile에 panic = \"abort\"가 필요합니다"
    );
}

/// host 산출물(build script·proc-macro)은 release에서도 strip하지 않습니다(XVI.9.1 후속).
///
/// `MACOSX_DEPLOYMENT_TARGET`이 12.0 이상이면 proc-macro dylib이 chained fixups로 링크되고, release 기본
/// strip(debuginfo)이 그 dylib를 깨뜨려 새 target에서 rustc가 proc-macro를 읽지 못합니다(E0463). 실제 빌드 검사는
/// `scripts/macos/release_proc_macro_check.sh`입니다.
#[test]
fn release_build_override_keeps_host_artifacts_unstripped() {
    let manifest = root_manifest();
    let section = profile_section(&manifest, "[profile.release.build-override]");
    assert!(
        section.contains("strip = \"none\""),
        "release build-override profile에 strip = \"none\"이 필요합니다"
    );
}

#[test]
fn dev_profile_keeps_unwind() {
    let manifest = root_manifest();
    let section = profile_section(&manifest, "[profile.dev]");
    assert!(
        !section.contains("panic = \"abort\""),
        "dev profile은 unwind를 유지해야 합니다"
    );
}
