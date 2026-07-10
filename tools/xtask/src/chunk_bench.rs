//! chunk 목표 크기 측정 harness (V.6.3).
//!
//! 64KiB, 256KiB, 1MiB 후보 각각에 대해 합성 데이터셋으로 저장·로드·Git delta를
//! 측정하고 raw 수치를 `docs/status/chunk-size-benchmark.md`에 기록합니다.
//! **결론은 자동 선택하지 않습니다** — 임계치 확정(V.6.4)의 ADR 입력 자료입니다.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use stellar_core_types::ids::StableId;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::KIND_VIEW_TREE;
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::chunk::store::ChunkStore;
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

/// 측정 후보 chunk 크기(byte).
pub const CANDIDATES: [(usize, &str); 3] = [
    (64 * 1024, "64KiB"),
    (256 * 1024, "256KiB"),
    (1024 * 1024, "1MiB"),
];

/// 합성 데이터셋 크기(8MiB).
const DATASET_LEN: usize = 8 * 1024 * 1024;

/// 한 후보의 측정 결과.
#[derive(Debug, Clone)]
pub struct CandidateResult {
    pub label: &'static str,
    pub chunk_size: usize,
    pub chunk_count: usize,
    pub save_ms: f64,
    pub load_ms: f64,
    /// 단일 지점 수정 후 재저장에서 다시 기록된 bytes.
    pub rewrite_bytes: u64,
    /// 두 번째 commit이 Git object store에 추가한 bytes.
    pub git_delta_bytes: u64,
}

/// 결정적 pseudo-random 데이터셋(LCG).
fn synthetic_dataset() -> Vec<u8> {
    let mut out = Vec::with_capacity(DATASET_LEN);
    let mut state: u64 = 0x5711_ce11_a5d1_2345;
    while out.len() < DATASET_LEN {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(DATASET_LEN);
    out
}

/// 데이터셋을 chunk_size로 잘라 (ChunkId, payload) 목록을 만듭니다.
fn chunk_dataset(data: &[u8], chunk_size: usize) -> Vec<(ChunkId, Vec<u8>)> {
    data.chunks(chunk_size)
        .enumerate()
        .map(|(i, part)| {
            let id = ChunkId::from_stable_id(StableId::new_v7(1_000 + i as u64, 0x0abc, 77));
            (id, part.to_vec())
        })
        .collect()
}

fn dirty_states(chunks: &[(ChunkId, Vec<u8>)]) -> Vec<LogicalChunkState> {
    chunks
        .iter()
        .map(|(id, payload)| LogicalChunkState::Dirty {
            chunk_id: *id,
            kind: KIND_VIEW_TREE,
            schema_version: 1,
            payload: payload.clone(),
        })
        .collect()
}

fn run_git(root: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "bench")
        .env("GIT_AUTHOR_EMAIL", "bench@invalid")
        .env("GIT_COMMITTER_NAME", "bench")
        .env("GIT_COMMITTER_EMAIL", "bench@invalid")
        .output()
        .map_err(|e| format!("git 실행 실패: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {:?} 실패: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// `.git/objects` 아래 파일 크기 합(byte).
fn git_objects_bytes(root: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![root.join(".git").join("objects")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = path.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

/// 한 후보를 측정합니다.
fn measure_candidate(
    work_root: &Path,
    chunk_size: usize,
    label: &'static str,
) -> Result<CandidateResult, String> {
    let root = work_root.join(format!("candidate-{chunk_size}"));
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    run_git(&root, &["init", "-q", "-b", "main"])?;

    let project_id = StableId::new_v7(42, 0x0abc, 77);
    let data = synthetic_dataset();
    let chunks = chunk_dataset(&data, chunk_size);
    let chunk_count = chunks.len();

    // 저장(generation 1) 측정.
    let mut memory = SavedGenerationState::default();
    let plan = build_save_plan(None, project_id, 1, &dirty_states(&chunks), vec![])
        .map_err(|e| e.to_string())?;
    let save_start = Instant::now();
    run_save(&root, &plan, &mut memory, &CancelToken::new()).map_err(|e| e.to_string())?;
    let save_ms = save_start.elapsed().as_secs_f64() * 1_000.0;

    // 첫 commit.
    run_git(&root, &["add", "--all"])?;
    run_git(&root, &["commit", "-q", "-m", "generation 1"])?;
    let objects_before = git_objects_bytes(&root);

    // 로드 측정: open + 모든 chunk 전체 검증 읽기.
    let load_start = Instant::now();
    let loaded = load_current_generation(&root, project_id).map_err(|e| e.to_string())?;
    let store = ChunkStore::new(&root);
    for pointer in &loaded.manifest.chunk_pointers {
        store
            .read_chunk(&pointer.content_hash, pointer.kind)
            .map_err(|e| e.to_string())?;
    }
    let load_ms = load_start.elapsed().as_secs_f64() * 1_000.0;

    // 단일 지점 수정: 데이터셋 중앙 16바이트 → chunk 하나만 변경.
    let mut modified_chunks = chunks;
    let mid_index = modified_chunks.len() / 2;
    for (i, byte) in modified_chunks[mid_index].1.iter_mut().take(16).enumerate() {
        *byte ^= 0xa5 ^ (i as u8);
    }
    let states: Vec<LogicalChunkState> = modified_chunks
        .iter()
        .enumerate()
        .map(|(i, (id, payload))| {
            if i == mid_index {
                LogicalChunkState::Dirty {
                    chunk_id: *id,
                    kind: KIND_VIEW_TREE,
                    schema_version: 1,
                    payload: payload.clone(),
                }
            } else {
                LogicalChunkState::Clean { chunk_id: *id }
            }
        })
        .collect();
    let (base_generation, base_hash) = memory.saved().expect("저장 성공");
    let base =
        stellar_storage::generation::loader::verify_generation(&root, &base_hash, project_id)
            .map_err(|e| e.to_string())?;
    debug_assert_eq!(base.manifest.generation, base_generation);
    let plan2 =
        build_save_plan(Some(&base), project_id, 1, &states, vec![]).map_err(|e| e.to_string())?;
    let rewrite_bytes: u64 = plan2
        .dirty_chunks
        .iter()
        .map(|w| w.payload.len() as u64)
        .sum();
    run_save(&root, &plan2, &mut memory, &CancelToken::new()).map_err(|e| e.to_string())?;

    // 두 번째 commit 후 Git delta.
    run_git(&root, &["add", "--all"])?;
    run_git(&root, &["commit", "-q", "-m", "generation 2"])?;
    let git_delta_bytes = git_objects_bytes(&root).saturating_sub(objects_before);

    Ok(CandidateResult {
        label,
        chunk_size,
        chunk_count,
        save_ms,
        load_ms,
        rewrite_bytes,
        git_delta_bytes,
    })
}

/// raw 결과를 ADR 입력 보고서 markdown으로 렌더링합니다.
pub fn render_report(results: &[CandidateResult]) -> String {
    let mut out = String::new();
    out.push_str("# chunk 목표 크기 측정 (V.6.3)\n\n");
    out.push_str(
        "<!-- 이 파일은 cargo xtask bench-chunk-size 가 생성합니다. 직접 수정하지 마세요. -->\n\n",
    );
    out.push_str("- 데이터셋: 결정적 pseudo-random 8MiB, 단일 지점 수정은 중앙 chunk 16바이트.\n");
    out.push_str("- 측정 환경: DEV-MAC 로컬 1회 실행 — 절대값은 참고용이며 후보 간 상대 비교가 목적입니다.\n");
    out.push_str(
        "- **본 보고서는 결론을 포함하지 않습니다.** 임계치 확정은 ADR(V.6.4)에서 수행합니다.\n\n",
    );
    out.push_str("## raw 결과\n\n");
    out.push_str("| 후보 | chunk 수 | 저장(ms) | 로드(ms) | 재기록 bytes | Git delta bytes |\n");
    out.push_str("|---|---|---|---|---|---|\n");
    for r in results {
        out.push_str(&format!(
            "| {} | {} | {:.1} | {:.1} | {} | {} |\n",
            r.label, r.chunk_count, r.save_ms, r.load_ms, r.rewrite_bytes, r.git_delta_bytes
        ));
    }
    out.push_str("\n## 해석 지침(ADR 입력)\n\n");
    out.push_str("- 저장·로드 시간: 작은 chunk는 파일 수·fsync 횟수가 늘고, 큰 chunk는 단일 변경의 재기록 비용이 늘어납니다.\n");
    out.push_str(
        "- 재기록 bytes: 단일 지점 수정 시 다시 쓰는 payload 총량 — chunk 크기에 비례합니다.\n",
    );
    out.push_str("- Git delta: content-addressed 파일 교체로 인해 두 번째 commit이 object store에 추가한 크기입니다.\n");
    out
}

/// harness 전체를 실행하고 보고서를 기록합니다.
pub fn run_bench(repo_root: &Path) -> Result<PathBuf, String> {
    let work = std::env::temp_dir().join(format!("stellar-chunk-bench-{}", std::process::id()));
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;

    let mut results = Vec::new();
    for (size, label) in CANDIDATES {
        results.push(measure_candidate(&work, size, label)?);
    }
    let _ = std::fs::remove_dir_all(&work);

    let report = render_report(&results);
    let out_path = repo_root
        .join("docs")
        .join("status")
        .join("chunk-size-benchmark.md");
    std::fs::write(&out_path, report).map_err(|e| e.to_string())?;
    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_is_deterministic() {
        assert_eq!(synthetic_dataset(), synthetic_dataset());
        assert_eq!(synthetic_dataset().len(), DATASET_LEN);
    }

    #[test]
    fn chunking_covers_dataset_exactly() {
        let data = synthetic_dataset();
        for (size, _) in CANDIDATES {
            let chunks = chunk_dataset(&data, size);
            let total: usize = chunks.iter().map(|(_, p)| p.len()).sum();
            assert_eq!(total, data.len());
            assert_eq!(chunks.len(), data.len().div_ceil(size));
        }
    }

    #[test]
    fn report_contains_all_candidates_and_no_conclusion() {
        let results: Vec<CandidateResult> = CANDIDATES
            .iter()
            .map(|(size, label)| CandidateResult {
                label,
                chunk_size: *size,
                chunk_count: 1,
                save_ms: 1.0,
                load_ms: 2.0,
                rewrite_bytes: 3,
                git_delta_bytes: 4,
            })
            .collect();
        let report = render_report(&results);
        for (_, label) in CANDIDATES {
            assert!(report.contains(label));
        }
        assert!(report.contains("결론을 포함하지 않습니다"));
    }
}
