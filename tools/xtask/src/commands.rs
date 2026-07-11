//! 명령 registry (I.4.1).
//!
//! `help`와 검증 명령을 모듈형으로 연결합니다.
//! 종료 코드: 성공 `0`, 입력 오류 `64`, 정책 위반 `65`, 내부 오류 `70`.

use std::path::Path;
use std::process::Command;

use crate::adr_policy::check_adr_policy;
use crate::check_core::run_check_core;
use crate::commit_message::validate_commit_subject;
use crate::crate_graph::check_crate_graph;
use crate::dependencies::validate_dependencies;
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
pub const COMMANDS: [&str; 17] = [
    "help",
    "generate-ledger",
    "generate-ffi-header",
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
    let names = direct_dependencies(repo_root)?;
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let errors = validate_dependencies(&content, &refs);
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
                .map(|p| p.code.to_string())
                .collect::<Vec<_>>(),
        ))
    }
}

fn cmd_validate_crate_graph(repo_root: &Path) -> Result<(), i32> {
    let edges = read_crate_edges(repo_root)?;
    let violations = check_crate_graph(&edges);
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

/// `crates/*/Cargo.toml`에서 내부(stellar-*) 직접 의존 edge를 추출합니다.
fn read_crate_edges(repo_root: &Path) -> Result<Vec<(String, String)>, i32> {
    let dir = repo_root.join("crates");
    let entries = std::fs::read_dir(&dir).map_err(|_| usage_err("crates 디렉터리 없음"))?;
    let mut edges = Vec::new();
    for entry in entries.flatten() {
        let manifest_path = entry.path().join("Cargo.toml");
        if !manifest_path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest_path).map_err(|_| EXIT_INTERNAL)?;
        let Some(name) = manifest_package_name(&text) else {
            continue;
        };
        for dep in manifest_internal_deps(&text) {
            edges.push((name.clone(), dep));
        }
    }
    Ok(edges)
}

fn manifest_package_name(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name = ") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn manifest_internal_deps(text: &str) -> Vec<String> {
    let Some(after) = text.split("[dependencies]").nth(1) else {
        return Vec::new();
    };
    let section = after.split("\n[").next().unwrap_or(after);
    section
        .lines()
        .filter_map(|line| {
            let key = line.split('=').next()?.trim();
            if key.starts_with("stellar-") {
                Some(key.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// `cargo tree`로 xtask의 직접 의존성 이름을 수집합니다.
fn direct_dependencies(repo_root: &Path) -> Result<Vec<String>, i32> {
    let output = Command::new("cargo")
        .current_dir(repo_root)
        .args(["tree", "-p", "xtask", "-e", "normal", "--depth", "1"])
        .output()
        .map_err(|e| internal(&format!("cargo tree 실행 실패: {e}")))?;
    if !output.status.success() {
        return Err(internal("cargo tree 종료 코드 비0"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut names = Vec::new();
    for line in text.lines().skip(1) {
        if let Some(rest) = line.split("── ").nth(1) {
            // 내부 workspace crate(path 의존, "(/…)" 표기)는 외부 공급망 승인
            // 대상이 아니므로 제외합니다(I.3.2의 범위는 외부 dependency).
            if rest.contains(" (/") {
                continue;
            }
            if let Some(name) = rest.split_whitespace().next() {
                names.push(name.to_string());
            }
        }
    }
    Ok(names)
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
}
