//! HEAD·commit·tree 파일 목록 provider (XIV.2.4).
//!
//! tree의 파일 목록과 mode·blob ID를 `git ls-tree -r -z --full-tree <tree>`로 읽습니다. 출력 레코드는
//! `<mode> SP <type> SP <oid> TAB <path>` + NUL이며 경로는 quote 없는 원시 바이트입니다.
//! `--full-tree`로 cwd와 무관하게 root 기준 전체 목록을 받습니다.
//!
//! 원천 해석([`resolve_tree`]):
//! - [`SnapshotSource::Head`]: `git rev-parse -q --verify HEAD^{commit}`. 실패(종료 코드 1)는 unborn
//!   branch로 보고 빈 snapshot을 돌려줍니다.
//! - [`SnapshotSource::Commit`]: `<oid>^{commit}`가 같은 ID로 해석되어야 합니다.
//! - [`SnapshotSource::Tree`]: `<oid>^{tree}`가 같은 ID로 해석되어야 합니다(commit ID를 tree로 쓰는
//!   실수를 거부).
//!
//! mode와 object 종류가 맞지 않는 레코드(예: `100644 commit`)는 해석 오류입니다. 모든 실행은 trust
//! gate를 거칩니다.

use std::path::Path;

use super::{SnapshotEntry, SnapshotError, SnapshotSource, SnapshotSourceKind};
use crate::decode::{split_nul_terminated, GitPath};
use crate::object::{FileMode, FileModeKind, ObjectId, ObjectKind};
use crate::ops::{failure_of, run_ok, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

const COMMAND: &str = "ls-tree -r -z";

/// tree 원천 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTree {
    /// HEAD가 아직 commit을 가리키지 않음(빈 snapshot).
    Unborn,
    /// 해석된 tree.
    Tree {
        /// 원천이 commit이면 그 commit.
        commit: Option<ObjectId>,
        /// tree ID.
        tree: ObjectId,
    },
}

/// tree snapshot(파일 목록).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSnapshot {
    /// 읽은 원천.
    pub source: SnapshotSource,
    /// 해석 결과.
    pub resolved: ResolvedTree,
    /// 파일 항목(Git tree 순서).
    pub entries: Vec<SnapshotEntry>,
}

/// `git rev-parse -q --verify <rev>`: 있으면 ID, 없으면(종료 코드 1) `None`.
pub(crate) fn verify_rev(
    git: &GatedGitRunner,
    worktree: &Path,
    rev: &str,
) -> Result<Option<ObjectId>, GitOpError> {
    let output = git.run(&argv(["rev-parse", "-q", "--verify", rev]), worktree)?;
    match output.exit_code {
        Some(0) => {
            let raw = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
            ObjectId::from_hex_bytes(raw).map(Some).ok_or_else(|| {
                GitOpError::Parse(ParseFailure::new(
                    "rev-parse --verify",
                    "object ID 아님",
                    &output.stdout,
                ))
            })
        }
        Some(1) => Ok(None),
        _ => Err(GitOpError::Git(failure_of(output))),
    }
}

/// 현재 HEAD의 commit ID. unborn이면 None이며 replace/lazy fetch 정책은 runner를 따릅니다.
pub fn head_commit(git: &GatedGitRunner, worktree: &Path) -> Result<Option<ObjectId>, GitOpError> {
    verify_rev(git, worktree, "HEAD^{commit}")
}

/// Head·Commit·Tree 원천을 tree ID로 해석합니다.
pub fn resolve_tree(
    git: &GatedGitRunner,
    worktree: &Path,
    source: &SnapshotSource,
) -> Result<ResolvedTree, SnapshotError> {
    let not_found = || SnapshotError::ObjectNotFound {
        source: source.clone(),
    };
    let commit = match source {
        SnapshotSource::Head => match head_commit(git, worktree)? {
            Some(commit) => commit,
            None => return Ok(ResolvedTree::Unborn),
        },
        SnapshotSource::Commit(oid) => {
            let peeled = verify_rev(git, worktree, &format!("{oid}^{{commit}}"))?;
            if peeled.as_ref() != Some(oid) {
                return Err(not_found());
            }
            oid.clone()
        }
        SnapshotSource::Tree(oid) => {
            let peeled = verify_rev(git, worktree, &format!("{oid}^{{tree}}"))?;
            if peeled.as_ref() != Some(oid) {
                return Err(not_found());
            }
            return Ok(ResolvedTree::Tree {
                commit: None,
                tree: oid.clone(),
            });
        }
        SnapshotSource::Index | SnapshotSource::WorkingTree => {
            return Err(SnapshotError::WrongSourceKind {
                expected: SnapshotSourceKind::GitTree,
                source: source.clone(),
            })
        }
    };
    let tree = verify_rev(git, worktree, &format!("{commit}^{{tree}}"))?.ok_or_else(not_found)?;
    Ok(ResolvedTree::Tree {
        commit: Some(commit),
        tree,
    })
}

/// mode에 맞는 object 종류.
fn expected_kind(mode: FileMode) -> Option<ObjectKind> {
    match mode.kind()? {
        FileModeKind::Regular | FileModeKind::Executable | FileModeKind::Symlink => {
            Some(ObjectKind::Blob)
        }
        FileModeKind::Gitlink => Some(ObjectKind::Commit),
        FileModeKind::Tree => Some(ObjectKind::Tree),
        FileModeKind::Absent => None,
    }
}

/// `git ls-tree -r -z` 출력을 해석합니다.
pub fn parse_ls_tree(stdout: &[u8]) -> Result<Vec<SnapshotEntry>, ParseFailure> {
    let records = split_nul_terminated(stdout)
        .map_err(|e| ParseFailure::new(COMMAND, e.to_string(), stdout))?;
    let mut entries = Vec::with_capacity(records.len());
    for record in records {
        let bad = |detail: &str| ParseFailure::new(COMMAND, detail, record);
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| bad("TAB 구분자 없음"))?;
        let (meta, path) = (&record[..tab], &record[tab + 1..]);
        if path.is_empty() {
            return Err(bad("빈 경로"));
        }
        let mut fields = meta.split(|b| *b == b' ');
        let (Some(mode), Some(kind), Some(oid), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(bad("필드 수가 3개가 아님"));
        };
        let mode = FileMode::from_octal_bytes(mode).ok_or_else(|| bad("잘못된 mode"))?;
        let kind = ObjectKind::from_name(kind).ok_or_else(|| bad("잘못된 object 종류"))?;
        let oid = ObjectId::from_hex_bytes(oid).ok_or_else(|| bad("잘못된 object ID"))?;
        if expected_kind(mode) != Some(kind) {
            return Err(bad("mode와 object 종류 불일치"));
        }
        entries.push(SnapshotEntry {
            path: GitPath::from_bytes(path),
            mode,
            oid,
        });
    }
    Ok(entries)
}

/// tree의 전체 파일 목록을 읽습니다(`git ls-tree -r -z --full-tree <tree>`).
pub fn read_tree_entries(
    git: &GatedGitRunner,
    worktree: &Path,
    tree: &ObjectId,
) -> Result<Vec<SnapshotEntry>, GitOpError> {
    let output = run_ok(
        git,
        worktree,
        &argv(["ls-tree", "-r", "-z", "--full-tree", tree.as_str()]),
    )?;
    Ok(parse_ls_tree(&output.stdout)?)
}

/// Head·Commit·Tree 원천의 snapshot을 읽습니다(unborn HEAD는 빈 목록).
pub fn read_tree_snapshot(
    git: &GatedGitRunner,
    worktree: &Path,
    source: &SnapshotSource,
) -> Result<TreeSnapshot, SnapshotError> {
    let resolved = resolve_tree(git, worktree, source)?;
    let entries = match &resolved {
        ResolvedTree::Unborn => Vec::new(),
        ResolvedTree::Tree { tree, .. } => read_tree_entries(git, worktree, tree)?,
    };
    Ok(TreeSnapshot {
        source: source.clone(),
        resolved,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const B1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
    const C1: &str = "8baef1b4abc478178b004d62031cf7fe6db6f903";

    #[test]
    fn parses_ls_tree_records() {
        let bytes = format!(
            "100644 blob {B1}\ta b/c.txt\0100755 blob {B1}\trun.sh\0120000 blob {B1}\tlink\0\
             160000 commit {C1}\tsub\0100644 blob {B1}\twith\ttab\0"
        )
        .into_bytes();
        let entries = parse_ls_tree(&bytes).unwrap();
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].path.as_bytes(), b"a b/c.txt");
        assert_eq!(entries[1].mode, FileMode::EXECUTABLE);
        assert_eq!(entries[2].kind(), Some(FileModeKind::Symlink));
        assert!(entries[3].is_gitlink());
        assert_eq!(entries[3].oid.as_str(), C1);
        // 경로 안의 TAB은 첫 TAB 이후이므로 경로 일부로 보존.
        assert_eq!(entries[4].path.as_bytes(), b"with\ttab");
    }

    #[test]
    fn non_utf8_path_bytes_are_preserved() {
        let mut bytes = format!("100644 blob {B1}\t").into_bytes();
        bytes.extend_from_slice(b"caf\xe9.txt\0");
        let entries = parse_ls_tree(&bytes).unwrap();
        assert_eq!(entries[0].path.as_bytes(), b"caf\xe9.txt");
    }

    #[test]
    fn malformed_records_are_rejected() {
        for bad in [
            format!("100644 blob {B1} a.txt\0"), // TAB 없음
            format!("100644 blob {B1}\t\0"),     // 빈 경로
            format!("100644 {B1}\ta\0"),         // 필드 부족
            format!("100644 blob {B1} x\ta\0"),  // 필드 초과
            format!("100645 blob {B1}\ta\0"),    // mode
            format!("100644 blub {B1}\ta\0"),    // 종류
            "100644 blob xyz\ta\0".to_string(),  // oid
            format!("100644 commit {C1}\ta\0"),  // mode/종류 불일치
            format!("160000 blob {B1}\ta\0"),    // gitlink는 commit
            format!("100644 blob {B1}\ta"),      // NUL 종료 없음
        ] {
            assert!(parse_ls_tree(bad.as_bytes()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn index_and_worktree_are_not_tree_sources() {
        use crate::runner::GitRunner;
        use crate::trust::GitTrust;
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
        for source in [SnapshotSource::Index, SnapshotSource::WorkingTree] {
            assert!(matches!(
                resolve_tree(&gate, &std::env::temp_dir(), &source),
                Err(SnapshotError::WrongSourceKind { .. })
            ));
        }
        assert_eq!(runner.spawn_count(), 0);
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            add_index_path, git_ok, hermetic_runner, init_repo_with_identity, system_git,
            write_blob, TempDir,
        };
        use crate::trust::{GatedRunError, GitTrust};

        fn entry<'a>(entries: &'a [SnapshotEntry], path: &[u8]) -> &'a SnapshotEntry {
            entries
                .iter()
                .find(|e| e.path.as_bytes() == path)
                .unwrap_or_else(|| panic!("항목 없음: {:?}", String::from_utf8_lossy(path)))
        }

        #[test]
        fn symlink_executable_unicode_fixture() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("tree-fixture");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            std::fs::create_dir_all(repo.join("design/한글 폴더")).unwrap();
            std::fs::write(repo.join("design/한글 폴더/토큰 🎨.yaml"), b"color: red\n").unwrap();
            std::fs::write(repo.join("tool.sh"), b"#!/bin/sh\necho hi\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(repo.join("tool.sh"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
            std::os::unix::fs::symlink("design/한글 폴더", repo.join("shortcut")).unwrap();
            git_ok(&runner, repo, ["add", "-A"]);
            // NFD 한글과 non-UTF8 경로는 파일 시스템 정규화를 피해 index에 직접 등록. macOS Git의
            // argv 정규화(core.precomposeUnicode)를 이 fixture 저장소에서만 꺼 NFD 바이트를 보존합니다.
            git_ok(&runner, repo, ["config", "core.precomposeUnicode", "false"]);
            let nfd = "\u{1112}\u{1161}\u{11ab}.txt"; // "한" 분해형
            add_index_path(&runner, repo, "100644", nfd.as_bytes(), b"nfd\n");
            add_index_path(&runner, repo, "100644", b"caf\xe9.bin", b"\x00\xff");
            git_ok(&runner, repo, ["commit", "-q", "-m", "fixture"]);

            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let snap = read_tree_snapshot(&gate, repo, &SnapshotSource::Head).unwrap();
            let head = git_ok(&runner, repo, ["rev-parse", "HEAD"]);
            let head = ObjectId::from_hex_bytes(head.stdout.trim_ascii()).unwrap();
            match &snap.resolved {
                ResolvedTree::Tree { commit, .. } => assert_eq!(commit.as_ref(), Some(&head)),
                other => panic!("{other:?}"),
            }
            assert_eq!(snap.entries.len(), 5);

            let token = entry(&snap.entries, "design/한글 폴더/토큰 🎨.yaml".as_bytes());
            assert_eq!(token.mode, FileMode::REGULAR);
            assert_eq!(
                token.oid.as_str(),
                write_blob(&runner, repo, b"color: red\n")
            );
            let tool = entry(&snap.entries, b"tool.sh");
            assert_eq!(tool.mode, FileMode::EXECUTABLE);
            let link = entry(&snap.entries, b"shortcut");
            assert_eq!(link.mode, FileMode::SYMLINK);
            // symlink blob 내용은 대상 경로 바이트.
            assert_eq!(
                link.oid.as_str(),
                write_blob(&runner, repo, "design/한글 폴더".as_bytes())
            );
            let nfd_entry = entry(&snap.entries, nfd.as_bytes());
            assert_eq!(nfd_entry.oid.as_str(), write_blob(&runner, repo, b"nfd\n"));
            assert_eq!(
                entry(&snap.entries, b"caf\xe9.bin").oid.as_str(),
                write_blob(&runner, repo, b"\x00\xff")
            );

            // Commit(hash)·Tree(hash) 원천도 같은 목록.
            let by_commit =
                read_tree_snapshot(&gate, repo, &SnapshotSource::Commit(head.clone())).unwrap();
            assert_eq!(by_commit.entries, snap.entries);
            let ResolvedTree::Tree { tree, .. } = snap.resolved.clone() else {
                panic!()
            };
            let by_tree =
                read_tree_snapshot(&gate, repo, &SnapshotSource::Tree(tree.clone())).unwrap();
            assert_eq!(by_tree.entries, snap.entries);
            assert_eq!(by_tree.resolved, ResolvedTree::Tree { commit: None, tree });
            // cwd가 하위 디렉터리여도 --full-tree로 전체 목록.
            let sub =
                read_tree_snapshot(&gate, &repo.join("design"), &SnapshotSource::Head).unwrap();
            assert_eq!(sub.entries, snap.entries);
        }

        #[test]
        fn unborn_head_is_empty_and_wrong_ids_are_rejected() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("tree-unborn");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let snap = read_tree_snapshot(&gate, repo, &SnapshotSource::Head).unwrap();
            assert_eq!(snap.resolved, ResolvedTree::Unborn);
            assert!(snap.entries.is_empty());

            std::fs::write(repo.join("a.txt"), b"a\n").unwrap();
            git_ok(&runner, repo, ["add", "a.txt"]);
            git_ok(&runner, repo, ["commit", "-q", "-m", "a"]);
            let head = git_ok(&runner, repo, ["rev-parse", "HEAD"]);
            let head = ObjectId::from_hex_bytes(head.stdout.trim_ascii()).unwrap();
            // commit ID를 Tree 원천으로 쓰면 거부.
            assert!(matches!(
                read_tree_snapshot(&gate, repo, &SnapshotSource::Tree(head)),
                Err(SnapshotError::ObjectNotFound { .. })
            ));
            // 없는 commit.
            let missing = ObjectId::parse(&"1".repeat(40)).unwrap();
            assert!(matches!(
                read_tree_snapshot(&gate, repo, &SnapshotSource::Commit(missing)),
                Err(SnapshotError::ObjectNotFound { .. })
            ));
            // blob ID를 Commit 원천으로 쓰면 거부.
            let blob = ObjectId::parse(&write_blob(&runner, repo, b"a\n")).unwrap();
            assert!(matches!(
                read_tree_snapshot(&gate, repo, &SnapshotSource::Commit(blob)),
                Err(SnapshotError::ObjectNotFound { .. })
            ));
        }

        #[test]
        fn untrusted_tree_read_spawns_nothing() {
            let Some(git) = system_git() else {
                return;
            };
            let runner = hermetic_runner(git);
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
            assert_eq!(
                read_tree_snapshot(&gate, &std::env::temp_dir(), &SnapshotSource::Head),
                Err(SnapshotError::Op(GitOpError::Gate(
                    GatedRunError::ProjectNotTrusted
                )))
            );
            assert_eq!(runner.spawn_count(), 0);
        }
    }
}
