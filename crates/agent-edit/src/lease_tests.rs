//! lease 대상 확인·요청 관리 test (XIX.4.1, XIX.4.2, XIX.4.5~XIX.4.7).

use super::*;
use stellar_ops::commands::document::definitions::{
    DocumentDefinition, PageDefinition, ViewDefinition,
};
use stellar_ops::commands::document::frame::FramePayload;
use stellar_ops::commands::document::records::{NodePayload, NodeRecord};
use stellar_ops::commands::lock::coordinator::Gesture;
use stellar_ops::commands::lock::lease::ExternalKind;
use stellar_ops::core_types::order_key::OrderKey;
use stellar_ops::test_support::TempDir;

fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

/// 문서 sid(1), View root sid(10). 10 ─ 11 ─ 13 ─ 14, 10 ─ 12.
fn model() -> DocumentModel {
    model_with(&[
        (10, None, 0x10u8),
        (11, Some(10), 0x10),
        (12, Some(10), 0x20),
        (13, Some(11), 0x10),
        (14, Some(13), 0x10),
    ])
}

/// 문서 sid(1), View root sid(10)에 (id, 부모, 순서 byte) node를 차례로 넣습니다.
fn model_with(nodes: &[(u64, Option<u64>, u8)]) -> DocumentModel {
    let mut definition = DocumentDefinition::new(sid(1), String::new());
    let mut page = PageDefinition::new(sid(2), String::new());
    page.add_view(ViewDefinition::new(sid(3), String::new(), sid(10)))
        .unwrap();
    definition.add_page(page).unwrap();
    let mut model = DocumentModel::new(definition);
    for &(id, parent, key) in nodes {
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

fn begin(targets: &[u64]) -> LeaseTargets {
    LeaseTargets {
        document: sid(1),
        targets: targets.iter().map(|t| sid(*t)).collect(),
        externals: Vec::new(),
    }
}

fn service(temp: &TempDir) -> AgentEditLeases {
    AgentEditLeases::new(open_project_coordinator(&temp.path, sid(500), &temp.path).unwrap())
}

#[test]
fn targets_are_resolved_against_the_document() {
    // 검증 기준(XIX.4.1): missing node·다른 문서·서로 겹치는 대상 거부. 형식 오류(빈 대상 등)는 브리지 해석기가
    // 거부합니다(`stellar_mcp::lease` test).
    let error = begin(&[13, 99])
        .resolve(&model(), LeaseId(sid(900)))
        .unwrap_err();
    assert_eq!(
        (error.code(), error.kind()),
        ("missing-node", ResultKind::NotFound)
    );
    let mut other = begin(&[13]);
    other.document = sid(77);
    assert_eq!(
        other
            .resolve(&model(), LeaseId(sid(900)))
            .unwrap_err()
            .kind(),
        ResultKind::NotFound
    );
    assert_eq!(
        begin(&[11, 14])
            .resolve(&model(), LeaseId(sid(900)))
            .unwrap_err()
            .kind(),
        ResultKind::InvalidArguments
    );
    // 정상 요청: 대상 root와 전역 resource.
    let mut ok = begin(&[13]);
    ok.externals.push(ExternalResource {
        kind: ExternalKind::ComponentDefinition,
        id: sid(600),
    });
    let lease = ok.resolve(&model(), LeaseId(sid(900))).unwrap();
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
    assert_eq!(leases.approve(&model, id), Ok(LeaseState::Granted));
    let table = leases.table().unwrap();
    assert_eq!(table.holder(&model, sid(14)), Some(id));
    assert_eq!(table.holder(&model, sid(12)), None);
    // 겹치는 다른 세션 요청은 거부되고, 겹치지 않는 요청은 함께 잠급니다(XIX.4.6).
    let other = LeaseId(sid(901));
    leases.submit(&model, other, &begin(&[11])).unwrap();
    assert!(matches!(
        leases.approve(&model, other),
        Err(LeaseServiceError::Conflict(Conflict::Subtree { holder, .. })) if holder == id
    ));
    assert_eq!(leases.state(other), None, "거부된 요청은 버립니다");
    leases.submit(&model, other, &begin(&[12])).unwrap();
    assert_eq!(leases.approve(&model, other), Ok(LeaseState::Granted));
    leases.release(id).unwrap();
    assert_eq!(leases.table().unwrap().len(), 1);
    assert_eq!(leases.release(id), Err(LeaseServiceError::Unknown(id)));
}

#[test]
fn approval_resolves_targets_against_the_current_document() {
    // 제출과 승인 사이에 대상이 옮겨지면 승인 시점 조상으로 잠그고, 사라졌으면 요청을 버립니다.
    let temp = TempDir::new("lease-approve-current");
    let mut leases = service(&temp);
    let id = LeaseId(sid(900));
    leases.submit(&model(), id, &begin(&[13])).unwrap();
    // 13을 12 아래로 옮긴 문서.
    let moved = model_with(&[
        (10, None, 0x10),
        (11, Some(10), 0x10),
        (12, Some(10), 0x20),
        (13, Some(12), 0x10),
        (14, Some(13), 0x10),
    ]);
    assert_eq!(leases.approve(&moved, id), Ok(LeaseState::Granted));
    let table = leases.table().unwrap();
    assert_eq!(
        table.get(id).unwrap().subtrees[0].ancestors,
        [sid(12), sid(10)]
    );
    // 겹침 판정도 새 조상으로 합니다: 12를 요청하면 겹치고, 11은 겹치지 않습니다.
    let other = LeaseId(sid(901));
    leases.submit(&moved, other, &begin(&[12])).unwrap();
    assert!(matches!(
        leases.approve(&moved, other),
        Err(LeaseServiceError::Conflict(Conflict::Subtree { .. }))
    ));
    leases.submit(&moved, other, &begin(&[11])).unwrap();
    assert_eq!(leases.approve(&moved, other), Ok(LeaseState::Granted));
    // 대상이 사라진 문서에서 승인하면 missing-node로 거부하고 요청을 버립니다.
    let third = LeaseId(sid(902));
    let without_14 = model_with(&[(10, None, 0x10), (11, Some(10), 0x10)]);
    let mut fresh = AgentEditLeases::new(
        open_project_coordinator(&temp.path.join("other"), sid(500), &temp.path).unwrap(),
    );
    fresh.submit(&model(), third, &begin(&[14])).unwrap();
    match fresh.approve(&without_14, third) {
        Err(LeaseServiceError::Target(error)) => assert_eq!(error.code(), "missing-node"),
        other => panic!("예상하지 않은 결과: {other:?}"),
    }
    assert_eq!(fresh.state(third), None);
    assert!(fresh.table().unwrap().is_empty());
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
        leases.approve(&model, id),
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
    let retried = leases.retry_waiting(&model);
    assert!(matches!(
        retried.as_slice(),
        [(_, Ok(LeaseState::WaitingForGesture(_)))]
    ));
    drag.end().unwrap();
    assert_eq!(
        leases.retry_waiting(&model),
        vec![(id, Ok(LeaseState::Granted))]
    );
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
    first.approve(&model, id).unwrap();
    assert_eq!(same.table().unwrap().len(), 1);
    assert!(other.table().unwrap().is_empty());
}
