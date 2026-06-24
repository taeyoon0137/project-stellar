// xtask — Project Stellar task runner.
// 명령 디스패치는 xtask 라이브러리의 command registry(commands::run)에 위임합니다.
// 내부 도구 crate이며 공개 API가 아니므로 missing_docs는 allow 합니다(II.1.4).
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use xtask::commands;

/// xtask 크레이트(`<repo>/tools/xtask`) 기준으로 저장소 루트를 계산합니다.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("xtask 크레이트는 <repo>/tools/xtask 아래에 있어야 합니다")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = commands::run(&args, &repo_root());
    ExitCode::from(u8::try_from(code).unwrap_or(70))
}
