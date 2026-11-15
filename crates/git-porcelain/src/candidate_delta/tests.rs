use super::*;
use crate::candidate_tree::{write_candidate_tree, CandidateFile};
use crate::fixture::GitFixture;
use crate::trust::{GatedRunError, GitTrust};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;

fn p(raw: &str) -> GitPath {
    GitPath::from_bytes(raw.as_bytes())
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

fn tree(fx: &GitFixture, files: &[CandidateFile]) -> CandidateTree {
    let parent = fx.root().join("candidates");
    std::fs::create_dir_all(&parent).unwrap();
    write_candidate_tree(&fx.gated(), fx.work(), &parent, files).unwrap()
}

fn fixture() -> Option<GitFixture> {
    let fx = GitFixture::with_system_git("candidate-delta").unwrap()?;
    for (path, body) in [
        ("README.md", &b"HEAD readme\n"[..]),
        ("stellar.yaml", b"schema: 1\n"),
        ("design/current.stellarref", b"generation-1\n"),
        ("design/other-document.starchunk", b"other document\0\xff"),
        ("design/removed.starchunk", b"removed chunk\n"),
        ("assets/script", b"executable content\n"),
    ] {
        fx.write(path, body).unwrap();
    }
    symlink("../missing-old-target", fx.work().join("assets/link")).unwrap();
    let head = fx.commit_all("base").unwrap();
    for path in ["vendor/module", "assets/module"] {
        fx.git([
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},{path}"),
        ])
        .unwrap();
    }
    fx.write("README.md", b"already staged readme\n").unwrap();
    fx.git(["add", "--", "README.md"]).unwrap();
    fx.write("README.md", b"unstaged readme\n").unwrap();
    fx.write(
        "design/current.stellarref",
        b"unselected worktree generation\n",
    )
    .unwrap();
    Some(fx)
}

fn unchanged() -> Vec<CandidateFile> {
    [
        "stellar.yaml",
        "design/current.stellarref",
        "design/other-document.starchunk",
        "design/removed.starchunk",
        "assets/script",
        "assets/link",
    ]
    .into_iter()
    .map(keep)
    .collect()
}

/// worktree·.git 전체 파일 bytes와 mode. symlink는 대상만 읽습니다.
fn disk_state(root: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
    let mut state = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                todo.push(path);
            } else {
                let content = if meta.file_type().is_symlink() {
                    std::fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_bytes()
                        .to_vec()
                } else {
                    std::fs::read(&path).unwrap()
                };
                state.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    (meta.permissions().mode(), content),
                );
            }
        }
    }
    state
}

#[test]
fn only_changed_paths_are_returned_with_exact_bytes_modes_and_deletions() {
    let Some(fx) = fixture() else { return };
    let special = p("assets/-new\t\n한글.bin");
    let files = vec![
        // 출처가 Bytes여도 내용이 같으면 unchanged입니다.
        bytes("stellar.yaml", FileModeKind::Regular, b"schema: 1\n"),
        bytes(
            "design/current.stellarref",
            FileModeKind::Regular,
            b"selected generation\n",
        ),
        keep("design/other-document.starchunk"),
        bytes(
            "assets/script",
            FileModeKind::Executable,
            b"executable content\n",
        ),
        bytes(
            "assets/link",
            FileModeKind::Symlink,
            b"../missing-new-target",
        ),
        CandidateFile::Bytes {
            path: special.clone(),
            kind: FileModeKind::Regular,
            bytes: vec![0, 255, b'\n'],
        },
    ];
    let candidate = tree(&fx, &files);
    let before = disk_state(fx.work());
    let candidate_before = disk_state(candidate.root());
    let delta = compute_candidate_delta(&fx.gated(), fx.work(), &candidate).unwrap();
    assert_eq!(delta.base_index(), candidate.base_index());
    assert_eq!(
        delta.changes(),
        &[
            FileDelta::Upsert {
                path: special,
                mode: FileMode::REGULAR,
                bytes: vec![0, 255, b'\n']
            },
            FileDelta::Upsert {
                path: p("assets/link"),
                mode: FileMode::SYMLINK,
                bytes: b"../missing-new-target".to_vec()
            },
            FileDelta::Upsert {
                path: p("assets/script"),
                mode: FileMode::EXECUTABLE,
                bytes: b"executable content\n".to_vec()
            },
            FileDelta::Upsert {
                path: p("design/current.stellarref"),
                mode: FileMode::REGULAR,
                bytes: b"selected generation\n".to_vec()
            },
            FileDelta::Remove {
                path: p("design/removed.starchunk")
            },
        ]
    );
    assert_eq!(
        compute_candidate_delta(&fx.gated(), fx.work(), &candidate).unwrap(),
        delta
    );
    assert_eq!(disk_state(fx.work()), before);
    assert_eq!(disk_state(candidate.root()), candidate_before);
}

#[test]
fn unchanged_candidate_emits_zero_updates_and_preserves_all_gitlinks() {
    let Some(fx) = fixture() else { return };
    let candidate = tree(&fx, &unchanged());
    let before = disk_state(fx.work());
    let delta = compute_candidate_delta(&fx.gated(), fx.work(), &candidate).unwrap();
    assert!(delta.changes().is_empty());
    assert_eq!(
        delta
            .base_index()
            .entries
            .iter()
            .filter(|e| e.is_gitlink())
            .count(),
        2
    );
    assert_eq!(
        std::fs::read(candidate.root().join("README.md")).unwrap(),
        b"already staged readme\n"
    );
    assert_eq!(disk_state(fx.work()), before);
}

#[test]
fn empty_index_adds_new_file_and_empty_candidate_removes_only_stellar_files() {
    let Some(fx) = GitFixture::with_system_git("candidate-delta-empty").unwrap() else {
        return;
    };
    let candidate = tree(
        &fx,
        &[bytes("stellar.yaml", FileModeKind::Regular, b"new\0")],
    );
    let delta = compute_candidate_delta(&fx.gated(), fx.work(), &candidate).unwrap();
    assert_eq!(
        delta.changes(),
        &[FileDelta::Upsert {
            path: p("stellar.yaml"),
            mode: FileMode::REGULAR,
            bytes: b"new\0".to_vec()
        }]
    );
    let Some(fx) = fixture() else { return };
    let candidate = tree(&fx, &[]);
    let delta = compute_candidate_delta(&fx.gated(), fx.work(), &candidate).unwrap();
    assert_eq!(delta.changes().len(), 6);
    assert!(delta
        .changes()
        .iter()
        .all(|change| matches!(change, FileDelta::Remove { .. })));
    assert!(!delta
        .changes()
        .iter()
        .any(|change| change.path() == &p("README.md")));
}

#[test]
fn stale_index_is_rejected_before_candidate_reads_and_untrusted_gate_spawns_nothing() {
    let Some(fx) = fixture() else { return };
    let candidate = tree(&fx, &unchanged());
    let untrusted = GatedGitRunner::new(fx.runner().clone(), GitTrust::NotTrusted);
    let before = fx.runner().spawn_count();
    assert_eq!(
        compute_candidate_delta(&untrusted, fx.work(), &candidate),
        Err(CandidateDeltaError::Index(GitOpError::Gate(
            GatedRunError::ProjectNotTrusted
        )))
    );
    assert_eq!(fx.runner().spawn_count(), before);
    fx.git(["add", "--", "README.md"]).unwrap();
    std::fs::remove_file(candidate.root().join("stellar.yaml")).unwrap();
    let before = disk_state(fx.work());
    assert_eq!(
        compute_candidate_delta(&fx.gated(), fx.work(), &candidate),
        Err(CandidateDeltaError::StaleBase)
    );
    assert_eq!(disk_state(fx.work()), before);
}

#[test]
fn missing_or_replaced_candidate_files_fail_without_following_symlinks() {
    let Some(fx) = fixture() else { return };
    let outside = fx.root().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("current.stellarref"), b"outside secret").unwrap();
    // 같은 index blob에서 만든 파일은 읽지 않으므로, 읽기 경로는 serializer bytes로 검사합니다.
    let files: Vec<_> = unchanged()
        .into_iter()
        .map(|file| match file.path().as_bytes() {
            b"design/current.stellarref" => bytes(
                "design/current.stellarref",
                FileModeKind::Regular,
                b"selected generation\n",
            ),
            _ => file,
        })
        .collect();
    for change in ["missing", "leaf-link", "parent-link", "mode"] {
        let candidate = tree(&fx, &files);
        let local = candidate.root().join("design/current.stellarref");
        match change {
            "missing" => std::fs::remove_file(&local).unwrap(),
            "leaf-link" => {
                std::fs::remove_file(&local).unwrap();
                symlink(outside.join("current.stellarref"), &local).unwrap();
            }
            "parent-link" => {
                std::fs::remove_dir_all(candidate.root().join("design")).unwrap();
                symlink(&outside, candidate.root().join("design")).unwrap();
            }
            _ => std::fs::set_permissions(&local, std::fs::Permissions::from_mode(0o755)).unwrap(),
        }
        let before = disk_state(fx.work());
        assert!(
            matches!(
                compute_candidate_delta(&fx.gated(), fx.work(), &candidate),
                Err(CandidateDeltaError::CandidateRead { .. })
            ),
            "{change}"
        );
        assert_eq!(disk_state(fx.work()), before);
        assert_eq!(
            std::fs::read(outside.join("current.stellarref")).unwrap(),
            b"outside secret"
        );
    }
}

#[test]
fn unchanged_index_blob_copies_are_neither_read_nor_staged() {
    let Some(fx) = fixture() else { return };
    let candidate = tree(&fx, &unchanged());
    // 임시 사본을 바꿔도 같은 index blob에서 만든 파일은 비교하지 않으므로 결과에 나오지 않습니다.
    std::fs::write(candidate.root().join("README.md"), b"tampered").unwrap();
    std::fs::write(
        candidate.root().join("design/other-document.starchunk"),
        b"tampered",
    )
    .unwrap();
    let before = fx.runner().spawn_count();
    let delta = compute_candidate_delta(&fx.gated(), fx.work(), &candidate).unwrap();
    assert!(delta.changes().is_empty());
    assert_eq!(fx.runner().spawn_count() - before, 2); // 전후 ls-files만, blob 조회 없음.
}

#[test]
fn gitlink_replacement_and_descendant_candidate_are_rejected() {
    let Some(fx) = fixture() else { return };
    for path in ["assets/module", "assets/module/new.bin"] {
        let candidate = tree(&fx, &[bytes(path, FileModeKind::Regular, b"replacement")]);
        assert_eq!(
            compute_candidate_delta(&fx.gated(), fx.work(), &candidate),
            Err(CandidateDeltaError::PreservedPathChanged {
                path: p("assets/module")
            })
        );
    }
}

#[test]
fn candidate_and_index_byte_limits_are_enforced() {
    let Some(fx) = GitFixture::with_system_git("candidate-delta-limits").unwrap() else {
        return;
    };
    let candidate = tree(
        &fx,
        &[
            bytes("assets/a", FileModeKind::Regular, b"123456"),
            bytes("assets/b", FileModeKind::Regular, b"abcdef"),
        ],
    );
    let limit = BlobLimits {
        max_blob_bytes: 5,
        max_batch_bytes: 5,
        max_total_bytes: 12,
    };
    assert_eq!(
        compute_candidate_delta_with_limits(&fx.gated(), fx.work(), &candidate, &limit),
        Err(CandidateDeltaError::CandidateTooLarge {
            path: p("assets/a"),
            max_bytes: 5
        })
    );
    let limit = BlobLimits {
        max_blob_bytes: 6,
        max_total_bytes: 11,
        ..limit
    };
    assert_eq!(
        compute_candidate_delta_with_limits(&fx.gated(), fx.work(), &candidate, &limit),
        Err(CandidateDeltaError::CandidateTotalTooLarge { max_bytes: 11 })
    );
    let limit = BlobLimits {
        max_total_bytes: 12,
        ..limit
    };
    assert_eq!(
        compute_candidate_delta_with_limits(&fx.gated(), fx.work(), &candidate, &limit)
            .unwrap()
            .changes()
            .len(),
        2
    );
    fx.write("assets/a", b"123456").unwrap();
    fx.write("assets/b", b"abcdef").unwrap();
    fx.git(["add", "--", "assets"]).unwrap();
    // 같은 blob에서 만든 파일은 상한 계산 대상이 아닙니다. 추적 파일 합이 상한을 넘어도 읽지 않습니다.
    let kept = tree(&fx, &[keep("assets/a"), keep("assets/b")]);
    let tiny = BlobLimits {
        max_blob_bytes: 1,
        max_batch_bytes: 1,
        max_total_bytes: 1,
    };
    let before = fx.runner().spawn_count();
    assert!(
        compute_candidate_delta_with_limits(&fx.gated(), fx.work(), &kept, &tiny)
            .unwrap()
            .changes()
            .is_empty()
    );
    assert_eq!(fx.runner().spawn_count() - before, 2); // 전후 ls-files만.
                                                       // serializer bytes는 기존 index blob과 비교하므로 index 쪽 상한을 적용합니다.
    let candidate = tree(
        &fx,
        &[
            bytes("assets/a", FileModeKind::Regular, b"123456"),
            bytes("assets/b", FileModeKind::Regular, b"abcdef"),
        ],
    );
    let before = fx.runner().spawn_count();
    let limit = BlobLimits {
        max_blob_bytes: 5,
        ..limit
    };
    assert!(matches!(
        compute_candidate_delta_with_limits(&fx.gated(), fx.work(), &candidate, &limit),
        Err(CandidateDeltaError::Blob(BlobError::BlobTooLarge {
            max_bytes: 5,
            ..
        }))
    ));
    assert_eq!(fx.runner().spawn_count() - before, 2); // ls-files + batch-check, content 없음.
    let limit = BlobLimits {
        max_blob_bytes: 6,
        max_total_bytes: 11,
        ..limit
    };
    assert_eq!(
        compute_candidate_delta_with_limits(&fx.gated(), fx.work(), &candidate, &limit),
        Err(CandidateDeltaError::Blob(BlobError::TotalTooLarge {
            max_bytes: 11
        }))
    );
    let limit = BlobLimits {
        max_total_bytes: 12,
        ..limit
    };
    let before = fx.runner().spawn_count();
    assert!(
        compute_candidate_delta_with_limits(&fx.gated(), fx.work(), &candidate, &limit)
            .unwrap()
            .changes()
            .is_empty()
    );
    assert_eq!(fx.runner().spawn_count() - before, 5); // 전후 ls-files + batch-check + content 2 batch.
}

#[test]
fn symlink_size_limit_uses_target_bytes_and_never_target_contents() {
    let Some(fx) = GitFixture::with_system_git("candidate-delta-link-limit").unwrap() else {
        return;
    };
    let candidate = tree(
        &fx,
        &[bytes("assets/link", FileModeKind::Symlink, b"../missing")],
    );
    let limit = BlobLimits {
        max_blob_bytes: 9,
        max_batch_bytes: 9,
        max_total_bytes: 9,
    };
    assert_eq!(
        compute_candidate_delta_with_limits(&fx.gated(), fx.work(), &candidate, &limit),
        Err(CandidateDeltaError::CandidateTooLarge {
            path: p("assets/link"),
            max_bytes: 9
        })
    );
    let delta = compute_candidate_delta(&fx.gated(), fx.work(), &candidate).unwrap();
    assert_eq!(
        delta.changes(),
        &[FileDelta::Upsert {
            path: p("assets/link"),
            mode: FileMode::SYMLINK,
            bytes: b"../missing".to_vec()
        }]
    );
}
