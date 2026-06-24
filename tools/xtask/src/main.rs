// xtask — Project Stellar task runner.
//
// 표준 라이브러리 + xtask 라이브러리를 사용하는 최소 명령 라우터입니다.
// 지원 명령: help, generate-ledger

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use xtask::task_status::generate::generate_ledger_file;

const USAGE: &str = "\
xtask — Project Stellar task runner

USAGE:
    cargo xtask <COMMAND>

COMMANDS:
    help              이 사용법을 출력합니다.
    generate-ledger   계획서에서 planning/task-status.yaml을 생성합니다.
";

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
    match args.first().map(String::as_str) {
        Some("help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("generate-ledger") => match generate_ledger_file(&repo_root()) {
            Ok(count) => {
                println!("planning/task-status.yaml 생성 완료: {count} tasks");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("generate-ledger 실패: {error}");
                ExitCode::from(70)
            }
        },
        Some(unknown) => {
            eprintln!("알 수 없는 명령: {unknown}");
            eprint!("{USAGE}");
            ExitCode::from(64)
        }
        None => {
            eprintln!("명령이 필요합니다.");
            eprint!("{USAGE}");
            ExitCode::from(64)
        }
    }
}
