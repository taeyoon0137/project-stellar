//! branch 목록·전환·생성·삭제 (XXVI.1.1).
//!
//! - 목록: `git for-each-ref --format=<NUL 구분 필드> refs/heads`(branch 이름·commit·HEAD 여부·upstream·
//!   ahead/behind). ref 이름에는 개행·NUL이 올 수 없으므로 줄 단위 + NUL 필드 해석이 안전합니다.
//! - 전환: `git switch --no-guess <branch>`. 같은 이름의 remote branch로 새 branch를 자동 생성하지 않으며,
//!   작업 트리 변경과 충돌하면 Git이 거부한 결과를 그대로 돌려줍니다(`--discard-changes` 미제공).
//! - 생성: `git branch --no-track -- <name> [<start>]`.
//! - 삭제: 기본은 `git branch -d`(병합되지 않은 branch는 Git이 거부, [`DestructiveLevel::Safe`]). 강제
//!   삭제(`-D`)는 [`DestructiveLevel::DiscardsRefs`] plan으로만 만들며 사용자 확인 후 실행합니다.
//!
//! branch 이름은 [`is_valid_ref_body`](crate::refname::is_valid_ref_body)로 먼저 검증합니다(위반 시 Git 미실행).

use std::path::Path;

use super::{execute_plan, CommandPlan, Confirmation, DestructiveLevel, PlanError};
use crate::object::ObjectId;
use crate::ops::{run_ok, GitOpError, ParseFailure};
use crate::refname::is_valid_ref_body;
use crate::runner::argv;
use crate::trust::GatedGitRunner;

/// upstream 대비 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackState {
    /// upstream 없음.
    NoUpstream,
    /// upstream과 같음.
    InSync,
    /// 앞섬·뒤처짐.
    Diverged {
        /// 앞선 commit 수.
        ahead: u64,
        /// 뒤처진 commit 수.
        behind: u64,
    },
    /// upstream이 사라짐.
    Gone,
}

/// local branch 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    /// branch 이름(`refs/heads/` 제외).
    pub name: String,
    /// 가리키는 commit.
    pub commit: ObjectId,
    /// 현재 HEAD branch인지.
    pub is_head: bool,
    /// upstream 짧은 이름.
    pub upstream: Option<String>,
    /// upstream 대비 상태.
    pub track: TrackState,
}

const FORMAT: &str =
    "--format=%(refname)%00%(objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream:track,nobracket)";
const COMMAND: &str = "for-each-ref refs/heads";

fn parse_track(raw: &str, has_upstream: bool) -> Option<TrackState> {
    if !has_upstream {
        return raw.is_empty().then_some(TrackState::NoUpstream);
    }
    match raw {
        "" => Some(TrackState::InSync),
        "gone" => Some(TrackState::Gone),
        _ => {
            let (mut ahead, mut behind) = (0, 0);
            for part in raw.split(", ") {
                let (key, n) = part.split_once(' ')?;
                let n: u64 = n.parse().ok()?;
                match key {
                    "ahead" => ahead = n,
                    "behind" => behind = n,
                    _ => return None,
                }
            }
            Some(TrackState::Diverged { ahead, behind })
        }
    }
}

/// branch 목록 출력을 해석합니다.
pub fn parse_branch_list(stdout: &[u8]) -> Result<Vec<BranchInfo>, ParseFailure> {
    let text = std::str::from_utf8(stdout)
        .map_err(|_| ParseFailure::new(COMMAND, "UTF-8 아님", stdout))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let bad = |d: &str| ParseFailure::new(COMMAND, d, line.as_bytes());
        let fields: Vec<&str> = line.split('\0').collect();
        let [refname, oid, head, upstream, track] = fields[..] else {
            return Err(bad("필드 수가 5개가 아님"));
        };
        let name = refname
            .strip_prefix("refs/heads/")
            .filter(|n| !n.is_empty())
            .ok_or_else(|| bad("refs/heads/ 아님"))?;
        let commit = ObjectId::parse(oid).ok_or_else(|| bad("잘못된 object ID"))?;
        let is_head = match head {
            "*" => true,
            " " | "" => false,
            _ => return Err(bad("잘못된 HEAD 표시")),
        };
        let upstream = (!upstream.is_empty()).then(|| upstream.to_string());
        let track = parse_track(track, upstream.is_some()).ok_or_else(|| bad("잘못된 track"))?;
        out.push(BranchInfo {
            name: name.to_string(),
            commit,
            is_head,
            upstream,
            track,
        });
    }
    Ok(out)
}

/// local branch 목록(이름 순).
pub fn list_branches(git: &GatedGitRunner, worktree: &Path) -> Result<Vec<BranchInfo>, GitOpError> {
    let out = run_ok(git, worktree, &argv(["for-each-ref", FORMAT, "refs/heads"]))?;
    Ok(parse_branch_list(&out.stdout)?)
}

fn require_name(name: &str) -> Result<(), PlanError> {
    if is_valid_ref_body(name) {
        Ok(())
    } else {
        Err(PlanError::InvalidInput("branch 이름"))
    }
}

/// branch로 전환합니다(`git switch --no-guess`, 작업 트리 충돌 시 Git 거부 결과 반환).
pub fn switch_branch(git: &GatedGitRunner, worktree: &Path, name: &str) -> Result<(), PlanError> {
    require_name(name)?;
    let plan = CommandPlan::new(argv(["switch", "--no-guess", name]), DestructiveLevel::Safe);
    execute_plan(git, worktree, &plan, None)?;
    Ok(())
}

/// branch를 만듭니다(`git branch --no-track -- <name> [<start>]`). `start`는 branch 이름 또는 commit ID.
pub fn create_branch(
    git: &GatedGitRunner,
    worktree: &Path,
    name: &str,
    start: Option<&str>,
) -> Result<(), PlanError> {
    require_name(name)?;
    let mut args = argv(["branch", "--no-track", "--", name]);
    if let Some(start) = start {
        if !(is_valid_ref_body(start) || ObjectId::parse(start).is_some()) {
            return Err(PlanError::InvalidInput("시작 지점"));
        }
        args.extend(argv([start]));
    }
    execute_plan(
        git,
        worktree,
        &CommandPlan::new(args, DestructiveLevel::Safe),
        None,
    )?;
    Ok(())
}

/// branch 삭제 plan. `force`면 병합되지 않은 branch도 지우는 `-D`(확인 필요).
pub fn plan_delete_branch(name: &str, force: bool) -> Result<CommandPlan, PlanError> {
    require_name(name)?;
    let (flag, level) = if force {
        ("-D", DestructiveLevel::DiscardsRefs)
    } else {
        ("-d", DestructiveLevel::Safe)
    };
    Ok(CommandPlan::new(argv(["branch", flag, "--", name]), level))
}

/// branch 삭제 plan을 실행합니다(강제 삭제는 확인 필요).
pub fn delete_branch(
    git: &GatedGitRunner,
    worktree: &Path,
    plan: &CommandPlan,
    confirmation: Option<&Confirmation>,
) -> Result<(), PlanError> {
    execute_plan(git, worktree, plan, confirmation)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const E1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

    #[test]
    fn parses_branch_list_with_tracking() {
        let out = format!(
            "refs/heads/feature/x\0{E1}\0 \0\0\nrefs/heads/main\0{E1}\0*\0origin/main\0ahead 1, behind 2\n\
             refs/heads/old\0{E1}\0 \0origin/old\0gone\nrefs/heads/sync\0{E1}\0 \0origin/sync\0\n\
             refs/heads/b\0{E1}\0 \0origin/b\0behind 3\n"
        );
        let list = parse_branch_list(out.as_bytes()).unwrap();
        let tracks: Vec<(&str, bool, TrackState)> = list
            .iter()
            .map(|b| (b.name.as_str(), b.is_head, b.track))
            .collect();
        assert_eq!(
            tracks,
            vec![
                ("feature/x", false, TrackState::NoUpstream),
                (
                    "main",
                    true,
                    TrackState::Diverged {
                        ahead: 1,
                        behind: 2
                    }
                ),
                ("old", false, TrackState::Gone),
                ("sync", false, TrackState::InSync),
                (
                    "b",
                    false,
                    TrackState::Diverged {
                        ahead: 0,
                        behind: 3
                    }
                ),
            ]
        );
        for bad in [
            format!("refs/tags/v1\0{E1}\0 \0\0\n"),
            "refs/heads/a\0zz\0 \0\0\n".to_string(),
            format!("refs/heads/a\0{E1}\0x\0\0\n"),
            format!("refs/heads/a\0{E1}\0 \0\n"),
            format!("refs/heads/a\0{E1}\0 \0o/a\0sideways 1\n"),
        ] {
            assert!(parse_branch_list(bad.as_bytes()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn invalid_names_spawn_nothing() {
        use crate::runner::GitRunner;
        use crate::trust::GitTrust;
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
        let dir = std::env::temp_dir();
        assert_eq!(
            switch_branch(&gate, &dir, "-f"),
            Err(PlanError::InvalidInput("branch 이름"))
        );
        assert!(create_branch(&gate, &dir, "a..b", None).is_err());
        assert!(create_branch(&gate, &dir, "ok", Some("--orphan")).is_err());
        assert!(plan_delete_branch("--all", true).is_err());
        assert_eq!(runner.spawn_count(), 0);
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;

        #[test]
        fn local_repo_branch_fixture() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("adv-branch");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "a.txt", b"1\n");
            git_ok(&runner, repo, ["branch", "merged"]);
            commit_file(&runner, repo, "a.txt", b"2\n");
            git_ok(&runner, repo, ["branch", "--set-upstream-to=merged"]);
            git_ok(&runner, repo, ["switch", "-q", "-c", "feature"]);
            commit_file(&runner, repo, "f.txt", b"f\n");
            git_ok(&runner, repo, ["switch", "-q", "main"]);

            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let list = list_branches(&gate, repo).unwrap();
            let names: Vec<&str> = list.iter().map(|b| b.name.as_str()).collect();
            assert_eq!(names, ["feature", "main", "merged"]);
            let main = &list[1];
            assert!(main.is_head);
            assert_eq!(main.upstream.as_deref(), Some("merged"));
            assert_eq!(
                main.track,
                TrackState::Diverged {
                    ahead: 1,
                    behind: 0
                }
            );

            // 생성·전환.
            create_branch(
                &gate,
                repo,
                "design/새 기능".replace(' ', "-").as_str(),
                Some("merged"),
            )
            .unwrap();
            switch_branch(&gate, repo, "feature").unwrap();
            let head = git_ok(&runner, repo, ["symbolic-ref", "--short", "HEAD"]);
            assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "feature");
            // 작업 트리 충돌 전환은 Git이 거부(결과 그대로).
            std::fs::write(repo.join("a.txt"), b"dirty\n").unwrap();
            assert!(matches!(
                switch_branch(&gate, repo, "merged"),
                Err(PlanError::Op(GitOpError::Git(_)))
            ));
            assert_eq!(std::fs::read(repo.join("a.txt")).unwrap(), b"dirty\n");
            git_ok(&runner, repo, ["checkout", "--", "a.txt"]);
            switch_branch(&gate, repo, "main").unwrap();

            // 병합된 branch는 -d로 삭제.
            let plan = plan_delete_branch("merged", false).unwrap();
            assert_eq!(plan.level(), DestructiveLevel::Safe);
            git_ok(&runner, repo, ["branch", "--unset-upstream"]);
            delete_branch(&gate, repo, &plan, None).unwrap();
            // 병합되지 않은 branch는 -d를 Git이 거부.
            let safe = plan_delete_branch("feature", false).unwrap();
            assert!(matches!(
                delete_branch(&gate, repo, &safe, None),
                Err(PlanError::Op(GitOpError::Git(_)))
            ));
            // 강제 삭제는 확인 없이는 실행 0.
            let force = plan_delete_branch("feature", true).unwrap();
            assert_eq!(force.preview(), "git branch -D -- feature");
            let before = runner.spawn_count();
            assert!(matches!(
                delete_branch(&gate, repo, &force, None),
                Err(PlanError::ConfirmationRequired { .. })
            ));
            assert_eq!(runner.spawn_count(), before);
            assert!(list_branches(&gate, repo)
                .unwrap()
                .iter()
                .any(|b| b.name == "feature"));
            delete_branch(&gate, repo, &force, Some(&Confirmation::confirm(&force))).unwrap();
            let names: Vec<String> = list_branches(&gate, repo)
                .unwrap()
                .into_iter()
                .map(|b| b.name)
                .collect();
            assert_eq!(names, ["design/새-기능", "main"]);
        }
    }
}
