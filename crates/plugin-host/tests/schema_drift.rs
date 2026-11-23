//! 생성 파일 drift 검사(XXI.1.1 schema fixture, XXI.4.1 bindings drift 0).
//!
//! 저장소의 생성 파일이 원본(Rust 상수·IDL)에서 다시 만든 내용과 byte 단위로 같아야 합니다. 다르면
//! `cargo run -p stellar-plugin-host --bin stellar-plugin-bindgen`으로 다시 씁니다.

use std::path::Path;

use stellar_plugin_host::generated_files::generated_files;

#[test]
fn generated_files_have_no_drift() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (path, content) in generated_files() {
        let current = std::fs::read_to_string(root.join(path))
            .unwrap_or_else(|e| panic!("{path}를 읽을 수 없습니다: {e}"));
        assert!(
            current == content,
            "{path}가 원본과 다릅니다. stellar-plugin-bindgen을 실행하세요"
        );
    }
}
