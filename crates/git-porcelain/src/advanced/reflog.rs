//! reflog 조회와 선택 항목 복구 (XXVI.2.4).
//!
//! - 조회: `git log -g --format=%H%x00%gD%x00%gs <ref> --`(commit·selector·reflog subject, 줄 단위 + NUL 필드).
//!   `<ref>`는 `HEAD` 또는 안전한 branch 이름만 받습니다.
//! - 복구: 선택한 reflog 항목의 commit에 **새 branch**를 만듭니다(`git branch --no-track -- <name> <oid>`).
//!   기존 branch·작업 트리를 바꾸지 않는 비파괴 복구라 [`DestructiveLevel::Safe`]입니다. 현재 branch를 그
//!   commit으로 되돌리려면 XXVI.2.3 reset plan(확인 필요)을 씁니다.

use std::path::Path;

use super::{execute_plan, CommandPlan, DestructiveLevel, PlanError};
use crate::object::ObjectId;
use crate::ops::{run_ok, GitOpError, ParseFailure};
use crate::refname::is_valid_ref_body;
use crate::runner::argv;
use crate::trust::GatedGitRunner;

/// reflog 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflogEntry {
    /// 그 시점의 commit.
    pub commit: ObjectId,
    /// selector(`HEAD@{2}`, `refs/heads/main@{0}` 등).
    pub selector: String,
    /// reflog subject(`commit: …`, `reset: moving to …` 등).
    pub subject: String,
}

const COMMAND: &str = "log -g";

/// reflog 출력을 해석합니다.
pub fn parse_reflog(stdout: &[u8]) -> Result<Vec<ReflogEntry>, ParseFailure> {
    let text = String::from_utf8_lossy(stdout);
    let mut out = Vec::new();
    for line in text.lines() {
        let bad = |d: &str| ParseFailure::new(COMMAND, d, line.as_bytes());
        let mut fields = line.splitn(3, '\0');
        let (Some(oid), Some(selector), Some(subject)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Err(bad("필드 수가 3개가 아님"));
        };
        if !selector.contains("@{") {
            return Err(bad("selector 형식 아님"));
        }
        out.push(ReflogEntry {
            commit: ObjectId::parse(oid).ok_or_else(|| bad("잘못된 object ID"))?,
            selector: selector.to_string(),
            subject: subject.to_string(),
        });
    }
    Ok(out)
}

/// `HEAD` 또는 branch의 reflog(최신 먼저). reflog가 없으면 Git 오류를 그대로 돌려줍니다.
pub fn read_reflog(
    git: &GatedGitRunner,
    worktree: &Path,
    reference: &str,
) -> Result<Vec<ReflogEntry>, PlanError> {
    if reference != "HEAD" && !is_valid_ref_body(reference) {
        return Err(PlanError::InvalidInput("reflog 대상"));
    }
    let out = run_ok(
        git,
        worktree,
        &argv(["log", "-g", "--format=%H%x00%gD%x00%gs", reference, "--"]),
    )?;
    Ok(parse_reflog(&out.stdout).map_err(GitOpError::Parse)?)
}

/// 선택한 reflog 항목에 새 branch를 만드는 복구 계획(비파괴).
pub fn plan_recover_branch(
    entry: &ReflogEntry,
    new_branch: &str,
) -> Result<CommandPlan, PlanError> {
    if !is_valid_ref_body(new_branch) {
        return Err(PlanError::InvalidInput("branch 이름"));
    }
    Ok(CommandPlan::new(
        argv([
            "branch",
            "--no-track",
            "--",
            new_branch,
            entry.commit.as_str(),
        ]),
        DestructiveLevel::Safe,
    ))
}

/// 복구 계획을 실행합니다(같은 이름의 branch가 있으면 Git이 거부).
pub fn recover_branch(
    git: &GatedGitRunner,
    worktree: &Path,
    plan: &CommandPlan,
) -> Result<(), PlanError> {
    execute_plan(git, worktree, plan, None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const E1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

    #[test]
    fn parses_reflog_lines() {
        let out = format!(
            "{E1}\0HEAD@{{0}}\0reset: moving to HEAD~1\n{E1}\0HEAD@{{1}}\0commit: 두 번째\n"
        );
        let list = parse_reflog(out.as_bytes()).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].subject, "commit: 두 번째");
        assert!(parse_reflog(format!("{E1}\0HEAD\0x\n").as_bytes()).is_err());
        assert!(parse_reflog(b"zz\0HEAD@{0}\0x\n").is_err());
        assert!(plan_recover_branch(&list[0], "-f").is_err());
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;

        #[test]
        fn reflog_fixture_recovers_lost_commit() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("adv-reflog");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "a.txt", b"1\n");
            commit_file(&runner, repo, "a.txt", b"lost work\n");
            let lost = git_ok(&runner, repo, ["rev-parse", "HEAD"]);
            let lost = ObjectId::from_hex_bytes(lost.stdout.trim_ascii()).unwrap();
            git_ok(&runner, repo, ["reset", "-q", "--hard", "HEAD~1"]);

            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let reflog = read_reflog(&gate, repo, "HEAD").unwrap();
            assert!(reflog[0].subject.starts_with("reset:"), "{:?}", reflog[0]);
            let entry = reflog.iter().find(|e| e.commit == lost).unwrap();
            assert_eq!(entry.selector, "HEAD@{1}");
            let branch_log = read_reflog(&gate, repo, "main").unwrap();
            assert!(branch_log.iter().any(|e| e.commit == lost));

            let plan = plan_recover_branch(entry, "rescued").unwrap();
            assert_eq!(plan.level(), DestructiveLevel::Safe);
            let head_before = git_ok(&runner, repo, ["rev-parse", "HEAD"]).stdout;
            recover_branch(&gate, repo, &plan).unwrap();
            let rescued = git_ok(&runner, repo, ["rev-parse", "rescued"]);
            assert_eq!(rescued.stdout.trim_ascii(), lost.as_str().as_bytes());
            // 복구는 현재 branch·작업 트리를 바꾸지 않음.
            assert_eq!(
                git_ok(&runner, repo, ["rev-parse", "HEAD"]).stdout,
                head_before
            );
            assert_eq!(std::fs::read(repo.join("a.txt")).unwrap(), b"1\n");
            // 같은 이름으로 다시 복구하면 Git이 거부(덮어쓰지 않음).
            assert!(matches!(
                recover_branch(&gate, repo, &plan),
                Err(PlanError::Op(GitOpError::Git(_)))
            ));
            // reflog 없는 대상·잘못된 이름.
            assert!(read_reflog(&gate, repo, "no-such-branch").is_err());
            assert!(matches!(
                read_reflog(&gate, repo, "--all"),
                Err(PlanError::InvalidInput(_))
            ));
        }
    }
}
