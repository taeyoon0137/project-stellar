//! reset/restore — 파괴 수준별 확인과 명령 미리보기 (XXVI.2.3).
//!
//! reset·restore는 모두 [`CommandPlan`]으로 먼저 만들어 미리보기(`git reset --hard HEAD~1` 등)와 파괴 수준을
//! 보여 주고, 사용자 확인([`Confirmation`]) 없이는 실행하지 않습니다(프로세스 생성 0).
//!
//! | 명령 | 수준 | 이유 |
//! |---|---|---|
//! | `reset --soft <target>` | [`DestructiveLevel::DiscardsRefs`] | branch를 옮김(이후 commit은 reflog로만 복구) |
//! | `reset --mixed <target>` | [`DestructiveLevel::DiscardsRefs`] | branch 이동 + staging 내용 폐기 |
//! | `reset --hard <target>` | [`DestructiveLevel::DiscardsWork`] | 커밋되지 않은 작업 트리·index 변경 폐기 |
//! | `restore --staged <paths>` | [`DestructiveLevel::DiscardsRefs`] | staged 내용 폐기(작업 트리 유지) |
//! | `restore --worktree <paths>` | [`DestructiveLevel::DiscardsWork`] | 작업 트리 변경 폐기 |
//!
//! 대상(target·source)은 `HEAD`, `HEAD~<n>`, `HEAD^`, 안전한 ref 이름, object ID만 받습니다. 경로는
//! `:(literal)` pathspec으로 넘겨 glob로 해석되지 않게 합니다.

use std::ffi::OsString;

use super::{CommandPlan, DestructiveLevel, PlanError};
use crate::decode::GitPath;
use crate::object::ObjectId;
use crate::refname::is_valid_ref_body;
use crate::runner::argv;

/// reset 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResetMode {
    /// `--soft`.
    Soft,
    /// `--mixed`.
    Mixed,
    /// `--hard`.
    Hard,
}

impl ResetMode {
    fn flag(self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        }
    }

    /// 파괴 수준.
    pub fn level(self) -> DestructiveLevel {
        match self {
            ResetMode::Soft | ResetMode::Mixed => DestructiveLevel::DiscardsRefs,
            ResetMode::Hard => DestructiveLevel::DiscardsWork,
        }
    }
}

/// reset·restore 대상 revision을 검증합니다.
pub fn is_valid_target(rev: &str) -> bool {
    if rev == "HEAD" || rev == "HEAD^" || ObjectId::parse(rev).is_some() {
        return true;
    }
    if let Some(n) = rev.strip_prefix("HEAD~") {
        return !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit());
    }
    is_valid_ref_body(rev)
}

/// reset 계획을 만듭니다.
pub fn plan_reset(mode: ResetMode, target: &str) -> Result<CommandPlan, PlanError> {
    if !is_valid_target(target) {
        return Err(PlanError::InvalidInput("reset 대상"));
    }
    Ok(CommandPlan::new(
        argv(["reset", "-q", mode.flag(), target, "--"]),
        mode.level(),
    ))
}

/// restore 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RestoreScope {
    /// index만(unstage).
    Staged,
    /// 작업 트리만.
    Worktree,
    /// index와 작업 트리 모두.
    Both,
}

#[cfg(unix)]
fn literal_pathspec(path: &GitPath) -> OsString {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = b":(literal)".to_vec();
    bytes.extend_from_slice(path.as_bytes());
    std::ffi::OsStr::from_bytes(&bytes).to_os_string()
}

#[cfg(not(unix))]
fn literal_pathspec(path: &GitPath) -> OsString {
    OsString::from(format!(":(literal){}", path.text().display()))
}

/// restore 계획을 만듭니다(`source`가 없으면 Git 기본: staged는 HEAD, worktree는 index).
pub fn plan_restore(
    paths: &[GitPath],
    scope: RestoreScope,
    source: Option<&str>,
) -> Result<CommandPlan, PlanError> {
    if paths.is_empty()
        || paths
            .iter()
            .any(|p| p.as_bytes().is_empty() || p.as_bytes().contains(&0))
    {
        return Err(PlanError::InvalidInput("restore 경로"));
    }
    let mut args = argv(["restore"]);
    let level = match scope {
        RestoreScope::Staged => {
            args.extend(argv(["--staged"]));
            DestructiveLevel::DiscardsRefs
        }
        RestoreScope::Worktree => {
            args.extend(argv(["--worktree"]));
            DestructiveLevel::DiscardsWork
        }
        RestoreScope::Both => {
            args.extend(argv(["--staged", "--worktree"]));
            DestructiveLevel::DiscardsWork
        }
    };
    if let Some(source) = source {
        if !is_valid_target(source) {
            return Err(PlanError::InvalidInput("restore source"));
        }
        args.push(OsString::from(format!("--source={source}")));
    }
    args.push(OsString::from("--"));
    args.extend(paths.iter().map(literal_pathspec));
    Ok(CommandPlan::new(args, level))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_have_levels_and_previews() {
        let soft = plan_reset(ResetMode::Soft, "HEAD~1").unwrap();
        assert_eq!(soft.level(), DestructiveLevel::DiscardsRefs);
        assert_eq!(soft.preview(), "git reset -q --soft HEAD~1 --");
        let hard = plan_reset(ResetMode::Hard, "main").unwrap();
        assert_eq!(hard.level(), DestructiveLevel::DiscardsWork);
        let restore = plan_restore(
            &[GitPath::from_bytes(b"design/a b.txt")],
            RestoreScope::Worktree,
            Some("HEAD"),
        )
        .unwrap();
        assert_eq!(restore.level(), DestructiveLevel::DiscardsWork);
        assert_eq!(
            restore.preview(),
            "git restore --worktree --source=HEAD -- ':(literal)design/a b.txt'"
        );
        let staged =
            plan_restore(&[GitPath::from_bytes(b"x")], RestoreScope::Staged, None).unwrap();
        assert_eq!(staged.level(), DestructiveLevel::DiscardsRefs);
        assert!(DestructiveLevel::DiscardsRefs.requires_confirmation());
    }

    #[test]
    fn invalid_targets_and_paths_are_rejected() {
        for bad in [
            "", "--hard", "-q", "HEAD~", "HEAD~x", "a..b", "@{-1}", "HEAD@{1}",
        ] {
            assert!(plan_reset(ResetMode::Mixed, bad).is_err(), "{bad:?}");
        }
        for ok in [
            "HEAD",
            "HEAD^",
            "HEAD~3",
            "main",
            "feature/x",
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
        ] {
            assert!(plan_reset(ResetMode::Mixed, ok).is_ok(), "{ok}");
        }
        assert!(plan_restore(&[], RestoreScope::Both, None).is_err());
        assert!(plan_restore(&[GitPath::from_bytes(b"a\0b")], RestoreScope::Both, None).is_err());
        assert!(
            plan_restore(&[GitPath::from_bytes(b"a")], RestoreScope::Both, Some("-x")).is_err()
        );
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::advanced::{execute_plan, Confirmation};
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::{GatedGitRunner, GitTrust};
        use std::path::Path;

        fn head(runner: &crate::runner::GitRunner, repo: &Path) -> Vec<u8> {
            git_ok(runner, repo, ["rev-parse", "HEAD"]).stdout
        }

        #[test]
        fn hard_reset_without_confirmation_spawns_nothing() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("adv-reset");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "a.txt", b"1\n");
            commit_file(&runner, repo, "a.txt", b"2\n");
            std::fs::write(repo.join("a.txt"), b"dirty work\n").unwrap();
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);

            let plan = plan_reset(ResetMode::Hard, "HEAD~1").unwrap();
            let head_before = head(&runner, repo);
            let spawns = runner.spawn_count();
            assert!(matches!(
                execute_plan(&gate, repo, &plan, None),
                Err(PlanError::ConfirmationRequired {
                    level: DestructiveLevel::DiscardsWork,
                    ..
                })
            ));
            // 다른 계획의 확인도 거부.
            let soft = plan_reset(ResetMode::Soft, "HEAD~1").unwrap();
            assert!(execute_plan(&gate, repo, &plan, Some(&Confirmation::confirm(&soft))).is_err());
            assert_eq!(runner.spawn_count(), spawns, "확인 없이 실행됨");
            assert_eq!(head(&runner, repo), head_before);
            assert_eq!(std::fs::read(repo.join("a.txt")).unwrap(), b"dirty work\n");

            execute_plan(&gate, repo, &plan, Some(&Confirmation::confirm(&plan))).unwrap();
            assert_ne!(head(&runner, repo), head_before);
            assert_eq!(std::fs::read(repo.join("a.txt")).unwrap(), b"1\n");
        }

        #[test]
        fn soft_mixed_and_restore_levels_execute_after_confirmation() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("adv-restore");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "a.txt", b"1\n");
            commit_file(&runner, repo, "a.txt", b"2\n");
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let run = |plan: &CommandPlan| {
                execute_plan(&gate, repo, plan, Some(&Confirmation::confirm(plan))).unwrap();
            };

            // soft: HEAD만 이동, index·작업 트리는 2 유지(staged 변경으로 보임).
            run(&plan_reset(ResetMode::Soft, "HEAD~1").unwrap());
            let cached = git_ok(&runner, repo, ["diff", "--cached", "--name-only"]);
            assert_eq!(String::from_utf8_lossy(&cached.stdout).trim(), "a.txt");
            assert_eq!(std::fs::read(repo.join("a.txt")).unwrap(), b"2\n");
            // restore --staged: unstage(작업 트리 유지).
            run(
                &plan_restore(&[GitPath::from_bytes(b"a.txt")], RestoreScope::Staged, None)
                    .unwrap(),
            );
            assert!(git_ok(&runner, repo, ["diff", "--cached", "--name-only"])
                .stdout
                .is_empty());
            assert_eq!(std::fs::read(repo.join("a.txt")).unwrap(), b"2\n");
            // restore --worktree: 작업 트리 변경 폐기.
            run(&plan_restore(
                &[GitPath::from_bytes(b"a.txt")],
                RestoreScope::Worktree,
                None,
            )
            .unwrap());
            assert_eq!(std::fs::read(repo.join("a.txt")).unwrap(), b"1\n");
            // mixed: staged 내용 폐기, 작업 트리 유지.
            std::fs::write(repo.join("a.txt"), b"3\n").unwrap();
            git_ok(&runner, repo, ["add", "a.txt"]);
            run(&plan_reset(ResetMode::Mixed, "HEAD").unwrap());
            assert!(git_ok(&runner, repo, ["diff", "--cached", "--name-only"])
                .stdout
                .is_empty());
            assert_eq!(std::fs::read(repo.join("a.txt")).unwrap(), b"3\n");
            // literal pathspec: glob 문자 경로가 다른 파일을 건드리지 않음.
            std::fs::write(repo.join("b*.txt"), b"x").unwrap();
            commit_file(&runner, repo, "b*.txt", b"glob\n");
            commit_file(&runner, repo, "bz.txt", b"z\n");
            std::fs::write(repo.join("b*.txt"), b"changed\n").unwrap();
            std::fs::write(repo.join("bz.txt"), b"changed\n").unwrap();
            run(&plan_restore(
                &[GitPath::from_bytes(b"b*.txt")],
                RestoreScope::Worktree,
                None,
            )
            .unwrap());
            assert_eq!(std::fs::read(repo.join("b*.txt")).unwrap(), b"glob\n");
            assert_eq!(std::fs::read(repo.join("bz.txt")).unwrap(), b"changed\n");
        }
    }
}
