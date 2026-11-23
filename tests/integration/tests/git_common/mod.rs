//! Git 종단 시나리오 공용 helper.
//!
//! - 저장소: git-porcelain fixture harness(XIV.14.1)의 local bare 원격 + working 저장소. 네트워크 없음,
//!   hermetic config, 고정 author·시각.
//! - 프로젝트: `stellar.yaml` + 제품 저장 순서로 기록한 텍스트 원본(Git 원본)과 Git 밖 chunk cache(ADR-0122).
//!   materialize한 Git snapshot에는 cache가 없으므로 열 때 원본으로 cache를 만듭니다([`open_project`]).
//! - 문서: 손으로 구성한 "카드 문서"(편집 대상 node를 이름으로 짚을 수 있음)와 seed 고정 random valid
//!   document(VI.10.5).
//! - 비교: 문서 validator(VI.10.1)를 통과한 영속 레코드를 [`SemanticSnapshot`](XIV.3.1)으로 펼쳐
//!   비교하고, 문서별 canonical semantic snapshot JSON(VI.10.4)으로도 대조합니다.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::value::{Rect, Size2, Vec2};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{CornerRadius, FramePayload};
use stellar_document::model::DocumentModel;
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::shape::{EllipseParams, LineCap, LineParams, RectangleParams, ShapeGeometry};
use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
use stellar_document::test_support::semantic_snapshot::semantic_snapshot_json;
use stellar_document::validate::validate_document;
use stellar_git_porcelain::fixture::GitFixture;
use stellar_git_porcelain::object::ObjectId;
use stellar_git_porcelain::snapshot::{
    materialize_snapshot, read_index_snapshot, read_tree_snapshot, MaterializedSnapshot,
    SnapshotSource, SnapshotSourceKind,
};
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::identity::parse_project_id;
use stellar_project::manifest::{Manifest, ProjectMeta, CURRENT_SCHEMA_VERSION};
use stellar_project::root::MANIFEST_FILE;
use stellar_project::saved_project::{SavedProject, SavedProjectLoader};
use stellar_project::yaml::write_canonical;
use stellar_semantic_diff::build::{ProjectMetadata, SnapshotBuilder};
use stellar_semantic_diff::diff::SemanticDiff;
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::{ObjectKind, SemanticObject, SemanticSnapshot};
use stellar_semantic_diff::value::SemanticValue;
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

/// fixture 프로젝트 id(UUID 텍스트).
pub const PROJECT_ID_TEXT: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

/// project metadata 항목 이름(매니페스트 `project.name`).
pub const PROJECT_NAME_KEY: &str = "name";

/// fixture 프로젝트 id.
pub fn project_id() -> StableId {
    parse_project_id(PROJECT_ID_TEXT).expect("고정 UUID")
}

/// 시스템 Git fixture. Git이 없으면 `None`(호출 테스트는 건너뜀).
pub fn git_fixture(tag: &str) -> Option<GitFixture> {
    let fixture = GitFixture::with_system_git(tag).expect("fixture 생성");
    if fixture.is_none() {
        eprintln!("시스템 Git 없음: {tag} 건너뜀");
    }
    fixture
}

/// fixture 안 materialize parent 디렉터리(fixture drop 시 함께 삭제).
pub fn scratch_dir(fixture: &GitFixture, name: &str) -> PathBuf {
    let dir = fixture.root().join(name);
    std::fs::create_dir_all(&dir).expect("scratch 디렉터리");
    dir
}

/// 디렉터리 안 항목 수(정리 확인용).
pub fn entry_count(dir: &Path) -> usize {
    std::fs::read_dir(dir).expect("디렉터리 읽기").count()
}

/// 유효한 `stellar.yaml`을 씁니다.
pub fn write_manifest(root: &Path, name: &str) {
    let manifest = Manifest {
        schema_version: CURRENT_SCHEMA_VERSION,
        project: ProjectMeta {
            id: PROJECT_ID_TEXT.to_string(),
            name: name.to_string(),
        },
        target_profiles: Vec::new(),
        dependencies: Vec::new(),
        style_defaults: None,
    };
    std::fs::write(
        root.join(MANIFEST_FILE),
        write_canonical(&manifest.to_yaml()),
    )
    .expect("stellar.yaml 쓰기");
}

/// 문서들을 제품 저장 순서로 저장합니다(디스크 기록). 문서가 하나면 텍스트 원본(Git 원본) → chunk cache → stamp
/// (ADR-0088·ADR-0113·ADR-0122)이고, 둘 이상이면 텍스트 원본이 담을 수 없어 chunk cache만 씁니다(Git 밖).
pub fn save_documents(root: &Path, documents: &[&DocumentModel]) {
    use stellar_project::document_io::component_source::read_project_library;
    use stellar_project::document_io::session::{
        commit, decode_source, read_source, read_source_dir, write_stamp, SourceBase,
        SourceLibrary, Stamp,
    };
    use stellar_project::path_resolver::ExternalAccess;
    let written = if let [model] = documents {
        // 병합 해결처럼 작업 트리 원본이 Git 줄 병합으로 깨졌으면 컴포넌트 원본만 이어 받고 문서를 새로 씁니다.
        let decoded = read_source(root)
            .expect("원본 읽기")
            .map(|files| (decode_source(&files), files));
        let (base, library) = match decoded {
            Some((Ok(project), files)) => (files.base.clone(), SourceLibrary::split(project).1),
            Some((Err(_), files)) => {
                let (components, _) = read_project_library(root, ExternalAccess::MetadataOnly)
                    .expect("컴포넌트 원본");
                (
                    files.base.clone(),
                    SourceLibrary {
                        components,
                        ..SourceLibrary::default()
                    },
                )
            }
            None => {
                let design = root.join("design");
                let existing = if design.is_dir() {
                    read_source_dir(&design).expect("design 읽기")
                } else {
                    Default::default()
                };
                let (components, _) = read_project_library(root, ExternalAccess::MetadataOnly)
                    .expect("컴포넌트 원본");
                (
                    SourceBase::of(&existing),
                    SourceLibrary {
                        components,
                        ..SourceLibrary::default()
                    },
                )
            }
        };
        let (mut copy, _) =
            DocumentModel::from_records(model.to_records().expect("records")).expect("model");
        copy.replace_context(model.context().live(model));
        Some(
            commit(root, &base, &library.join(copy))
                .expect("원본 저장")
                .base,
        )
    } else {
        None
    };
    save_cache(root, documents);
    if let Some(written) = written {
        let manifest = stellar_storage::generation::current::read_current_pointer(root).unwrap();
        write_stamp(
            root,
            &Stamp {
                source: written.digest(),
                manifest,
            },
        )
        .expect("stamp");
    }
}

/// 문서들을 chunk cache(Git 밖)에만 저장합니다.
pub fn save_cache(root: &Path, documents: &[&DocumentModel]) {
    let base = load_current_generation(root, project_id()).ok();
    let serialized: Vec<_> = documents
        .iter()
        .map(|model| serialize_document(model).expect("문서 직렬화"))
        .collect();
    let states: Vec<_> = serialized
        .iter()
        .flat_map(|d| d.to_dirty_states())
        .collect();
    let roots = serialized.iter().map(|d| d.document_id).collect();
    let plan = build_save_plan(base.as_ref(), project_id(), 1, &states, roots).expect("저장 계획");
    let mut memory = SavedGenerationState::new(
        base.as_ref()
            .map(|b| (b.manifest.generation, b.manifest_hash)),
    );
    run_save(root, &plan, &mut memory, &CancelToken::new()).expect("저장 실행");
}

/// HEAD·Commit·Tree·Index 원천을 `parent` 아래 `.git` 없는 virtual project로 materialize합니다
/// (XIV.2.4·XIV.2.5·XIV.2.6).
pub fn materialize_source(
    gate: &GatedGitRunner,
    work: &Path,
    source: &SnapshotSource,
    parent: &Path,
) -> MaterializedSnapshot {
    let entries = match source.kind() {
        SnapshotSourceKind::GitTree => {
            read_tree_snapshot(gate, work, source)
                .unwrap_or_else(|e| panic!("{source} tree 읽기: {e}"))
                .entries
        }
        SnapshotSourceKind::Index => {
            let index = read_index_snapshot(gate, work).expect("index 읽기");
            assert!(!index.has_conflicts(), "index에 unmerged 항목");
            index.entries
        }
        SnapshotSourceKind::WorkingTree => {
            panic!("WorkingTree는 디스크 loader(XIV.2.7)로 읽습니다")
        }
    };
    let snapshot = materialize_snapshot(gate, work, &entries, parent)
        .unwrap_or_else(|e| panic!("{source} materialize: {e}"));
    assert!(
        !snapshot.root().join(".git").exists(),
        "{source}: .git 없음"
    );
    assert!(snapshot.skipped_gitlinks().is_empty());
    snapshot
}

/// virtual project root를 실제 프로젝트 loader(V.4.4)로 엽니다. Git snapshot에는 chunk cache가 없으므로 먼저 원본으로
/// cache를 만듭니다(ADR-0122, 작업 트리면 원본과 짝이 아닌 cache만 다시 만듦).
pub fn open_project(root: &Path) -> SavedProject {
    stellar_project::cache::prepare_snapshot(root)
        .unwrap_or_else(|e| panic!("{} cache 준비: {e}", root.display()));
    SavedProjectLoader::new()
        .open(root)
        .unwrap_or_else(|e| panic!("{} 열기: {e}", root.display()))
}

/// 프로젝트 하나의 영속 상태(매니페스트 이름 + 문서 레코드, 문서 id 순).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectState {
    /// 매니페스트 `project.name`.
    pub name: String,
    /// 문서 영속 레코드(canonical 순서, 문서 id 오름차순).
    pub documents: Vec<DocumentRecords>,
}

impl ProjectState {
    /// 메모리 모델에서 기대 상태를 만듭니다(각 문서는 VI.10.1 validator를 통과해야 함).
    pub fn expected(name: &str, models: &[&DocumentModel]) -> ProjectState {
        let mut documents: Vec<DocumentRecords> = models
            .iter()
            .map(|m| m.to_records().expect("레코드"))
            .collect();
        documents.sort_by_key(|d| d.definition.id);
        for records in &documents {
            validate_document(records).expect("기대 문서는 VI.10.1 validator를 통과");
        }
        ProjectState {
            name: name.to_string(),
            documents,
        }
    }

    /// 저장된 프로젝트의 모든 문서를 디스크 chunk에서 로드합니다(VI.10.3, 각 문서 VI.10.1 검증).
    pub fn load(project: &SavedProject) -> ProjectState {
        let mut ids = project.document_ids().expect("문서 목록");
        ids.sort();
        let documents = ids
            .into_iter()
            .map(|id| {
                let loaded = project.load_document(id).expect("문서 로드");
                let records = loaded.model.to_records().expect("레코드");
                validate_document(&records).expect("로드한 문서는 VI.10.1 validator를 통과");
                records
            })
            .collect();
        ProjectState {
            name: project.manifest.project.name.clone(),
            documents,
        }
    }

    /// 프로젝트 metadata + 모든 문서의 semantic snapshot(XIV.3.1).
    pub fn semantic(&self) -> SemanticSnapshot {
        let mut builder = SnapshotBuilder::new();
        builder
            .add_project(&ProjectMetadata {
                id: project_id(),
                entries: vec![(PROJECT_NAME_KEY.to_string(), self.name.clone())],
            })
            .expect("project metadata");
        for records in &self.documents {
            builder.add_document(records).expect("문서 snapshot");
        }
        builder.finish()
    }

    /// 문서별 canonical semantic snapshot JSON(VI.10.4).
    pub fn document_json(&self) -> Vec<String> {
        self.documents.iter().map(semantic_snapshot_json).collect()
    }
}

/// 두 상태가 의미상 같은지(문서별 VI.10.4 JSON과 전체 semantic snapshot).
pub fn assert_same_state(actual: &ProjectState, expected: &ProjectState, label: &str) {
    assert_eq!(actual.name, expected.name, "{label}: 프로젝트 이름");
    assert_eq!(
        actual.document_json(),
        expected.document_json(),
        "{label}: 문서 semantic snapshot JSON"
    );
    assert_eq!(
        actual.semantic(),
        expected.semantic(),
        "{label}: semantic snapshot"
    );
}

/// 원천을 materialize해 실제 loader로 연 상태.
pub fn source_state(
    gate: &GatedGitRunner,
    work: &Path,
    source: &SnapshotSource,
    parent: &Path,
) -> ProjectState {
    let snapshot = materialize_source(gate, work, source, parent);
    ProjectState::load(&open_project(snapshot.root()))
}

/// commit 원천.
pub fn commit_source(oid: &ObjectId) -> SnapshotSource {
    SnapshotSource::Commit(oid.clone())
}

/// diff의 변경 요약 `(종류 이름, 객체, 속성 경로)` 집합.
pub fn change_summary(
    diff: &SemanticDiff,
) -> BTreeSet<(&'static str, StableId, Option<PropertyPath>)> {
    diff.iter()
        .map(|c| (c.kind().name(), c.object_id(), c.property_path().cloned()))
        .collect()
}

/// diff가 건드린 객체 집합.
pub fn touched_objects(diff: &SemanticDiff) -> BTreeSet<StableId> {
    diff.iter().map(|c| c.object_id()).collect()
}

/// snapshot 구조 불변식: 모든 parent가 snapshot 안에 있고, 같은 parent 아래 같은 순서 공간(자식 node,
/// modifier stack 항목)의 OrderKey가 유일합니다(VI.10.1 OrderKey uniqueness의 snapshot 대응).
pub fn assert_snapshot_structure(snapshot: &SemanticSnapshot, label: &str) {
    let mut keys = BTreeSet::new();
    for object in snapshot.objects() {
        if let Some(parent) = object.parent() {
            assert!(
                snapshot.contains(parent),
                "{label}: {:?}의 parent {parent:?} 없음",
                object.id()
            );
            if let Some(key) = object.order_key() {
                let modifier = matches!(object.kind(), ObjectKind::Modifier(_));
                assert!(
                    keys.insert((parent, modifier, key.clone())),
                    "{label}: parent {parent:?} 아래 OrderKey 중복 {key:?}"
                );
            }
        }
    }
}

/// snapshot 정의 정합: 각 Document의 `DOCUMENT_PAGES`는 그 문서를 parent로 둔 Page 집합과, 각 Page의
/// `PAGE_VIEWS`는 그 Page를 parent로 둔 View 집합과 같고, 각 View의 root는 parent 없는 node입니다
/// (VI.10.1 root count의 snapshot 대응). 목록에 없는 Page·View가 있으면 문서로 되돌릴 수 없습니다.
pub fn assert_definition_coherent(snapshot: &SemanticSnapshot, label: &str) {
    let ids = |object: &SemanticObject, key| match object.property(&object.path(&[key])) {
        Some(SemanticValue::Ids(ids)) => ids.iter().copied().collect::<BTreeSet<_>>(),
        other => panic!("{label}: {:?} 목록 속성 없음: {other:?}", object.id()),
    };
    let children_of = |parent: StableId, kind: ObjectKind| {
        snapshot
            .objects()
            .filter(|o| o.kind() == kind && o.parent() == Some(parent))
            .map(|o| o.id())
            .collect::<BTreeSet<_>>()
    };
    for object in snapshot.objects() {
        match object.kind() {
            ObjectKind::Document => assert_eq!(
                ids(object, keys::DOCUMENT_PAGES),
                children_of(object.id(), ObjectKind::Page),
                "{label}: 문서 {:?}의 페이지 목록과 Page 객체",
                object.id()
            ),
            ObjectKind::Page => assert_eq!(
                ids(object, keys::PAGE_VIEWS),
                children_of(object.id(), ObjectKind::View),
                "{label}: 페이지 {:?}의 View 목록과 View 객체",
                object.id()
            ),
            ObjectKind::View => {
                let root = match object.property(&object.path(&[keys::VIEW_ROOT_FRAME])) {
                    Some(SemanticValue::Id(root)) => *root,
                    other => panic!("{label}: View root 속성 없음: {other:?}"),
                };
                let root = snapshot
                    .get(root)
                    .unwrap_or_else(|| panic!("{label}: View root {root:?} 없음"));
                assert_eq!(root.parent(), None, "{label}: View root는 parent 없음");
            }
            _ => {}
        }
    }
}

// --- 카드 문서 ---

/// 카드 문서의 n번째 StableId. random valid document(timestamp 1.8e12 대역)와 겹치지 않습니다.
pub fn nid(n: u64) -> StableId {
    StableId::new_v7(1_700_000_000_000 + n, 0x0a11, 0x5eed_0000 + n)
}

/// 카드 문서 정의·node 번호.
pub mod card {
    /// 문서.
    pub const DOCUMENT: u64 = 1;
    /// 페이지.
    pub const PAGE: u64 = 2;
    /// View.
    pub const VIEW: u64 = 3;
    /// View root Frame.
    pub const ROOT: u64 = 10;
    /// 카드 Frame(root 자식).
    pub const CARD: u64 = 11;
    /// 배경 사각형(카드 자식).
    pub const BACKGROUND: u64 = 12;
    /// 아이콘 타원(카드 자식).
    pub const ICON: u64 = 13;
    /// 구분선(root 자식).
    pub const DIVIDER: u64 = 14;
    /// 그룹 Fragment(root 자식).
    pub const GROUP: u64 = 15;
    /// 그룹 안 잎 사각형.
    pub const LEAF: u64 = 16;
    /// ours가 추가하는 node 번호.
    pub const OURS_NEW: u64 = 21;
    /// theirs가 추가하는 node 번호.
    pub const THEIRS_NEW: u64 = 22;
    /// working tree 편집이 추가하는 node 번호.
    pub const WORKTREE_NEW: u64 = 23;
    /// ours가 추가하는 페이지·그 View·그 root.
    pub const OURS_PAGE: [u64; 3] = [40, 41, 42];
    /// ours가 첫 페이지에 추가하는 View·그 root.
    pub const OURS_VIEW: [u64; 2] = [43, 44];
    /// theirs가 추가하는 페이지·그 View·그 root.
    pub const THEIRS_PAGE: [u64; 3] = [50, 51, 52];
    /// theirs가 첫 페이지에 추가하는 View·그 root.
    pub const THEIRS_VIEW: [u64; 2] = [53, 54];
}

fn key(byte: u8) -> OrderKey {
    OrderKey::from_bytes(vec![byte]).expect("OrderKey")
}

fn record(n: u64, parent: Option<u64>, order: u8, name: &str, payload: NodePayload) -> NodeRecord {
    NodeRecord {
        id: nid(n),
        parent: parent.map(nid),
        order_key: key(order),
        visible: true,
        locked: false,
        display_name: name.to_string(),
        payload,
    }
}

fn rectangle(width: f32, height: f32) -> NodePayload {
    NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
        size: Size2::new(width, height).expect("size"),
        corner_radius: CornerRadius::new(4.0, 4.0, 0.0, 0.0).expect("radius"),
    }))
}

/// 카드 문서(View 하나, Frame·Fragment·사각형·타원·선).
///
/// ```text
/// 루트(Frame)
/// ├── 카드(Frame 320×200)
/// │   ├── 배경(사각형)
/// │   └── 아이콘(타원)
/// ├── 구분선(선)
/// └── 그룹(Fragment)
///     └── 잎(사각형)
/// ```
pub fn card_document() -> DocumentModel {
    use card::*;
    let mut definition = DocumentDefinition::new(nid(DOCUMENT), "카드 문서".to_string());
    let mut page = PageDefinition::new(nid(PAGE), "페이지".to_string());
    page.add_view(ViewDefinition::new(nid(VIEW), "홈".to_string(), nid(ROOT)))
        .expect("View");
    definition.add_page(page).expect("페이지");
    let mut model = DocumentModel::new(definition);
    let nodes = [
        record(
            ROOT,
            None,
            0x10,
            "루트",
            NodePayload::Frame(FramePayload::default()),
        ),
        record(
            CARD,
            Some(ROOT),
            0x10,
            "카드",
            NodePayload::Frame(FramePayload {
                size: Size2::new(320.0, 200.0).expect("size"),
                ..FramePayload::default()
            }),
        ),
        record(BACKGROUND, Some(CARD), 0x10, "배경", rectangle(100.0, 40.0)),
        record(
            ICON,
            Some(CARD),
            0x20,
            "아이콘",
            NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
                bounds: Rect::new(
                    Vec2::new(8.0, 8.0).expect("origin"),
                    Size2::new(24.0, 24.0).expect("size"),
                ),
            })),
        ),
        record(
            DIVIDER,
            Some(ROOT),
            0x20,
            "구분선",
            NodePayload::Shape(ShapeGeometry::Line(LineParams {
                start: Vec2::ZERO,
                end: Vec2::new(300.0, 0.0).expect("end"),
                cap: LineCap::Round,
            })),
        ),
        record(
            GROUP,
            Some(ROOT),
            0x30,
            "그룹",
            NodePayload::Fragment(FragmentPayload),
        ),
        record(LEAF, Some(GROUP), 0x10, "잎", rectangle(16.0, 16.0)),
    ];
    for node in nodes {
        model.insert_node(node).expect("node 삽입");
    }
    model
}

/// 레코드 단위 편집을 적용하고 VI.10.1 validator를 통과한 모델로 다시 만듭니다(VI.10.3).
pub fn edit(model: &DocumentModel, f: impl FnOnce(&mut DocumentRecords)) -> DocumentModel {
    let mut records = model.to_records().expect("레코드");
    f(&mut records);
    DocumentModel::from_records(records)
        .expect("편집 결과는 VI.10.1 validator를 통과")
        .0
}

/// 카드 문서 node 레코드(n번).
pub fn node_mut(records: &mut DocumentRecords, n: u64) -> &mut NodeRecord {
    records
        .nodes
        .iter_mut()
        .find(|r| r.id == nid(n))
        .unwrap_or_else(|| panic!("node {n} 없음"))
}

/// node 표시 이름 변경.
pub fn rename(records: &mut DocumentRecords, n: u64, name: &str) {
    node_mut(records, n).display_name = name.to_string();
}

/// Frame 크기 변경.
pub fn resize_frame(records: &mut DocumentRecords, n: u64, width: f32, height: f32) {
    match &mut node_mut(records, n).payload {
        NodePayload::Frame(frame) => frame.size = Size2::new(width, height).expect("size"),
        other => panic!("node {n}은 Frame이 아님: {other:?}"),
    }
}

/// 선 끝점 변경.
pub fn move_line_end(records: &mut DocumentRecords, n: u64, x: f32, y: f32) {
    match &mut node_mut(records, n).payload {
        NodePayload::Shape(ShapeGeometry::Line(line)) => line.end = Vec2::new(x, y).expect("end"),
        other => panic!("node {n}은 선이 아님: {other:?}"),
    }
}

/// 자식 없는 node 삭제.
pub fn remove_leaf(records: &mut DocumentRecords, n: u64) {
    let id = nid(n);
    assert!(
        records.nodes.iter().all(|r| r.parent != Some(id)),
        "node {n}에 자식이 있음"
    );
    records.nodes.retain(|r| r.id != id);
    records.modifier_stacks.retain(|s| s.node != id);
}

/// `parent` 마지막 자식 뒤에 사각형 node를 추가합니다(`OrderKey::between(last, None)`).
///
/// 같은 base에서 두 branch가 각각 부르면 같은 OrderKey가 나옵니다(XIV.10.6 재배치 대상).
pub fn append_rectangle(records: &mut DocumentRecords, n: u64, parent: u64, name: &str) {
    let parent_id = nid(parent);
    let last = records
        .nodes
        .iter()
        .filter(|r| r.parent == Some(parent_id))
        .map(|r| r.order_key.clone())
        .max();
    records.nodes.push(NodeRecord {
        id: nid(n),
        parent: Some(parent_id),
        order_key: OrderKey::between(last.as_ref(), None),
        visible: true,
        locked: false,
        display_name: name.to_string(),
        payload: rectangle(12.0, 12.0),
    });
}

fn push_root(records: &mut DocumentRecords, root: u64) {
    records.nodes.push(NodeRecord {
        id: nid(root),
        parent: None,
        order_key: OrderKey::between(None, None),
        visible: true,
        locked: false,
        display_name: format!("root {root}"),
        payload: NodePayload::Frame(FramePayload::default()),
    });
}

/// 문서 끝에 View 하나(root Frame 포함)를 가진 새 페이지를 추가합니다.
pub fn append_page(records: &mut DocumentRecords, page: u64, view: u64, root: u64, name: &str) {
    let mut definition = PageDefinition::new(nid(page), name.to_string());
    definition
        .add_view(ViewDefinition::new(nid(view), name.to_string(), nid(root)))
        .expect("View");
    records.definition.add_page(definition).expect("페이지");
    push_root(records, root);
}

/// 카드 문서 첫 페이지 끝에 새 View(root Frame 포함)를 추가합니다.
pub fn append_view(records: &mut DocumentRecords, view: u64, root: u64, name: &str) {
    let page = records
        .definition
        .pages
        .iter_mut()
        .find(|p| p.id == nid(card::PAGE))
        .expect("카드 문서 페이지");
    page.add_view(ViewDefinition::new(nid(view), name.to_string(), nid(root)))
        .expect("View");
    push_root(records, root);
}

/// 레코드에서 node의 OrderKey.
pub fn order_key_of(records: &DocumentRecords, n: u64) -> OrderKey {
    records
        .nodes
        .iter()
        .find(|r| r.id == nid(n))
        .map(|r| r.order_key.clone())
        .unwrap_or_else(|| panic!("node {n} 없음"))
}

// --- seed 고정 random 문서 ---

/// seed 고정 random valid document(VI.10.5). node가 하나 이상 있음을 확인합니다.
pub fn seeded_document(seed: u64) -> DocumentModel {
    let model = random_valid_document(seed);
    assert!(model.node_count() > 0, "seed {seed}: node 없음");
    model
}

/// seed로 고른 node 하나의 이름과 visible을 바꾼 문서(VI.10.1 validator 통과).
///
/// 반환: (편집한 모델, 편집한 node id).
pub fn seeded_edit(model: &DocumentModel, seed: u64) -> (DocumentModel, StableId) {
    let mut rng = SplitMix64::new(seed ^ 0x6172_6765_5f65_6469);
    let mut target = StableId::NIL;
    let edited = edit(model, |records| {
        let index = rng.below(records.nodes.len() as u64) as usize;
        let node = &mut records.nodes[index];
        node.display_name = format!("seed-{seed} 편집");
        node.visible = !node.visible;
        target = node.id;
    });
    (edited, target)
}
