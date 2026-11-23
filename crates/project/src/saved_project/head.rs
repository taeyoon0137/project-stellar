//! 신뢰 작업 트리 HEAD의 현재 저장 세대만 private snapshot으로 엽니다.
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use stellar_document::component::definition::ComponentLibrary;
use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
use stellar_git_porcelain::location::worktree_root;
use stellar_git_porcelain::object::{FileModeKind, ObjectId};
use stellar_git_porcelain::runner::GitRunner;
use stellar_git_porcelain::snapshot::tree::{head_commit, read_tree_entries};
use stellar_git_porcelain::snapshot::{
    materialize_snapshot, read_blobs_with_limits, resolve_tree, BlobLimits, MaterializedSnapshot,
    ResolvedTree, SnapshotEntry, SnapshotSource,
};
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_storage::chunk::path::ChunkObjectPath;
use stellar_storage::generation::current::{current_pointer_path, read_current_pointer};
use stellar_storage::generation::loader::manifest_path;
use stellar_storage::generation::manifest::GenerationManifest;
use stellar_storage::hash::ContentHash;

use super::{
    parse_manifest_bytes, SavedProject, SavedProjectLoader, StableId, StorageReadLimits,
    MANIFEST_FILE, MAX_INPUT_BYTES,
};
use crate::capability::git_trust;
use crate::document_io::component_source::{
    library_from_blobs, ComponentFiles, EXT, MAX_COMPONENT_FILES, MAX_PROJECT_SOURCE_BYTES,
};
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
        let location = worktree_root(&git, root).map_err(|_| HeadUnavailable)?;
        let root = root.canonicalize().map_err(|_| HeadUnavailable)?;
        if location != root {
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
        Ok(head_commit(&self.git, &self.root)
            .map_err(|_| HeadUnavailable)?
            .as_ref()
            == Some(&self.commit))
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

    fn component_library(
        &self,
        entries: &[SnapshotEntry],
    ) -> Result<(ComponentLibrary, ComponentFiles), HeadUnavailable> {
        let sources = component_entries(entries)?;
        if sources.is_empty() {
            return Ok((ComponentLibrary::new(), ComponentFiles::new()));
        }
        let ids: Vec<_> = sources.iter().map(|entry| entry.oid.clone()).collect();
        let mut blobs = read_blobs_with_limits(
            &self.git,
            &self.root,
            &ids,
            &BlobLimits {
                max_blob_bytes: MAX_INPUT_BYTES as u64,
                max_batch_bytes: 4 * 1024 * 1024,
                max_total_bytes: MAX_PROJECT_SOURCE_BYTES,
            },
        )
        .map_err(|_| HeadUnavailable)?;
        let mut files = Vec::with_capacity(sources.len());
        for entry in sources {
            // 두 경로가 같은 bytes면 파일명·StableId 계약상 적어도 하나는 잘못되었습니다.
            let bytes = blobs.remove(&entry.oid).ok_or(HeadUnavailable)?;
            files.push((entry.path.to_path_buf().ok_or(HeadUnavailable)?, bytes));
        }
        library_from_blobs(files).map_err(|_| HeadUnavailable)
    }

    /// 현재 세대와 프로젝트 공유 컴포넌트 파일을 고정한 tree에서 읽습니다.
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
        let (library, component_files) = self.component_library(&entries)?;
        // HEAD tree에는 Git 원본(텍스트 원본)만 있고 chunk cache는 없습니다(ADR-0122). 원본이 있으면 원본 파일만
        // materialize해 그 안에서 cache를 만들고, 옛 commit(옛 배치 `design/current`만 있음)은 옛 배치 파일을
        // materialize해 cache 위치로 옮겨 엽니다.
        let document = format!(
            "{}/{}",
            crate::document_io::session::DESIGN_DIR,
            crate::document_io::source_v4::project::DOCUMENT_PATH
        );
        let mut selected = vec![manifest.clone()];
        if regular(Path::new(&document)).is_ok() {
            for entry in &entries {
                let path = entry.path.as_bytes();
                if path.starts_with(b"design/")
                    && path.ends_with(b".stellar.yaml")
                    && matches!(
                        entry.kind(),
                        Some(FileModeKind::Regular | FileModeKind::Executable)
                    )
                {
                    selected.push(entry.clone());
                }
            }
        } else {
            let legacy = |relative: &Path| {
                stellar_storage::location::legacy_relative_path(relative).ok_or(HeadUnavailable)
            };
            let pointer = regular(&legacy(&current_pointer_path(Path::new("")))?)?;
            let limits = StorageReadLimits::default();
            let bytes = self.read_blob(pointer, limits.current_pointer_bytes)?;
            let hash: ContentHash = std::str::from_utf8(&bytes)
                .ok()
                .and_then(|s| s.strip_suffix('\n'))
                .and_then(|s| s.parse().ok())
                .ok_or(HeadUnavailable)?;
            let generation = regular(&legacy(&manifest_path(Path::new(""), &hash))?)?;
            let bytes = self.read_blob(generation, limits.manifest_bytes)?;
            if ContentHash::compute(&bytes) != hash {
                return Err(HeadUnavailable);
            }
            let decoded = GenerationManifest::decode(&bytes).map_err(|_| HeadUnavailable)?;
            if decoded.project_id != project {
                return Err(HeadUnavailable);
            }
            if !library.is_empty()
                && decoded
                    .chunk_pointers
                    .iter()
                    .any(|p| p.kind == stellar_storage::chunk::kinds::KIND_COMPONENT_LIBRARY)
            {
                return Err(HeadUnavailable);
            }
            selected.push(pointer.clone());
            selected.push(generation.clone());
            let unique: BTreeSet<_> = decoded
                .chunk_pointers
                .iter()
                .map(|p| p.content_hash)
                .collect();
            for hash in unique {
                selected.push(
                    regular(&legacy(
                        &ChunkObjectPath::for_hash(&hash).to_relative_path(),
                    )?)?
                    .clone(),
                );
            }
        }
        let parent = std::env::temp_dir()
            .canonicalize()
            .map_err(|_| HeadUnavailable)?;
        if parent.starts_with(&self.root) {
            return Err(HeadUnavailable);
        }
        let files = materialize_snapshot(&self.git, &self.root, &selected, &parent)
            .map_err(|_| HeadUnavailable)?;
        crate::cache::prepare_snapshot(files.root()).map_err(|_| HeadUnavailable)?;
        let saved = SavedProjectLoader::for_materialized_snapshot()
            .open(files.root())
            .map_err(|_| HeadUnavailable)?;
        if saved.project_id != project {
            return Err(HeadUnavailable);
        }
        let manifest_hash = read_current_pointer(files.root()).map_err(|_| HeadUnavailable)?;
        Ok(HeadGeneration {
            manifest_hash,
            component_files,
            library,
            saved,
            _files: files,
        })
    }
}

fn component_entries(entries: &[SnapshotEntry]) -> Result<Vec<&SnapshotEntry>, HeadUnavailable> {
    let mut sources = Vec::new();
    for entry in entries {
        let path = entry.path.as_bytes();
        if path == b"design" || path == b"design/components" {
            // ls-tree -r은 디렉터리 자체를 나열하지 않습니다. 링크·파일·submodule을
            // 빈 라이브러리로 취급하거나 실제 작업 트리에서 따라가지 않습니다.
            return Err(HeadUnavailable);
        }
        let Some(name) = path.strip_prefix(b"design/components/") else {
            continue;
        };
        let mut segments = name.split(|byte| *byte == b'/');
        if !segments
            .next()
            .is_some_and(|direct| direct.ends_with(EXT.as_bytes()))
        {
            continue;
        }
        if segments.next().is_some() {
            return Err(HeadUnavailable);
        }
        if sources.len() == MAX_COMPONENT_FILES
            || !matches!(
                entry.kind(),
                Some(FileModeKind::Regular | FileModeKind::Executable)
            )
        {
            return Err(HeadUnavailable);
        }
        sources.push(entry);
    }
    Ok(sources)
}

/// 참조할 chunk가 남아 있는 동안 임시 디렉터리를 보존합니다.
pub struct HeadGeneration {
    /// 고정 Git commit의 검증된 저장 세대 manifest hash.
    pub manifest_hash: ContentHash,
    /// 검증한 HEAD 컴포넌트별 경로와 bytes hash.
    pub component_files: ComponentFiles,
    library: ComponentLibrary,
    saved: SavedProject,
    _files: MaterializedSnapshot,
}

impl HeadGeneration {
    /// 고정한 HEAD tree에서 검증한 프로젝트 공용 정의.
    pub fn components(&self) -> &ComponentLibrary {
        &self.library
    }

    /// 검증된 저장 세대.
    pub fn project(&self) -> &SavedProject {
        &self.saved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_git_porcelain::{decode::GitPath, object::FileMode};

    #[test]
    fn component_paths_are_bounded_before_blob_reads() {
        let entry = |path: &str| SnapshotEntry {
            path: GitPath::from_bytes(path.as_bytes()),
            mode: FileMode::REGULAR,
            oid: ObjectId::parse("1111111111111111111111111111111111111111").unwrap(),
        };
        let mut files = vec![
            entry("design/components/not-a-source.txt"),
            entry("design/components/examples/other.stellar.yaml"),
        ];
        for i in 0..MAX_COMPONENT_FILES {
            files.push(entry(&format!("design/components/{i}.stellar.yaml")));
        }
        assert_eq!(
            component_entries(&files).unwrap().len(),
            MAX_COMPONENT_FILES
        );
        files.push(entry("design/components/extra.stellar.yaml"));
        assert!(component_entries(&files).is_err());
        assert!(
            component_entries(&[entry("design/components/invalid.stellar.yaml/inside")]).is_err()
        );
    }
}
