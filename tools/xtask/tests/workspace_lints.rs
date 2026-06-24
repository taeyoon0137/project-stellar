//! II.1.4 workspace lint 선언 검증.
//!
//! `unsafe_op_in_unsafe_fn = "deny"`는 컴파일 단계에서 강제됩니다(unsafe fn 내부의
//! unsafe 연산이 명시적 unsafe 블록 없이 사용되면 컴파일 실패). trybuild 의존성이
//! 승인되지 않아, 본 테스트는 lint 선언이 존재하는지 검사합니다.

fn root_manifest() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml"))
        .expect("root Cargo.toml")
}

#[test]
fn unsafe_op_in_unsafe_fn_is_deny() {
    let manifest = root_manifest();
    let lints = manifest
        .split("[workspace.lints.rust]")
        .nth(1)
        .expect("[workspace.lints.rust] 섹션 필요");
    let section = lints.split("\n[").next().unwrap_or(lints);
    assert!(
        section.contains("unsafe_op_in_unsafe_fn = \"deny\""),
        "workspace lint에 unsafe_op_in_unsafe_fn = \"deny\" 필요"
    );
}

#[test]
fn missing_docs_is_warn() {
    let manifest = root_manifest();
    let lints = manifest
        .split("[workspace.lints.rust]")
        .nth(1)
        .expect("[workspace.lints.rust] 섹션 필요");
    let section = lints.split("\n[").next().unwrap_or(lints);
    assert!(
        section.contains("missing_docs = \"warn\""),
        "workspace lint에 missing_docs = \"warn\" 필요"
    );
}
