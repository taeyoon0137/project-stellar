//! BeginAgentEdit·lease 요청 test (XIX.4.1, XIX.4.2, XIX.4.5~XIX.4.7).

use super::*;
use crate::json::{parse, JsonLimits};
use stellar_ops::commands::document::definitions::{
    DocumentDefinition, PageDefinition, ViewDefinition,
};
use stellar_ops::commands::document::frame::FramePayload;
use stellar_ops::commands::document::records::{NodePayload, NodeRecord};
use stellar_ops::commands::lock::coordinator::Gesture;
use stellar_ops::core_types::order_key::OrderKey;
use stellar_ops::test_support::TempDir;

pub(crate) fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

/// 문서 sid(1), View root sid(10). 10 ─ 11 ─ 13 ─ 14, 10 ─ 12.
pub(crate) fn model() -> DocumentModel {
    let mut definition = DocumentDefinition::new(sid(1), String::new());
    let mut page = PageDefinition::new(sid(2), String::new());
    page.add_view(ViewDefinition::new(sid(3), String::new(), sid(10)))
        .unwrap();
    definition.add_page(page).unwrap();
    let mut model = DocumentModel::new(definition);
    for (id, parent, key) in [
        (10, None, 0x10u8),
        (11, Some(10), 0x10),
        (12, Some(10), 0x20),
        (13, Some(11), 0x10),
        (14, Some(13), 0x10),
    ] {
        model
            .insert_node(NodeRecord {
                id: sid(id),
                parent: parent.map(sid),
                order_key: OrderKey::from_bytes(vec![key]).unwrap(),
                visible: true,
                locked: false,
                display_name: format!("n{id}"),
                payload: NodePayload::Frame(FramePayload::default()),
            })
            .unwrap();
    }
    model
}

fn args(text: &str) -> JsonValue {
    parse(text, JsonLimits::default()).unwrap()
}

fn begin(targets: &[u64]) -> BeginAgentEdit {
    BeginAgentEdit {
        document: sid(1),
        targets: targets.iter().map(|t| sid(*t)).collect(),
        externals: Vec::new(),
        description: "버튼 정리".into(),
    }
}

fn service(temp: &TempDir) -> AgentEditLeases {
    AgentEditLeases::new(open_project_coordinator(&temp.path, sid(500), &temp.path).unwrap())
}

#[test]
fn tool_definition_is_attached_only_and_well_formed() {
    assert!(crate::agent_protocol::catalog::tools::valid_tool_name(
        BEGIN_AGENT_EDIT.name
    ));
    assert_eq!(BEGIN_AGENT_EDIT.availability, Availability::AttachedOnly);
    assert_eq!(BEGIN_AGENT_EDIT.grant, Permission::Propose);
    let schema = args(BEGIN_AGENT_EDIT.input_schema);
    assert_eq!(
        schema
            .get("additionalProperties")
            .and_then(JsonValue::as_bool),
        Some(false)
    );
}

#[test]
fn begin_agent_edit_rejects_empty_and_missing_targets() {
    // 검증 기준(XIX.4.1): empty/missing node 거부.
    let doc = sid(1);
    let empty = format!(r#"{{"document":"{doc}","targets":[],"description":"x"}}"#);
    assert_eq!(
        parse_begin_agent_edit(&args(&empty)).unwrap_err().code,
        "empty-targets"
    );
    let missing = format!(
        r#"{{"document":"{doc}","targets":["{}","{}"],"description":"작업"}}"#,
        sid(13),
        sid(99)
    );
    let request = parse_begin_agent_edit(&args(&missing)).unwrap();
    let error = request.resolve(&model(), LeaseId(sid(900))).unwrap_err();
    assert_eq!(
        (error.code, error.kind),
        ("missing-node", ResultKind::NotFound)
    );
    for bad in [
        format!(r#"{{"document":"{doc}","description":"x"}}"#),
        format!(r#"{{"document":"{doc}","targets":["nope"],"description":"x"}}"#),
        format!(
            r#"{{"document":"{doc}","targets":["{0}","{0}"],"description":"x"}}"#,
            sid(13)
        ),
        format!(
            r#"{{"document":"{doc}","targets":["{}"],"description":"  "}}"#,
            sid(13)
        ),
        format!(
            r#"{{"document":"{doc}","targets":["{}"],"description":"x","externals":[{{"kind":"script","id":"{}"}}]}}"#,
            sid(13),
            sid(5)
        ),
        format!(
            r#"{{"document":"{doc}","targets":["{}"],"description":"x","externals":[{{"kind":"token","id":"{}","path":"/"}}]}}"#,
            sid(13),
            sid(5)
        ),
    ] {
        assert_eq!(
            parse_begin_agent_edit(&args(&bad)).unwrap_err().kind,
            ResultKind::InvalidArguments,
            "{bad}"
        );
    }
    // 다른 문서, 서로 겹치는 대상.
    let mut other = begin(&[13]);
    other.document = sid(77);
    assert_eq!(
        other.resolve(&model(), LeaseId(sid(900))).unwrap_err().kind,
        ResultKind::NotFound
    );
    assert_eq!(
        begin(&[11, 14])
            .resolve(&model(), LeaseId(sid(900)))
            .unwrap_err()
            .kind,
        ResultKind::InvalidArguments
    );
    // 정상 요청: 대상 root와 전역 resource.
    let ok = format!(
        r#"{{"document":"{doc}","targets":["{}"],"description":"작업","externals":[{{"kind":"component-definition","id":"{}"}}]}}"#,
        sid(13),
        sid(600)
    );
    let lease = parse_begin_agent_edit(&args(&ok))
        .unwrap()
        .resolve(&model(), LeaseId(sid(900)))
        .unwrap();
    assert_eq!(lease.subtrees[0].root, sid(13));
    assert_eq!(lease.subtrees[0].ancestors, [sid(11), sid(10)]);
    assert_eq!(
        lease.externals,
        [ExternalResource {
            kind: ExternalKind::ComponentDefinition,
            id: sid(600)
        }]
    );
}

#[test]
fn lease_is_created_only_after_approval() {
    // XIX.4.2: 승인 후에 AgentEditLease를 만듭니다. 승인 전에는 아무것도 잠그지 않습니다.
    let temp = TempDir::new("lease-approval");
    let mut leases = service(&temp);
    let model = model();
    let id = LeaseId(sid(900));
    assert_eq!(
        leases.submit(&model, id, &begin(&[13])),
        Ok(LeaseState::AwaitingApproval)
    );
    assert!(leases.table().unwrap().is_empty());
    assert_eq!(leases.approve(id), Ok(LeaseState::Granted));
    let table = leases.table().unwrap();
    assert_eq!(table.holder(&model, sid(14)), Some(id));
    assert_eq!(table.holder(&model, sid(12)), None);
    // 겹치는 다른 세션 요청은 거부되고, 겹치지 않는 요청은 함께 잠급니다(XIX.4.6).
    let other = LeaseId(sid(901));
    leases.submit(&model, other, &begin(&[11])).unwrap();
    assert!(matches!(
        leases.approve(other),
        Err(LeaseServiceError::Conflict(Conflict::Subtree { holder, .. })) if holder == id
    ));
    assert_eq!(leases.state(other), None, "거부된 요청은 버립니다");
    leases.submit(&model, other, &begin(&[12])).unwrap();
    assert_eq!(leases.approve(other), Ok(LeaseState::Granted));
    leases.release(id).unwrap();
    assert_eq!(leases.table().unwrap().len(), 1);
    assert_eq!(leases.release(id), Err(LeaseServiceError::Unknown(id)));
}

#[test]
fn approved_request_waits_for_gesture_end() {
    // 검증 기준(XIX.4.7): drag 중 lock 대기 fixture.
    let temp = TempDir::new("lease-gesture");
    let mut leases = service(&temp);
    let model = model();
    let drag = leases
        .coordinator()
        .begin_gesture(Gesture {
            id: sid(700),
            nodes: vec![SubtreeLock::capture(&model, sid(14)).unwrap()],
        })
        .unwrap();
    let id = LeaseId(sid(900));
    leases.submit(&model, id, &begin(&[13])).unwrap();
    assert!(matches!(
        leases.approve(id),
        Ok(LeaseState::WaitingForGesture(Conflict::Gesture { .. }))
    ));
    assert!(matches!(
        leases.state(id),
        Some(LeaseState::WaitingForGesture(_))
    ));
    assert!(
        leases.table().unwrap().is_empty(),
        "drag 중에는 잠그지 않습니다"
    );
    let retried = leases.retry_waiting();
    assert!(matches!(
        retried.as_slice(),
        [(_, Ok(LeaseState::WaitingForGesture(_)))]
    ));
    drag.end().unwrap();
    assert_eq!(leases.retry_waiting(), vec![(id, Ok(LeaseState::Granted))]);
    assert_eq!(leases.state(id), Some(LeaseState::Granted));
}

#[test]
fn worktrees_are_separate_project_identities() {
    // XIX.4.5: 다른 worktree는 별도 project identity라 lease를 공유하지 않습니다. 같은 프로젝트를 다른 경로
    // (symlink)로 열면 같은 identity입니다.
    let temp = TempDir::new("lease-worktree");
    let main = temp.path.join("main");
    let worktree = temp.path.join("worktree");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::create_dir_all(&worktree).unwrap();
    let link = temp.path.join("main-link");
    std::os::unix::fs::symlink(&main, &link).unwrap();
    let locks = temp.path.join("locks");
    let model = model();
    let mut first =
        AgentEditLeases::new(open_project_coordinator(&locks, sid(500), &main).unwrap());
    let same = AgentEditLeases::new(open_project_coordinator(&locks, sid(500), &link).unwrap());
    let other =
        AgentEditLeases::new(open_project_coordinator(&locks, sid(500), &worktree).unwrap());
    let id = LeaseId(sid(900));
    first.submit(&model, id, &begin(&[13])).unwrap();
    first.approve(id).unwrap();
    assert_eq!(same.table().unwrap().len(), 1);
    assert!(other.table().unwrap().is_empty());
}
