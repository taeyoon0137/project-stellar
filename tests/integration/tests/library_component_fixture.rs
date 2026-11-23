//! Button·TextInput·Modal·List·Radio 합성 예제 component library fixture의 문서 검증 (IX.9.1).
//!
//! document test-support의 합성 fixture(`component_library_fixture`·`fixture_scene`)를 **영속 문서 레코드**
//! (`NodePayload::ComponentInstance`)로 옮깁니다. 같은 레코드에서 읽은 instance payload는 fixture library의
//! PropertyOverride 규칙(IX.2.3, `check_payload_overrides`)과 instance 펼침(IX.3, `InstanceExpansion::walk`)을 통과합니다.
//! 제품 문서 validator(`validate_document`) 0 오류 확인은 validator 결함 때문에 아직 실패합니다(아래 ignore 사유).

use stellar_core_types::ids::StableId;
use stellar_core_types::text::UserText;
use stellar_document::component::expansion::{InstanceExpansion, SceneIndex, ScenePayloads};
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::component::override_rules::check_payload_overrides;
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::encode_payload;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::test_support::component_fixtures::{
    component_library_fixture, components, fixture_scene, sid, SCENE_ROOT,
};
use stellar_document::text::{Paragraph, TextPayload};
use stellar_document::validate::validate_document;

/// fixture scene(NodeColumns + payload bytes)을 영속 문서 레코드 모델로 옮깁니다.
fn persisted_document() -> DocumentModel {
    let (columns, payloads) = fixture_scene().expect("fixture scene");
    let mut definition = DocumentDefinition::new(sid(8000), "IX.9.1 문서".into());
    let mut page = PageDefinition::new(sid(8001), "페이지".into());
    page.add_view(ViewDefinition::new(
        sid(8002),
        "View".into(),
        sid(SCENE_ROOT),
    ))
    .unwrap();
    definition.add_page(page).unwrap();
    let mut model = DocumentModel::new(definition);
    // 부모가 먼저 오도록 깊이 순으로 넣습니다.
    let mut fields: Vec<_> = columns
        .iter_live()
        .filter_map(|handle| columns.get(handle))
        .collect();
    let depth = |f: &stellar_document::node_columns::NodeFields| {
        let mut d = 0;
        let mut parent = f.parent;
        while let Some(p) = parent {
            d += 1;
            parent = columns.get(p).and_then(|f| f.parent);
        }
        d
    };
    fields.sort_by_key(|f| (depth(f), f.stable_id));
    for f in fields {
        let payload = match f.kind {
            NodeKind::Frame => NodePayload::Frame(FramePayload::default()),
            NodeKind::ComponentInstance => NodePayload::ComponentInstance(
                ComponentInstancePayload::decode(&payloads[&f.stable_id])
                    .expect("instance payload"),
            ),
            NodeKind::Text => NodePayload::Text(
                TextPayload::new(UserText::new("본문"), vec![Paragraph { style: 0 }], vec![])
                    .unwrap(),
            ),
            other => panic!("fixture에 없는 종류 {other:?}"),
        };
        model
            .insert_node(NodeRecord {
                id: f.stable_id,
                parent: f.parent.and_then(|p| columns.get(p)).map(|p| p.stable_id),
                order_key: f.order_key,
                visible: f.visible,
                locked: f.locked,
                display_name: f.display_name.clone(),
                payload,
            })
            .expect("레코드 삽입");
    }
    model
}

#[test]
fn persisted_fixture_instances_follow_library_rules() {
    let fixture = component_library_fixture().expect("fixture library");
    let model = persisted_document();
    let records = model.to_records().expect("영속 레코드");

    // 다섯 컴포넌트 instance가 모두 영속 ComponentInstance 레코드입니다.
    let instances: Vec<StableId> = records
        .nodes
        .iter()
        .filter(|r| matches!(r.payload, NodePayload::ComponentInstance(_)))
        .map(|r| r.id)
        .collect();
    assert_eq!(instances.len(), components().len());

    // 같은 레코드에서 다시 만든 scene으로 override 규칙·instance 펼침을 확인합니다(fixture library 기준).
    let mut payloads = ScenePayloads::new();
    for record in &records.nodes {
        payloads.insert(record.id, encode_payload(&record.payload));
    }
    let index = SceneIndex::build(model.columns(), &payloads);
    for (name, component) in components() {
        let record = records
            .nodes
            .iter()
            .find(|r| {
                matches!(&r.payload, NodePayload::ComponentInstance(p) if p.component() == component)
            })
            .unwrap_or_else(|| panic!("{name} instance"));
        let NodePayload::ComponentInstance(payload) = &record.payload else {
            unreachable!()
        };
        let entry = fixture.library.get(component).expect("정의");
        check_payload_overrides(entry, payload, record.id)
            .unwrap_or_else(|e| panic!("{name} override: {e:?}"));
        let expansion = InstanceExpansion::new(&fixture.library, &index, record.id)
            .unwrap_or_else(|e| panic!("{name} 펼침: {e:?}"));
        let walk = expansion
            .walk()
            .unwrap_or_else(|e| panic!("{name} walk: {e:?}"));
        assert!(walk.len() > 1, "{name} 정의 tree가 펼쳐짐");
    }
}

/// 계획서 IX.9.1 검증 열 "document validator 0 오류".
///
/// 지금은 실패합니다: 제품 validator(`validate_document`, VI.10.1)는 부모가 Frame·Fragment일 때만 자식을 허용해서, Modal
/// instance가 body slot에 투영하는 instance 소유 Text(IX.3 slot override)를 `ParentNotContainer`로 거부합니다. validator가
/// ComponentInstance의 slot content 자식을 받도록 고쳐지면(document crate, 이 작업 소유 밖) `#[ignore]`를 지웁니다.
#[test]
#[ignore = "document validator가 instance 소유 slot content를 ParentNotContainer로 거부(IX.9.1 보고서 finding)"]
fn composed_component_library_fixture_passes_document_validator() {
    let records = persisted_document().to_records().expect("영속 레코드");
    let diagnostics = validate_document(&records).expect("document validator 0 오류");
    assert!(
        diagnostics.unresolved.is_empty(),
        "{:?}",
        diagnostics.unresolved
    );
}
