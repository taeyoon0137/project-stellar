//! Git 통합 corpus: 특수 경로·mode·LFS pointer·손상 프로젝트 (XIV.14.6).
//!
//! XIV.14.1 fixture harness(hermetic·네트워크 없음)로 실제 Git 저장소를 만들고, 각 scenario에서 status·
//! snapshot provider·materialize·LFS 판정·semantic 실패 분리 정책이 정해진 결과를 내는지 고정합니다.
//!
//! | scenario | 정책 |
//! |---|---|
//! | non-UTF8 경로 | Git 경로 bytes를 원시 그대로 보존(status·index·tree·Stellar 분류), 표시는 lossy 표기, 파일 시스템이 표현하지 못하면 materialize는 typed error로 실패하고 부분 결과를 남기지 않음 |
//! | symlink | mode `120000`·target bytes 보존, 작업 트리 목록은 따라가지 않고 Symlink, materialize는 링크만 만들고 따라가지 않음 |
//! | executable | mode `100755` 보존, materialize `0755`, 실행 비트 제거는 worktree mode 변경 `.M` |
//! | LFS pointer | object 없는 pointer는 일반 missing과 다른 진단 코드, materialize는 pointer bytes 그대로(filter 미실행) |
//! | 손상 프로젝트 | raw status 유지·semantic만 실패, 자동 수정 없음, HEAD·Index snapshot은 정상 내용으로 계속 읽힘 |
//!
//! 이 모듈은 테스트 빌드 전용입니다.

use std::ffi::OsStr;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::classify::StellarFileKind;
use crate::decode::GitPath;
use crate::file_status::{read_stellar_file_statuses, FileChange, StellarFileStatus};
use crate::fixture::GitFixture;
use crate::lfs::pointer::{inspect_asset_file, parse_lfs_pointer, AssetFileState};
use crate::object::FileMode;
use crate::project_status::{read_project_status, SemanticStatusProvider};
use crate::snapshot::working_tree::{
    list_working_tree_files, read_working_tree_file, WorkingTreeFileKind,
};
use crate::snapshot::{
    materialize_snapshot, read_index_snapshot, read_tree_snapshot, MaterializeError, SnapshotEntry,
    SnapshotSource,
};
use crate::status::{read_status, StatusEntry, StatusOptions};
use crate::trust::GatedGitRunner;

fn fixture(tag: &str) -> Option<GitFixture> {
    GitFixture::with_system_git(tag).expect("fixture 생성")
}

/// 기본 Stellar 프로젝트 파일(매니페스트·current pointer·chunk)을 commit합니다.
fn commit_project(fx: &GitFixture) {
    fx.write("stellar.yaml", b"schema_version: 1\nproject_id: fixture\n")
        .unwrap();
    fx.write("design/current.stellarref", b"gen-1\n").unwrap();
    fx.write("design/objects/aa/one.starchunk", b"CHUNK:nodes-1")
        .unwrap();
    fx.commit_all("project").unwrap();
}

fn entry<'a>(entries: &'a [SnapshotEntry], path: &[u8]) -> &'a SnapshotEntry {
    entries
        .iter()
        .find(|e| e.path.as_bytes() == path)
        .unwrap_or_else(|| panic!("항목 없음: {:?}", String::from_utf8_lossy(path)))
}

fn status_of<'a>(statuses: &'a [StellarFileStatus], path: &[u8]) -> &'a StellarFileStatus {
    statuses
        .iter()
        .find(|s| s.path.as_bytes() == path)
        .unwrap_or_else(|| panic!("상태 없음: {:?}", String::from_utf8_lossy(path)))
}

/// 임시 parent 디렉터리(drop 시 삭제).
struct Scratch(PathBuf);

impl Scratch {
    fn new(fx: &GitFixture, name: &str) -> Scratch {
        let path = fx.root().join(name);
        std::fs::create_dir_all(&path).unwrap();
        Scratch(path)
    }

    fn is_empty(&self) -> bool {
        std::fs::read_dir(&self.0).unwrap().next().is_none()
    }
}

/// `.git`을 제외한 작업 트리 파일 bytes(경로순).
fn tree_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for item in std::fs::read_dir(&dir).unwrap() {
            let item = item.unwrap();
            let path = item.path();
            if path.file_name() == Some(OsStr::new(".git")) {
                continue;
            }
            let kind = item.file_type().unwrap();
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_symlink() {
                out.push((
                    path.clone(),
                    std::fs::read_link(&path)
                        .unwrap()
                        .into_os_string()
                        .into_vec(),
                ));
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn non_utf8_stellar_path_bytes_are_preserved_and_never_lossily_materialized() {
    let Some(fx) = fixture("corpus-non-utf8") else {
        return; // git 미설치 환경 건너뜀.
    };
    commit_project(&fx);
    // APFS는 UTF-8이 아닌 이름의 파일을 만들 수 없으므로 index에만 등록하고 commit합니다.
    let raw: &[u8] = b"assets/caf\xe9.png";
    let out = fx
        .runner()
        .run_with_input(
            &crate::runner::argv(["hash-object", "-w", "--stdin"]),
            fx.work(),
            b"\x89PNG-raw",
        )
        .unwrap();
    let oid = String::from_utf8(out.stdout).unwrap().trim().to_string();
    let mut cacheinfo = format!("100644,{oid},").into_bytes();
    cacheinfo.extend_from_slice(raw);
    fx.git([
        OsStr::new("update-index"),
        OsStr::new("--add"),
        OsStr::new("--cacheinfo"),
        OsStr::from_bytes(&cacheinfo),
    ])
    .unwrap();
    fx.git(["commit", "-q", "-m", "raw path"]).unwrap();
    let gate: GatedGitRunner = fx.gated();

    // index·HEAD tree는 원시 bytes 그대로.
    let index = read_index_snapshot(&gate, fx.work()).unwrap();
    assert_eq!(entry(&index.entries, raw).oid.as_str(), oid);
    let head = read_tree_snapshot(&gate, fx.work(), &SnapshotSource::Head).unwrap();
    assert_eq!(entry(&head.entries, raw).mode, FileMode::REGULAR);
    // status: 작업 트리에 없는 tracked 경로 → `.D`, Stellar asset으로 분류.
    let statuses = read_stellar_file_statuses(&gate, fx.work()).unwrap();
    let status = status_of(&statuses, raw);
    assert_eq!(status.kind, StellarFileKind::Asset);
    assert_eq!(status.unstaged, Some(FileChange::Deleted));
    assert!(status.in_index);
    // 표시는 lossy 표기(원시 bytes는 별도 보존).
    let text = GitPath::from_bytes(raw).text();
    assert!(text.is_lossy());
    assert!(text.display().contains('\u{FFFD}'));
    assert_eq!(text.raw(), raw);

    // materialize: 표현할 수 없는 이름은 typed error, 부분 snapshot을 남기지 않습니다.
    let scratch = Scratch::new(&fx, "materialize-non-utf8");
    let result = materialize_snapshot(&gate, fx.work(), &head.entries, &scratch.0);
    let refused = match &result {
        Err(MaterializeError::Io {
            path: Some(path), ..
        }) => {
            assert_eq!(path.as_bytes(), raw);
            assert!(
                scratch.is_empty(),
                "실패한 materialize의 임시 디렉터리가 남음"
            );
            true
        }
        // non-UTF8 이름을 허용하는 파일 시스템: 같은 bytes의 이름과 내용(lossy 이름 아님).
        Ok(snapshot) => {
            let local = snapshot.root().join(OsStr::from_bytes(raw));
            assert_eq!(std::fs::read(local).unwrap(), b"\x89PNG-raw");
            false
        }
        Err(other) => panic!("예상하지 못한 materialize 결과: {other:?}"),
    };
    // macOS APFS는 UTF-8이 아닌 이름을 거부(EILSEQ)하므로 typed error 경로여야 합니다.
    if cfg!(target_os = "macos") {
        assert!(
            refused,
            "macOS에서는 non-UTF8 이름 materialize가 거부되어야 함"
        );
    }
}

#[test]
fn symlink_and_executable_modes_are_preserved_without_following_links() {
    let Some(fx) = fixture("corpus-modes") else {
        return;
    };
    commit_project(&fx);
    fx.write("assets/tool.bin", b"#!/bin/sh\necho hi\n")
        .unwrap();
    let tool = fx.work().join("assets/tool.bin");
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    // 내부 target과 외부(절대) target symlink. 외부 target은 존재하지 않아도 됩니다.
    std::os::unix::fs::symlink("../stellar.yaml", fx.work().join("design/manifest-link")).unwrap();
    std::os::unix::fs::symlink(
        "/nonexistent/stellar-outside.png",
        fx.work().join("assets/outside.png"),
    )
    .unwrap();
    fx.commit_all("modes").unwrap();
    let gate = fx.gated();

    let head = read_tree_snapshot(&gate, fx.work(), &SnapshotSource::Head).unwrap();
    assert_eq!(
        entry(&head.entries, b"assets/tool.bin").mode,
        FileMode::EXECUTABLE
    );
    assert_eq!(
        entry(&head.entries, b"design/manifest-link").mode,
        FileMode::SYMLINK
    );
    assert_eq!(
        entry(&head.entries, b"assets/outside.png").mode,
        FileMode::SYMLINK
    );
    let index = read_index_snapshot(&gate, fx.work()).unwrap();
    assert_eq!(
        entry(&index.entries, b"assets/tool.bin").mode,
        FileMode::EXECUTABLE
    );

    // 작업 트리 목록: 링크를 따라가지 않고 Symlink, 내용은 target bytes.
    let listing = list_working_tree_files(&gate, fx.work()).unwrap();
    let file = |path: &[u8]| {
        listing
            .files
            .iter()
            .find(|f| f.path.as_bytes() == path)
            .unwrap()
            .clone()
    };
    assert_eq!(
        file(b"assets/tool.bin").kind,
        WorkingTreeFileKind::Executable
    );
    let outside = file(b"assets/outside.png");
    assert_eq!(outside.kind, WorkingTreeFileKind::Symlink);
    assert_eq!(
        read_working_tree_file(fx.work(), &outside).unwrap(),
        b"/nonexistent/stellar-outside.png"
    );

    // materialize: 실행 파일 0755, symlink는 링크만(target 미추적).
    let scratch = Scratch::new(&fx, "materialize-modes");
    let snapshot = materialize_snapshot(&gate, fx.work(), &head.entries, &scratch.0).unwrap();
    let mode = std::fs::metadata(snapshot.root().join("assets/tool.bin"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755);
    for (link, target) in [
        ("design/manifest-link", "../stellar.yaml"),
        ("assets/outside.png", "/nonexistent/stellar-outside.png"),
    ] {
        let local = snapshot.root().join(link);
        assert!(std::fs::symlink_metadata(&local)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_link(&local).unwrap(), PathBuf::from(target));
    }
    assert_eq!(snapshot.symlinks(), 2);
    drop(snapshot);

    // 실행 비트 제거 → worktree mode 변경(`.M`, 100755 → 100644).
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o644)).unwrap();
    let report = read_status(&gate, fx.work(), StatusOptions::default()).unwrap();
    let changed = report
        .entries
        .iter()
        .find_map(|e| match e {
            StatusEntry::Ordinary(c) if c.path.as_bytes() == b"assets/tool.bin" => Some(c.clone()),
            _ => None,
        })
        .expect("mode 변경 항목");
    assert_eq!(changed.mode_index, FileMode::EXECUTABLE);
    assert_eq!(changed.mode_worktree, FileMode::REGULAR);
    let statuses = read_stellar_file_statuses(&gate, fx.work()).unwrap();
    assert_eq!(
        status_of(&statuses, b"assets/tool.bin").unstaged,
        Some(FileChange::Modified)
    );
}

#[test]
fn lfs_pointer_without_object_is_distinct_from_a_missing_file() {
    let Some(fx) = fixture("corpus-lfs") else {
        return;
    };
    commit_project(&fx);
    let pointer = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 12345\n",
        "ab".repeat(32)
    );
    fx.write("assets/big.psd", pointer.as_bytes()).unwrap();
    fx.commit_all("lfs pointer").unwrap();
    let gate = fx.gated();

    // 작업 트리: object 없는 pointer → missing LFS 진단(일반 missing과 다른 코드).
    let path = GitPath::from_bytes(b"assets/big.psd");
    let state = inspect_asset_file(fx.work(), &path, Some(12345));
    match &state {
        AssetFileState::LfsObjectMissing { pointer } => {
            assert_eq!(pointer.size, 12345);
            assert_eq!(pointer.oid, "ab".repeat(32));
        }
        other => panic!("{other:?}"),
    }
    let missing = inspect_asset_file(fx.work(), &GitPath::from_bytes(b"assets/gone.psd"), None);
    assert_eq!(missing, AssetFileState::Missing);
    assert_eq!(
        state.diagnostic_code(),
        Some(AssetFileState::LFS_OBJECT_MISSING_CODE)
    );
    assert_eq!(
        missing.diagnostic_code(),
        Some(AssetFileState::MISSING_CODE)
    );
    assert_ne!(state.diagnostic_code(), missing.diagnostic_code());

    // HEAD materialize: filter를 실행하지 않고 pointer bytes 그대로.
    let head = read_tree_snapshot(&gate, fx.work(), &SnapshotSource::Head).unwrap();
    let scratch = Scratch::new(&fx, "materialize-lfs");
    let snapshot = materialize_snapshot(&gate, fx.work(), &head.entries, &scratch.0).unwrap();
    let bytes = std::fs::read(snapshot.root().join("assets/big.psd")).unwrap();
    assert_eq!(bytes, pointer.as_bytes());
    assert_eq!(parse_lfs_pointer(&bytes).map(|p| p.size), Some(12345));
    assert_eq!(
        inspect_asset_file(snapshot.root(), &path, Some(12345)).diagnostic_code(),
        Some(AssetFileState::LFS_OBJECT_MISSING_CODE)
    );
}

/// 매니페스트·chunk 형식을 검사하는 fixture semantic provider(쓰기 없음).
struct ManifestCheckingProvider;

impl SemanticStatusProvider for ManifestCheckingProvider {
    type Status = ();
    type Error = &'static str;

    fn semantic_status(
        &self,
        _git: &GatedGitRunner,
        worktree: &Path,
        _files: &[StellarFileStatus],
    ) -> Result<(), &'static str> {
        let manifest = std::fs::read(worktree.join("stellar.yaml")).map_err(|_| "manifest 없음")?;
        if !manifest.starts_with(b"schema_version: 1\n") {
            return Err("manifest 손상");
        }
        let chunk = std::fs::read(worktree.join("design/objects/aa/one.starchunk"))
            .map_err(|_| "chunk 없음")?;
        if !chunk.starts_with(b"CHUNK:") {
            return Err("chunk 손상");
        }
        Ok(())
    }
}

#[test]
fn corrupt_project_keeps_raw_status_and_readable_git_snapshots() {
    let Some(fx) = fixture("corpus-corrupt") else {
        return;
    };
    commit_project(&fx);
    fx.write("README.md", b"# P\n").unwrap();
    fx.commit_all("readme").unwrap();
    let gate = fx.gated();
    assert_eq!(
        read_project_status(&gate, fx.work(), &ManifestCheckingProvider)
            .unwrap()
            .semantic,
        Ok(())
    );

    // 작업 트리의 매니페스트와 chunk 손상, README 수정.
    fx.write("stellar.yaml", b"\xff\xfe not yaml").unwrap();
    fx.write("design/objects/aa/one.starchunk", b"\x00garbage")
        .unwrap();
    fx.write("README.md", b"# P\n\nmore\n").unwrap();
    let before = tree_bytes(fx.work());
    let index_before = read_index_snapshot(&gate, fx.work()).unwrap();

    let status = read_project_status(&gate, fx.work(), &ManifestCheckingProvider).unwrap();
    assert_eq!(status.semantic, Err("manifest 손상"));
    assert_eq!(status.repository.counts.unstaged, 3);
    assert_eq!(
        status
            .raw
            .iter()
            .map(|r| (r.path.as_bytes(), r.xy.as_str()))
            .collect::<Vec<_>>(),
        vec![(&b"README.md"[..], ".M")]
    );
    let changed: Vec<(&[u8], StellarFileKind)> = status
        .stellar_files
        .iter()
        .map(|s| (s.path.as_bytes(), s.kind))
        .collect();
    assert_eq!(
        changed,
        vec![
            (
                &b"design/objects/aa/one.starchunk"[..],
                StellarFileKind::Design
            ),
            (b"stellar.yaml", StellarFileKind::Manifest),
        ]
    );
    // 자동 수정 없음.
    assert_eq!(tree_bytes(fx.work()), before);
    assert_eq!(read_index_snapshot(&gate, fx.work()).unwrap(), index_before);

    // Index snapshot은 손상 전(정상) 내용으로 계속 materialize됩니다(HEAD·Index 쪽 semantic 입력 유지).
    let scratch = Scratch::new(&fx, "materialize-index");
    let snapshot =
        materialize_snapshot(&gate, fx.work(), &index_before.entries, &scratch.0).unwrap();
    assert_eq!(
        std::fs::read(snapshot.root().join("stellar.yaml")).unwrap(),
        b"schema_version: 1\nproject_id: fixture\n"
    );
    assert_eq!(
        std::fs::read(snapshot.root().join("design/objects/aa/one.starchunk")).unwrap(),
        b"CHUNK:nodes-1"
    );
    assert!(!snapshot.root().join(".git").exists());
}
