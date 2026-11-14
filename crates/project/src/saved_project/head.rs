//! 신뢰 작업 트리 HEAD의 현재 저장 세대만 private snapshot으로 엽니다.
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
use stellar_git_porcelain::location::locate_repository;
use stellar_git_porcelain::object::{FileModeKind, ObjectId};
use stellar_git_porcelain::runner::GitRunner;
use stellar_git_porcelain::snapshot::tree::read_tree_entries;
use stellar_git_porcelain::snapshot::{
    materialize_snapshot, read_blobs_with_limits, resolve_tree, BlobLimits, MaterializedSnapshot,
    ResolvedTree, SnapshotEntry, SnapshotSource,
};
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_storage::chunk::path::ChunkObjectPath;
use stellar_storage::generation::loader::manifest_path;
use stellar_storage::generation::manifest::GenerationManifest;
use stellar_storage::hash::ContentHash;

use super::{
    parse_manifest_bytes, SavedProject, SavedProjectLoader, StableId, StorageReadLimits,
    MANIFEST_FILE, MAX_INPUT_BYTES,
};
use crate::capability::git_trust;
use crate::trust::TrustStatus;

/// 현재 commit 또는 그 안의 검증된 저장 세대를 읽을 수 없음.
#[derive(Debug)]
pub struct HeadUnavailable;

/// 작업 트리와 commit/tree를 묶은 읽기 핸들. ref·경로를 client가 주입하지 않습니다.
pub struct ProjectHead {
    git: GatedGitRunner,
    root: PathBuf,
    commit: ObjectId,
    tree: ObjectId,
}

impl ProjectHead {
    /// 주어진 root가 Git 작업 트리 최상위이고 HEAD가 commit을 가리키는지 확인합니다.
    pub fn open(root: &Path, trust: TrustStatus) -> Result<Self, HeadUnavailable> {
        let executable = discover_git(&GitDiscoveryConfig::default()).ok_or(HeadUnavailable)?;
        let runner = GitRunner::new(executable, Vec::new(), Duration::from_secs(10))
            .with_env_overrides(vec![
                ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
                ("GIT_NO_LAZY_FETCH".into(), "1".into()),
                ("GIT_ALLOW_PROTOCOL".into(), "".into()),
                ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
            ]);
        let git = GatedGitRunner::new(runner, git_trust(trust));
        let location = locate_repository(&git, root).map_err(|_| HeadUnavailable)?;
        let root = root.canonicalize().map_err(|_| HeadUnavailable)?;
        if location.worktree_root != root {
            return Err(HeadUnavailable);
        }
        let ResolvedTree::Tree {
            commit: Some(commit),
            tree,
        } = resolve_tree(&git, &root, &SnapshotSource::Head).map_err(|_| HeadUnavailable)?
        else {
            return Err(HeadUnavailable);
        };
        Ok(Self {
            git,
            root,
            commit,
            tree,
        })
    }

    /// Git에서 확인한 작업 트리의 실제 경로.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 고정한 commit ID.
    pub fn commit(&self) -> &str {
        self.commit.as_str()
    }

    /// HEAD가 여전히 같은 commit/tree를 가리키는지 검사합니다.
    pub fn is_current(&self) -> Result<bool, HeadUnavailable> {
        Ok(
            matches!(resolve_tree(&self.git, &self.root, &SnapshotSource::Head).map_err(|_| HeadUnavailable)?,
            ResolvedTree::Tree { commit: Some(commit), tree } if commit == self.commit && tree == self.tree),
        )
    }

    fn read_blob(&self, entry: &SnapshotEntry, max_bytes: u64) -> Result<Vec<u8>, HeadUnavailable> {
        let mut blobs = read_blobs_with_limits(
            &self.git,
            &self.root,
            std::slice::from_ref(&entry.oid),
            &BlobLimits {
                max_blob_bytes: max_bytes,
                ..BlobLimits::IN_MEMORY
            },
        )
        .map_err(|_| HeadUnavailable)?;
        blobs.remove(&entry.oid).ok_or(HeadUnavailable)
    }

    /// 현재 세대가 참조하는 chunk만 복원합니다. 과거 세대·일반 파일·에셋은 읽지 않습니다.
    pub fn saved(&self, project: StableId) -> Result<HeadGeneration, HeadUnavailable> {
        let entries =
            read_tree_entries(&self.git, &self.root, &self.tree).map_err(|_| HeadUnavailable)?;
        let files: HashMap<_, _> = entries.iter().map(|e| (e.path.as_bytes(), e)).collect();
        let regular = |path: &Path| {
            files
                .get(path.as_os_str().as_encoded_bytes())
                .copied()
                .filter(|entry| {
                    matches!(
                        entry.kind(),
                        Some(FileModeKind::Regular | FileModeKind::Executable)
                    )
                })
                .ok_or(HeadUnavailable)
        };
        let manifest = regular(Path::new(MANIFEST_FILE))?;
        let header = parse_manifest_bytes(self.read_blob(manifest, MAX_INPUT_BYTES as u64)?)
            .map_err(|_| HeadUnavailable)?;
        if header.project.id.parse::<StableId>().ok() != Some(project) {
            return Err(HeadUnavailable);
        }
        let pointer = regular(Path::new("design/current"))?;
        let limits = StorageReadLimits::default();
        let bytes = self.read_blob(pointer, limits.current_pointer_bytes)?;
        let hash: ContentHash = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|s| s.strip_suffix('\n'))
            .and_then(|s| s.parse().ok())
            .ok_or(HeadUnavailable)?;
        let generation = regular(&manifest_path(Path::new(""), &hash))?;
        let bytes = self.read_blob(generation, limits.manifest_bytes)?;
        if ContentHash::compute(&bytes) != hash {
            return Err(HeadUnavailable);
        }
        let decoded = GenerationManifest::decode(&bytes).map_err(|_| HeadUnavailable)?;
        if decoded.project_id != project {
            return Err(HeadUnavailable);
        }
        let mut selected = vec![manifest.clone(), pointer.clone(), generation.clone()];
        let unique: BTreeSet<_> = decoded
            .chunk_pointers
            .iter()
            .map(|p| p.content_hash)
            .collect();
        for hash in unique {
            selected.push(regular(&ChunkObjectPath::for_hash(&hash).to_relative_path())?.clone());
        }
        let parent = std::env::temp_dir()
            .canonicalize()
            .map_err(|_| HeadUnavailable)?;
        if parent.starts_with(&self.root) {
            return Err(HeadUnavailable);
        }
        let files = materialize_snapshot(&self.git, &self.root, &selected, &parent)
            .map_err(|_| HeadUnavailable)?;
        let saved = SavedProjectLoader::for_materialized_snapshot()
            .open(files.root())
            .map_err(|_| HeadUnavailable)?;
        if saved.project_id != project {
            return Err(HeadUnavailable);
        }
        Ok(HeadGeneration {
            saved,
            _files: files,
        })
    }
}

/// 참조할 chunk가 남아 있는 동안 임시 디렉터리를 보존합니다.
pub struct HeadGeneration {
    saved: SavedProject,
    _files: MaterializedSnapshot,
}

impl HeadGeneration {
    /// 검증된 저장 세대.
    pub fn project(&self) -> &SavedProject {
        &self.saved
    }
}
