//! stellar-cli 실행 파일 진입점 (XXIV.1.1).
//!
//! 인자 해석과 명령 실행은 라이브러리 [`stellar_cli::cli::run`]에 위임합니다. UTF-8이 아닌
//! 인자는 panic 없이 사용법 오류로 처리합니다.

use std::io::Write;
use std::process::ExitCode;

use stellar_cli::cli;

fn main() -> ExitCode {
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut out = stdout.lock();
    let mut err = stderr.lock();

    let mut args = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.into_string() {
            Ok(text) => args.push(text),
            Err(raw) => {
                let _ = writeln!(
                    err,
                    "stellar-cli: UTF-8이 아닌 인자는 지원하지 않습니다: {raw:?}\n\
                     `stellar-cli --help`로 사용법을 확인하세요."
                );
                return ExitCode::from(exit_byte(cli::EXIT_USAGE));
            }
        }
    }

    let code = cli::run(&args, &mut out, &mut err);
    ExitCode::from(exit_byte(code))
}

/// 종료 코드를 프로세스 종료 byte로 변환합니다. 범위를 벗어나면 sysexits `EX_SOFTWARE`(70).
fn exit_byte(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(70)
}
