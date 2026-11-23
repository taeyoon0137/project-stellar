//! migration × Git working tree 통합 fixture (V.8.5).
//!
//! migration 결과는 자동으로 Git commit되지 않습니다. chunk cache는 Git 밖
//! (`.stellar/cache/chunks/`, ADR-0122)이므로 migration은 Git working tree를
//! 바꾸지 않고(`git status`가 빔) cache 안에서만 pointer와 새 manifest를 씁니다.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use stellar_core_types::ids::StableId;
use stellar_storage::chunk::envelope::ChunkKindCode;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::chunk::store::{ChunkStore, WriteOutcome};
use stellar_storage::generation::current::write_current_pointer;
use stellar_storage::generation::loader::{manifest_path, write_manifest_file};
use stellar_storage::generation::manifest::GenerationManifest;
use stellar_storage::migration::registry::MigrationRegistry;
use stellar_storage::migration::runner::{noop_v1_to_v2, run_migration};

struct TempDir {
    path: PathBuf,
}
impl TempDir {
    fn new() -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-miggit-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@invalid")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@invalid")
        .output()
        .expect("git 실행");
    assert!(
        output.status.success(),
        "git {:?} 실패: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const KIND: ChunkKindCode = ChunkKindCode(0x8000_0001);

fn stable(ts: u64) -> StableId {
    StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
}

#[test]
fn migration_leaves_only_expected_working_tree_changes() {
    // 검증 기준(V.8.5): git status에 expected files만 변경.
    let dir = TempDir::new();
    let project_id = stable(42);
    git(&dir.path, &["init", "-q", "-b", "main"]);

    // v1 프로젝트 구성 후 baseline commit.
    let store = ChunkStore::new(&dir.path);
    let WriteOutcome::Written { hash, .. } = store.write_chunk(KIND, b"chunk-a").unwrap() else {
        panic!("Written이어야 함");
    };
    let manifest = GenerationManifest {
        generation: 1,
        parent_generation: None,
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
    let manifest_hash = write_manifest_file(&dir.path, &manifest).unwrap();
    write_current_pointer(&dir.path, &manifest_hash).unwrap();
    // Git 원본 파일 하나(텍스트 원본 자리). cache는 `.stellar/cache/.gitignore`로 Git 밖입니다.
    std::fs::write(dir.path.join("stellar.yaml"), b"schema_version: 1\n").unwrap();
    git(&dir.path, &["add", "--all"]);
    let tracked = git(&dir.path, &["ls-files", "--cached"]);
    assert_eq!(
        tracked.trim(),
        "stellar.yaml",
        "cache가 Git에 들어감: {tracked}"
    );
    git(&dir.path, &["commit", "-q", "-m", "baseline v1"]);
    let head_before = git(&dir.path, &["rev-parse", "HEAD"]);

    // noop v1→v2 migration 실행.
    let mut registry = MigrationRegistry::new();
    registry.register(noop_v1_to_v2()).unwrap();
    let outcome = run_migration(&dir.path, project_id, &registry, 2).unwrap();

    // 자동 commit 없음: HEAD 불변.
    let head_after = git(&dir.path, &["rev-parse", "HEAD"]);
    assert_eq!(
        head_before, head_after,
        "migration이 Git commit을 만들면 안 됨"
    );

    // Git working tree 변경 없음: pointer 전환과 새 manifest는 Git 밖 cache 안에만 있습니다.
    let status = git(
        &dir.path,
        &["status", "--porcelain", "--untracked-files=all"],
    );
    assert_eq!(status, "", "예상 밖의 변경: {status}");
    assert!(manifest_path(&dir.path, &outcome.new_manifest_hash).is_file());
    assert_eq!(
        stellar_storage::generation::current::read_current_pointer(&dir.path).unwrap(),
        outcome.new_manifest_hash
    );
}
