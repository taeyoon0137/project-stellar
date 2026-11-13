//! 저장소 위치 판별 (XIV.1.2).
//!
//! worktree root, 개별 Git dir, common Git dir를 **`git rev-parse` 결과로만** 판별합니다.
//! `<root>/.git`이 Git dir라고 가정하거나 gitfile 내용을 직접 해석하지 않습니다(경로 추측 금지).
//!
//! - `--is-inside-work-tree`: worktree 내부인지(`true`/`false`). 실패하면 Git이 저장소로 인식하지
//!   못한 것입니다.
//! - `--show-toplevel`: worktree root.
//! - `--git-dir`: 이 worktree의 Git dir(linked worktree면 `<common>/worktrees/<name>`).
//! - `--git-common-dir`: 저장소 공통 Git dir.
//!
//! 경로 질의에는 `--path-format=absolute`를 붙여 Git이 절대·canonical 경로를 출력하게 합니다.
//! 질의마다 별도 실행해 stdout 전체(끝의 개행 1개 제외)가 정확히 한 값이 되게 하므로, 개행을
//! 포함한 경로도 줄 단위 분해 없이 그대로 보존합니다. 모든 실행은 trust gate(XIV.1.1)를 거칩니다.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::command::{GitCommandFailure, GitCommandResult};
use crate::decode::path_from_git_bytes;
use crate::runner::argv;
use crate::trust::{GatedGitRunner, GatedRunError};

/// worktree 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeKind {
    /// 주 worktree(Git dir == common Git dir).
    Main,
    /// `git worktree add`로 만든 linked worktree(Git dir != common Git dir).
    Linked,
}

/// worktree root의 `.git` 항목 구조(Git dir 위치 판별에는 쓰지 않는 관찰값).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotGitEntry {
    /// `.git` 디렉터리.
    Directory,
    /// `.git` gitfile(linked worktree·submodule 등에서 Git dir를 가리키는 파일).
    GitFile,
    /// `.git`이 symlink.
    Symlink,
    /// `.git` 항목 없음.
    Missing,
}

/// `git rev-parse`로 판별한 저장소 위치.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryLocation {
    /// worktree root(`--show-toplevel`).
    pub worktree_root: PathBuf,
    /// 이 worktree의 Git dir(`--git-dir`).
    pub git_dir: PathBuf,
    /// 공통 Git dir(`--git-common-dir`).
    pub common_dir: PathBuf,
    /// worktree 종류(Git dir와 common Git dir 비교).
    pub kind: WorktreeKind,
    /// worktree root의 `.git` 항목 구조.
    pub dot_git: DotGitEntry,
}

/// 위치 판별 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationError {
    /// trust gate 거부 또는 Git 실행 실패.
    Gate(GatedRunError),
    /// Git이 cwd를 저장소로 인식하지 못함(`--is-inside-work-tree` 비영 종료, stderr 보존).
    NotARepository(GitCommandFailure),
    /// 저장소이지만 worktree 내부가 아님(bare repo·Git dir 내부 등).
    NotInsideWorkTree,
    /// 경로 질의가 비영 종료.
    Git(GitCommandFailure),
    /// 질의 결과가 기대 형식이 아님(원시 출력 보존).
    UnexpectedOutput {
        /// 질의 플래그.
        query: &'static str,
        /// 원시 stdout.
        raw: Vec<u8>,
    },
}

impl std::fmt::Display for LocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocationError::Gate(e) => write!(f, "{e}"),
            LocationError::NotARepository(e) => write!(f, "Git 저장소로 인식되지 않음: {e}"),
            LocationError::NotInsideWorkTree => f.write_str("Git worktree 내부가 아닙니다"),
            LocationError::Git(e) => write!(f, "{e}"),
            LocationError::UnexpectedOutput { query, raw } => write!(
                f,
                "git rev-parse {query} 출력 형식 오류: {:?}",
                String::from_utf8_lossy(raw)
            ),
        }
    }
}

impl std::error::Error for LocationError {}

/// cwd가 속한 저장소의 worktree root·Git dir·common Git dir를 판별합니다.
pub fn locate_repository(
    git: &GatedGitRunner,
    cwd: &Path,
) -> Result<RepositoryLocation, LocationError> {
    let inside = rev_parse(git, cwd, &["--is-inside-work-tree"]).map_err(|e| match e {
        LocationError::Git(failure) => LocationError::NotARepository(failure),
        other => other,
    })?;
    if !parse_bool_output("--is-inside-work-tree", &inside)? {
        return Err(LocationError::NotInsideWorkTree);
    }

    let worktree_root = query_path(git, cwd, "--show-toplevel")?;
    let git_dir = query_path(git, cwd, "--git-dir")?;
    let common_dir = query_path(git, cwd, "--git-common-dir")?;

    // `--path-format=absolute`는 canonical 경로이므로 그대로 비교합니다.
    let kind = if git_dir == common_dir {
        WorktreeKind::Main
    } else {
        WorktreeKind::Linked
    };
    let dot_git = dot_git_entry(&worktree_root);
    Ok(RepositoryLocation {
        worktree_root,
        git_dir,
        common_dir,
        kind,
        dot_git,
    })
}

/// `git rev-parse <flags>`를 실행해 성공 stdout을 반환합니다.
fn rev_parse(git: &GatedGitRunner, cwd: &Path, flags: &[&str]) -> Result<Vec<u8>, LocationError> {
    let mut args: Vec<OsString> = argv(["rev-parse"]);
    args.extend(argv(flags));
    let output = git.run(&args, cwd).map_err(LocationError::Gate)?;
    let result = GitCommandResult::from(output)
        .require_success()
        .map_err(LocationError::Git)?;
    Ok(result.stdout().to_vec())
}

/// 절대 경로 질의(`--path-format=absolute <query>`).
fn query_path(
    git: &GatedGitRunner,
    cwd: &Path,
    query: &'static str,
) -> Result<PathBuf, LocationError> {
    let raw = rev_parse(git, cwd, &["--path-format=absolute", query])?;
    parse_path_output(query, &raw)
}

/// 단일 값 출력에서 끝의 개행 1개만 제거합니다.
fn strip_one_newline(raw: &[u8]) -> &[u8] {
    raw.strip_suffix(b"\n").unwrap_or(raw)
}

/// `true`/`false` 출력을 해석합니다.
pub fn parse_bool_output(query: &'static str, raw: &[u8]) -> Result<bool, LocationError> {
    match strip_one_newline(raw) {
        b"true" => Ok(true),
        b"false" => Ok(false),
        _ => Err(LocationError::UnexpectedOutput {
            query,
            raw: raw.to_vec(),
        }),
    }
}

/// 경로 질의 출력을 절대 경로로 해석합니다. 비어 있거나 절대 경로가 아니면 오류입니다.
///
/// `--path-format`을 모르는 구버전 Git은 플래그를 그대로 출력하므로(`--path-format=...` 줄이
/// 섞임) 절대 경로 검사에서 걸러집니다.
pub fn parse_path_output(query: &'static str, raw: &[u8]) -> Result<PathBuf, LocationError> {
    let bytes = strip_one_newline(raw);
    let unexpected = || LocationError::UnexpectedOutput {
        query,
        raw: raw.to_vec(),
    };
    if bytes.is_empty() {
        return Err(unexpected());
    }
    let path = path_from_git_bytes(bytes).ok_or_else(unexpected)?;
    if !path.is_absolute() {
        return Err(unexpected());
    }
    Ok(path)
}

/// worktree root의 `.git` 항목 구조를 관찰합니다(symlink를 따르지 않음).
fn dot_git_entry(worktree_root: &Path) -> DotGitEntry {
    match std::fs::symlink_metadata(worktree_root.join(".git")) {
        Ok(meta) if meta.file_type().is_symlink() => DotGitEntry::Symlink,
        Ok(meta) if meta.is_dir() => DotGitEntry::Directory,
        Ok(meta) if meta.is_file() => DotGitEntry::GitFile,
        _ => DotGitEntry::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::GitRunner;
    use crate::trust::GitTrust;
    use std::time::Duration;

    #[test]
    fn parses_bool_outputs() {
        assert_eq!(parse_bool_output("q", b"true\n"), Ok(true));
        assert_eq!(parse_bool_output("q", b"false\n"), Ok(false));
        assert_eq!(parse_bool_output("q", b"true"), Ok(true));
        for bad in [&b""[..], b"yes\n", b"true\n\n", b"TRUE\n"] {
            assert!(
                matches!(
                    parse_bool_output("q", bad),
                    Err(LocationError::UnexpectedOutput { .. })
                ),
                "입력: {bad:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn parses_path_outputs_and_rejects_non_absolute() {
        assert_eq!(
            parse_path_output("q", b"/a/b c/repo\n"),
            Ok(PathBuf::from("/a/b c/repo"))
        );
        // 개행을 포함한 경로도 끝 개행 1개만 제거하고 보존.
        assert_eq!(
            parse_path_output("q", b"/a/line\nbreak\n"),
            Ok(PathBuf::from("/a/line\nbreak"))
        );
        for bad in [
            &b""[..],
            b"\n",
            b".git\n",
            // 구버전 Git이 모르는 플래그를 그대로 출력한 형태.
            b"--path-format=absolute\n/a/repo/.git\n",
        ] {
            assert!(
                matches!(
                    parse_path_output("q", bad),
                    Err(LocationError::UnexpectedOutput { .. })
                ),
                "입력: {bad:?}"
            );
        }
    }

    #[test]
    fn untrusted_location_probe_spawns_nothing() {
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
        assert_eq!(
            locate_repository(&gate, &std::env::temp_dir()),
            Err(LocationError::Gate(GatedRunError::ProjectNotTrusted))
        );
        assert_eq!(runner.spawn_count(), 0);
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            canonical, commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git,
            TempDir,
        };

        fn trusted(git: PathBuf) -> GatedGitRunner {
            GatedGitRunner::new(hermetic_runner(git), GitTrust::Trusted)
        }

        #[test]
        fn normal_repo_from_root_and_subdirectory() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("loc-normal");
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(repo.join("design/sub")).unwrap();
            let gate = trusted(git);
            init_repo_with_identity(gate.runner().unwrap(), &repo);

            let root = canonical(&repo);
            for cwd in [repo.clone(), repo.join("design/sub")] {
                let loc = locate_repository(&gate, &cwd).expect("위치 판별");
                assert_eq!(loc.worktree_root, root, "cwd={cwd:?}");
                assert_eq!(loc.git_dir, root.join(".git"));
                assert_eq!(loc.common_dir, root.join(".git"));
                assert_eq!(loc.kind, WorktreeKind::Main);
                assert_eq!(loc.dot_git, DotGitEntry::Directory);
            }
        }

        #[test]
        fn linked_worktree_reports_separate_git_dir_and_gitfile() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("loc-linked");
            let main = dir.path().join("main");
            let linked = dir.path().join("linked-wt");
            std::fs::create_dir_all(&main).unwrap();
            let gate = trusted(git);
            let runner = gate.runner().unwrap();
            init_repo_with_identity(runner, &main);
            commit_file(runner, &main, "stellar.yaml", b"schema_version: 1\n");
            let linked_s = linked.to_str().unwrap().to_string();
            git_ok(
                runner,
                &main,
                ["worktree", "add", "-q", "-b", "feature", &linked_s],
            );

            let main_root = canonical(&main);
            let linked_root = canonical(&linked);

            let loc = locate_repository(&gate, &linked).expect("linked 위치 판별");
            assert_eq!(loc.worktree_root, linked_root);
            assert_eq!(loc.common_dir, main_root.join(".git"));
            // linked worktree의 Git dir는 common dir 아래 worktrees/<name>(Git이 정한 경로).
            assert_ne!(loc.git_dir, loc.common_dir);
            assert!(
                loc.git_dir
                    .starts_with(main_root.join(".git").join("worktrees")),
                "git_dir={:?}",
                loc.git_dir
            );
            assert!(loc.git_dir.is_dir());
            assert_eq!(loc.kind, WorktreeKind::Linked);
            assert_eq!(loc.dot_git, DotGitEntry::GitFile);

            // 같은 저장소의 주 worktree는 Main.
            let main_loc = locate_repository(&gate, &main).expect("main 위치 판별");
            assert_eq!(main_loc.kind, WorktreeKind::Main);
            assert_eq!(main_loc.common_dir, loc.common_dir);
            assert_eq!(main_loc.worktree_root, main_root);
        }

        #[test]
        fn non_repository_is_reported_with_stderr() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("loc-none");
            let gate = trusted(git);
            match locate_repository(&gate, dir.path()) {
                Err(LocationError::NotARepository(failure)) => {
                    assert_ne!(failure.exit_code, Some(0));
                    assert!(!failure.stderr.is_empty());
                }
                other => panic!("예상과 다른 결과: {other:?}"),
            }
        }

        #[test]
        fn inside_git_dir_is_not_inside_work_tree() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("loc-gitdir");
            let gate = trusted(git);
            init_repo_with_identity(gate.runner().unwrap(), dir.path());
            assert_eq!(
                locate_repository(&gate, &dir.path().join(".git")),
                Err(LocationError::NotInsideWorkTree)
            );
        }
    }
}
