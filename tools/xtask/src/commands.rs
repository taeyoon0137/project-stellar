//! 명령 registry (I.4.1).
//!
//! `help`와 검증 명령을 모듈형으로 연결합니다.
//! 종료 코드: 성공 `0`, 입력 오류 `64`, 정책 위반 `65`, 내부 오류 `70`.

use std::path::Path;
use std::process::Command;

use crate::adr_policy::check_adr_policy;
use crate::check_core::run_check_core;
use crate::commit_message::validate_commit_subject;
use crate::crate_graph::check_workspace_graph;
use crate::crate_graph::manifest::scan_workspace;
use crate::dependencies::{
    parse_workspace_direct_deps, validate_dependencies_with, PlanProgress, CARGO_TREE_ARGS,
};
use crate::ffi_header::generate_header;
use crate::pr_body::validate_pr_body;
use crate::storage_fixtures::check_canonical_primitives;
use crate::sync::sync_plan;
use crate::task_report::validate::{validate_report, ReportKind};
use crate::task_status::decode::decode_collection;
use crate::task_status::generate::{generate_ledger_file, validate_all};
use crate::task_status::model::TaskState;
use crate::task_status::verify::verify_task;
use crate::versions::validate_versions;
use crate::yaml::restricted_loader::load_restricted;

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 64;
pub const EXIT_POLICY: i32 = 65;
pub const EXIT_INTERNAL: i32 = 70;

/// 등록된 명령 이름(help 포함).
pub const COMMANDS: [&str; 18] = [
    "help",
    "generate-ledger",
    "generate-ffi-header",
    "generate-command-schema",
    "validate-task-status",
    "validate-task-report",
    "validate-pr-body",
    "validate-commit-message",
    "validate-versions",
    "validate-dependencies",
    "validate-adr",
    "verify-task",
    "sync-plan",
    "check-core",
    "validate-crate-graph",
    "validate-storage-fixtures",
    "bench-chunk-size",
    "test-migrations",
    "replay",
];

/// 사용법 문자열. 각 명령이 정확히 한 번 나타납니다.
pub fn usage() -> String {
    let mut out = String::from(
        "xtask — Project Stellar task runner\n\nUSAGE:\n    cargo xtask <COMMAND> [ARGS]\n\nCOMMANDS:\n",
    );
    for command in COMMANDS {
        out.push_str(&format!("    {command}\n"));
    }
    out.push_str(
        "\nEXIT CODES:\n    0  성공\n    64 입력 오류\n    65 정책 위반\n    70 내부 오류\n",
    );
    out
}

/// 명령을 디스패치하고 종료 코드를 반환합니다.
pub fn run(args: &[String], repo_root: &Path) -> i32 {
    let Some(command) = args.first() else {
        eprintln!("명령이 필요합니다.");
        eprint!("{}", usage());
        return EXIT_USAGE;
    };
    let rest = &args[1..];
    let result = match command.as_str() {
        "help" => {
            print!("{}", usage());
            Ok(())
        }
        "generate-ledger" => generate_ledger_file(repo_root)
            .map(|count| println!("planning/task-status.yaml 생성 완료: {count} tasks"))
            .map_err(|e| internal(&e)),
        "generate-ffi-header" => cmd_generate_ffi_header(repo_root, rest),
        "generate-command-schema" => cmd_generate_command_schema(repo_root, rest),
        "validate-task-status" => cmd_validate_task_status(repo_root, rest),
        "validate-task-report" => cmd_validate_task_report(repo_root, rest),
        "validate-pr-body" => cmd_validate_pr_body(repo_root, rest),
        "validate-commit-message" => cmd_validate_commit_message(rest),
        "validate-versions" => cmd_validate_versions(repo_root),
        "validate-dependencies" => cmd_validate_dependencies(repo_root),
        "validate-adr" => cmd_validate_adr(rest),
        "verify-task" => cmd_verify_task(repo_root, rest),
        "sync-plan" => cmd_sync_plan(repo_root),
        "check-core" => match run_check_core(repo_root) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("check-core 실패: {}", e.message);
                Err(e.code)
            }
        },
        "validate-crate-graph" => cmd_validate_crate_graph(repo_root),
        "validate-storage-fixtures" => cmd_validate_storage_fixtures(repo_root),
        "bench-chunk-size" => match crate::chunk_bench::run_bench(repo_root) {
            Ok(path) => {
                println!("chunk 크기 벤치마크 기록 완료: {}", path.display());
                Ok(())
            }
            Err(e) => Err(internal(&e)),
        },
        "test-migrations" => match crate::migration_fixtures::run_test_migrations() {
            Ok(count) => {
                println!("migration golden fixture 통과: {count} fixtures");
                Ok(())
            }
            Err(e) => Err(policy_err(&[e])),
        },
        "replay" => crate::replay::run_cli(rest, repo_root).map_err(|(code, message)| {
            eprintln!("{message}");
            code
        }),
        other => {
            eprintln!("알 수 없는 명령: {other}");
            eprint!("{}", usage());
            return EXIT_USAGE;
        }
    };
    match result {
        Ok(()) => EXIT_OK,
        Err(code) => code,
    }
}

fn usage_err(message: &str) -> i32 {
    eprintln!("{message}");
    EXIT_USAGE
}

fn policy_err(problems: &[String]) -> i32 {
    eprintln!("정책 위반:");
    for p in problems {
        eprintln!("  - {p}");
    }
    EXIT_POLICY
}

fn internal(message: &str) -> i32 {
    eprintln!("내부 오류: {message}");
    EXIT_INTERNAL
}

fn read(repo_root: &Path, rel: &str) -> Result<String, i32> {
    std::fs::read_to_string(repo_root.join(rel))
        .map_err(|_| usage_err(&format!("파일 없음: {rel}")))
}

fn cmd_validate_task_status(repo_root: &Path, rest: &[String]) -> Result<(), i32> {
    let rel = rest
        .first()
        .map_or("planning/task-status.yaml", String::as_str);
    let content = read(repo_root, rel)?;
    let value = load_restricted(&content)
        .map_err(|e| policy_err(&[format!("loader/{}", e.kind.code())]))?;
    let records =
        decode_collection(&value).map_err(|e| policy_err(&[format!("decode/{}", e.code)]))?;
    let problems = validate_all(&records);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(policy_err(&problems))
    }
}

fn cmd_validate_task_report(repo_root: &Path, rest: &[String]) -> Result<(), i32> {
    let Some(rel) = rest.first() else {
        return Err(usage_err("사용법: validate-task-report <path>"));
    };
    let content = read(repo_root, rel)?;
    let file = Path::new(rel)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let stem = file.strip_suffix(".md").unwrap_or(file);
    let (kind, id) = match stem.strip_suffix("-blocked") {
        Some(id) => (ReportKind::Blocked, id),
        None => (ReportKind::Complete, stem),
    };
    let errors = validate_report(&content, kind, id);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(policy_err(
            &errors
                .iter()
                .map(|e| e.code.to_string())
                .collect::<Vec<_>>(),
        ))
    }
}

fn cmd_validate_pr_body(repo_root: &Path, rest: &[String]) -> Result<(), i32> {
    let Some(rel) = rest.first() else {
        return Err(usage_err("사용법: validate-pr-body <path>"));
    };
    let content = read(repo_root, rel)?;
    let errors = validate_pr_body(&content);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(policy_err(
            &errors
                .iter()
                .map(|e| e.code.to_string())
                .collect::<Vec<_>>(),
        ))
    }
}

fn cmd_validate_commit_message(rest: &[String]) -> Result<(), i32> {
    if rest.is_empty() {
        return Err(usage_err("사용법: validate-commit-message <subject>"));
    }
    let subject = rest.join(" ");
    match validate_commit_subject(&subject) {
        None => Ok(()),
        Some(code) => Err(policy_err(&[code.to_string()])),
    }
}

fn cmd_validate_versions(repo_root: &Path) -> Result<(), i32> {
    let content = read(repo_root, "config/versions.yaml")?;
    let toolchain = read(repo_root, "rust-toolchain.toml")?;
    let channel = toolchain
        .lines()
        .find_map(|l| l.trim().strip_prefix("channel = "))
        .map(|v| v.trim().trim_matches('"').to_string())
        .ok_or_else(|| internal("rust-toolchain.toml channel 없음"))?;
    let errors = validate_versions(&content, &channel);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(policy_err(
            &errors
                .iter()
                .map(|e| e.code.to_string())
                .collect::<Vec<_>>(),
        ))
    }
}

fn cmd_validate_dependencies(repo_root: &Path) -> Result<(), i32> {
    let content = read(repo_root, "config/dependencies.yaml")?;
    // XXIX.1.2: planned_for 행의 존재·완료 여부를 계획서에서 확인합니다.
    let plan = read(repo_root, "docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md")?;
    let progress = PlanProgress::from_plan(&plan);
    let names = direct_dependencies(repo_root)?;
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let errors = validate_dependencies_with(&content, &refs, Some(&progress));
    if errors.is_empty() {
        Ok(())
    } else {
        Err(policy_err(
            &errors
                .iter()
                .map(|e| format!("{}: {}", e.code, e.message))
                .collect::<Vec<_>>(),
        ))
    }
}

fn cmd_validate_storage_fixtures(repo_root: &Path) -> Result<(), i32> {
    let content = read(repo_root, "docs/fixtures/storage/canonical-primitives.yaml")?;
    let errors = check_canonical_primitives(&content);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(policy_err(
            &errors
                .iter()
                .map(|e| e.code.to_string())
                .collect::<Vec<_>>(),
        ))
    }
}

fn cmd_validate_adr(rest: &[String]) -> Result<(), i32> {
    let paths: Vec<&str> = rest.iter().map(String::as_str).collect();
    let errors = check_adr_policy(&paths);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(policy_err(
            &errors
                .iter()
                .map(|e| e.code.to_string())
                .collect::<Vec<_>>(),
        ))
    }
}

fn cmd_verify_task(repo_root: &Path, rest: &[String]) -> Result<(), i32> {
    let Some(id) = rest.first() else {
        return Err(usage_err("사용법: verify-task <WBS-ID>"));
    };
    let content = read(repo_root, "planning/task-status.yaml")?;
    let value = load_restricted(&content)
        .map_err(|e| policy_err(&[format!("loader/{}", e.kind.code())]))?;
    let records =
        decode_collection(&value).map_err(|e| policy_err(&[format!("decode/{}", e.code)]))?;

    let report_exists = records
        .iter()
        .find(|r| &r.id == id)
        .and_then(|r| r.report.as_ref())
        .is_some_and(|rel| repo_root.join(rel).is_file());
    let workspace_ready = records
        .iter()
        .any(|r| r.id == "II.1.2" && r.status == TaskState::Verified);

    let problems = verify_task(&records, id, report_exists, workspace_ready);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(policy_err(
            &problems
                .iter()
                .map(|p| p.code.to_string())
                .collect::<Vec<_>>(),
        ))
    }
}

fn cmd_sync_plan(repo_root: &Path) -> Result<(), i32> {
    let plan = read(repo_root, "docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md")?;
    let ledger_yaml = read(repo_root, "planning/task-status.yaml")?;
    let value = load_restricted(&ledger_yaml)
        .map_err(|e| policy_err(&[format!("loader/{}", e.kind.code())]))?;
    let records =
        decode_collection(&value).map_err(|e| policy_err(&[format!("decode/{}", e.code)]))?;
    let problems = sync_plan(&plan, &records);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(policy_err(
            &problems
                .iter()
                .map(|p| format!("{}: {}", p.code, p.message))
                .collect::<Vec<_>>(),
        ))
    }
}

fn cmd_validate_crate_graph(repo_root: &Path) -> Result<(), i32> {
    // XXIX.1.1: crates/*, tools/stellar-*와 Cargo workspace member 전체(자동 path member 포함,
    // xtask·bench fixture 제외) manifest의 모든 의존 표를 읽고 전이 도달도 검사합니다.
    let graph = scan_workspace(repo_root).map_err(|e| match e.code {
        "missing-crates-dir" => usage_err(&e.message),
        "io" => internal(&e.message),
        code => policy_err(&[format!("{code}: {}", e.message)]),
    })?;
    let mut violations = check_workspace_graph(&graph);
    // XXIV.4.7: `stellar` 도움말 명령과 도달 crate의 AI 무관성 inventory.
    let help_path = repo_root.join(crate::crate_graph::ai_inventory::HELP_FIXTURE);
    let help = std::fs::read_to_string(&help_path)
        .map_err(|e| internal(&format!("{}: {e}", help_path.display())))?;
    violations.extend(crate::crate_graph::ai_inventory::check_cli_ai_inventory(
        &graph, &help,
    ));
    if violations.is_empty() {
        Ok(())
    } else {
        Err(policy_err(
            &violations
                .iter()
                .map(|v| v.message.clone())
                .collect::<Vec<_>>(),
        ))
    }
}

fn cmd_generate_command_schema(repo_root: &Path, rest: &[String]) -> Result<(), i32> {
    let path = crate::command_schema::SCHEMA_PATH;
    let check = rest.iter().any(|a| a == "--check");
    match crate::command_schema::generate_file(repo_root, check) {
        Ok(true) if check => {
            println!("CommandTransaction schema drift 없음: {path}");
            Ok(())
        }
        Ok(false) => Err(policy_err(&[format!(
            "CommandTransaction schema drift: {path}가 생성 결과와 다릅니다. `cargo xtask generate-command-schema`로 재생성하세요."
        )])),
        Ok(_) => {
            println!("CommandTransaction schema 생성 완료: {path}");
            Ok(())
        }
        Err(e) => Err(internal(&e)),
    }
}

fn cmd_generate_ffi_header(repo_root: &Path, rest: &[String]) -> Result<(), i32> {
    let path = crate::ffi_header::HEADER_PATH;
    let check = rest.iter().any(|a| a == "--check");
    if check {
        match generate_header(repo_root, true) {
            Ok(true) => {
                println!("FFI 헤더 drift 없음: {path}");
                Ok(())
            }
            Ok(false) => Err(policy_err(&[format!(
                "FFI 헤더 drift: {path}가 템플릿과 다릅니다. `cargo xtask generate-ffi-header`로 재생성하세요."
            )])),
            Err(e) => Err(policy_err(&[format!("FFI 헤더 drift: {e}")])),
        }
    } else {
        match generate_header(repo_root, false) {
            Ok(_) => {
                println!("FFI 헤더 생성 완료: {path}");
                Ok(())
            }
            Err(e) => Err(internal(&e)),
        }
    }
}

/// `cargo tree`로 workspace 모든 member의 외부 직접 의존성 이름을 수집합니다(XXIX.1.2).
///
/// normal·build·dev 의존을 모든 target과 모든 feature(optional 의존 포함)에 대해 모으고, 내부
/// workspace path 의존은 제외합니다. 인자는 [`CARGO_TREE_ARGS`]입니다.
fn direct_dependencies(repo_root: &Path) -> Result<Vec<String>, i32> {
    let output = Command::new("cargo")
        .current_dir(repo_root)
        .args(CARGO_TREE_ARGS)
        .output()
        .map_err(|e| internal(&format!("cargo tree 실행 실패: {e}")))?;
    if !output.status.success() {
        return Err(internal("cargo tree 종료 코드 비0"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_workspace_direct_deps(&text).map_err(|e| internal(&format!("{}: {}", e.code, e.message)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).to_path_buf()
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn usage_lists_each_command_once() {
        let text = usage();
        for command in COMMANDS {
            let count = text.matches(&format!("    {command}\n")).count();
            assert_eq!(count, 1, "{command}");
        }
    }

    #[test]
    fn commit_message_exit_codes() {
        assert_eq!(
            run(
                &args(&["validate-commit-message", "Task(I.1.1): t"]),
                &root()
            ),
            EXIT_OK
        );
        assert_eq!(
            run(&args(&["validate-commit-message", "Docs: x"]), &root()),
            EXIT_OK
        );
        assert_eq!(
            run(&args(&["validate-commit-message", "Style: x"]), &root()),
            EXIT_POLICY
        );
        assert_eq!(
            run(&args(&["validate-commit-message"]), &root()),
            EXIT_USAGE
        );
    }

    #[test]
    fn adr_exit_codes() {
        assert_eq!(
            run(&args(&["validate-adr", "crates/storage/x.rs"]), &root()),
            EXIT_POLICY
        );
        assert_eq!(run(&args(&["validate-adr", "docs/x.md"]), &root()), EXIT_OK);
    }

    #[test]
    fn unknown_and_empty_are_usage_errors() {
        assert_eq!(run(&args(&["bogus"]), &root()), EXIT_USAGE);
        assert_eq!(run(&[], &root()), EXIT_USAGE);
    }

    #[test]
    fn help_succeeds() {
        assert_eq!(run(&args(&["help"]), &root()), EXIT_OK);
    }

    #[test]
    fn validate_versions_and_status_on_real_files() {
        assert_eq!(run(&args(&["validate-versions"]), &root()), EXIT_OK);
        assert_eq!(run(&args(&["validate-task-status"]), &root()), EXIT_OK);
    }

    /// 테스트가 끝나면 지우는 임시 디렉터리.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            let dir =
                std::env::temp_dir().join(format!("xtask-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn direct_dependencies_include_optional_dependencies() {
        // workspace 밖 path crate(`ext`)를 기본 feature로 켜지지 않는 optional 의존으로 둡니다.
        // workspace 디렉터리 밖에 있어야 자동 member가 되지 않고 외부 직접 의존으로 보입니다.
        let tmp = TempDir::new("optional-deps");
        tmp.write(
            "ws/Cargo.toml",
            "[workspace]\nresolver = \"2\"\nmembers = [\"a\"]\n",
        );
        tmp.write(
            "ws/a/Cargo.toml",
            "[package]\nname = \"stellar-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [dependencies]\nextdep = { path = \"../../ext\", optional = true }\n\n\
             [features]\nx = [\"dep:extdep\"]\n",
        );
        tmp.write("ws/a/src/lib.rs", "");
        tmp.write(
            "ext/Cargo.toml",
            "[package]\nname = \"extdep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        tmp.write("ext/src/lib.rs", "");
        let ws = tmp.0.join("ws");

        assert_eq!(direct_dependencies(&ws), Ok(vec!["extdep".to_string()]));

        // `--all-features`를 빼면 같은 의존이 출력에서 빠집니다(이 fixture가 optional 경로를 검사함).
        let without: Vec<&str> = CARGO_TREE_ARGS
            .iter()
            .copied()
            .filter(|arg| *arg != "--all-features")
            .collect();
        let output = Command::new("cargo")
            .current_dir(&ws)
            .args(&without)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8_lossy(&output.stdout);
        assert_eq!(parse_workspace_direct_deps(&text), Ok(Vec::new()));
    }

    #[test]
    fn validate_crate_graph_on_real_workspace() {
        // XXIX.1.1: 검사 대상 manifest의 모든 의존 표와 전이 도달 규칙을 통과합니다.
        assert_eq!(run(&args(&["validate-crate-graph"]), &root()), EXIT_OK);
    }
}
