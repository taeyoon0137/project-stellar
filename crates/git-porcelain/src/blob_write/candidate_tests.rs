use super::*;
use crate::candidate_delta::compute_candidate_delta;
use crate::candidate_tree::{write_candidate_tree, CandidateFile};
use crate::fixture::GitFixture;
use crate::object::FileModeKind;
use crate::snapshot::read_index_snapshot;
use crate::trust::GitTrust;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn p(path: &str) -> GitPath {
    GitPath::from_bytes(path.as_bytes())
}

fn keep(path: &str) -> CandidateFile {
    CandidateFile::KeepIndex { path: p(path) }
}

fn bytes(path: &str, kind: FileModeKind, bytes: &[u8]) -> CandidateFile {
    CandidateFile::Bytes {
        path: p(path),
        kind,
        bytes: bytes.to_vec(),
    }
}

fn fixture() -> GitFixture {
    let fx = GitFixture::with_system_git("blob-candidate")
        .unwrap()
        .expect("system Git required");
    for (path, bytes) in [
        ("README.md", b"already staged\n".as_slice()),
        ("stellar.yaml", b"schema: 1\n"),
        ("design/current.stellarref", b"generation-1\n"),
        ("design/remove.starchunk", b"remove me\n"),
        ("assets/script", b"executable content\n"),
    ] {
        fx.write(path, bytes).unwrap();
    }
    fx.git(["add", "-A"]).unwrap();
    fx.write("README.md", b"unstaged\n").unwrap();
    fx.write("design/current.stellarref", b"unselected working bytes\n")
        .unwrap();
    fx
}

fn delta(fx: &GitFixture, files: &[CandidateFile]) -> CandidateDelta {
    let parent = fx.root().join("candidates");
    std::fs::create_dir_all(&parent).unwrap();
    let candidate = write_candidate_tree(&fx.gated(), fx.work(), &parent, files).unwrap();
    compute_candidate_delta(&fx.gated(), fx.work(), &candidate).unwrap()
}

fn unchanged() -> Vec<CandidateFile> {
    [
        "stellar.yaml",
        "design/current.stellarref",
        "design/remove.starchunk",
        "assets/script",
    ]
    .into_iter()
    .map(keep)
    .collect()
}

/// object 기록은 허용하되 실제 작업 파일의 bytes·mode는 동일해야 합니다.
fn working_bytes(root: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name() == Some(std::ffi::OsStr::new(".git")) {
                continue;
            }
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                todo.push(path);
            } else {
                let bytes = if metadata.file_type().is_symlink() {
                    std::fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_bytes()
                        .to_vec()
                } else {
                    std::fs::read(&path).unwrap()
                };
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    (metadata.permissions().mode(), bytes),
                );
            }
        }
    }
    out
}

#[test]
fn candidate_pipeline_writes_once_per_upsert_and_preserves_modes_deletions_and_bytes() {
    let fx = fixture();
    let gate = fx.gated();
    let delta = delta(
        &fx,
        &[
            keep("stellar.yaml"),
            bytes(
                "design/current.stellarref",
                FileModeKind::Regular,
                b"selected\n",
            ),
            bytes(
                "assets/script",
                FileModeKind::Executable,
                b"executable content\n",
            ),
            bytes("assets/link", FileModeKind::Symlink, b"../missing-target"),
            bytes(
                "assets/-new\t\n한글.bin",
                FileModeKind::Regular,
                b"\0\xff\r\n$(literal);\0",
            ),
        ],
    );
    let index_before = read_index_snapshot(&gate, fx.work()).unwrap();
    let index_bytes = std::fs::read(fx.work().join(".git/index")).unwrap();
    let working_before = working_bytes(fx.work());
    let count = gate.spawn_count();
    let written = write_candidate_blobs(&gate, fx.work(), &delta).unwrap();
    assert_eq!(gate.spawn_count() - count, 4);
    assert_eq!(written.base_index(), delta.base_index());
    assert_eq!(written.changes().len(), 5);
    for (expected, actual) in delta.changes().iter().zip(written.changes()) {
        match (expected, actual) {
            (
                FileDelta::Upsert { path, mode, bytes },
                WrittenFileDelta::Upsert {
                    path: actual_path,
                    mode: actual_mode,
                    oid,
                },
            ) => {
                assert_eq!(actual_path, path);
                assert_eq!(actual_mode, mode);
                assert_eq!(
                    fx.git(["cat-file", "blob", oid.as_str()]).unwrap().stdout,
                    *bytes
                );
            }
            (FileDelta::Remove { path }, WrittenFileDelta::Remove { path: actual_path }) => {
                assert_eq!(actual_path, path);
                assert_eq!(path, &p("design/remove.starchunk"));
            }
            _ => panic!("candidate 변경 종류가 기록 후 바뀜"),
        }
    }
    // 실제 object DB에 같은 후보를 동시에 기록해도 경로·mode·OID 결과는 같습니다.
    let parallel_count = gate.spawn_count();
    std::thread::scope(|scope| {
        let pending: Vec<_> = (0..3)
            .map(|_| scope.spawn(|| write_candidate_blobs(&gate, fx.work(), &delta).unwrap()))
            .collect();
        for result in pending {
            assert_eq!(result.join().unwrap(), written);
        }
    });
    assert_eq!(gate.spawn_count() - parallel_count, 12);
    assert_eq!(read_index_snapshot(&gate, fx.work()).unwrap(), index_before);
    assert_eq!(
        std::fs::read(fx.work().join(".git/index")).unwrap(),
        index_bytes
    );
    assert_eq!(working_bytes(fx.work()), working_before);
}

#[test]
fn unchanged_and_delete_only_candidates_spawn_no_git_writes() {
    let fx = fixture();
    let gate = fx.gated();
    let no_op = delta(&fx, &unchanged());
    let remove_only = delta(
        &fx,
        &[
            keep("stellar.yaml"),
            keep("design/current.stellarref"),
            keep("assets/script"),
        ],
    );
    let index_bytes = std::fs::read(fx.work().join(".git/index")).unwrap();
    let working_before = working_bytes(fx.work());
    for (delta, expected) in [
        (&no_op, Vec::new()),
        (
            &remove_only,
            vec![WrittenFileDelta::Remove {
                path: p("design/remove.starchunk"),
            }],
        ),
    ] {
        let before = gate.spawn_count();
        let written = write_candidate_blobs(&gate, fx.work(), delta).unwrap();
        assert_eq!(gate.spawn_count(), before);
        assert_eq!(written.changes(), expected);
        assert_eq!(written.base_index(), delta.base_index());
    }
    assert_eq!(
        std::fs::read(fx.work().join(".git/index")).unwrap(),
        index_bytes
    );
    assert_eq!(working_bytes(fx.work()), working_before);
}

#[test]
fn real_sha256_repository_returns_sha256_oid_and_exact_blob_bytes() {
    let fx = fixture();
    let repo = fx.root().join("sha256");
    std::fs::create_dir(&repo).unwrap();
    fx.git_in(
        &repo,
        ["init", "-q", "--object-format=sha256", "-b", "main"],
    )
    .unwrap();
    fx.write_in(&repo, "anchor", b"staged anchor\n").unwrap();
    fx.git_in(&repo, ["add", "--", "anchor"]).unwrap();
    let gate = GatedGitRunner::new(fx.runner().clone(), GitTrust::Trusted);
    let index_before = read_index_snapshot(&gate, &repo).unwrap();
    let index_bytes = std::fs::read(repo.join(".git/index")).unwrap();
    let working_before = working_bytes(&repo);
    let bytes = b"sha256\0\xff\n";
    let before = gate.spawn_count();
    let oid = write_blob(&gate, &repo, bytes).unwrap();
    assert_eq!(gate.spawn_count() - before, 1);
    assert!(oid.is_sha256());
    assert_eq!(
        fx.git_in(&repo, ["cat-file", "blob", oid.as_str()])
            .unwrap()
            .stdout,
        bytes
    );
    assert_eq!(read_index_snapshot(&gate, &repo).unwrap(), index_before);
    assert_eq!(std::fs::read(repo.join(".git/index")).unwrap(), index_bytes);
    assert_eq!(working_bytes(&repo), working_before);
}

#[test]
fn git_object_write_failure_returns_structured_error_and_leaves_index_and_worktree() {
    let fx = fixture();
    let delta = delta(
        &fx,
        &[bytes("stellar.yaml", FileModeKind::Regular, b"new bytes\n")],
    );
    let blocked_objects = fx.root().join("not-an-object-directory");
    std::fs::write(&blocked_objects, b"regular file").unwrap();
    let gate = GatedGitRunner::new(
        fx.runner().clone().with_added_env(vec![(
            OsString::from("GIT_OBJECT_DIRECTORY"),
            blocked_objects.as_os_str().to_os_string(),
        )]),
        GitTrust::Trusted,
    );
    let index_bytes = std::fs::read(fx.work().join(".git/index")).unwrap();
    let working_before = working_bytes(fx.work());
    let before = gate.spawn_count();
    let error = write_candidate_blobs(&gate, fx.work(), &delta).unwrap_err();
    assert_eq!(gate.spawn_count() - before, 1);
    let GitOpError::Git(failure) = error else {
        panic!("실제 Git의 비영 종료가 구조화된 Git 오류여야 함");
    };
    assert_ne!(failure.exit_code, Some(0));
    assert!(!failure.stderr.is_empty());
    assert_eq!(
        std::fs::read(fx.work().join(".git/index")).unwrap(),
        index_bytes
    );
    assert_eq!(working_bytes(fx.work()), working_before);
    assert_eq!(std::fs::read(blocked_objects).unwrap(), b"regular file");
}
