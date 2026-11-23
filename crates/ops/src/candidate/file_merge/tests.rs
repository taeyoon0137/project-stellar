use super::*;
use std::path::PathBuf;

use stellar_project::document_io::component_source::{
    add_frame, create, edit_root, resize, ComponentRootEdit, ComponentSource,
};
use stellar_semantic_diff::core_types::color::{Color, ColorSpace};
use stellar_semantic_diff::core_types::value::Size2;
use stellar_storage::hash::ContentHash;

use crate::core_types::ids::StableId;
use crate::test_support::TempDir;

pub(crate) const A: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";
pub(crate) const AR: &str = "0195c530-3acd-7bc1-8414-c213f61cb966";
pub(crate) const CHILD: &str = "0195c530-3acd-7bc1-8414-c213f61cb969";

/// 폴더 하나에 Card 컴포넌트(90x60)를 만들고 (프로젝트 상대 경로, 파일 경로)를 돌려줍니다.
pub(crate) fn card(dir: &Path) -> (PathBuf, PathBuf) {
    let folder = dir.join("design/components");
    std::fs::create_dir_all(&folder).unwrap();
    let source =
        ComponentSource::new(A.parse().unwrap(), "Card".into(), AR.parse().unwrap()).unwrap();
    let (path, hash) = create(&folder, &source).unwrap();
    resize(&path, hash, Size2::new(90.0, 60.0).unwrap()).unwrap();
    let relative = path.strip_prefix(dir).unwrap().to_path_buf();
    (relative, path)
}

fn hash(path: &Path) -> ContentHash {
    ContentHash::compute(&std::fs::read(path).unwrap())
}

/// 파일에 편집을 적용한 bytes를 얻고 파일은 원래대로 둡니다.
pub(crate) fn edited(path: &Path, f: impl FnOnce(&Path, ContentHash)) -> Vec<u8> {
    let before = std::fs::read(path).unwrap();
    f(path, hash(path));
    let after = std::fs::read(path).unwrap();
    std::fs::write(path, before).unwrap();
    after
}

pub(crate) fn resized(path: &Path, width: f32, height: f32) -> Vec<u8> {
    edited(path, |path, hash| {
        resize(path, hash, Size2::new(width, height).unwrap()).unwrap();
    })
}

#[test]
fn unchanged_file_re_encodes_to_identical_bytes() {
    let dir = TempDir::new("file-merge-identity");
    let (relative, path) = card(&dir.path);
    let bytes = std::fs::read(&path).unwrap();
    let outcome = merge_semantic_file(&relative, &bytes, &bytes, &bytes).unwrap();
    assert_eq!(outcome, FileMergeOutcome::Merged(bytes));
}

#[test]
fn adjacent_line_edits_merge_by_property() {
    let dir = TempDir::new("file-merge-adjacent");
    let (relative, path) = card(&dir.path);
    let base = std::fs::read(&path).unwrap();
    let ours = resized(&path, 144.0, 60.0);
    let theirs = resized(&path, 90.0, 96.0);
    let FileMergeOutcome::Merged(merged) =
        merge_semantic_file(&relative, &base, &ours, &theirs).unwrap()
    else {
        panic!("clean merge 기대");
    };
    assert_eq!(merged, resized(&path, 144.0, 96.0));
    // 배경과 자식 Frame 추가도 함께 합칩니다.
    let background = edited(&path, |path, hash| {
        let color = Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap();
        edit_root(path, hash, ComponentRootEdit::Background(Some(color))).unwrap();
    });
    let child = edited(&path, |path, hash| {
        add_frame(
            path,
            hash,
            AR.parse().unwrap(),
            CHILD.parse::<StableId>().unwrap(),
            "Badge".into(),
            Size2::new(10.0, 10.0).unwrap(),
        )
        .unwrap();
    });
    let outcome = merge_semantic_file(&relative, &base, &background, &child).unwrap();
    let FileMergeOutcome::Merged(bytes) = outcome else {
        panic!("clean merge 기대: {outcome:?}");
    };
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.contains("Badge") && text.contains("srgb 1 0 0 1"),
        "{text}"
    );
}

#[test]
fn same_property_conflict_returns_entries_and_no_bytes() {
    let dir = TempDir::new("file-merge-conflict");
    let (relative, path) = card(&dir.path);
    let base = std::fs::read(&path).unwrap();
    let ours = resized(&path, 144.0, 60.0);
    let theirs = resized(&path, 128.0, 60.0);
    let FileMergeOutcome::Conflicts(conflicts) =
        merge_semantic_file(&relative, &base, &ours, &theirs).unwrap()
    else {
        panic!("conflict 기대");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, "property-value");
    assert_eq!(conflicts[0].object.to_string(), AR);
}

#[test]
fn additions_deletions_and_bad_inputs() {
    let dir = TempDir::new("file-merge-edges");
    let (relative, path) = card(&dir.path);
    let bytes = std::fs::read(&path).unwrap();
    // 한쪽만 추가: 그 파일 그대로.
    assert_eq!(
        merge_semantic_file(&relative, b"", &bytes, b"").unwrap(),
        FileMergeOutcome::Merged(bytes.clone())
    );
    // 양쪽 삭제.
    assert_eq!(
        merge_semantic_file(&relative, &bytes, b"", b"").unwrap(),
        FileMergeOutcome::Deleted
    );
    // 지원하지 않는 경로, 깨진 bytes.
    assert_eq!(
        merge_semantic_file(Path::new("README.md"), &bytes, &bytes, &bytes),
        Err(FileMergeError::UnsupportedPath)
    );
    let marker = [b"<".repeat(7), b" ours\n".to_vec()].concat();
    assert!(matches!(
        merge_semantic_file(&relative, &bytes, &marker, &bytes),
        Err(FileMergeError::Decode { side: "ours", .. })
    ));
    // 파일명 StableId 접미사가 다른 경로는 codec이 거부합니다.
    assert!(matches!(
        merge_semantic_file(
            Path::new("design/components/Card-000000000000.stellar.yaml"),
            &bytes,
            &bytes,
            &bytes
        ),
        Err(FileMergeError::Decode { side: "base", .. })
    ));
}
