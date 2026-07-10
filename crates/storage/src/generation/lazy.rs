//! 지연 로딩 loader (V.6.6).
//!
//! 프로젝트 열기에서 **DocumentManifest chunk까지만** 먼저 읽고, ViewTree
//! subtree chunk는 **요청 시** 읽습니다. 비활성 view의 chunk 파일에는 읽기
//! I/O가 발생하지 않습니다.
//!
//! 계측: 모든 chunk payload 읽기는 이 모듈의 단일 경로
//! ([`LazyGeneration::read_chunk_payload`])를 지나며, [`ChunkReadRecorder`]가
//! 파일 읽기 직전에 호출됩니다. 테스트는 recorder로 "어떤 chunk 파일이
//! 읽혔는가"를 검증합니다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stellar_core_types::ids::StableId;

use crate::chunk::id::ChunkId;
use crate::chunk::kinds::KIND_DOCUMENT_MANIFEST;
use crate::chunk::pointer::ChunkPointer;
use crate::chunk::store::{ChunkStore, ChunkStoreError};

use super::loader::{load_current_generation, LoadError, LoadedGeneration};

/// chunk 파일 읽기 직전에 호출되는 계측 recorder.
pub trait ChunkReadRecorder {
    /// `pointer`의 chunk 파일을 읽기 직전 호출됩니다.
    fn on_read(&mut self, pointer: &ChunkPointer);
}

/// 계측하지 않는 기본 recorder.
#[derive(Debug, Default)]
pub struct NoopRecorder;

impl ChunkReadRecorder for NoopRecorder {
    fn on_read(&mut self, _pointer: &ChunkPointer) {}
}

/// lazy 로딩 오류.
#[derive(Debug)]
pub enum LazyLoadError {
    /// 열기(구조 검증) 실패.
    Load(LoadError),
    /// chunk 읽기 실패.
    Chunk(ChunkStoreError),
    /// 요청한 ChunkId가 manifest에 없습니다.
    UnknownChunkId {
        /// 요청 id.
        chunk_id: ChunkId,
    },
}

impl std::fmt::Display for LazyLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LazyLoadError::Load(e) => write!(f, "열기 실패: {e}"),
            LazyLoadError::Chunk(e) => write!(f, "chunk 읽기 실패: {e}"),
            LazyLoadError::UnknownChunkId { chunk_id } => {
                write!(f, "manifest에 없는 ChunkId입니다: {chunk_id}")
            }
        }
    }
}

impl std::error::Error for LazyLoadError {}

impl From<LoadError> for LazyLoadError {
    fn from(e: LoadError) -> LazyLoadError {
        LazyLoadError::Load(e)
    }
}

impl From<ChunkStoreError> for LazyLoadError {
    fn from(e: ChunkStoreError) -> LazyLoadError {
        LazyLoadError::Chunk(e)
    }
}

/// 지연 로딩된 generation.
#[derive(Debug)]
pub struct LazyGeneration {
    root: PathBuf,
    loaded: LoadedGeneration,
    by_id: BTreeMap<ChunkId, ChunkPointer>,
    /// 열기 시점에 미리 읽은 DocumentManifest payload들(chunk_id 순).
    document_manifests: Vec<(ChunkId, Vec<u8>)>,
}

impl LazyGeneration {
    /// 프로젝트를 엽니다: 구조 검증(V.4.4, 존재 확인만) 후 **DocumentManifest
    /// kind chunk만** payload를 읽습니다. 다른 kind는 읽지 않습니다.
    pub fn open(
        root: &Path,
        expected_project_id: StableId,
        recorder: &mut dyn ChunkReadRecorder,
    ) -> Result<LazyGeneration, LazyLoadError> {
        let loaded = load_current_generation(root, expected_project_id)?;
        let by_id: BTreeMap<ChunkId, ChunkPointer> = loaded
            .manifest
            .chunk_pointers
            .iter()
            .map(|p| (p.chunk_id, *p))
            .collect();

        let store = ChunkStore::new(root);
        let mut document_manifests = Vec::new();
        for pointer in loaded.manifest.chunk_pointers.iter() {
            if pointer.kind != KIND_DOCUMENT_MANIFEST {
                continue;
            }
            recorder.on_read(pointer);
            let payload = store.read_chunk(&pointer.content_hash, pointer.kind)?;
            document_manifests.push((pointer.chunk_id, payload));
        }

        Ok(LazyGeneration {
            root: root.to_path_buf(),
            loaded,
            by_id,
            document_manifests,
        })
    }

    /// 열기 시점에 읽은 DocumentManifest payload들.
    pub fn document_manifests(&self) -> &[(ChunkId, Vec<u8>)] {
        &self.document_manifests
    }

    /// 열린 manifest.
    pub fn manifest(&self) -> &crate::generation::manifest::GenerationManifest {
        &self.loaded.manifest
    }

    /// 요청 시 chunk payload를 읽습니다(전체 검증, V.2.6).
    ///
    /// 모든 lazy 읽기는 이 경로를 지나므로 recorder가 실제 파일 읽기와 1:1로
    /// 대응합니다.
    pub fn read_chunk_payload(
        &self,
        chunk_id: ChunkId,
        recorder: &mut dyn ChunkReadRecorder,
    ) -> Result<Vec<u8>, LazyLoadError> {
        let pointer = self
            .by_id
            .get(&chunk_id)
            .ok_or(LazyLoadError::UnknownChunkId { chunk_id })?;
        recorder.on_read(pointer);
        let store = ChunkStore::new(&self.root);
        Ok(store.read_chunk(&pointer.content_hash, pointer.kind)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::kinds::KIND_VIEW_TREE;
    use crate::chunk::store::WriteOutcome;
    use crate::generation::current::write_current_pointer;
    use crate::generation::loader::write_manifest_file;
    use crate::generation::manifest::GenerationManifest;
    use crate::hash::ContentHash;
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("stellar-lazy-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    /// 읽힌 chunk 파일(hash)을 기록하는 계측 recorder.
    #[derive(Default)]
    struct Recorder {
        read_hashes: BTreeSet<ContentHash>,
    }
    impl ChunkReadRecorder for Recorder {
        fn on_read(&mut self, pointer: &ChunkPointer) {
            self.read_hashes.insert(pointer.content_hash);
        }
    }

    /// DocumentManifest 1개 + ViewTree 3개로 구성된 프로젝트 fixture.
    /// (doc pointer, view pointers) 반환.
    fn project(dir: &TempDir, project_id: StableId) -> (ChunkPointer, Vec<ChunkPointer>) {
        let store = ChunkStore::new(&dir.path);
        let mut pointers = Vec::new();

        let WriteOutcome::Written { hash, .. } = store
            .write_chunk(KIND_DOCUMENT_MANIFEST, b"document-manifest-payload")
            .unwrap()
        else {
            panic!("Written이어야 함");
        };
        let doc_pointer = ChunkPointer {
            chunk_id: ChunkId::from_stable_id(stable(100)),
            content_hash: hash,
            kind: KIND_DOCUMENT_MANIFEST,
            schema_version: 1,
        };
        pointers.push(doc_pointer);

        let mut view_pointers = Vec::new();
        for i in 0..3u64 {
            let payload = format!("view-subtree-{i}").into_bytes();
            let WriteOutcome::Written { hash, .. } =
                store.write_chunk(KIND_VIEW_TREE, &payload).unwrap()
            else {
                panic!("Written이어야 함");
            };
            let pointer = ChunkPointer {
                chunk_id: ChunkId::from_stable_id(stable(1_000 + i)),
                content_hash: hash,
                kind: KIND_VIEW_TREE,
                schema_version: 1,
            };
            view_pointers.push(pointer);
            pointers.push(pointer);
        }

        let manifest = GenerationManifest {
            generation: 1,
            parent_generation: None,
            project_id,
            schema_version: 1,
            chunk_pointers: pointers,
            root_document_ids: vec![stable(100)],
        };
        let manifest_hash = write_manifest_file(&dir.path, &manifest).unwrap();
        write_current_pointer(&dir.path, &manifest_hash).unwrap();
        (doc_pointer, view_pointers)
    }

    #[test]
    fn open_reads_only_document_manifest() {
        // 검증 기준(V.6.6): 비활성 view chunk에 read가 발생하지 않는 계측 테스트.
        let dir = TempDir::new();
        let project_id = stable(42);
        let (doc, views) = project(&dir, project_id);

        let mut recorder = Recorder::default();
        let lazy = LazyGeneration::open(&dir.path, project_id, &mut recorder).unwrap();

        // 열기에서 읽힌 chunk는 DocumentManifest 하나뿐.
        assert_eq!(recorder.read_hashes.len(), 1);
        assert!(recorder.read_hashes.contains(&doc.content_hash));
        for view in &views {
            assert!(
                !recorder.read_hashes.contains(&view.content_hash),
                "비활성 view chunk가 읽혔습니다"
            );
        }
        assert_eq!(lazy.document_manifests().len(), 1);
        assert_eq!(lazy.document_manifests()[0].1, b"document-manifest-payload");
    }

    #[test]
    fn view_chunk_is_read_only_on_request() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let (doc, views) = project(&dir, project_id);

        let mut recorder = Recorder::default();
        let lazy = LazyGeneration::open(&dir.path, project_id, &mut recorder).unwrap();

        // view 0만 요청.
        let payload = lazy
            .read_chunk_payload(views[0].chunk_id, &mut recorder)
            .unwrap();
        assert_eq!(payload, b"view-subtree-0");

        // 읽힌 파일은 doc + view0 정확히 둘.
        assert_eq!(recorder.read_hashes.len(), 2);
        assert!(recorder.read_hashes.contains(&doc.content_hash));
        assert!(recorder.read_hashes.contains(&views[0].content_hash));
        assert!(!recorder.read_hashes.contains(&views[1].content_hash));
        assert!(!recorder.read_hashes.contains(&views[2].content_hash));
    }

    #[test]
    fn corrupted_inactive_view_does_not_block_open() {
        // 비활성 view chunk의 payload가 손상돼도(존재만 하면) 열기는 성공해야
        // 합니다 — 열기에서 그 파일을 읽지 않는다는 간접 증거.
        let dir = TempDir::new();
        let project_id = stable(42);
        let (_, views) = project(&dir, project_id);
        let victim = crate::chunk::path::ChunkObjectPath::for_hash(&views[2].content_hash);
        let file = dir.path.join(victim.to_relative_path());
        let mut bytes = std::fs::read(&file).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&file, &bytes).unwrap();

        let mut recorder = Recorder::default();
        let lazy = LazyGeneration::open(&dir.path, project_id, &mut recorder).unwrap();
        // 손상된 chunk를 요청하면 그때 검증 오류가 잡습니다.
        assert!(lazy
            .read_chunk_payload(views[2].chunk_id, &mut recorder)
            .is_err());
    }

    #[test]
    fn unknown_chunk_id_is_rejected() {
        let dir = TempDir::new();
        let project_id = stable(42);
        project(&dir, project_id);
        let mut recorder = Recorder::default();
        let lazy = LazyGeneration::open(&dir.path, project_id, &mut recorder).unwrap();
        let err = lazy
            .read_chunk_payload(ChunkId::from_stable_id(stable(9_999)), &mut recorder)
            .unwrap_err();
        assert!(matches!(err, LazyLoadError::UnknownChunkId { .. }));
    }
}
