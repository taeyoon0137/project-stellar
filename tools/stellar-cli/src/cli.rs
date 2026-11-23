//! 명령 registry와 도움말 (XXIV.1.1, XXIV.1.2).
//!
//! 인자 해석은 순수 함수 [`run`]이 담당합니다. 출력 대상(stdout·stderr)을 인자로 받으므로
//! 테스트는 메모리 버퍼로 실행할 수 있습니다. stdout·stderr 역할과 결과 JSON 스키마는
//! [`crate::output`]에 정의합니다. 요약하면 다음과 같습니다.
//!
//! - 도움말: 사용자가 요청한 결과이므로 stdout에 텍스트로 씁니다.
//! - 명령 실행(성공·실패): stdout에 결과 envelope JSON 한 줄, 사람용 진단은 stderr.
//! - 사용법 오류(64): stdout에 아무것도 쓰지 않고 stderr로만 안내합니다.

use std::io::Write;

use crate::commands;
use crate::exit::ExitStatus;
use crate::json;
use crate::output::{self, CliError, Success};

/// 사용자가 부르는 실행 이름(XXIV.4.1). 도움말·사용법·stderr 진단 접두어에 씁니다. package·library
/// 이름(`stellar-cli`)과 결과 schema 식별자([`crate::output::SCHEMA_ID`])는 바꾸지 않습니다.
pub const PROGRAM: &str = "stellar";

/// 성공 종료 코드([`ExitStatus::Ok`]).
pub const EXIT_OK: i32 = 0;
/// 사용법 오류 종료 코드(sysexits `EX_USAGE`, [`ExitStatus::Usage`]).
pub const EXIT_USAGE: i32 = 64;
/// 출력 기록 실패 종료 코드(sysexits `EX_IOERR`, [`ExitStatus::IoErr`]).
pub const EXIT_IOERR: i32 = 74;

/// 등록된 명령: (이름, 한 줄 설명). 도움말에 이 순서로 나타납니다.
pub const COMMANDS: &[(&str, &str)] = &[
    ("help", "이 도움말을 표시합니다"),
    ("version", "도구 이름·버전을 JSON으로 출력합니다"),
    (
        "inspect",
        "<경로>의 프로젝트 root와 stellar.yaml을 읽기 전용으로 검사해 JSON으로 출력합니다(--structure: 문서 개요와 revision 추가)",
    ),
    (
        "query",
        "<경로> <대상>: 저장된 프로젝트를 읽기 전용으로 조회해 revision과 함께 JSON으로 출력합니다",
    ),
    (
        "validate",
        "<경로>: 저장된 모든 문서를 문서 validator로 다시 검증합니다(오류가 있으면 종료 코드 1)",
    ),
    (
        "diagnose",
        "<경로> [--kind <종류>]... [--severity info|warning|error] [--document <id>|--project] [--source <id>] [--limit <n>] [--cursor <c>]: 무결성 진단(미해결 참조·migration 등)을 종류·심각도·문서로 필터해 page로 출력합니다(진단이 있어도 종료 코드 0)",
    ),
    (
        "semantic-diff",
        "<base 경로> <target 경로>: 같은 프로젝트 두 상태의 semantic diff를 출력합니다",
    ),
    (
        "semantic-merge",
        "<base> <ours> <theirs>: 3-way semantic merge 결과와 conflict를 출력합니다(conflict가 있으면 종료 코드 1, 파일을 쓰지 않음)",
    ),
    (
        "migrate",
        "<경로> [--check]: generation 스키마를 이 도구의 버전으로 migration합니다(--check: 판정만)",
    ),
    (
        "diff-summary",
        "<base> <target> [--output <파일>] [--include-content]: semantic diff 요약 artifact를 만듭니다(기본: 프로젝트 텍스트·이름 redaction)",
    ),
    (
        "ci",
        "<경로>: CI 검사(lockfile·검증·migration 필요 여부)를 실행합니다(하나라도 실패하면 종료 코드 1)",
    ),
    (
        "export-web",
        "<경로> <문서 id> <View id> [--frame <Frame id>]: 저장 Frame·Fragment·Rectangle의 HTML+CSS(layout record나 기본값 세트 차이가 있으면 CSS layout 출력)와 revision·형식·진단을 JSON으로 출력합니다(--frame은 View 안 Frame 하나, 미지원 값은 실패)",
    ),
    (
        "export-styles",
        "<경로> <문서 id> <View id> [--frame <Frame id>] [--target web|react-native]: layout 스타일을 대상별로 출력합니다(web: 특이도 0 기반 규칙과 요소별 선언·출처, react-native: StyleSheet 모듈). 명시 값과 활성 기본값 세트·대상 기본값의 차이만 냅니다",
    ),
    (
        "export",
        "swiftui|appkit|compose <경로> <문서 id> <View id> [--frame <Frame id>] [--platform ios|macos]: 저장 View를 export-web과 같은 투영으로 SwiftUI·AppKit·Compose 소스로 출력합니다(구조·layout·기본 접근성, 미지원 node는 실패)",
    ),
    (
        "css",
        "copy <경로> <문서 id> <node id> | paste <경로> <문서 id> <node id,…> <입력 파일|-> [--dry-run]: node 속성을 CSS longhand 선언 블록으로 복사하거나, 선언 블록을 한 transaction으로 붙여넣습니다(받지 못한 선언은 이유와 함께 건너뜀)",
    ),
    (
        "style-defaults",
        "list <경로> | show <경로> [<세트>] | set <경로> <세트> [--base <revision>] [--dry-run] | resolve <경로> <문서 id> <node id>: 기본값 세트 목록·값 표(layout·payload longhand)·선택(SetStyleDefaults를 base revision 검사·저장 잠금으로 적용, stellar.yaml 한 줄)과 node layout longhand 값의 출처(explicit·default(<세트>))",
    ),
    (
        "apply",
        "<경로> <입력 파일|-> [--dry-run]: CommandTransaction JSON(base revision 포함)을 검증해 세대 전환 저장으로 기록합니다(--dry-run: semantic diff만)",
    ),
    (
        "context",
        "<경로> <문서 id> [--node <node id> [--instances <id,…>]]: 문서 Component Context의 key·provision·requirement·Slot 공개 목록과 진단을, --node를 주면 그 위치의 key별 해석 값·출처·경로와 선택기 후보를 출력합니다",
    ),
    ("component-source", "create <폴더> <이름> <id> <root-id> | check <폴더> | resize <파일> <원본 hash> <너비> <높이> | set-fill <파일> <원본 hash> <none 또는 색 공간과 RGBA> | add-frame <파일> <원본 hash> <parent-id> <새-id> <이름> <너비> <높이>: 컴포넌트별 Free Frame 트리"),
    (
        "git",
        "status <경로> [--cursor <c>] [--limit <n>] | stage|unstage <경로> <대상>... [--with <대상>]... | commit <경로> -m <메시지>: 신뢰한 프로젝트의 의미 단위 Git 상태·stage·commit(push 없음, 필수 동반 변경은 자동 포함하지 않음)",
    ),
    (
        "text-source",
        "export <프로젝트> <새 파일> | check <파일> | adopt <프로젝트>: 단일 View 텍스트 원본을 opt-in으로 내보내거나 읽기 전용 검사합니다. adopt는 텍스트 원본이 없는 chunk 프로젝트를 검증된 변환으로 design/ 텍스트 원본(Git 원본)으로 옮깁니다",
    ),
    (
        "diagnostics",
        "preview | export --output <파일> | scan <파일> | consent [show|grant|decline|revoke] | enqueue | schema [--data-dir <디렉터리>] [--project <경로>]...: 로컬 진단 store의 전송 예정 bytes를 보여 주고, 같은 bytes를 진단 번들로 내보내고, 파일의 수집 금지 정보를 검사합니다(탐지가 있으면 종료 코드 1). consent는 텔레메트리 동의를 보고 바꾸며 철회하면 전송 대기열을 버리고, enqueue는 동의했을 때만 preview를 대기열에 넣습니다. schema는 진단 schema와 꺼진 기능(터미널·MCP·에이전트) field 부재 검사 결과를 냅니다(네트워크 전송 없음)",
    ),
    ("merge-driver", "<%O> <%A> <%B> <%P> | install <프로젝트> | resolve <프로젝트> [--resolve <번호>=<action>]... [--apply]: Git merge driver로 컴포넌트 원본을 semantic merge합니다(conflict가 있으면 %A를 바꾸지 않고 nonzero). install은 신뢰한 프로젝트의 저장소 로컬 config와 .gitattributes에만 등록하고, resolve는 unmerged 경로의 semantic conflict와 해결 결과를 보여 주고 --apply면 검증한 결과를 쓰고 git add합니다(continue·commit은 하지 않음)"),
];

/// `query` 대상: (사용법, 한 줄 설명). 도움말에 이 순서로 나타납니다.
pub const QUERY_TARGETS: &[(&str, &str)] = &[
    (
        "project",
        "프로젝트 구조(문서·페이지·View 개요, chunk 요약)",
    ),
    (
        "tree <문서 id> [--cursor <c>] [--limit <n>]",
        "문서 트리 한 page(깊이 우선, 기본 1000·최대 10000 node, next_cursor로 이어 조회)",
    ),
    ("node <문서 id> <node id>", "node 공통 필드와 속성"),
    (
        "component <컴포넌트 id>",
        "저장된 공유 정의의 이름·루트·원본 hash·노드 트리와 Frame 속성",
    ),
    ("tokens", "token 집합(저장 포맷 미정: chunk 수만 보고)"),
    (
        "components",
        "design/components의 컴포넌트 원본과 generation의 component-library chunk 수",
    ),
];

/// 도움말로 인식하는 인자.
const HELP_ALIASES: [&str; 3] = ["help", "--help", "-h"];
/// `version`으로 인식하는 인자.
const VERSION_ALIASES: [&str; 3] = ["version", "--version", "-V"];

/// 도움말 문자열. 각 명령이 정확히 한 번 나타납니다.
pub fn usage() -> String {
    let mut out = format!(
        "{PROGRAM} — Project Stellar headless 도구\n\n\
         사용법:\n    {PROGRAM} <명령> [인자]\n\n명령:\n"
    );
    let width = COMMANDS
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    for (name, summary) in COMMANDS {
        out.push_str(&format!("    {name:<width$}  {summary}\n"));
    }
    out.push_str("\nquery 대상:\n");
    for (usage, summary) in QUERY_TARGETS {
        out.push_str(&format!("    {usage}\n        {summary}\n"));
    }
    out.push_str(
        "\n출력:\n    \
         stdout  명령 결과 JSON 한 줄(schema \"stellar-cli/result\" v1). 도움말만 텍스트입니다.\n    \
         stderr  사람이 읽는 진단. 사용법 오류는 stderr에만 씁니다.\n\n종료 코드:\n",
    );
    for status in ExitStatus::ALL {
        out.push_str(&format!(
            "    {:>2}  {:<11}  {}\n",
            status.code(),
            status.name(),
            status.summary()
        ));
    }
    out.push_str(&format!(
        "\n앱 UI·GUI·네트워크 없이 동작합니다. `{PROGRAM} --help`로 이 도움말을 다시 볼 수 있습니다.\n"
    ));
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
                let _ = writeln!(err, "{PROGRAM}: 도움말 출력 실패: {e}");
                EXIT_IOERR
            }
        };
    }
    let (name, outcome) = if VERSION_ALIASES.contains(&command.as_str()) {
        ("version", commands::version(rest))
    } else if command == "inspect" {
        ("inspect", commands::inspect(rest))
    } else if command == "query" && rest.iter().any(|arg| arg == "--source-span") {
        ("query", crate::text_source::query_source_span(rest))
    } else if command == "query" {
        ("query", commands::query(rest))
    } else if command == "export-web" {
        ("export-web", crate::web_frame::run(rest))
    } else if command == "export-styles" {
        ("export-styles", crate::export_styles::run(rest))
    } else if command == "export" {
        ("export", crate::export_native::run(rest))
    } else if command == "css" {
        ("css", crate::css::run(rest))
    } else if command == "style-defaults" {
        ("style-defaults", crate::style_defaults::run(rest))
    } else if command == "apply" {
        ("apply", crate::apply::run(rest))
    } else if command == "context" {
        ("context", crate::context::run(rest))
    } else if command == "validate" {
        ("validate", crate::project_cmds::validate(rest))
    } else if command == "diagnose" {
        ("diagnose", crate::diagnose::run(rest))
    } else if command == "semantic-diff" {
        ("semantic-diff", crate::project_cmds::semantic_diff(rest))
    } else if command == "semantic-merge" {
        ("semantic-merge", crate::project_cmds::semantic_merge(rest))
    } else if command == "migrate" {
        ("migrate", crate::project_cmds::migrate_command(rest))
    } else if command == "ci" {
        ("ci", crate::project_cmds::ci(rest))
    } else if command == "diff-summary" {
        ("diff-summary", crate::summary::run(rest))
    } else if command == "component-source" {
        ("component-source", crate::component_source::run(rest))
    } else if command == "git" {
        ("git", crate::git::run(rest))
    } else if command == "text-source" {
        ("text-source", crate::text_source::run(rest))
    } else if command == "diagnostics" {
        ("diagnostics", crate::diagnostics::run(rest))
    } else if command == "merge-driver" {
        ("merge-driver", crate::merge_driver::run(rest))
    } else {
        return usage_error(err, &format!("알 수 없는 명령: {command}"));
    };
    emit(name, outcome, out, err)
}

/// 명령 결과를 stdout(JSON)·stderr(진단)로 내보내고 종료 코드를 반환합니다.
fn emit(
    command: &str,
    outcome: Result<Success, CliError>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (document, status, stderr) = match &outcome {
        Ok(success) => (
            output::success_envelope(command, success),
            ExitStatus::Ok,
            output::stderr_text(command, None, &success.diagnostics),
        ),
        Err(e) if e.status == ExitStatus::Usage => {
            return usage_error(err, &format!("{command}: {}", e.message));
        }
        Err(e) => (
            output::error_envelope(command, e),
            e.status,
            output::stderr_text(command, Some(e), &e.diagnostics),
        ),
    };
    let mut line = json::write(&document);
    line.push('\n');
    if let Err(e) = out.write_all(line.as_bytes()).and_then(|()| out.flush()) {
        let _ = writeln!(err, "{PROGRAM} {command}: 결과 출력 실패: {e}");
        return EXIT_IOERR;
    }
    let _ = err.write_all(stderr.as_bytes());
    i32::from(status.code())
}

/// 사용법 오류를 stderr에 알리고 [`EXIT_USAGE`]를 반환합니다. message에 든 인자 echo는
/// [`output::escape_for_terminal`]을 거칩니다.
fn usage_error(err: &mut dyn Write, message: &str) -> i32 {
    let _ = writeln!(
        err,
        "{PROGRAM}: {}\n`{PROGRAM} --help`로 사용법을 확인하세요.",
        output::escape_for_terminal(message)
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
    fn usage_error_echo_escapes_control_characters() {
        for args in [
            &["\u{1b}]0;PWNED\u{7}\nstellar: 위조"][..],
            &["help", "\u{1b}[2J\u{202e}x"][..],
            &["inspect", "--\u{1b}[31m"][..],
            &["version", "\r\u{2066}"][..],
        ] {
            let (code, out, err) = run_args(args);
            assert_eq!(code, EXIT_USAGE, "{args:?}");
            assert!(out.is_empty(), "{args:?}");
            for forbidden in ['\u{1b}', '\u{7}', '\r', '\u{202e}', '\u{2066}'] {
                assert!(!err.contains(forbidden), "{args:?}: {err:?}");
            }
            assert_eq!(
                err.matches('\n').count(),
                2,
                "{args:?}: 안내 두 줄: {err:?}"
            );
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

    #[test]
    fn exit_constants_match_exit_status() {
        assert_eq!(EXIT_OK, i32::from(ExitStatus::Ok.code()));
        assert_eq!(EXIT_USAGE, i32::from(ExitStatus::Usage.code()));
        assert_eq!(EXIT_IOERR, i32::from(ExitStatus::IoErr.code()));
    }

    #[test]
    fn usage_lists_every_exit_code() {
        let text = usage();
        for status in ExitStatus::ALL {
            assert!(
                text.lines().any(
                    |l| l.trim_start().starts_with(&format!("{} ", status.code()))
                        && l.contains(status.name())
                ),
                "{status:?}"
            );
        }
    }

    #[test]
    fn command_usage_errors_write_nothing_to_stdout() {
        for args in [
            &["version", "extra"][..],
            &["inspect"][..],
            &["inspect", "--json", "x"][..],
            &["inspect", "--bogus"][..],
            &["inspect", "a", "b"][..],
            &["inspect", "a", "--structure", "--structure"][..],
            &["query"][..],
            &["query", "p"][..],
            &["query", "--x", "project"][..],
            &["query", "p", "nothing"][..],
            &["query", "p", "project", "extra"][..],
            &["query", "p", "tree"][..],
            &["query", "p", "tree", "not-an-id"][..],
            &[
                "query",
                "p",
                "tree",
                "0195c530-3acd-7bc1-8414-c213f61cb965",
                "--limit",
                "0",
            ][..],
            &[
                "query",
                "p",
                "tree",
                "0195c530-3acd-7bc1-8414-c213f61cb965",
                "--limit",
            ][..],
            &[
                "query",
                "p",
                "tree",
                "0195c530-3acd-7bc1-8414-c213f61cb965",
                "--bogus",
            ][..],
            &["query", "p", "node", "0195c530-3acd-7bc1-8414-c213f61cb965"][..],
            &["apply"][..],
            &["apply", "p"][..],
            &["apply", "p", "in.json", "extra"][..],
            &["apply", "p", "in.json", "--force"][..],
            &["apply", "p", "in.json", "--dry-run", "--dry-run"][..],
            &["validate"][..],
            &["validate", "a", "b"][..],
            &["validate", "--strict", "a"][..],
            &["semantic-diff", "a"][..],
            &["semantic-merge", "a", "b"][..],
            &["semantic-merge", "a", "b", "c", "d"][..],
            &["migrate"][..],
            &["migrate", "a", "--force"][..],
            &["migrate", "a", "--check", "--check"][..],
            &["ci"][..],
            &["ci", "a", "b"][..],
            &["ci", "--strict", "a"][..],
            &["text-source"][..],
            &["text-source", "export", "p"][..],
            &["text-source", "export", "p", "out", "extra"][..],
            &["text-source", "check"][..],
            &["text-source", "check", "file", "extra"][..],
            &["text-source", "unknown", "file"][..],
            &["text-source", "check", "--force"][..],
            &["diff-summary", "a"][..],
            &["diff-summary", "a", "b", "--output"][..],
            &["diff-summary", "a", "b", "--redact"][..],
            &[
                "diff-summary",
                "a",
                "b",
                "--include-content",
                "--include-content",
            ][..],
        ] {
            let (code, out, err) = run_args(args);
            assert_eq!(code, EXIT_USAGE, "{args:?}");
            assert!(out.is_empty(), "{args:?}: 사용법 오류는 stdout에 쓰지 않음");
            assert!(err.contains("--help"), "{args:?}");
        }
    }

    #[test]
    fn version_aliases_emit_one_json_line() {
        for alias in VERSION_ALIASES {
            let (code, out, err) = run_args(&[alias]);
            assert_eq!(code, EXIT_OK, "{alias}");
            assert!(err.is_empty(), "{alias}");
            assert_eq!(out.matches('\n').count(), 1, "{alias}: 정확히 한 줄");
            let doc = json::parse(out.trim_end(), json::JsonLimits::default()).unwrap();
            assert_eq!(
                doc.get("command").and_then(json::JsonValue::as_str),
                Some("version")
            );
        }
    }
}
