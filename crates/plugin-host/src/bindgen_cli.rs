//! 생성 파일을 다시 씁니다(ADR-0097 결정 1·6).
//!
//! `cargo run -p stellar-plugin-host --bin stellar-plugin-bindgen`
//!
//! 저장소 루트에서 실행합니다. `--check`이면 쓰지 않고 다른 파일이 있으면 1로 끝납니다.

use std::path::Path;
use std::process::ExitCode;

use stellar_plugin_host::generated_files::generated_files;

fn main() -> ExitCode {
    let check = std::env::args().any(|a| a == "--check");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut drift = 0;
    for (path, content) in generated_files() {
        let target = root.join(path);
        let current = std::fs::read_to_string(&target).unwrap_or_default();
        if current == content {
            continue;
        }
        if check {
            eprintln!("생성 파일이 원본과 다릅니다: {path}");
            drift += 1;
        } else if let Err(e) = std::fs::write(&target, content) {
            eprintln!("{path}를 쓸 수 없습니다: {e}");
            return ExitCode::FAILURE;
        } else {
            println!("갱신: {path}");
        }
    }
    if drift > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
