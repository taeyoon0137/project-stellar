//! reachable content hash mark scanner (V.7.2).
//!
//! 보존 대상 manifest들(V.7.1 정책 결과)에서 도달 가능한 content hash를
//! 수집합니다. mark 집합은:
//!
//! - 보존 manifest 파일 자신의 hash.
//! - 각 manifest가 가리키는 모든 chunk pointer의 hash.
//!
//! chunk는 content-addressed로 세대 간 공유되므로, 어떤 보존 세대라도 참조하는
//! chunk는 mark됩니다 — 한 generation이 삭제 대상이어도 공유 chunk는 제거되지
//! 않습니다. sweep(V.7.3)은 mark되지 않은 파일만 다룹니다.

use std::collections::BTreeSet;
use std::path::Path;

use crate::generation::loader::{read_manifest_file, LoadError};
use crate::hash::ContentHash;

/// 보존 manifest들에서 도달 가능한 hash 집합을 만듭니다.
///
/// manifest 읽기·검증 실패는 오류입니다 — 보존 대상이 읽히지 않으면 sweep을
/// 진행하면 안 됩니다(과잉 삭제 방지).
pub fn mark_reachable(
    root: &Path,
    retained_manifests: &[ContentHash],
) -> Result<BTreeSet<ContentHash>, LoadError> {
    let mut reachable = BTreeSet::new();
    for manifest_hash in retained_manifests {
        let manifest = read_manifest_file(root, manifest_hash)?;
        reachable.insert(*manifest_hash);
        for pointer in &manifest.chunk_pointers {
            reachable.insert(pointer.content_hash);
        }
    }
    Ok(reachable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::pointer::ChunkPointer;
    use crate::chunk::store::{ChunkStore, WriteOutcome};
    use crate::generation::loader::write_manifest_file;
    use crate::generation::manifest::GenerationManifest;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_core_types::ids::StableId;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("stellar-mark-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    const KIND: ChunkKindCode = ChunkKindCode(0x8000_0001);

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn write_chunk(dir: &TempDir, payload: &[u8]) -> ContentHash {
        let store = ChunkStore::new(&dir.path);
        let (WriteOutcome::Written { hash, .. } | WriteOutcome::Reused { hash, .. }) =
            store.write_chunk(KIND, payload).unwrap();
        hash
    }

    fn write_generation(dir: &TempDir, n: u64, chunk_hashes: &[(u64, ContentHash)]) -> ContentHash {
        let manifest = GenerationManifest {
            generation: n,
            parent_generation: (n > 1).then(|| n - 1),
            project_id: stable(42),
            schema_version: 1,
            chunk_pointers: chunk_hashes
                .iter()
                .map(|(id_ts, hash)| ChunkPointer {
                    chunk_id: ChunkId::from_stable_id(stable(*id_ts)),
                    content_hash: *hash,
                    kind: KIND,
                    schema_version: 1,
                })
                .collect(),
            root_document_ids: vec![],
        };
        write_manifest_file(&dir.path, &manifest).unwrap()
    }

    #[test]
    fn shared_chunk_survives_generation_removal() {
        // 검증 기준(V.7.2): shared chunk가 한 generation 삭제로 제거되지 않음.
        let dir = TempDir::new();
        let shared = write_chunk(&dir, b"shared-across-generations");
        let only_gen1 = write_chunk(&dir, b"only-in-gen-1");
        let only_gen2 = write_chunk(&dir, b"only-in-gen-2");

        let gen1 = write_generation(&dir, 1, &[(100, shared), (101, only_gen1)]);
        let gen2 = write_generation(&dir, 2, &[(100, shared), (102, only_gen2)]);

        // generation 1은 보존 대상에서 제외(삭제 예정), generation 2만 보존.
        let reachable = mark_reachable(&dir.path, &[gen2]).unwrap();

        assert!(reachable.contains(&shared), "공유 chunk는 mark되어야 함");
        assert!(reachable.contains(&only_gen2));
        assert!(reachable.contains(&gen2), "보존 manifest 자신도 mark");
        assert!(
            !reachable.contains(&only_gen1),
            "gen1 전용 chunk는 unmark(회수 후보)"
        );
        assert!(!reachable.contains(&gen1), "미보존 manifest는 unmark");
    }

    #[test]
    fn multiple_retained_manifests_union_their_chunks() {
        let dir = TempDir::new();
        let a = write_chunk(&dir, b"chunk-a");
        let b = write_chunk(&dir, b"chunk-b");
        let gen1 = write_generation(&dir, 1, &[(100, a)]);
        let gen2 = write_generation(&dir, 2, &[(101, b)]);

        let reachable = mark_reachable(&dir.path, &[gen1, gen2]).unwrap();
        assert!(reachable.contains(&a));
        assert!(reachable.contains(&b));
        assert!(reachable.contains(&gen1));
        assert!(reachable.contains(&gen2));
        assert_eq!(reachable.len(), 4);
    }

    #[test]
    fn unreadable_retained_manifest_is_an_error() {
        // 보존 대상 manifest가 없으면 sweep을 진행하면 안 됩니다.
        let dir = TempDir::new();
        let ghost = ContentHash::compute(b"ghost-manifest");
        assert!(mark_reachable(&dir.path, &[ghost]).is_err());
    }

    #[test]
    fn empty_retained_set_marks_nothing() {
        let dir = TempDir::new();
        assert!(mark_reachable(&dir.path, &[]).unwrap().is_empty());
    }
}
