//! 종단 시나리오: 인스턴스 자식(투영된 Slot content)이 있는 문서의 경로 확인(ADR-0129 Consequences, ADR-0133 결정 6).
//!
//! ComponentInstance의 자식은 그 인스턴스 slot override에 투영된 Slot content입니다(ADR-0129 결정 3). 이 시험은 그런
//! 문서를 다음 경로로 보냅니다.
//!
//! - scene 그리기: 전체 변환에서 content가 slot host 가상 node 아래에 한 번만 들어가고, content 편집·삭제의 증분
//!   편집(`scene_edits`)이 새로 만든 전체 변환과 같습니다.
//! - semantic diff: snapshot → 문서 레코드 역투영이 원래 레코드와 같고, Slot content 지우기의 diff를 적용하면 결과
//!   snapshot과 같습니다.
//! - 텍스트 원본 codec(View v4): 쓰고 다시 읽은 레코드가 원래와 같습니다.
//! - 저장 chunk: 저장하고 다시 연 문서가 원래와 같습니다.
//! - codegen(Web 내보내기 `stellar export-web`): Slot content 투영 출력은 아직 지원하지 않으므로, 틀린 HTML을 내지 않고
//!   blocking 진단 `unsupported-component-instance`로 거부합니다(HTML 없음, 원본 무변경).

mod common;

use common::builders::{frame, new_document};
use common::{assert_same_records, assert_valid, key, sid, TempDir};
use stellar_commands::command::Command;
use stellar_commands::detach::plan_slot_content_delete;
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::request::CommandRequest;
use stellar_commands::state::DocumentState;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::value::{FiniteF32, Size2, Transform2D};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, ComponentSlotId, DefinitionNode, DefinitionTree,
};
use stellar_document::component::instance::{ComponentInstancePayload, SlotOverride};
use stellar_document::component::slot::{ComponentSlot, SlotCardinality};
use stellar_document::frame::{keys as frame_keys, FramePayload};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::encode_payload;
use stellar_document::records::NodePayload;
use stellar_project::document_io::load::{load_document, MemoryChunks};
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::document_io::text_source;
use stellar_scene::document::{document_scene_source, scene_edits, SceneSourceOptions};
use stellar_scene::source::SceneKey;
use stellar_semantic_diff::apply::apply;
use stellar_semantic_diff::build::SnapshotBuilder;
use stellar_semantic_diff::diff::diff;
use stellar_semantic_diff::records::document_records;
use stellar_semantic_diff::snapshot::SemanticSnapshot;

const SPACE: u64 = 41;

fn id(n: u64) -> StableId {
    sid(SPACE, n)
}

const DOCUMENT: u64 = 1;
const ROOT: u64 = 10;
const INSTANCE: u64 = 30;
const CONTENT: u64 = 31;
const INNER: u64 = 32;
const CARD_ROOT: u64 = 901;
const BODY_HOST: u64 = 902;

fn card() -> ComponentRef {
    ComponentRef::new(id(900))
}

fn body() -> ComponentSlotId {
    ComponentSlotId::new(id(940))
}

/// Card 컴포넌트: root 901(100×80) 아래 body 902(slot host, Frame만, 0..=2개)와 장식 903.
fn library() -> ComponentLibrary {
    let payload = |w: f32, h: f32, x: f32, y: f32| {
        encode_payload(&NodePayload::Frame(FramePayload {
            size: Size2::new(w, h).unwrap(),
            transform: Transform2D::new(1., 0., 0., 1., x, y).unwrap(),
            ..FramePayload::default()
        }))
    };
    let mut tree = DefinitionTree::new(card());
    for (n, parent, order, name, bytes) in [
        (CARD_ROOT, None, 0x40, "card", payload(100., 80., 0., 0.)),
        (
            BODY_HOST,
            Some(CARD_ROOT),
            0x40,
            "body",
            payload(80., 40., 10., 10.),
        ),
        (
            903,
            Some(CARD_ROOT),
            0x80,
            "decor",
            payload(20., 10., 70., 60.),
        ),
    ] {
        tree.insert(DefinitionNode {
            id: id(n),
            kind: NodeKind::Frame,
            parent: parent.map(id),
            order_key: key(order),
            visible: true,
            locked: false,
            display_name: name.into(),
            payload: bytes,
        })
        .unwrap();
    }
    let mut definition =
        ComponentDefinition::new(id(900), "Card".into(), NodeRef::new(id(CARD_ROOT)));
    definition.add_slot(body()).unwrap();
    let mut library = ComponentLibrary::new();
    library.insert(definition, tree).unwrap();
    library
        .add_slot(
            card(),
            ComponentSlot::new(
                body(),
                NodeRef::new(id(BODY_HOST)),
                [NodeKind::Frame],
                SlotCardinality::new(0, Some(2)).unwrap(),
                Vec::new(),
            ),
        )
        .unwrap();
    library
}

/// View root 10 아래 Card 인스턴스 30(배치 (200, 100))과 body Slot에 투영한 Frame 31(자식 Frame 32).
fn document() -> DocumentModel {
    let mut model = new_document(
        id(DOCUMENT),
        "인스턴스 자식",
        &[(id(2), "페이지", vec![(id(3), "화면", id(ROOT))])],
    );
    let mut payload = ComponentInstancePayload::new(card());
    payload.set_slot_override(body(), SlotOverride::new(vec![id(CONTENT)]).unwrap());
    let mut instance = frame(
        id(INSTANCE),
        id(ROOT),
        key(0x30),
        "카드",
        (0., 0.),
        (1., 1.),
    );
    instance.payload = NodePayload::ComponentInstance(payload);
    model.insert_node(instance).unwrap();
    let mut stack = ModifierStack::new(id(INSTANCE));
    stack
        .insert(ModifierEntry {
            id: ModifierId::new(id(130)),
            order: OrderKey::between(None, None),
            kind: ModifierKind::Transform,
            enabled: true,
            payload: ModifierPayload::Transform(
                Transform2D::new(1., 0., 0., 1., 200., 100.).unwrap(),
            ),
        })
        .unwrap();
    model.set_modifier_stack(stack).unwrap();
    model
        .insert_node(frame(
            id(CONTENT),
            id(INSTANCE),
            key(0x10),
            "본문",
            (4., 4.),
            (60., 20.),
        ))
        .unwrap();
    model
        .insert_node(frame(
            id(INNER),
            id(CONTENT),
            key(0x10),
            "안쪽",
            (2., 2.),
            (10., 10.),
        ))
        .unwrap();
    assert_valid(&model, "인스턴스 자식 문서");
    model
}

fn options(library: &ComponentLibrary) -> SceneSourceOptions<'_> {
    SceneSourceOptions {
        library: Some(library),
        ..SceneSourceOptions::default()
    }
}

fn execute(state: &mut DocumentState, command: Command) -> stellar_commands::change::ChangeSet {
    state
        .execute(&CommandRequest::at(state, command))
        .expect("command 적용")
        .changes
}

#[test]
fn scene_projects_slot_content_once_and_incremental_edits_match_a_rebuild() {
    let library = library();
    let mut state = DocumentState::new(document());
    let mut current =
        document_scene_source(state.model(), id(ROOT), options(&library)).expect("scene 변환");
    let content = SceneKey::node(id(CONTENT));
    let host = SceneKey::virtual_node(&[id(INSTANCE)], id(BODY_HOST));
    // content는 slot host 가상 node 아래 한 번, 그 자식은 content 아래에 들어갑니다.
    assert_eq!(current.source.parent(content), Some(host));
    assert_eq!(
        current.source.parent(SceneKey::node(id(INNER))),
        Some(content)
    );
    assert_eq!(current.source.keys().filter(|k| *k == content).count(), 1);
    assert!(current.skipped.is_empty(), "{:?}", current.skipped);

    // content 편집(폭)의 증분 편집은 새 전체 변환과 같습니다.
    let changes = execute(
        &mut state,
        Command::SetProperty(SetProperty {
            node: id(CONTENT),
            key: frame_keys::WIDTH,
            value: PropertyValue::NonNegative(FiniteF32::new(72.).unwrap()),
        }),
    );
    scene_edits(state.model(), &mut current, &changes, options(&library)).expect("증분 편집");
    let rebuilt = document_scene_source(state.model(), id(ROOT), options(&library)).unwrap();
    assert_eq!(
        current.source, rebuilt.source,
        "content 편집 뒤 증분 = 전체"
    );

    // content 안쪽 node 편집도 같습니다.
    let changes = execute(
        &mut state,
        Command::SetProperty(SetProperty::visibility(id(INNER), false)),
    );
    scene_edits(state.model(), &mut current, &changes, options(&library)).unwrap();
    let rebuilt = document_scene_source(state.model(), id(ROOT), options(&library)).unwrap();
    assert_eq!(
        current.source, rebuilt.source,
        "안쪽 node 편집 뒤 증분 = 전체"
    );

    // Slot content 지우기(place-slot-content)의 증분 편집도 같고, content는 scene에서 빠집니다.
    let plan = plan_slot_content_delete(state.model(), &library, id(CONTENT), true).unwrap();
    let changes = execute(&mut state, Command::PlaceSlotContent(Box::new(plan)));
    scene_edits(state.model(), &mut current, &changes, options(&library)).unwrap();
    let rebuilt = document_scene_source(state.model(), id(ROOT), options(&library)).unwrap();
    assert_eq!(
        current.source, rebuilt.source,
        "content 지우기 뒤 증분 = 전체"
    );
    assert!(current.source.get(content).is_none());
}

#[test]
fn scene_incremental_edits_match_a_rebuild_for_slot_drop_and_detach() {
    use stellar_commands::detach::{plan_detach, InstanceRestructure, NodePlacement};
    use stellar_core_types::clock::ManualClock;
    use stellar_core_types::ids::StableIdGenerator;
    use stellar_document::layout_record::LayoutRecord;
    let library = library();
    let mut model = document();
    model
        .insert_node(frame(
            id(50),
            id(ROOT),
            key(0x60),
            "밖",
            (300., 300.),
            (30., 12.),
        ))
        .unwrap();
    let mut state = DocumentState::new(model);
    let mut current = document_scene_source(state.model(), id(ROOT), options(&library)).unwrap();

    // 끌어 놓기: root의 Frame 50을 body Slot 끝에 넣는 place-slot-content(runtime plan_slot_drop과 같은 모양).
    let mut record = state.model().record(id(INSTANCE)).unwrap();
    let NodePayload::ComponentInstance(payload) = &mut record.payload else {
        panic!("인스턴스");
    };
    payload.set_slot_override(
        body(),
        SlotOverride::new(vec![id(CONTENT), id(50)]).unwrap(),
    );
    let drop = InstanceRestructure {
        node: id(INSTANCE),
        record,
        layout: LayoutRecord::new(id(INSTANCE)),
        stack: None,
        insert: Vec::new(),
        insert_stacks: Vec::new(),
        insert_layouts: Vec::new(),
        insert_guides: Vec::new(),
        moves: vec![NodePlacement {
            node: id(50),
            parent: id(INSTANCE),
            order_key: key(0x20),
        }],
        remove: Vec::new(),
    };
    let changes = execute(&mut state, Command::PlaceSlotContent(Box::new(drop)));
    assert_valid(state.model(), "끌어 놓기 뒤");
    scene_edits(state.model(), &mut current, &changes, options(&library)).unwrap();
    let rebuilt = document_scene_source(state.model(), id(ROOT), options(&library)).unwrap();
    assert_eq!(current.source, rebuilt.source, "끌어 놓기 뒤 증분 = 전체");
    assert_eq!(
        current.source.parent(SceneKey::node(id(50))),
        Some(SceneKey::virtual_node(&[id(INSTANCE)], id(BODY_HOST)))
    );

    // Detach: 인스턴스가 정의 root 복제 Frame이 되고 content가 복제 slot host 아래로 옮겨집니다.
    let detach = plan_detach(
        state.model(),
        &library,
        &[],
        id(INSTANCE),
        &mut StableIdGenerator::new(9),
        &ManualClock::new(5),
    )
    .unwrap();
    let changes = execute(&mut state, Command::DetachInstance(Box::new(detach)));
    assert_valid(state.model(), "Detach 뒤");
    scene_edits(state.model(), &mut current, &changes, options(&library)).unwrap();
    let rebuilt = document_scene_source(state.model(), id(ROOT), options(&library)).unwrap();
    assert_eq!(current.source, rebuilt.source, "Detach 뒤 증분 = 전체");
    let host = state.model().record(id(CONTENT)).unwrap().parent.unwrap();
    assert_eq!(
        current.source.parent(SceneKey::node(id(CONTENT))),
        Some(SceneKey::node(host))
    );
    assert!(current.skipped.is_empty(), "{:?}", current.skipped);
}

fn snapshot(model: &DocumentModel) -> SemanticSnapshot {
    let mut builder = SnapshotBuilder::new();
    builder
        .add_document(&model.to_records().unwrap())
        .expect("문서 투영");
    builder.finish()
}

#[test]
fn semantic_snapshot_round_trips_instance_children_and_diffs_slot_content_removal() {
    let library = library();
    let model = document();
    let base = snapshot(&model);
    // snapshot → 레코드 역투영이 원래 레코드와 같습니다(Git merge 결과 문서 경로).
    let projection = document_records(&base, id(DOCUMENT)).expect("역투영");
    let mut expected = model.to_records().unwrap();
    expected.sort_canonical();
    assert_eq!(projection.records, expected);
    assert_eq!(projection.instances, [id(INSTANCE)]);

    // Slot content 지우기 diff: content subtree 삭제와 인스턴스 slot override 변경이 함께 나오고, 적용하면 결과와 같습니다.
    let mut state = DocumentState::new(model);
    let plan = plan_slot_content_delete(state.model(), &library, id(CONTENT), false).unwrap();
    execute(&mut state, Command::PlaceSlotContent(Box::new(plan)));
    let target = snapshot(state.model());
    let changes = diff(&base, &target);
    let touched: Vec<StableId> = changes.changes().iter().map(|c| c.object_id()).collect();
    for node in [INSTANCE, CONTENT, INNER] {
        assert!(touched.contains(&id(node)), "{node}: {touched:?}");
    }
    assert_eq!(apply(&base, changes.changes()).as_ref(), Ok(&target));
    let projection = document_records(&target, id(DOCUMENT)).unwrap();
    let mut expected = state.model().to_records().unwrap();
    expected.sort_canonical();
    assert_eq!(projection.records, expected);
}

#[test]
fn web_export_refuses_instances_with_slot_content_instead_of_dropping_it() {
    use stellar_ops::snapshot::open_snapshot;
    use stellar_ops::test_support::{save_documents, scaffold_project};
    use stellar_ops::web_frame::export_web_frame;
    let dir = TempDir::new("instance-slot-web");
    scaffold_project(&dir.path);
    let model = document();
    save_documents(&dir.path, &[&model]);
    let snapshot = open_snapshot(&dir.path).expect("프로젝트 열기");
    let result = export_web_frame(&snapshot, id(DOCUMENT), id(3)).expect("내보내기 결과");
    assert_eq!(result.html, None, "Slot content를 뺀 HTML을 내지 않습니다");
    assert!(
        result.diagnostics.iter().any(|d| d.error
            && d.code == "unsupported-component-instance"
            && d.node == id(INSTANCE)),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn text_source_and_chunks_round_trip_instance_children() {
    let model = document();
    // 텍스트 원본(View v4): 쓰고 다시 열고, 다시 연 문서를 같은 파일로 저장하면 bytes가 같습니다.
    let dir = TempDir::new("instance-slot-text");
    let path = dir.path.join("view.stellar.yaml");
    let written = text_source::save_new(&path, &model).expect("텍스트 원본 쓰기");
    let opened = text_source::open(&path).expect("텍스트 원본 읽기");
    assert_same_records(&model, &opened.model, "텍스트 원본 왕복");
    assert_eq!(
        text_source::save(&path, &opened.model, opened.source_hash).expect("다시 쓰기"),
        written,
        "결정적 쓰기"
    );

    // 저장 chunk.
    let serialized = serialize_document(&model).expect("직렬화");
    let mut chunks = MemoryChunks::new();
    chunks.insert_document(&serialized);
    let loaded = load_document(&serialized.manifest.payload, &mut chunks).expect("다시 열기");
    assert_same_records(&model, &loaded.model, "chunk 왕복");
}
