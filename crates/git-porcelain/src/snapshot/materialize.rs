//! snapshot을 임시 virtual project 디렉터리로 materialize (XIV.2.6).
//!
//! [`SnapshotEntry`] 목록(HEAD·commit·tree·index)의 blob을 `git cat-file --batch`
//! ([`read_blobs`])로 읽어 **새 임시 디렉터리**에 원시 바이트 그대로 씁니다. Semantic snapshot
//! loader는 이 디렉터리를 일반 프로젝트 폴더처럼 읽습니다.
//!
//! 안전 규칙:
//! - **`.git`을 만들지 않습니다.** 경로 component가 `.git`(대소문자 무시, 끝 공백·점 무시, macOS가
//!   무시하는 Unicode 문자 제거 후 비교)이면 거부합니다. 빈 component, `.`, `..`, 절대 경로도
//!   거부합니다. 모든 경로를 먼저 검증하므로 거부 시 Git을 실행하지 않고 파일도 만들지 않습니다.
//! - 파일·symlink는 이미 있으면 실패하는 방식(`create_new`)으로 만듭니다. 대소문자만 다른 경로나
//!   파일/디렉터리 충돌은 덮어쓰지 않고 [`MaterializeError::PathCollision`]입니다.
//! - 상위 component가 symlink이면 그 아래에 쓰지 않습니다(snapshot 밖으로 쓰기 방지). symlink는
//!   대상 경로 바이트로 링크만 만들고 따라가지 않습니다.
//! - mode: `100755`는 `0755`, 일반 파일은 `0644`, root 디렉터리는 `0700`(unix).
//! - submodule(gitlink)은 내용이 다른 저장소이므로 만들지 않고 [`MaterializedSnapshot`]에 기록합니다.
//! - 실패하면 만든 디렉터리 전체를 지웁니다. 성공 결과도 drop 시 지우며, 보존하려면
//!   [`MaterializedSnapshot::keep`]을 호출합니다.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::blob::{read_blobs, BlobError, BATCH_CHUNK};
use super::SnapshotEntry;
use crate::decode::GitPath;
use crate::object::{FileModeKind, ObjectId};
use crate::trust::GatedGitRunner;

/// 경로 거부 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsafePathReason {
    /// 빈 경로.
    Empty,
    /// `/`로 시작(절대 경로).
    Absolute,
    /// 빈 component(`a//b`, 끝 `/`).
    EmptyComponent,
    /// `.` 또는 `..`.
    DotComponent,
    /// `.git` 또는 그와 같게 해석될 수 있는 이름.
    DotGit,
    /// NUL 포함.
    Nul,
    /// 이 플랫폼에서 표현할 수 없는 경로(Windows의 non-UTF8 등).
    NotRepresentable,
}

/// materialize 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializeError {
    /// blob 읽기 실패.
    Blob(BlobError),
    /// 안전하지 않은 경로(Git 실행·파일 생성 전 거부).
    UnsafePath {
        /// 경로.
        path: GitPath,
        /// 이유.
        reason: UnsafePathReason,
    },
    /// 같은 위치에 이미 항목이 있음(대소문자 충돌, 파일/디렉터리 충돌, symlink 상위).
    PathCollision {
        /// 경로.
        path: GitPath,
    },
    /// mode를 materialize할 수 없음(tree·없음 mode 등).
    UnsupportedMode {
        /// 경로.
        path: GitPath,
    },
    /// 이 플랫폼에서 symlink를 만들 수 없음.
    SymlinkUnsupported {
        /// 경로.
        path: GitPath,
    },
    /// 파일 시스템 오류.
    Io {
        /// 관련 경로(임시 root면 `None`).
        path: Option<GitPath>,
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl std::fmt::Display for MaterializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaterializeError::Blob(e) => write!(f, "{e}"),
            MaterializeError::UnsafePath { path, reason } => write!(
                f,
                "안전하지 않은 snapshot 경로 {:?}: {reason:?}",
                path.text().display()
            ),
            MaterializeError::PathCollision { path } => {
                write!(f, "snapshot 경로 충돌: {:?}", path.text().display())
            }
            MaterializeError::UnsupportedMode { path } => {
                write!(f, "materialize할 수 없는 mode: {:?}", path.text().display())
            }
            MaterializeError::SymlinkUnsupported { path } => write!(
                f,
                "이 플랫폼에서 symlink를 만들 수 없음: {:?}",
                path.text().display()
            ),
            MaterializeError::Io { path, kind } => match path {
                Some(p) => write!(f, "파일 시스템 오류 {:?}: {kind:?}", p.text().display()),
                None => write!(f, "임시 디렉터리 오류: {kind:?}"),
            },
        }
    }
}

impl std::error::Error for MaterializeError {}

impl From<BlobError> for MaterializeError {
    fn from(e: BlobError) -> MaterializeError {
        MaterializeError::Blob(e)
    }
}

/// materialize된 임시 프로젝트 디렉터리(drop 시 삭제, `keep`으로 보존).
#[derive(Debug)]
pub struct MaterializedSnapshot {
    root: PathBuf,
    keep: bool,
    files: u64,
    symlinks: u64,
    skipped_gitlinks: Vec<GitPath>,
}

impl MaterializedSnapshot {
    /// 임시 프로젝트 root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 만든 일반·실행 파일 수.
    pub fn files(&self) -> u64 {
        self.files
    }

    /// 만든 symlink 수.
    pub fn symlinks(&self) -> u64 {
        self.symlinks
    }

    /// 만들지 않은 submodule(gitlink) 경로.
    pub fn skipped_gitlinks(&self) -> &[GitPath] {
        &self.skipped_gitlinks
    }

    /// drop 시 삭제하지 않고 경로를 돌려줍니다(호출자가 정리 책임).
    pub fn keep(mut self) -> PathBuf {
        self.keep = true;
        self.root.clone()
    }
}

impl Drop for MaterializedSnapshot {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// macOS(HFS+/APFS)가 이름 비교에서 무시하는 Unicode 문자.
fn is_ignorable_char(c: char) -> bool {
    matches!(c,
        '\u{200C}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{206A}'..='\u{206F}' | '\u{FEFF}')
}

/// component가 `.git`으로 해석될 수 있는지.
fn is_dot_git(component: &[u8]) -> bool {
    let normalized: Vec<u8> = match std::str::from_utf8(component) {
        Ok(text) => text
            .chars()
            .filter(|c| !is_ignorable_char(*c))
            .collect::<String>()
            .into_bytes(),
        Err(_) => component.to_vec(),
    };
    let mut trimmed: &[u8] = &normalized;
    while let Some(rest) = trimmed
        .strip_suffix(b" ")
        .or_else(|| trimmed.strip_suffix(b"."))
    {
        trimmed = rest;
    }
    trimmed.eq_ignore_ascii_case(b".git")
}

/// snapshot 경로를 검증하고 component로 나눕니다.
pub fn validate_snapshot_path(raw: &[u8]) -> Result<Vec<&[u8]>, UnsafePathReason> {
    if raw.is_empty() {
        return Err(UnsafePathReason::Empty);
    }
    if raw.contains(&0) {
        return Err(UnsafePathReason::Nul);
    }
    if raw.starts_with(b"/") {
        return Err(UnsafePathReason::Absolute);
    }
    let components: Vec<&[u8]> = raw.split(|b| *b == b'/').collect();
    for c in &components {
        if c.is_empty() {
            return Err(UnsafePathReason::EmptyComponent);
        }
        if *c == b"." || *c == b".." {
            return Err(UnsafePathReason::DotComponent);
        }
        if is_dot_git(c) {
            return Err(UnsafePathReason::DotGit);
        }
    }
    Ok(components)
}

#[cfg(unix)]
fn component_os(c: &[u8]) -> Option<&std::ffi::OsStr> {
    use std::os::unix::ffi::OsStrExt;
    Some(std::ffi::OsStr::from_bytes(c))
}

#[cfg(not(unix))]
fn component_os(c: &[u8]) -> Option<&std::ffi::OsStr> {
    std::str::from_utf8(c).ok().map(std::ffi::OsStr::new)
}

/// `parent` 아래에 0700 권한의 새 고유 디렉터리를 만듭니다.
pub(crate) fn create_unique_root(parent: &Path) -> Result<PathBuf, MaterializeError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let io = |e: std::io::Error| MaterializeError::Io {
        path: None,
        kind: e.kind(),
    };
    for _ in 0..64 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let root = parent.join(format!(
            "stellar-snapshot-{}-{n}-{nanos:08x}",
            std::process::id()
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&root) {
            Ok(()) => return Ok(root),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io(e)),
        }
    }
    Err(MaterializeError::Io {
        path: None,
        kind: std::io::ErrorKind::AlreadyExists,
    })
}

/// root 아래 상위 디렉터리를 symlink를 따르지 않고 만듭니다.
fn ensure_parent_dirs(
    root: &Path,
    components: &[&[u8]],
    path: &GitPath,
) -> Result<PathBuf, MaterializeError> {
    let mut current = root.to_path_buf();
    let io = |e: std::io::Error| MaterializeError::Io {
        path: Some(path.clone()),
        kind: e.kind(),
    };
    for c in &components[..components.len() - 1] {
        let os = component_os(c).ok_or_else(|| MaterializeError::UnsafePath {
            path: path.clone(),
            reason: UnsafePathReason::NotRepresentable,
        })?;
        current.push(os);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(MaterializeError::PathCollision { path: path.clone() }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(io)?;
            }
            Err(e) => return Err(io(e)),
        }
    }
    let last = components[components.len() - 1];
    let os = component_os(last).ok_or_else(|| MaterializeError::UnsafePath {
        path: path.clone(),
        reason: UnsafePathReason::NotRepresentable,
    })?;
    current.push(os);
    Ok(current)
}

fn write_file(
    target: &Path,
    bytes: &[u8],
    executable: bool,
    path: &GitPath,
) -> Result<(), MaterializeError> {
    use std::io::Write;
    let map = |e: std::io::Error| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            MaterializeError::PathCollision { path: path.clone() }
        } else {
            MaterializeError::Io {
                path: Some(path.clone()),
                kind: e.kind(),
            }
        }
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(map)?;
    file.write_all(bytes).map_err(map)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(mode)).map_err(map)?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(())
}

#[cfg(unix)]
fn write_symlink(target: &Path, link_bytes: &[u8], path: &GitPath) -> Result<(), MaterializeError> {
    use std::os::unix::ffi::OsStrExt;
    std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(link_bytes), target).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            MaterializeError::PathCollision { path: path.clone() }
        } else {
            MaterializeError::Io {
                path: Some(path.clone()),
                kind: e.kind(),
            }
        }
    })
}

#[cfg(not(unix))]
fn write_symlink(_target: &Path, _link: &[u8], path: &GitPath) -> Result<(), MaterializeError> {
    Err(MaterializeError::SymlinkUnsupported { path: path.clone() })
}

/// snapshot 항목을 `parent` 아래 새 임시 디렉터리에 materialize합니다(`.git` 없음).
pub fn materialize_snapshot(
    git: &GatedGitRunner,
    worktree: &Path,
    entries: &[SnapshotEntry],
    parent: &Path,
) -> Result<MaterializedSnapshot, MaterializeError> {
    // 1) 모든 경로·mode를 먼저 검증(Git 실행·파일 생성 전).
    for entry in entries {
        validate_snapshot_path(entry.path.as_bytes()).map_err(|reason| {
            MaterializeError::UnsafePath {
                path: entry.path.clone(),
                reason,
            }
        })?;
        match entry.kind() {
            Some(
                FileModeKind::Regular
                | FileModeKind::Executable
                | FileModeKind::Symlink
                | FileModeKind::Gitlink,
            ) => {}
            _ => {
                return Err(MaterializeError::UnsupportedMode {
                    path: entry.path.clone(),
                })
            }
        }
    }

    // 2) 임시 root(실패 시 drop guard가 삭제).
    let mut snapshot = MaterializedSnapshot {
        root: create_unique_root(parent)?,
        keep: false,
        files: 0,
        symlinks: 0,
        skipped_gitlinks: Vec::new(),
    };

    // 3) blob을 chunk 단위로 읽어 씁니다.
    let content_entries: Vec<&SnapshotEntry> = entries
        .iter()
        .filter(|e| {
            if e.is_gitlink() {
                snapshot.skipped_gitlinks.push(e.path.clone());
                false
            } else {
                true
            }
        })
        .collect();
    for chunk in content_entries.chunks(BATCH_CHUNK) {
        let oids: Vec<ObjectId> = chunk.iter().map(|e| e.oid.clone()).collect();
        let blobs = read_blobs(git, worktree, &oids)?;
        for entry in chunk {
            let components = validate_snapshot_path(entry.path.as_bytes()).map_err(|reason| {
                MaterializeError::UnsafePath {
                    path: entry.path.clone(),
                    reason,
                }
            })?;
            let target = ensure_parent_dirs(&snapshot.root, &components, &entry.path)?;
            let bytes = blobs
                .get(&entry.oid)
                .ok_or_else(|| MaterializeError::Blob(BlobError::Missing(entry.oid.clone())))?;
            match entry.kind() {
                Some(FileModeKind::Symlink) => {
                    write_symlink(&target, bytes, &entry.path)?;
                    snapshot.symlinks += 1;
                }
                Some(FileModeKind::Executable) => {
                    write_file(&target, bytes, true, &entry.path)?;
                    snapshot.files += 1;
                }
                _ => {
                    write_file(&target, bytes, false, &entry.path)?;
                    snapshot.files += 1;
                }
            }
        }
    }
    Ok(snapshot)
}

/// 경로·종류 목록을 새 임시 디렉터리에 materialize하되 내용은 `read`가 항목마다 공급합니다(Git 실행 없음,
/// XXIII.1.3 작업 트리 source).
///
/// [`materialize_snapshot`]과 같은 안전 규칙(먼저 모든 경로·종류 검증, `.git` 거부, `create_new`, 상위
/// symlink 거부, 실패 시 전체 삭제)을 씁니다. 종류는 일반·실행 파일과 symlink만 허용합니다. symlink 내용은
/// 대상 경로 바이트입니다. 내용을 한 항목씩 읽어 쓰므로 전체를 메모리에 모으지 않습니다.
pub(crate) fn materialize_with<F>(
    entries: &[(GitPath, FileModeKind)],
    parent: &Path,
    mut read: F,
) -> Result<MaterializedSnapshot, MaterializeError>
where
    F: FnMut(usize, &GitPath, FileModeKind) -> Result<Vec<u8>, MaterializeError>,
{
    for (path, kind) in entries {
        validate_snapshot_path(path.as_bytes()).map_err(|reason| MaterializeError::UnsafePath {
            path: path.clone(),
            reason,
        })?;
        if !matches!(
            kind,
            FileModeKind::Regular | FileModeKind::Executable | FileModeKind::Symlink
        ) {
            return Err(MaterializeError::UnsupportedMode { path: path.clone() });
        }
    }
    let mut snapshot = MaterializedSnapshot {
        root: create_unique_root(parent)?,
        keep: false,
        files: 0,
        symlinks: 0,
        skipped_gitlinks: Vec::new(),
    };
    for (index, (path, kind)) in entries.iter().enumerate() {
        let components = validate_snapshot_path(path.as_bytes()).map_err(|reason| {
            MaterializeError::UnsafePath {
                path: path.clone(),
                reason,
            }
        })?;
        let target = ensure_parent_dirs(&snapshot.root, &components, path)?;
        let bytes = read(index, path, *kind)?;
        match kind {
            FileModeKind::Symlink => {
                write_symlink(&target, &bytes, path)?;
                snapshot.symlinks += 1;
            }
            FileModeKind::Executable => {
                write_file(&target, &bytes, true, path)?;
                snapshot.files += 1;
            }
            _ => {
                write_file(&target, &bytes, false, path)?;
                snapshot.files += 1;
            }
        }
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialize_with_writes_supplied_contents_and_validates_first() {
        let parent = std::env::temp_dir();
        let entries = vec![
            (GitPath::from_bytes(b"a/b.txt"), FileModeKind::Regular),
            (GitPath::from_bytes(b"run.sh"), FileModeKind::Executable),
        ];
        let snap =
            materialize_with(&entries, &parent, |i, _, _| Ok(format!("{i}").into_bytes())).unwrap();
        assert_eq!(std::fs::read(snap.root().join("a/b.txt")).unwrap(), b"0");
        assert_eq!(std::fs::read(snap.root().join("run.sh")).unwrap(), b"1");
        assert_eq!((snap.files(), snap.symlinks()), (2, 0));
        assert!(!snap.root().join(".git").exists());
        // 검증 실패면 디렉터리를 만들지 않고 read도 호출하지 않습니다.
        let mut calls = 0;
        let bad = vec![
            (GitPath::from_bytes(b"ok.txt"), FileModeKind::Regular),
            (GitPath::from_bytes(b".git/config"), FileModeKind::Regular),
        ];
        assert!(matches!(
            materialize_with(&bad, &parent, |_, _, _| {
                calls += 1;
                Ok(Vec::new())
            }),
            Err(MaterializeError::UnsafePath { .. })
        ));
        let tree = vec![(GitPath::from_bytes(b"dir"), FileModeKind::Tree)];
        assert!(matches!(
            materialize_with(&tree, &parent, |_, _, _| Ok(Vec::new())),
            Err(MaterializeError::UnsupportedMode { .. })
        ));
        assert_eq!(calls, 0);
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        use UnsafePathReason::*;
        let cases: [(&[u8], UnsafePathReason); 14] = [
            (b"", Empty),
            (b"/etc/passwd", Absolute),
            (b"a//b", EmptyComponent),
            (b"a/", EmptyComponent),
            (b"./a", DotComponent),
            (b"a/../../b", DotComponent),
            (b".git/config", DotGit),
            (b"sub/.git", DotGit),
            (b".GIT/hooks/pre-commit", DotGit),
            (b".git./x", DotGit),
            (b".git  /x", DotGit),
            (".g\u{200c}it/x".as_bytes(), DotGit),
            ("\u{feff}.git".as_bytes(), DotGit),
            (b"a\0b", Nul),
        ];
        for (raw, reason) in cases {
            assert_eq!(
                validate_snapshot_path(raw),
                Err(reason),
                "{:?}",
                String::from_utf8_lossy(raw)
            );
        }
        for ok in [
            &b"stellar.yaml"[..],
            b"design/objects/sha256/ab/x.starchunk",
            b".gitignore",
            b".gitattributes",
            b".github/workflows/x",
            b"a/.git-x",
            b"caf\xe9",
        ] {
            assert!(validate_snapshot_path(ok).is_ok(), "{ok:?}");
        }
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::object::FileMode;
        use crate::runner::GitRunner;
        use crate::snapshot::{read_index_snapshot, read_tree_snapshot, SnapshotSource};
        use crate::test_support::{
            git_ok, hermetic_runner, init_repo_with_identity, system_git, write_blob, TempDir,
        };
        use crate::trust::GitTrust;
        use std::os::unix::fs::PermissionsExt;

        /// 결정론적 의사 난수 바이트(xorshift).
        fn pseudo_random(len: usize, mut seed: u64) -> Vec<u8> {
            (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed & 0xff) as u8
                })
                .collect()
        }

        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                out.push(path.clone());
                if entry.file_type().unwrap().is_dir() {
                    walk(&path, out);
                }
            }
        }

        struct Fixture {
            _dir: TempDir,
            repo: PathBuf,
            out: PathBuf,
            runner: GitRunner,
            files: Vec<(&'static str, Vec<u8>)>,
        }

        fn fixture() -> Option<Fixture> {
            let git = system_git()?;
            let dir = TempDir::new("materialize");
            let repo = dir.path().join("repo");
            let out = dir.path().join("out");
            std::fs::create_dir_all(&repo).unwrap();
            std::fs::create_dir_all(&out).unwrap();
            let runner = hermetic_runner(git);
            init_repo_with_identity(&runner, &repo);
            let all_bytes: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
            let files: Vec<(&'static str, Vec<u8>)> = vec![
                ("stellar.yaml", b"schema_version: 1\n".to_vec()),
                (
                    "design/objects/sha256/ab/abcdef.starchunk",
                    pseudo_random(1 << 20, 0x5eed),
                ),
                ("design/objects/sha256/00/zeros.starchunk", vec![0u8; 8192]),
                ("assets/all-bytes.bin", all_bytes),
                ("assets/empty.bin", Vec::new()),
                ("design/crlf.txt", b"a\r\nb\r\n\r\n".to_vec()),
            ];
            for (p, bytes) in &files {
                let path = repo.join(p);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, bytes).unwrap();
            }
            std::fs::write(repo.join("tool.sh"), b"#!/bin/sh\n").unwrap();
            std::fs::set_permissions(repo.join("tool.sh"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
            std::os::unix::fs::symlink("design/objects", repo.join("objects-link")).unwrap();
            std::os::unix::fs::symlink("../../outside/target 한글", repo.join("assets/escape"))
                .unwrap();
            git_ok(&runner, &repo, ["add", "-A"]);
            git_ok(&runner, &repo, ["commit", "-q", "-m", "fixture"]);
            Some(Fixture {
                _dir: dir,
                repo,
                out,
                runner,
                files,
            })
        }

        fn assert_matches_fixture(fx: &Fixture, root: &Path) {
            for (p, bytes) in &fx.files {
                assert_eq!(&std::fs::read(root.join(p)).unwrap(), bytes, "{p}");
            }
            assert_eq!(
                std::fs::read_link(root.join("objects-link")).unwrap(),
                PathBuf::from("design/objects")
            );
            assert_eq!(
                std::fs::read_link(root.join("assets/escape")).unwrap(),
                PathBuf::from("../../outside/target 한글")
            );
            let mode = std::fs::metadata(root.join("tool.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
            let mode = std::fs::metadata(root.join("stellar.yaml"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o644);
            // `.git`은 어디에도 없음.
            let mut all = Vec::new();
            walk(root, &mut all);
            assert!(
                !all.iter().any(|p| p
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case(".git"))),
                "{all:?}"
            );
        }

        #[test]
        fn binary_chunk_and_symlink_bytes_are_identical_from_head_and_index() {
            let Some(fx) = fixture() else {
                return; // git 미설치 환경 건너뜀.
            };
            let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
            let head = read_tree_snapshot(&gate, &fx.repo, &SnapshotSource::Head).unwrap();
            let snap = materialize_snapshot(&gate, &fx.repo, &head.entries, &fx.out).unwrap();
            assert_eq!(snap.files(), 7);
            assert_eq!(snap.symlinks(), 2);
            assert!(snap.skipped_gitlinks().is_empty());
            assert_matches_fixture(&fx, snap.root());
            let root_mode = std::fs::metadata(snap.root()).unwrap().permissions().mode();
            assert_eq!(root_mode & 0o777, 0o700);
            // symlink를 따라가 snapshot 밖에 쓰지 않음.
            assert!(!fx.out.parent().unwrap().join("outside").exists());

            // index snapshot도 같은 bytes.
            let index = read_index_snapshot(&gate, &fx.repo).unwrap();
            let snap2 = materialize_snapshot(&gate, &fx.repo, &index.entries, &fx.out).unwrap();
            assert_matches_fixture(&fx, snap2.root());
            assert_ne!(snap.root(), snap2.root());

            // drop하면 임시 디렉터리 삭제, keep이면 보존.
            let root = snap.root().to_path_buf();
            drop(snap);
            assert!(!root.exists());
            let kept = snap2.keep();
            assert!(kept.join("stellar.yaml").exists());
            std::fs::remove_dir_all(&kept).unwrap();
        }

        #[test]
        fn gitlinks_are_skipped_and_recorded() {
            let Some(fx) = fixture() else {
                return;
            };
            let head = git_ok(&fx.runner, &fx.repo, ["rev-parse", "HEAD"]);
            let head = String::from_utf8(head.stdout).unwrap().trim().to_string();
            let cacheinfo = format!("160000,{head},libs/sub");
            git_ok(
                &fx.runner,
                &fx.repo,
                ["update-index", "--add", "--cacheinfo", cacheinfo.as_str()],
            );
            let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
            let index = read_index_snapshot(&gate, &fx.repo).unwrap();
            let snap = materialize_snapshot(&gate, &fx.repo, &index.entries, &fx.out).unwrap();
            assert_eq!(snap.skipped_gitlinks(), &[GitPath::from_bytes(b"libs/sub")]);
            assert!(!snap.root().join("libs/sub").exists());
        }

        #[test]
        fn unsafe_entries_spawn_nothing_and_create_nothing() {
            let Some(fx) = fixture() else {
                return;
            };
            let runner = hermetic_runner(system_git().unwrap());
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let oid = ObjectId::parse(&write_blob(&fx.runner, &fx.repo, b"evil\n")).unwrap();
            for bad in [&b".git/hooks/post-checkout"[..], b"x/../../escape", b"/abs"] {
                let entries = vec![
                    SnapshotEntry {
                        path: GitPath::from_bytes(b"ok.txt"),
                        mode: FileMode::REGULAR,
                        oid: oid.clone(),
                    },
                    SnapshotEntry {
                        path: GitPath::from_bytes(bad),
                        mode: FileMode::REGULAR,
                        oid: oid.clone(),
                    },
                ];
                assert!(matches!(
                    materialize_snapshot(&gate, &fx.repo, &entries, &fx.out),
                    Err(MaterializeError::UnsafePath { .. })
                ));
            }
            assert_eq!(runner.spawn_count(), 0);
            assert_eq!(std::fs::read_dir(&fx.out).unwrap().count(), 0);
        }

        #[test]
        fn collisions_and_symlink_parents_do_not_escape() {
            let Some(fx) = fixture() else {
                return;
            };
            let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
            let blob = ObjectId::parse(&write_blob(&fx.runner, &fx.repo, b"x\n")).unwrap();
            let outside = fx.out.parent().unwrap().join("outside-dir");
            std::fs::create_dir_all(&outside).unwrap();
            let link_target = outside.to_str().unwrap().as_bytes().to_vec();
            let link_blob =
                ObjectId::parse(&write_blob(&fx.runner, &fx.repo, &link_target)).unwrap();
            let entry = |p: &[u8], mode: FileMode, oid: &ObjectId| SnapshotEntry {
                path: GitPath::from_bytes(p),
                mode,
                oid: oid.clone(),
            };
            // symlink 상위 아래에 쓰기 → 충돌(밖에 쓰지 않음).
            let escape = vec![
                entry(b"link", FileMode::SYMLINK, &link_blob),
                entry(b"link/evil.txt", FileMode::REGULAR, &blob),
            ];
            assert!(matches!(
                materialize_snapshot(&gate, &fx.repo, &escape, &fx.out),
                Err(MaterializeError::PathCollision { .. })
            ));
            assert!(!outside.join("evil.txt").exists());
            // 파일/디렉터리 충돌.
            let df = vec![
                entry(b"x", FileMode::REGULAR, &blob),
                entry(b"x/y", FileMode::REGULAR, &blob),
            ];
            assert!(matches!(
                materialize_snapshot(&gate, &fx.repo, &df, &fx.out),
                Err(MaterializeError::PathCollision { .. })
            ));
            // 같은 경로 두 번 → 덮어쓰지 않고 충돌.
            let dup = vec![
                entry(b"same.txt", FileMode::REGULAR, &blob),
                entry(b"same.txt", FileMode::REGULAR, &link_blob),
            ];
            assert!(matches!(
                materialize_snapshot(&gate, &fx.repo, &dup, &fx.out),
                Err(MaterializeError::PathCollision { .. })
            ));
            // 실패한 시도는 임시 디렉터리를 남기지 않음.
            assert_eq!(std::fs::read_dir(&fx.out).unwrap().count(), 0);
            // 없는 blob.
            let missing = ObjectId::parse(&"7".repeat(40)).unwrap();
            assert!(matches!(
                materialize_snapshot(
                    &gate,
                    &fx.repo,
                    &[entry(b"m.txt", FileMode::REGULAR, &missing)],
                    &fx.out
                ),
                Err(MaterializeError::Blob(BlobError::Missing(_)))
            ));
            assert_eq!(std::fs::read_dir(&fx.out).unwrap().count(), 0);
        }
    }
}
