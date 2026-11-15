//! `cargo xtask check-core` (II.1.6).
//!
//! 먼저 기준 커밋 대비 정책 검사(사용자 결정 D9: ADR 보호 경로, 커밋 제목)를 하고, 전체 workspace fmt → clippy →
//! test → doc test를 고정 순서로 실행해 첫 실패에서 중단합니다. 명령별 로그를 `target/xtask-logs/<step>.log`에
//! 저장합니다. Phase I의 xtask-only 부트스트랩 검사와 이름·동작을 혼합하지 않습니다. CI에는 넣지 않습니다(제품
//! 검사는 로컬).
//!
//! 기준 커밋은 `STELLAR_CHECK_BASE`(커밋 표현)가 있으면 그 값, 없으면 현재 branch의 upstream, 그것도 없으면
//! `origin/main`과 HEAD의 merge-base입니다. 그 뒤 작업 트리(미커밋·추적 안 된 파일 포함)에서 바뀐 경로에
//! `validate-adr`와 같은 규칙을, merge가 아닌 각 커밋 제목에 `validate-commit-message`와 같은 규칙을 적용합니다.

use std::path::Path;
use std::process::Command;

use crate::adr_policy::check_adr_policy;
use crate::commit_message::validate_commit_subject;

/// check-core 단계 정의.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckStep {
    pub name: &'static str,
    pub args: &'static [&'static str],
}

/// 고정 실행 순서.
pub const STEPS: [CheckStep; 4] = [
    CheckStep {
        name: "fmt",
        args: &["fmt", "--all", "--", "--check"],
    },
    CheckStep {
        name: "clippy",
        args: &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    },
    CheckStep {
        name: "test",
        args: &["test", "--workspace", "--locked"],
    },
    CheckStep {
        name: "doc-test",
        args: &["test", "--workspace", "--doc", "--locked"],
    },
];

/// check-core 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckCoreError {
    /// 입력/실행 불가(70) 또는 검사 실패(65).
    pub code: i32,
    pub message: String,
}

/// 단계를 순서대로 실행하고 첫 실패에서 멈춥니다(순수 로직, 테스트용).
///
/// `exec`는 각 단계를 실행해 성공 여부를 반환합니다. 실패 시 그 단계 이름을 반환합니다.
pub fn run_steps<F>(steps: &[CheckStep], mut exec: F) -> Result<(), &'static str>
where
    F: FnMut(&CheckStep) -> bool,
{
    for step in steps {
        if !exec(step) {
            return Err(step.name);
        }
    }
    Ok(())
}

/// 기준 커밋 대비 정책 위반 목록(순수 로직, 테스트용). 빈 Vec이면 통과입니다.
pub fn policy_problems(changed_paths: &[&str], subjects: &[&str]) -> Vec<String> {
    let mut problems: Vec<String> = check_adr_policy(changed_paths)
        .into_iter()
        .map(|error| format!("{}: {}", error.code, error.message))
        .collect();
    for subject in subjects {
        if let Some(code) = validate_commit_subject(subject) {
            problems.push(format!("커밋 제목 {code}: {subject}"));
        }
    }
    problems
}

fn git(repo_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 기준 커밋(merge-base)을 정합니다(모듈 문서).
fn base_commit(repo_root: &Path) -> Result<String, CheckCoreError> {
    let requested = std::env::var("STELLAR_CHECK_BASE").ok();
    let candidates = requested
        .iter()
        .map(String::as_str)
        .chain(["@{upstream}", "origin/main"]);
    for candidate in candidates {
        if let Some(base) = git(repo_root, &["merge-base", "HEAD", candidate]) {
            return Ok(base.trim().to_owned());
        }
        if requested.as_deref() == Some(candidate) {
            return Err(CheckCoreError {
                code: 70,
                message: format!(
                    "STELLAR_CHECK_BASE {candidate}와 HEAD의 merge-base를 찾지 못했습니다"
                ),
            });
        }
    }
    Err(CheckCoreError {
        code: 70,
        message: "기준 커밋을 정하지 못했습니다(upstream·origin/main 없음). STELLAR_CHECK_BASE로 지정하세요".into(),
    })
}

/// 기준 커밋 대비 ADR 보호 경로·커밋 제목 검사(사용자 결정 D9).
pub fn run_policy_checks(repo_root: &Path) -> Result<(), CheckCoreError> {
    let base = base_commit(repo_root)?;
    let internal = |what: &str| CheckCoreError {
        code: 70,
        message: format!("git {what}을 읽지 못했습니다"),
    };
    let diff = git(repo_root, &["diff", "--name-only", &base]).ok_or_else(|| internal("diff"))?;
    let untracked = git(repo_root, &["ls-files", "--others", "--exclude-standard"])
        .ok_or_else(|| internal("ls-files"))?;
    let log = git(
        repo_root,
        &[
            "log",
            "--no-merges",
            "--format=%s",
            &format!("{base}..HEAD"),
        ],
    )
    .ok_or_else(|| internal("log"))?;
    let changed: Vec<&str> = diff.lines().chain(untracked.lines()).collect();
    let subjects: Vec<&str> = log.lines().collect();
    let problems = policy_problems(&changed, &subjects);
    if problems.is_empty() {
        return Ok(());
    }
    Err(CheckCoreError {
        code: 65,
        message: format!(
            "기준 커밋 {base} 대비 정책 위반:\n  {}",
            problems.join("\n  ")
        ),
    })
}

/// 실제 cargo 명령으로 check-core를 실행합니다. 정책 검사가 먼저입니다.
pub fn run_check_core(repo_root: &Path) -> Result<(), CheckCoreError> {
    run_policy_checks(repo_root)?;
    let log_dir = repo_root.join("target/xtask-logs");
    std::fs::create_dir_all(&log_dir).map_err(|e| CheckCoreError {
        code: 70,
        message: format!("로그 디렉터리 생성 실패: {e}"),
    })?;

    let mut spawn_error: Option<String> = None;
    let result = run_steps(&STEPS, |step| {
        match Command::new("cargo")
            .current_dir(repo_root)
            .args(step.args)
            .output()
        {
            Ok(output) => {
                let mut log = output.stdout.clone();
                log.extend_from_slice(&output.stderr);
                let _ = std::fs::write(log_dir.join(format!("{}.log", step.name)), &log);
                output.status.success()
            }
            Err(e) => {
                spawn_error = Some(format!("{}: {e}", step.name));
                false
            }
        }
    });

    match result {
        Ok(()) => Ok(()),
        Err(step_name) => {
            if let Some(message) = spawn_error {
                Err(CheckCoreError { code: 70, message })
            } else {
                Err(CheckCoreError {
                    code: 65,
                    message: format!("check-core 단계 실패: {step_name} (로그: target/xtask-logs/{step_name}.log)"),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_in_fixed_order() {
        let names: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
        assert_eq!(names, ["fmt", "clippy", "test", "doc-test"]);
    }

    #[test]
    fn policy_checks_use_the_adr_and_commit_subject_rules() {
        assert!(policy_problems(&["docs/x.md"], &["Docs: 문서 고침"]).is_empty());
        assert!(policy_problems(
            &["crates/storage/src/x.rs", "docs/adr/ADR-0061-x.md"],
            &["Fix: 저장 고침"]
        )
        .is_empty());
        let problems = policy_problems(&["crates/ffi/src/lib.rs"], &["fix: 소문자", "Docs: 정상"]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].starts_with("missing-adr"));
        assert!(problems[1].contains("fix: 소문자"));
    }

    #[test]
    fn stops_at_first_failure() {
        let mut executed = Vec::new();
        let result = run_steps(&STEPS, |step| {
            executed.push(step.name);
            step.name != "clippy" // clippy에서 실패
        });
        assert_eq!(result, Err("clippy"));
        // clippy 이후(test, doc-test)는 실행되지 않아야 함.
        assert_eq!(executed, ["fmt", "clippy"]);
    }

    #[test]
    fn runs_all_steps_on_success() {
        let mut executed = Vec::new();
        let result = run_steps(&STEPS, |step| {
            executed.push(step.name);
            true
        });
        assert_eq!(result, Ok(()));
        assert_eq!(executed.len(), 4);
    }
}
