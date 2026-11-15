//! 테스트 전용 프로젝트 fixture 생성기. 프로덕션 경로에서 사용하지 않습니다.
//!
//! `stellar-ops`의 단위 테스트와, `stellar-ops`·diagnostics에만 의존할 수 있는 상위 도구
//! (`stellar-cli`, ADR-0005 §8)의 통합 테스트가 같은 fixture를 만들도록 `test-support` feature로
//! 노출합니다. 다른 crate는 `[dev-dependencies]`에서만 켭니다.
#![cfg(any(test, feature = "test-support"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::text::{Paragraph, StyleRun, TextPayload, TextRange};
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::identity::parse_project_id;
use stellar_project::scaffold::write_default_scaffold;
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::hash::ContentHash;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

use crate::core_types::ids::StableId;
use crate::core_types::order_key::OrderKey;
use crate::core_types::text::UserText;

/// fixture 프로젝트의 `project.id`.
pub const PROJECT_ID_TEXT: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

/// 테스트 무작위 문서 생성기(VI.10.5) 재노출.
pub use stellar_document::test_support::random_document::random_valid_document;

/// 문서 모델 타입 재노출(fixture 작성용).
pub use stellar_document::model::DocumentModel as FixtureDocument;
/// 테스트에서 실제 새 문서와 같은 영속 스타일 표를 구성합니다.
pub use stellar_document::text_styles::DocumentTextStyles;

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
    save_documents_with_schema(root, documents, 1);
}

/// [`save_documents`]와 같되 generation 스키마 버전을 지정합니다(migration fixture용).
pub fn save_documents_with_schema(root: &Path, documents: &[&DocumentModel], schema: u32) {
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
    let plan = build_save_plan(base.as_ref(), project_id(), schema, &states, roots).expect("plan");
    let mut memory = SavedGenerationState::new(
        base.as_ref()
            .map(|b| (b.manifest.generation, b.manifest_hash)),
    );
    run_save(root, &plan, &mut memory, &CancelToken::new()).expect("save");
}

/// v3 이전 클라이언트와 호환되는, 고정된 v1 root Frame 테스트 원본.
pub fn legacy_component_yaml(
    id: StableId,
    name: &str,
    root: StableId,
    width: u32,
    height: u32,
) -> String {
    format!(
        r#"format: stellar-component
schema_version: "1"
component:
  id: "{id}"
  name: {name}
  root: "{root}"
root_node:
  id: "{root}"
  parent: none
  order_key: "80"
  visible: "true"
  locked: "false"
  display_name: Root
  kind: frame
  payload:
    size: "{width} {height}"
    transform: "1 0 0 1 0 0"
    clipping:
      kind: frame-bounds
    background: none
    border: none
    corner_radius: "0 0 0 0"
    layout_mode: none
"#
    )
}

/// 이전 실행 파일과 같은 원본에서 비교하기 위해 v1 root Frame 컴포넌트들을 만듭니다.
pub fn component_diff_fixture(folder: &Path, count: u64) -> std::path::PathBuf {
    use stellar_project::document_io::component_source::ComponentSource;
    std::fs::create_dir_all(folder).unwrap();
    let mut first = None;
    for index in 0..count {
        let component = StableId::new_v7(1_900_000_000_000, 7, index * 2);
        let root = StableId::new_v7(1_900_000_000_000, 7, index * 2 + 1);
        let name = format!("Frame{index}");
        let source = ComponentSource::new(component, name.clone(), root).unwrap();
        let path = folder.join(source.filename().unwrap());
        // create는 기존 라이브러리 전체에서 ID 중복을 검사합니다. fixture에는 위에서 만든 고유 ID만 씁니다.
        let yaml = legacy_component_yaml(component, &name, root, 0, 0);
        std::fs::write(&path, yaml).unwrap();
        first.get_or_insert(path);
    }
    first.expect("컴포넌트 fixture 한 개 이상")
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

/// 대형 문서 fixture: View 하나의 root Frame 아래 Frame group(group당 Fragment 최대 499개)으로
/// 정확히 `node_count`개 node를 만듭니다(`node_count >= 1`).
pub fn large_document(node_count: usize) -> DocumentModel {
    const GROUP: usize = 500;
    let id = |n: u64| StableId::new_v7(1_900_000_000_000 + n, 0x0123, n);
    let document_id = id(1);
    let view_id = id(2);
    let page_id = id(3);
    let root = id(4);
    let mut definition = DocumentDefinition::new(document_id, "Large".to_string());
    let mut page = PageDefinition::new(page_id, "Page".to_string());
    page.add_view(ViewDefinition::new(view_id, "View".to_string(), root))
        .expect("view");
    definition.add_page(page).expect("page");
    let mut model = DocumentModel::new(definition);
    let record =
        |id: StableId, parent: Option<StableId>, order_key: OrderKey, payload| NodeRecord {
            id,
            parent,
            order_key,
            visible: true,
            locked: false,
            display_name: String::new(),
            payload,
        };
    model
        .insert_node(record(
            root,
            None,
            OrderKey::between(None, None),
            NodePayload::Frame(FramePayload::default()),
        ))
        .expect("root");
    let remaining = node_count.saturating_sub(1);
    let groups = remaining.div_ceil(GROUP);
    let group_keys = OrderKey::evenly_spaced(groups);
    let mut next = 100u64;
    let mut created = 1usize;
    for group_key in group_keys {
        if created >= node_count {
            break;
        }
        let group = id(next);
        next += 1;
        model
            .insert_node(record(
                group,
                Some(root),
                group_key,
                NodePayload::Frame(FramePayload::default()),
            ))
            .expect("group");
        created += 1;
        let children = (node_count - created).min(GROUP - 1);
        for key in OrderKey::evenly_spaced(children) {
            model
                .insert_node(record(
                    id(next),
                    Some(group),
                    key,
                    NodePayload::Fragment(FragmentPayload),
                ))
                .expect("fragment");
            next += 1;
            created += 1;
        }
    }
    model
}

/// 서로 다른 누락 Path를 clip하는 Frame을 만들어 실제 validator 경고를 생성합니다.
pub fn document_with_unresolved_clips(count: usize) -> DocumentModel {
    use crate::core_types::reference::NodeRef;
    use stellar_document::frame::Clipping;
    let mut records = large_document(count + 1).to_records().expect("records");
    let root = records.definition.pages[0].views[0].root_frame;
    for (index, record) in records
        .nodes
        .iter_mut()
        .filter(|n| n.id != root)
        .enumerate()
    {
        record.payload = NodePayload::Frame(FramePayload {
            clipping: Clipping::Path(NodeRef::new(StableId::new_v7(
                1_950_000_000_000,
                0,
                index as u64,
            ))),
            ..FramePayload::default()
        });
    }
    DocumentModel::from_records(records)
        .expect("미해결 참조만 포함한 문서")
        .0
}

/// 단일 View의 Text 원본 왕복 fixture. Unicode와 비정규화된 run 순서를 보존해야 합니다.
pub fn text_document() -> DocumentModel {
    let mut records = large_document(4).to_records().expect("records");
    let node = records.nodes.last_mut().expect("text leaf");
    node.display_name = "TextFixture".into();
    node.payload = NodePayload::Text(
        TextPayload::new(
            UserText::new("Original 한글 e\u{301} 👩‍💻\nsecond line\r\n"),
            vec![
                Paragraph { style: 17 },
                Paragraph { style: 0 },
                Paragraph { style: u32::MAX },
            ],
            vec![
                StyleRun {
                    range: TextRange::new(8, 12).unwrap(),
                    style: 91,
                },
                StyleRun {
                    range: TextRange::new(0, 10).unwrap(),
                    style: 3,
                },
                StyleRun {
                    range: TextRange::new(5, 5).unwrap(),
                    style: u32::MAX,
                },
            ],
        )
        .expect("text payload"),
    );
    DocumentModel::from_records(records)
        .expect("text document")
        .0
}

/// current generation에서 ViewTree kind chunk(ViewTree·subtree) 파일을 모두 망가뜨립니다.
///
/// DocumentManifest chunk는 그대로 두므로 프로젝트는 열리고 문서 내용 읽기만 실패합니다.
/// 망가뜨린 파일 수를 반환합니다.
pub fn corrupt_view_tree_chunks(root: &Path) -> usize {
    let loaded = load_current_generation(root, project_id()).expect("generation");
    let mut count = 0;
    for pointer in &loaded.manifest.chunk_pointers {
        if pointer.kind == stellar_storage::chunk::kinds::KIND_VIEW_TREE {
            let relative =
                stellar_storage::chunk::path::ChunkObjectPath::for_hash(&pointer.content_hash)
                    .to_relative_path();
            std::fs::write(root.join(relative), b"broken").expect("chunk 손상");
            count += 1;
        }
    }
    count
}

/// 저장 View 접근성 HTML fixture.
pub mod accessibility;

/// 저장 View 고정 Frame HTML+CSS fixture.
pub mod web_frame;

/// 임시 fixture의 기존 신뢰 등록을 준비하는 타입. 제품 API에는 재노출하지 않습니다.
pub use stellar_project::{identity::ProjectIdentity, trust_store::FileTrustStore};

/// 임시 신뢰 저장소에서 프로젝트를 선택합니다(전역 환경 변수 변경 없이 cache 검증).
pub fn select_trusted_project(
    project: StableId,
    data: PathBuf,
) -> Result<crate::trusted::TrustedProject, crate::trusted::TrustedReadError> {
    crate::trusted::TrustedProject::select_in(project, data)
}
