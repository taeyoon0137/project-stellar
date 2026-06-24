//! `cargo xtask check-core` (II.1.6).
//!
//! 전체 workspace fmt → clippy → test → doc test를 고정 순서로 실행하고 첫 실패에서 중단합니다.
//! 명령별 로그를 `target/xtask-logs/<step>.log`에 저장합니다.
//! Phase I의 xtask-only 부트스트랩 검사와 이름·동작을 혼합하지 않습니다.

use std::path::Path;
use std::process::Command;

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

/// 실제 cargo 명령으로 check-core를 실행합니다.
pub fn run_check_core(repo_root: &Path) -> Result<(), CheckCoreError> {
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
