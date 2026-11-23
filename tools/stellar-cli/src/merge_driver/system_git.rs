//! 시스템 Git adapter — ops `MergeGit` 포트의 CLI 구현 (ADR-0081 결정 7).
//!
//! ops·CLI의 허용 crate graph에 git-porcelain이 없어 이 adapter가 시스템 Git을 고정 argv로 실행합니다. shell을
//! 거치지 않고, 경로 인자 앞에 `--`를 두며, `core.fsmonitor`를 끄고 `GIT_OPTIONAL_LOCKS=0`·`GIT_TERMINAL_PROMPT=0`을
//! 씁니다. Git 탐색은 기술 아키텍처 §24.1 순서(표준 설치 경로 → `PATH`)입니다. ops→git-porcelain 경로가 생기면
//! `GatedGitRunner` 기반 구현으로 바꿉니다.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use stellar_ops::candidate::git_port::{
    GitOperation, GitPortError, MergeGit, StageBlob, UnmergedEntry,
};

/// 표준 설치 경로(탐색 1순위).
const STANDARD_PATHS: [&str; 3] = [
    "/opt/homebrew/bin/git",
    "/usr/local/bin/git",
    "/usr/bin/git",
];

/// 시스템 Git 실행 파일.
#[derive(Debug, Clone)]
pub struct SystemGit {
    executable: PathBuf,
}

impl SystemGit {
    /// 표준 설치 경로, 그다음 `PATH`에서 Git을 찾습니다.
    pub fn discover() -> Option<SystemGit> {
        let standard = STANDARD_PATHS.iter().map(PathBuf::from);
        let from_path = std::env::var_os("PATH")
            .map(|paths| {
                std::env::split_paths(&paths)
                    .filter(|dir| dir.is_absolute())
                    .map(|dir| dir.join("git"))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        standard
            .chain(from_path)
            .find(|path| path.is_file())
            .map(|executable| SystemGit { executable })
    }

    fn output(&self, cwd: &Path, args: &[&OsStr]) -> Result<Output, GitPortError> {
        Command::new(&self.executable)
            .args(["-c", "core.fsmonitor=false"])
            .args(args)
            .current_dir(cwd)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| GitPortError(format!("git 실행 실패: {e}")))
    }

    fn run(&self, cwd: &Path, args: &[&OsStr]) -> Result<Vec<u8>, GitPortError> {
        let output = self.output(cwd, args)?;
        if !output.status.success() {
            return Err(GitPortError(format!(
                "git {}: 종료 {:?}: {}",
                args.first()
                    .map(|a| a.to_string_lossy())
                    .unwrap_or_default(),
                output.status.code(),
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

impl MergeGit for SystemGit {
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
        match output.status.code() {
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
}
