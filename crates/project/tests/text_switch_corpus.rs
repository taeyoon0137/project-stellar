//! V.10.7 전환 준비 corpus: 디스크에 저장된 generation을 텍스트 원본으로 옮겼다가 다시 읽어 semantic snapshot이
//! 같은지 확인합니다. 기본 열기·저장은 바꾸지 않습니다(전환은 V.10.5·V.10.6 연결 뒤).

mod common;

use std::path::Path;

use common::git::{save_documents, write_manifest};
use common::golden::golden_document;
use common::TempDir;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::test_support::random_document::random_valid_document;
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_project::document_io::source_v4::project::{decode, encode, from_saved};
use stellar_project::saved_project::SavedProjectLoader;

/// 저장된 프로젝트 하나를 옮기고 같은 의미인지 확인합니다. 지원하지 않는 의미면 `false`.
fn convert(root: &Path) -> bool {
    let saved = SavedProjectLoader::new().open(root).unwrap();
    let ids = saved.document_ids().unwrap();
    let [document] = ids.as_slice() else {
        return false;
    };
    let model = saved.load_document(*document).unwrap().model;
    let expected = model_snapshot_json(&model).unwrap();
    let project = match from_saved(&saved.generation, model, ComponentLibrary::new()) {
        Ok(project) => project,
        Err(error) => panic!("{}: {error}", root.display()),
    };
    let encoded = encode(&project).unwrap();
    let decoded = decode(&encoded.files).unwrap();
    assert_eq!(model_snapshot_json(&decoded.document).unwrap(), expected);
    assert_eq!(
        decoded.semantic_key().unwrap(),
        project.semantic_key().unwrap()
    );
    assert_eq!(decoded.images, project.images);
    true
}

#[test]
fn saved_generations_convert_to_text_and_back_with_the_same_semantic_snapshot() {
    let dir = TempDir::new("text-switch-corpus");
    let mut converted = 0;
    for seed in 0..60_u64 {
        let root = dir.path.join(format!("random-{seed}"));
        std::fs::create_dir_all(&root).unwrap();
        write_manifest(&root, "Corpus");
        save_documents(&root, &[&random_valid_document(seed)]);
        converted += usize::from(convert(&root));
    }
    for i in 0..12_u64 {
        let root = dir.path.join(format!("golden-{i}"));
        std::fs::create_dir_all(&root).unwrap();
        write_manifest(&root, "Golden");
        save_documents(&root, &[&golden_document(i)]);
        converted += usize::from(convert(&root));
    }
    assert_eq!(converted, 72);
}

#[test]
fn checked_in_window_session_fixture_converts() {
    // 저장소에 커밋된 실제 generation fixture(앱 창 session 저장 결과).
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../runtime/tests/fixtures/window-session-project");
    let dir = TempDir::new("text-switch-fixture");
    let copy = dir.path.join("project");
    copy_dir(&root, &copy);
    assert!(convert(&copy));
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
