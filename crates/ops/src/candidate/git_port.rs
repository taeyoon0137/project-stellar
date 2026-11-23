//! Semantic Merge 해결·등록이 쓰는 Git 접근 포트 (ADR-0081 결정 7).
//!
//! 해결 흐름은 필요한 Git 동작만 [`MergeGit`] trait로 받습니다. 제품 구현은 [`GatedMergeGit`]이며 git-porcelain의
//! [`GatedGitRunner`]를 거칩니다(ADR-0081 결정 7 개정). 그래서 Stellar Git 엔진(ADR-0080)과 같은 실행기·환경
//! allowlist·timeout·신뢰 gate를 쓰고, 신뢰하지 않은 프로젝트에서는 프로세스를 하나도 만들지 않습니다. 고정 argv,
//! shell 없음, 경로 앞 `--`, `core.fsmonitor=false`, `GIT_OPTIONAL_LOCKS=0`·`GIT_TERMINAL_PROMPT=0`입니다.
//!
//! **없는 동작**: `merge --continue`·`rebase --continue`·`cherry-pick --continue`·`commit`·`push`와 전역·시스템
//! config 쓰기는 포트에 없습니다(XIV.12.5, XXIV.2.3). 해결 흐름은 구조적으로 그 명령을 부를 수 없습니다.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

use stellar_git_porcelain::runner::GitOutput;
use stellar_git_porcelain::trust::{GatedGitRunner, GatedRunError, GitTrust};

use crate::git::{os_git_trust, system_git_runner, GitError};

/// unmerged index의 stage blob 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageBlob {
    /// 파일 mode(예: `100644`).
    pub mode: String,
    /// blob object id.
    pub oid: String,
}

/// unmerged 경로 하나(`git ls-files -u` 묶음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmergedEntry {
    /// 작업 트리 기준 경로(포트 호출 폴더 기준).
    pub path: PathBuf,
    /// stage 1(base)·2(ours)·3(theirs). 없는 stage는 그쪽에 파일이 없음입니다.
    pub stages: [Option<StageBlob>; 3],
}

/// 진행 중인 Git 작업.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOperation {
    /// 진행 중 작업 없음.
    None,
    /// `git merge`.
    Merge,
    /// `git rebase`.
    Rebase,
    /// `git cherry-pick`.
    CherryPick,
    /// `git revert`.
    Revert,
}

impl GitOperation {
    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            GitOperation::None => "none",
            GitOperation::Merge => "merge",
            GitOperation::Rebase => "rebase",
            GitOperation::CherryPick => "cherry-pick",
            GitOperation::Revert => "revert",
        }
    }
}

/// Git 실행 실패(종료 코드·stderr 요약).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitPortError(pub String);

impl fmt::Display for GitPortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Git 실행 실패: {}", self.0)
    }
}

impl std::error::Error for GitPortError {}

/// Semantic Merge 해결·등록이 쓰는 Git 동작.
pub trait MergeGit {
    /// `git ls-files -u -z`: unmerged 경로와 stage blob(`worktree` 기준 경로).
    fn unmerged(&self, worktree: &Path) -> Result<Vec<UnmergedEntry>, GitPortError>;

    /// blob bytes. `limit` bytes를 넘으면 읽지 않고 실패합니다.
    fn blob(&self, worktree: &Path, oid: &str, limit: u64) -> Result<Vec<u8>, GitPortError>;

    /// `git add -- <paths>`(resolved 처리, XIV.12.4). 경로는 `worktree` 기준입니다.
    fn add(&self, worktree: &Path, paths: &[PathBuf]) -> Result<(), GitPortError>;

    /// 진행 중인 merge·rebase·cherry-pick·revert.
    fn operation(&self, worktree: &Path) -> Result<GitOperation, GitPortError>;

    /// 저장소 로컬 config 값(`git config --local --get`). 없으면 `None`.
    fn local_config(&self, worktree: &Path, key: &str) -> Result<Option<String>, GitPortError>;

    /// 저장소 로컬 config 값 쓰기(`git config --local`). 전역·시스템 config는 쓰지 않습니다.
    fn set_local_config(&self, worktree: &Path, key: &str, value: &str)
        -> Result<(), GitPortError>;
}

/// [`GatedGitRunner`] 기반 [`MergeGit`] 제품 구현.
#[derive(Debug, Clone)]
pub struct GatedMergeGit {
    git: GatedGitRunner,
}

impl GatedMergeGit {
    /// 주어진 gate 실행기로 만듭니다. 실행기는 Git 동작 환경(`GIT_OPTIONAL_LOCKS=0`, `GIT_TERMINAL_PROMPT=0`)을
    /// 덧붙여 씁니다.
    pub fn new(git: GatedGitRunner) -> GatedMergeGit {
        let trust = git.trust();
        let runner = match git.runner() {
            Ok(runner) => runner.clone().with_added_env(vec![
                ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
                ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ]),
            // 신뢰하지 않으면 실행기를 쓰지 않으므로 그대로 둡니다(모든 호출이 실행 전에 거부됨).
            Err(_) => return GatedMergeGit { git },
        };
        GatedMergeGit {
            git: GatedGitRunner::new(runner, trust),
        }
    }

    /// 시스템 Git과 OS 사용자 데이터의 신뢰 등록으로 `root`용 실행기를 만듭니다(CLI·앱 공용,
    /// [`GitSession::open_for_os_user`](crate::git::GitSession::open_for_os_user)와 같은 판정).
    pub fn for_os_user(root: &Path) -> Result<GatedMergeGit, GitError> {
        let trust = os_git_trust(root);
        if trust == GitTrust::NotTrusted {
            return Err(GitError::NotTrusted);
        }
        Ok(GatedMergeGit::new(GatedGitRunner::new(
            system_git_runner()?,
            trust,
        )))
    }

    /// gate 실행기.
    pub fn gated(&self) -> &GatedGitRunner {
        &self.git
    }

    fn output(&self, cwd: &Path, args: &[&OsStr]) -> Result<GitOutput, GitPortError> {
        let mut argv: Vec<OsString> = vec!["-c".into(), "core.fsmonitor=false".into()];
        argv.extend(args.iter().map(|a| a.to_os_string()));
        self.git.run(&argv, cwd).map_err(|e| match e {
            GatedRunError::ProjectNotTrusted => {
                GitPortError("신뢰하지 않은 프로젝트에서는 Git을 실행하지 않습니다".into())
            }
            other => GitPortError(format!("git 실행 실패: {other}")),
        })
    }

    fn run(&self, cwd: &Path, args: &[&OsStr]) -> Result<Vec<u8>, GitPortError> {
        let output = self.output(cwd, args)?;
        if !output.success() {
            return Err(GitPortError(format!(
                "git {}: 종료 {:?}: {}",
                args.first()
                    .map(|a| a.to_string_lossy())
                    .unwrap_or_default(),
                output.exit_code,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(output.stdout)
    }
}

fn os(text: &str) -> &OsStr {
    OsStr::new(text)
}

fn text(bytes: Vec<u8>) -> Result<String, GitPortError> {
    String::from_utf8(bytes).map_err(|_| GitPortError("UTF-8이 아닌 Git 출력".into()))
}

/// `git ls-files -u -z` 출력: `<mode> <oid> <stage>\t<path>\0`.
pub fn parse_unmerged(bytes: &[u8]) -> Result<Vec<UnmergedEntry>, GitPortError> {
    let mut entries: Vec<UnmergedEntry> = Vec::new();
    for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let record = std::str::from_utf8(record)
            .map_err(|_| GitPortError("UTF-8이 아닌 unmerged 경로".into()))?;
        let (meta, path) = record
            .split_once('\t')
            .ok_or_else(|| GitPortError("ls-files 형식".into()))?;
        let mut fields = meta.split(' ');
        let (Some(mode), Some(oid), Some(stage), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(GitPortError("ls-files 형식".into()));
        };
        let index = match stage {
            "1" => 0,
            "2" => 1,
            "3" => 2,
            _ => return Err(GitPortError(format!("알 수 없는 stage {stage}"))),
        };
        let path = PathBuf::from(path);
        if entries.last().is_none_or(|e| e.path != path) {
            entries.push(UnmergedEntry {
                path,
                stages: [None, None, None],
            });
        }
        let entry = entries.last_mut().expect("방금 넣음");
        entry.stages[index] = Some(StageBlob {
            mode: mode.to_owned(),
            oid: oid.to_owned(),
        });
    }
    Ok(entries)
}

impl MergeGit for GatedMergeGit {
    fn unmerged(&self, worktree: &Path) -> Result<Vec<UnmergedEntry>, GitPortError> {
        parse_unmerged(&self.run(worktree, &[os("ls-files"), os("-u"), os("-z")])?)
    }

    fn blob(&self, worktree: &Path, oid: &str, limit: u64) -> Result<Vec<u8>, GitPortError> {
        let size: u64 = text(self.run(worktree, &[os("cat-file"), os("-s"), os(oid)])?)?
            .trim()
            .parse()
            .map_err(|_| GitPortError("cat-file 크기 형식".into()))?;
        if size > limit {
            return Err(GitPortError(format!("blob이 {limit} bytes를 넘습니다")));
        }
        self.run(worktree, &[os("cat-file"), os("blob"), os(oid)])
    }

    fn add(&self, worktree: &Path, paths: &[PathBuf]) -> Result<(), GitPortError> {
        let mut args = vec![os("add"), os("-A"), os("--")];
        args.extend(paths.iter().map(|p| p.as_os_str()));
        self.run(worktree, &args).map(|_| ())
    }

    fn operation(&self, worktree: &Path) -> Result<GitOperation, GitPortError> {
        for (marker, operation) in [
            ("rebase-merge", GitOperation::Rebase),
            ("rebase-apply", GitOperation::Rebase),
            ("CHERRY_PICK_HEAD", GitOperation::CherryPick),
            ("REVERT_HEAD", GitOperation::Revert),
            ("MERGE_HEAD", GitOperation::Merge),
        ] {
            let path = text(self.run(worktree, &[os("rev-parse"), os("--git-path"), os(marker)])?)?;
            if worktree.join(path.trim()).exists() {
                return Ok(operation);
            }
        }
        Ok(GitOperation::None)
    }

    fn local_config(&self, worktree: &Path, key: &str) -> Result<Option<String>, GitPortError> {
        let output = self.output(
            worktree,
            &[os("config"), os("--local"), os("--get"), os(key)],
        )?;
        match output.exit_code {
            Some(0) => Ok(Some(text(output.stdout)?.trim_end_matches('\n').to_owned())),
            Some(1) => Ok(None),
            code => Err(GitPortError(format!(
                "git config --get {key}: 종료 {code:?}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))),
        }
    }

    fn set_local_config(
        &self,
        worktree: &Path,
        key: &str,
        value: &str,
    ) -> Result<(), GitPortError> {
        self.run(worktree, &[os("config"), os("--local"), os(key), os(value)])
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_git_porcelain::runner::GitRunner;

    #[test]
    fn unmerged_records_are_grouped_by_path() {
        let bytes = b"100644 aaa 1\tdesign/components/a.stellar.yaml\0\
100644 bbb 2\tdesign/components/a.stellar.yaml\0\
100644 ccc 3\tdesign/components/a.stellar.yaml\0\
100644 ddd 2\tREADME.md\0\
100644 eee 3\tREADME.md\0";
        let entries = parse_unmerged(bytes).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].stages[2].as_ref().unwrap().oid, "ccc");
        assert!(entries[1].stages[0].is_none(), "양쪽 추가는 stage 1 없음");
        assert!(parse_unmerged(b"100644 aaa 4\tx\0").is_err());
        assert!(parse_unmerged(b"garbage\0").is_err());
    }

    #[test]
    fn untrusted_project_spawns_no_git_process() {
        let runner = GitRunner::new(
            PathBuf::from("/usr/bin/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let git = GatedMergeGit::new(GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted));
        let dir = std::env::temp_dir();
        assert!(git.unmerged(&dir).is_err());
        assert!(git.add(&dir, &[PathBuf::from("x")]).is_err());
        assert!(git.local_config(&dir, "merge.stellar.driver").is_err());
        assert_eq!(git.gated().spawn_count(), 0);
        assert_eq!(runner.spawn_count(), 0);
    }
}
