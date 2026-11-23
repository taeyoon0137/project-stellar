//! 정상 세대 fallback scanner (V.4.5).
//!
//! current generation이 손상되면 보존된 manifest 목록(`design/generations/`)에서
//! **완전히 검증되는**(manifest hash·구조·project id·chunk 존재) 세대 중 generation
//! number가 가장 큰 것을 찾습니다. parent chain은 manifest의 parent generation
//! number로 이어지며, 손상 세대는 건너뛰고 다음 후보로 내려갑니다.
//!
//! scanner는 읽기만 합니다 — current pointer 변경·복구는 V.4.6에서 다룹니다.

use std::fmt;
use std::path::{Path, PathBuf};

use stellar_core_types::ids::StableId;

use crate::hash::ContentHash;

use super::loader::{verify_generation, LoadedGeneration};

/// scanner 오류(디렉터리 접근 실패).
#[derive(Debug)]
pub struct ScanError {
    /// 실패한 경로.
    pub path: PathBuf,
    /// 원인.
    pub source: std::io::Error,
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "generation 디렉터리 읽기 실패 ({}): {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for ScanError {}

/// manifest 파일 이름(`<algo>-<hex>.starmanifest`)에서 hash를 복원합니다.
/// 형식이 아니면 `None`(무시).
fn hash_from_file_name(name: &str) -> Option<ContentHash> {
    let stem = name.strip_suffix(".starmanifest")?;
    let (algo, hex) = stem.split_once('-')?;
    format!("{algo}:{hex}").parse().ok()
}

/// 보존된 manifest 목록에서 최신 정상 세대를 찾습니다.
///
/// 반환: 완전히 검증된 세대 중 generation number 최대인 것. 정상 세대가 하나도
/// 없으면 `None`. generation number가 같은 후보가 여럿이면 manifest hash 사전식
/// 순서로 결정론적으로 선택합니다.
pub fn scan_latest_valid_generation(
    root: &Path,
    expected_project_id: StableId,
) -> Result<Option<LoadedGeneration>, ScanError> {
    let dir = crate::location::cache_dir(root).join(crate::location::GENERATIONS_DIR);
    if !dir.is_dir() {
        return Ok(None);
    }
    let entries = std::fs::read_dir(&dir).map_err(|source| ScanError {
        path: dir.clone(),
        source,
    })?;

    // 결정론을 위해 후보 hash를 정렬해 순회합니다.
    let mut candidates: Vec<ContentHash> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ScanError {
            path: dir.clone(),
            source,
        })?;
        let name = entry.file_name();
        if let Some(hash) = hash_from_file_name(&name.to_string_lossy()) {
            candidates.push(hash);
        }
    }
    candidates.sort();

    let mut best: Option<LoadedGeneration> = None;
    for hash in candidates {
        // 검증 실패(손상·누락 chunk·잘못된 project id)는 후보에서 제외할 뿐
        // 오류가 아닙니다.
        let Ok(loaded) = verify_generation(root, &hash, expected_project_id) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some(current) => loaded.manifest.generation > current.manifest.generation,
        };
        if better {
            best = Some(loaded);
        }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::pointer::ChunkPointer;
    use crate::chunk::store::{ChunkStore, WriteOutcome};
    use crate::generation::loader::{manifest_path, write_manifest_file};
    use crate::generation::manifest::GenerationManifest;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-scanner-test-{}-{n}", std::process::id()));
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

    /// generation n의 manifest를 만들어 기록하고 hash를 반환합니다.
    fn write_generation(dir: &TempDir, project_id: StableId, n: u64) -> ContentHash {
        let store = ChunkStore::new(&dir.path);
        let payload = format!("gen-{n}-chunk").into_bytes();
        let outcome = store.write_chunk(KIND, &payload).unwrap();
        let (WriteOutcome::Written { hash, .. } | WriteOutcome::Reused { hash, .. }) = outcome;
        let manifest = GenerationManifest {
            generation: n,
            parent_generation: (n > 1).then(|| n - 1),
            project_id,
            schema_version: 1,
            chunk_pointers: vec![ChunkPointer {
                chunk_id: ChunkId::from_stable_id(stable(1_000)),
                content_hash: hash,
                kind: KIND,
                schema_version: 1,
            }],
            root_document_ids: vec![stable(7_000)],
        };
        write_manifest_file(&dir.path, &manifest).unwrap()
    }

    #[test]
    fn latest_generation_is_selected_when_all_valid() {
        let dir = TempDir::new();
        let project_id = stable(42);
        for n in 1..=3 {
            write_generation(&dir, project_id, n);
        }
        let best = scan_latest_valid_generation(&dir.path, project_id)
            .unwrap()
            .unwrap();
        assert_eq!(best.manifest.generation, 3);
    }

    #[test]
    fn two_newest_corrupted_falls_back_to_third() {
        // 검증 기준(V.4.5): 최신 2세대 손상 fixture에서 세 번째 정상 세대 선택.
        let dir = TempDir::new();
        let project_id = stable(42);
        let mut hashes = Vec::new();
        for n in 1..=4 {
            hashes.push(write_generation(&dir, project_id, n));
        }
        // generation 4·3 manifest를 손상.
        for hash in &hashes[2..] {
            let path = manifest_path(&dir.path, hash);
            let mut bytes = std::fs::read(&path).unwrap();
            bytes[15] ^= 0x40;
            std::fs::write(&path, &bytes).unwrap();
        }
        let best = scan_latest_valid_generation(&dir.path, project_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            best.manifest.generation, 2,
            "세 번째(뒤에서) 정상 세대 선택"
        );
    }

    #[test]
    fn generation_with_missing_chunk_is_skipped() {
        let dir = TempDir::new();
        let project_id = stable(42);
        write_generation(&dir, project_id, 1);
        let gen2 = write_generation(&dir, project_id, 2);
        // generation 2의 chunk 파일 제거 → 정상 세대 아님.
        let manifest = crate::generation::loader::read_manifest_file(&dir.path, &gen2).unwrap();
        let chunk_path =
            crate::chunk::path::ChunkObjectPath::for_hash(&manifest.chunk_pointers[0].content_hash);
        std::fs::remove_file(dir.path.join(chunk_path.to_relative_path())).unwrap();

        let best = scan_latest_valid_generation(&dir.path, project_id)
            .unwrap()
            .unwrap();
        assert_eq!(best.manifest.generation, 1);
    }

    #[test]
    fn no_valid_generation_returns_none() {
        let dir = TempDir::new();
        assert!(scan_latest_valid_generation(&dir.path, stable(42))
            .unwrap()
            .is_none());
        // 다른 프로젝트의 세대만 있으면 역시 None.
        write_generation(&dir, stable(1), 1);
        assert!(scan_latest_valid_generation(&dir.path, stable(42))
            .unwrap()
            .is_none());
    }

    #[test]
    fn unrelated_files_are_ignored() {
        let dir = TempDir::new();
        let project_id = stable(42);
        write_generation(&dir, project_id, 1);
        let gen_dir = crate::location::cache_dir(&dir.path).join(crate::location::GENERATIONS_DIR);
        std::fs::write(gen_dir.join("README.txt"), b"not a manifest").unwrap();
        std::fs::write(gen_dir.join("bad-name.starmanifest"), b"junk").unwrap();
        let best = scan_latest_valid_generation(&dir.path, project_id)
            .unwrap()
            .unwrap();
        assert_eq!(best.manifest.generation, 1);
    }
}
