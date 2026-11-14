//! 종단 시나리오: 컴포넌트 정의 resource(ComponentLibrary, IX.1)를 문서와 같은 generation에 저장하고 디스크에서
//! 다시 엽니다.
//!
//! 현재 기준선에는 ComponentLibrary를 chunk로 쓰는 제품 serializer가 없고(`KIND_COMPONENT_LIBRARY` kind만
//! 정의됨), ComponentInstance node는 영속 레코드(`NodePayload`)로 표현할 수 없어 command로 문서에 배치할 수
//! 없습니다. 그래서 이 테스트는 library canonical bytes를 storage chunk kind로 직접 저장해, 저장 transaction·
//! lazy loader·document_io가 문서 chunk와 library chunk를 섞어도 서로의 의미를 바꾸지 않는지 확인합니다.

mod common;

use std::collections::BTreeSet;

use common::builders::{frame, new_document, rectangle, text};
use common::{assert_same_document, key, sid, Editor, TempDir};
use stellar_commands::command::Command;
use stellar_commands::node_ops::InsertNode;
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::text::UserText;
use stellar_core_types::value::Size2;
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, ComponentSlotId, DefinitionNode, DefinitionTree,
};
use stellar_document::component::instance::{ComponentInstancePayload, SlotOverride};
use stellar_document::component::slot::{ComponentSlot, SlotCardinality};
use stellar_document::frame::{CornerRadius, FramePayload};
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::encode_payload;
use stellar_document::records::NodePayload;
use stellar_document::shape::{RectangleParams, ShapeGeometry};
use stellar_document::text::{Paragraph, TextPayload};
use stellar_project::document_io::load::load_document_from_generation;
use stellar_project::document_io::serialize::{serialize_document, SerializedDocument};
use stellar_project::identity::parse_project_id;
use stellar_project::scaffold::write_default_scaffold;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::KIND_COMPONENT_LIBRARY;
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::chunk::pointer::ChunkPointer;
use stellar_storage::generation::lazy::{ChunkReadRecorder, LazyGeneration};

const SPACE: u64 = 51;
const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb969";

fn id(n: u64) -> StableId {
    sid(SPACE, n)
}

#[derive(Default)]
struct Recorder {
    reads: Vec<ChunkId>,
}

impl ChunkReadRecorder for Recorder {
    fn on_read(&mut self, pointer: &ChunkPointer) {
        self.reads.push(pointer.chunk_id);
    }
}

fn definition_node(
    n: u64,
    kind: NodeKind,
    parent: Option<u64>,
    order: u8,
    payload: Vec<u8>,
) -> DefinitionNode {
    DefinitionNode {
        id: id(n),
        kind,
        parent: parent.map(id),
        order_key: key(order),
        visible: true,
        locked: false,
        display_name: format!("정의 {n}"),
        payload,
    }
}

fn frame_payload(width: f32, height: f32) -> Vec<u8> {
    encode_payload(&NodePayload::Frame(FramePayload {
        size: Size2::new(width, height).expect("크기"),
        background: Some(Color::new(ColorSpace::Srgb, 0.9, 0.9, 1.0, 1.0).expect("색")),
        ..FramePayload::default()
    }))
}

fn text_payload(content: &str) -> Vec<u8> {
    encode_payload(&NodePayload::Text(
        TextPayload::new(
            UserText::new(content),
            vec![Paragraph { style: 0 }],
            Vec::new(),
        )
        .expect("텍스트"),
    ))
}

/// Icon(200 ← 201 Shape, 202 badge host)과 Card(100 ← 101 Text, 106 Icon 인스턴스 ← 108 Text)를 담은 library.
fn library() -> ComponentLibrary {
    let icon = ComponentRef::new(id(2));
    let badge = ComponentSlotId::new(id(650));
    let mut library = ComponentLibrary::new();
    let mut icon_tree = DefinitionTree::new(icon);
    icon_tree
        .insert(definition_node(
            200,
            NodeKind::Frame,
            None,
            0x80,
            frame_payload(24.0, 24.0),
        ))
        .expect("icon root");
    icon_tree
        .insert(definition_node(
            201,
            NodeKind::Shape,
            Some(200),
            0x40,
            encode_payload(&NodePayload::Shape(ShapeGeometry::Rectangle(
                RectangleParams {
                    size: Size2::new(20.0, 20.0).expect("크기"),
                    corner_radius: CornerRadius::ZERO,
                },
            ))),
        ))
        .expect("icon shape");
    icon_tree
        .insert(definition_node(
            202,
            NodeKind::Frame,
            Some(200),
            0x60,
            frame_payload(8.0, 8.0),
        ))
        .expect("badge host");
    let mut icon_definition =
        ComponentDefinition::new(id(2), "아이콘".into(), NodeRef::new(id(200)));
    icon_definition.add_slot(badge).expect("slot 선언");
    library
        .insert(icon_definition, icon_tree)
        .expect("icon 등록");
    library
        .add_slot(
            icon,
            ComponentSlot::new(
                badge,
                NodeRef::new(id(202)),
                [NodeKind::Text],
                SlotCardinality::at_most(1),
                Vec::new(),
            ),
        )
        .expect("slot record");

    let mut nested = ComponentInstancePayload::new(icon);
    nested.set_slot_override(badge, SlotOverride::new(vec![id(108)]).expect("override"));
    let mut card_tree = DefinitionTree::new(ComponentRef::new(id(1)));
    card_tree
        .insert(definition_node(
            100,
            NodeKind::Frame,
            None,
            0x80,
            frame_payload(200.0, 120.0),
        ))
        .expect("card root");
    card_tree
        .insert(definition_node(
            101,
            NodeKind::Text,
            Some(100),
            0x20,
            text_payload("카드 제목"),
        ))
        .expect("card text");
    card_tree
        .insert(definition_node(
            106,
            NodeKind::ComponentInstance,
            Some(100),
            0x70,
            nested.encode(),
        ))
        .expect("nested instance");
    card_tree
        .insert(definition_node(
            108,
            NodeKind::Text,
            Some(106),
            0x40,
            text_payload("3"),
        ))
        .expect("badge content");
    library
        .insert(
            ComponentDefinition::new(id(1), "카드".into(), NodeRef::new(id(100))),
            card_tree,
        )
        .expect("card 등록");
    library
}

fn dirty(
    document: &SerializedDocument,
    previous: Option<&SerializedDocument>,
) -> Vec<LogicalChunkState> {
    document
        .chunks()
        .into_iter()
        .map(|chunk| {
            let same = previous.is_some_and(|p| {
                p.chunks()
                    .iter()
                    .any(|c| c.chunk_id == chunk.chunk_id && c.payload == chunk.payload)
            });
            if same {
                LogicalChunkState::Clean {
                    chunk_id: chunk.chunk_id,
                }
            } else {
                LogicalChunkState::Dirty {
                    chunk_id: chunk.chunk_id,
                    kind: chunk.kind,
                    schema_version: chunk.schema_version,
                    payload: chunk.payload.clone(),
                }
            }
        })
        .collect()
}

#[test]
fn component_library_chunk_round_trips_beside_documents() {
    let sandbox = TempDir::new("components");
    let root = sandbox.path.join("project");
    std::fs::create_dir_all(&root).expect("프로젝트 폴더");
    write_default_scaffold(&root, "컴포넌트 프로젝트", PROJECT_ID).expect("scaffold");
    let project_id = parse_project_id(PROJECT_ID).expect("project id");

    // 1. library canonical bytes는 decode 왕복(재검증 포함)이 byte-identical입니다.
    let library = library();
    let library_bytes = library.encode();
    let decoded = ComponentLibrary::decode(&library_bytes).expect("library decode");
    assert_eq!(decoded, library);
    assert_eq!(decoded.encode(), library_bytes);
    assert_eq!(library.owner_of(id(108)), Some(ComponentRef::new(id(1))));

    // 2. command로 만든 문서와 library chunk를 한 generation에 저장합니다.
    let mut editor = Editor::new(
        new_document(
            id(10),
            "화면",
            &[(id(11), "페이지", vec![(id(12), "홈", id(20))])],
        ),
        SPACE,
    );
    editor.apply(
        vec![Command::InsertNode(InsertNode {
            nodes: vec![
                frame(
                    id(21),
                    id(20),
                    key(0x10),
                    "목록",
                    (0.0, 0.0),
                    (400.0, 300.0),
                ),
                text(id(22), id(21), key(0x10), "제목", "추천"),
                rectangle(id(23), id(21), key(0x20), "구분", 400.0, 1.0),
            ],
            modifier_stacks: Vec::new(),
        })],
        "문서 구성",
    );
    let library_chunk = ChunkId::from_stable_id(id(9_000));
    let library_state = LogicalChunkState::Dirty {
        chunk_id: library_chunk,
        kind: KIND_COMPONENT_LIBRARY,
        schema_version: 1,
        payload: library_bytes.clone(),
    };
    let first = serialize_document(editor.model()).expect("직렬화");
    let mut states = dirty(&first, None);
    states.push(library_state);
    common::save_states(&root, project_id, &states, vec![id(10)]);

    // 3. lazy 열기는 DocumentManifest만, 문서 로드는 문서 chunk만 읽습니다(library chunk 미접근).
    let mut open_reads = Recorder::default();
    let lazy = LazyGeneration::open(&root, project_id, &mut open_reads).expect("열기");
    assert_eq!(open_reads.reads, vec![first.manifest.chunk_id]);
    let mut reads = Recorder::default();
    let loaded = load_document_from_generation(&lazy, id(10), &mut reads).expect("문서 로드");
    assert!(!reads.reads.contains(&library_chunk));
    assert_same_document(editor.model(), &loaded.model, "문서 재열기");
    let mut library_reads = Recorder::default();
    let stored = lazy
        .read_chunk_payload(library_chunk, &mut library_reads)
        .expect("library chunk 읽기");
    assert_eq!(library_reads.reads, vec![library_chunk]);
    assert_eq!(ComponentLibrary::decode(&stored).expect("decode"), library);

    // 4. 문서만 편집해 증분 저장하면 library chunk는 clean으로 재사용되어 같은 content hash를 유지합니다.
    editor.apply(vec![Command::rename_node(id(22), "오늘의 추천")], "편집");
    let second = serialize_document(editor.model()).expect("직렬화");
    let mut states = dirty(&second, Some(&first));
    states.push(LogicalChunkState::Clean {
        chunk_id: library_chunk,
    });
    let base_pointer = lazy
        .manifest()
        .chunk_pointers
        .iter()
        .find(|p| p.chunk_id == library_chunk)
        .copied()
        .expect("library pointer");
    let plan = common::save_states(&root, project_id, &states, vec![id(10)]);
    let written: BTreeSet<ChunkId> = plan.dirty_chunks.iter().map(|w| w.chunk_id).collect();
    assert!(
        !written.contains(&library_chunk),
        "library chunk 재기록 없음"
    );
    let reopened = LazyGeneration::open(&root, project_id, &mut Recorder::default()).expect("열기");
    let pointer = reopened
        .manifest()
        .chunk_pointers
        .iter()
        .find(|p| p.chunk_id == library_chunk)
        .copied()
        .expect("library pointer");
    assert_eq!(pointer, base_pointer);
    let stored = reopened
        .read_chunk_payload(library_chunk, &mut Recorder::default())
        .expect("읽기");
    assert_eq!(stored, library_bytes);
    let loaded = load_document_from_generation(&reopened, id(10), &mut Recorder::default())
        .expect("문서 로드");
    assert_same_document(editor.model(), &loaded.model, "증분 저장 재열기");
}
