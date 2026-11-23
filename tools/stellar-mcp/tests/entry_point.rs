//! XIX.9.1 검증: 실행 파일 package는 진입점만 갖습니다(ADR-0005 §3·§8).
//!
//! `tools/stellar-mcp/src`에는 `main.rs`(표준 입출력 handle을 열어 `entry::run`을 부르는 함수 하나)와
//! `lib.rs`(mcp-core 재노출)만 있어야 합니다. 인자 해석·backend·protocol 처리는 `crates/mcp-core`에 둡니다.

use std::path::Path;

const SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

/// 주석과 빈 줄을 뺀 코드 줄.
fn code_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect()
}

#[test]
fn executable_package_has_only_the_library_entry_point() {
    let mut files: Vec<String> = std::fs::read_dir(SRC)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["lib.rs", "main.rs"], "src에는 진입점 두 파일만");

    let lib = std::fs::read_to_string(Path::new(SRC).join("lib.rs")).unwrap();
    assert_eq!(code_lines(&lib), ["pub use stellar_mcp_core::*;"]);

    let main = std::fs::read_to_string(Path::new(SRC).join("main.rs")).unwrap();
    let code = code_lines(&main);
    assert_eq!(
        code.iter().filter(|line| line.contains("fn ")).count(),
        1,
        "main.rs에는 main 함수 하나만"
    );
    assert!(code
        .iter()
        .any(|line| line.contains("stellar_mcp::entry::run(")));
    // protocol·backend 처리 흔적이 없습니다.
    for banned in [
        "match ", "if ", "loop", "while ", "for ", "json", "server", "headless", "attach",
    ] {
        assert!(
            !code.iter().any(|line| line.contains(banned)),
            "main.rs에 진입점 밖 로직이 있습니다: {banned}"
        );
    }
}
