// xtask — Project Stellar task runner (I.1.9).
//
// 표준 라이브러리만 사용하는 최소 명령 라우터입니다.
// 지원 명령: help
// 이후 단계(I.2~I.4)에서 validator·verify-task·sync-plan 명령을 추가합니다.

use std::process::ExitCode;

const USAGE: &str = "\
xtask — Project Stellar task runner

USAGE:
    cargo xtask <COMMAND>

COMMANDS:
    help    이 사용법을 출력합니다.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
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
