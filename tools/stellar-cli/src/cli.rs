//! 명령 registry와 도움말 (XXIV.1.1).
//!
//! 인자 해석은 순수 함수 [`run`]이 담당합니다. 출력 대상(stdout·stderr)을 인자로 받으므로
//! 테스트는 메모리 버퍼로 실행할 수 있습니다. 도움말은 사용자가 요청한 결과이므로 stdout에,
//! 사용법 오류 안내는 stderr에 씁니다.

use std::io::Write;

/// 성공 종료 코드.
pub const EXIT_OK: i32 = 0;
/// 사용법 오류 종료 코드(sysexits `EX_USAGE`).
pub const EXIT_USAGE: i32 = 64;
/// 출력 기록 실패 종료 코드(sysexits `EX_IOERR`).
pub const EXIT_IOERR: i32 = 74;

/// 등록된 명령: (이름, 한 줄 설명). 도움말에 이 순서로 나타납니다.
pub const COMMANDS: &[(&str, &str)] = &[("help", "이 도움말을 표시합니다")];

/// 도움말로 인식하는 인자.
const HELP_ALIASES: [&str; 3] = ["help", "--help", "-h"];

/// 도움말 문자열. 각 명령이 정확히 한 번 나타납니다.
pub fn usage() -> String {
    let mut out = String::from(
        "stellar-cli — Project Stellar headless 도구\n\n\
         사용법:\n    stellar-cli <명령> [인자]\n\n명령:\n",
    );
    let width = COMMANDS
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    for (name, summary) in COMMANDS {
        out.push_str(&format!("    {name:<width$}  {summary}\n"));
    }
    out.push_str(
        "\n앱 UI·GUI·네트워크 없이 동작합니다. `stellar-cli --help`로 이 도움말을 다시 볼 수 있습니다.\n",
    );
    out
}

/// 인자를 해석해 명령을 실행하고 종료 코드를 반환합니다.
///
/// `args`는 프로그램 이름을 제외한 인자입니다. `out`은 명령 결과(stdout), `err`는 진단(stderr)
/// 대상입니다.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let Some(command) = args.first() else {
        return usage_error(err, "명령이 필요합니다.");
    };
    let rest = &args[1..];
    if HELP_ALIASES.contains(&command.as_str()) {
        if let Some(extra) = rest.first() {
            return usage_error(err, &format!("help는 인자를 받지 않습니다: {extra}"));
        }
        return match out.write_all(usage().as_bytes()).and_then(|()| out.flush()) {
            Ok(()) => EXIT_OK,
            Err(e) => {
                let _ = writeln!(err, "stellar-cli: 도움말 출력 실패: {e}");
                EXIT_IOERR
            }
        };
    }
    usage_error(err, &format!("알 수 없는 명령: {command}"))
}

/// 사용법 오류를 stderr에 알리고 [`EXIT_USAGE`]를 반환합니다.
fn usage_error(err: &mut dyn Write, message: &str) -> i32 {
    let _ = writeln!(
        err,
        "stellar-cli: {message}\n`stellar-cli --help`로 사용법을 확인하세요."
    );
    EXIT_USAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_args(args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn help_aliases_print_usage_to_stdout() {
        for alias in HELP_ALIASES {
            let (code, out, err) = run_args(&[alias]);
            assert_eq!(code, EXIT_OK, "{alias}");
            assert_eq!(out, usage(), "{alias}");
            assert!(err.is_empty(), "{alias}: stderr는 비어야 함");
        }
    }

    #[test]
    fn usage_lists_each_command_once() {
        let text = usage();
        for (name, summary) in COMMANDS {
            let line = text
                .lines()
                .filter(|l| l.trim_start().starts_with(name) && l.contains(summary))
                .count();
            assert_eq!(line, 1, "{name}");
        }
    }

    #[test]
    fn missing_or_unknown_command_is_usage_error() {
        for args in [&[][..], &["frobnicate"][..], &["help", "extra"][..]] {
            let (code, out, err) = run_args(args);
            assert_eq!(code, EXIT_USAGE, "{args:?}");
            assert!(out.is_empty(), "{args:?}: stdout에 결과가 없어야 함");
            assert!(err.contains("--help"), "{args:?}: 안내는 stderr");
        }
    }

    #[test]
    fn output_failure_is_io_error() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut err = Vec::new();
        let code = run(&["--help".to_string()], &mut Broken, &mut err);
        assert_eq!(code, EXIT_IOERR);
        assert!(!err.is_empty());
    }
}
