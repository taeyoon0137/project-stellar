//! 테스트 전용 프로젝트 fixture 생성기. 프로덕션 경로에서 사용하지 않습니다.
//!
//! `stellar-ops`의 단위 테스트와, `stellar-ops`·diagnostics에만 의존할 수 있는 상위 도구
//! (`stellar-cli`, ADR-0005 §8)의 통합 테스트가 같은 fixture를 만들도록 `test-support` feature로
//! 노출합니다. 다른 crate는 `[dev-dependencies]`에서만 켭니다.
#![cfg(any(test, feature = "test-support"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use stellar_document::model::DocumentModel;
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::identity::parse_project_id;
use stellar_project::scaffold::write_default_scaffold;
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::hash::ContentHash;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

use crate::core_types::ids::StableId;

/// fixture 프로젝트의 `project.id`.
pub const PROJECT_ID_TEXT: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

/// 테스트 무작위 문서 생성기(VI.10.5) 재노출.
pub use stellar_document::test_support::random_document::random_valid_document;

/// 문서 모델 타입 재노출(fixture 작성용).
pub use stellar_document::model::DocumentModel as FixtureDocument;

/// 테스트 임시 디렉터리(drop 시 삭제).
#[derive(Debug)]
pub struct TempDir {
    /// 디렉터리 경로.
    pub path: PathBuf,
}

impl TempDir {
    /// 프로세스·호출마다 고유한 빈 디렉터리를 만듭니다.
    pub fn new(label: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stellar-ops-fixture-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("임시 디렉터리 생성");
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// fixture 프로젝트 id.
pub fn project_id() -> StableId {
    parse_project_id(PROJECT_ID_TEXT).expect("fixture project id")
}

/// 기본 scaffold(IV.4.3)와 빈 `.git` 디렉터리를 만듭니다(저장된 generation 없음).
pub fn scaffold_project(root: &Path) {
    std::fs::create_dir_all(root).expect("root 생성");
    write_default_scaffold(root, "Fixture", PROJECT_ID_TEXT).expect("scaffold");
    std::fs::create_dir_all(root.join(".git")).expect(".git 생성");
}

/// 문서들을 실제 저장 파이프라인(V.5)으로 current generation 위에 저장합니다.
///
/// 주어진 문서가 새 generation의 문서 전체가 됩니다.
pub fn save_documents(root: &Path, documents: &[&DocumentModel]) {
    let base = load_current_generation(root, project_id()).ok();
    let serialized: Vec<_> = documents
        .iter()
        .map(|model| serialize_document(model).expect("serialize"))
        .collect();
    let states: Vec<_> = serialized
        .iter()
        .flat_map(|d| d.to_dirty_states())
        .collect();
    let roots = serialized.iter().map(|d| d.document_id).collect();
    let plan = build_save_plan(base.as_ref(), project_id(), 1, &states, roots).expect("plan");
    let mut memory = SavedGenerationState::new(
        base.as_ref()
            .map(|b| (b.manifest.generation, b.manifest_hash)),
    );
    run_save(root, &plan, &mut memory, &CancelToken::new()).expect("save");
}

/// 디렉터리 아래 모든 파일의 (상대 경로 → content hash). 실행 전후 비교로 "파일 변경 0"을 검사합니다.
pub fn file_digest(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(relative, ContentHash::compute(&bytes).to_string());
            }
        }
    }
    out
}
