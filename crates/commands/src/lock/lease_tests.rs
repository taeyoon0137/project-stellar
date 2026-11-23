//! lease 판정 test (XIX.4.2~XIX.4.4, XIX.4.6, XIX.5.2, XIX.5.3).

use super::*;
use crate::command::test_fixtures::{definition, frame, sid};
use crate::command::Command;
use crate::id::{AgentProvenance, CommandId};
use crate::node_ops::{DeleteSubtree, InsertNode, MoveNode};
use crate::request::CommandRequest;
use stellar_core_types::order_key::OrderKey;

/// root(10) ─ 11 ─ 13 ─ 14, root ─ 12.
fn state() -> DocumentState {
    let mut model = DocumentModel::new(definition());
    for (id, parent, key) in [
        (10, None, 0x10),
        (11, Some(10), 0x10),
        (12, Some(10), 0x20),
        (13, Some(11), 0x10),
        (14, Some(13), 0x10),
    ] {
        model.insert_node(frame(id, parent, key)).expect("node");
    }
    DocumentState::with_revision(model, stellar_core_types::revision::DocumentRevision(7))
}

fn lease(id: u64, model: &DocumentModel, roots: &[u64]) -> Lease {
    Lease {
        id: LeaseId(sid(id)),
        subtrees: roots
            .iter()
            .map(|root| SubtreeLock::capture(model, sid(*root)).expect("root"))
            .collect(),
        externals: Vec::new(),
    }
}

fn table(state: &DocumentState, roots: &[u64]) -> LeaseTable {
    let mut table = LeaseTable::new();
    table.insert(lease(900, state.model(), roots)).unwrap();
    table
}

fn user() -> CommandOrigin {
    CommandOrigin::User
}

fn agent(id: u64) -> CommandOrigin {
    CommandOrigin::FutureAgent(AgentProvenance::new(sid(id)))
}

fn tx(state: &DocumentState, origin: CommandOrigin, command: Command) -> CommandTransaction {
    CommandTransaction::single(
        CommandId::from_stable_id(sid(5000)),
        origin,
        CommandRequest::at(state, command),
    )
}

fn key(byte: u8) -> OrderKey {
    OrderKey::from_bytes(vec![byte]).unwrap()
}

fn rename(node: u64) -> Command {
    Command::rename_node(sid(node), "바뀐 이름")
}

fn insert(id: u64, parent: u64) -> Command {
    Command::InsertNode(InsertNode::single(frame(id, Some(parent), 0x30)))
}

fn delete(root: u64) -> Command {
    Command::DeleteSubtree(DeleteSubtree { root: sid(root) })
}

fn move_to(node: u64, parent: u64, byte: u8) -> Command {
    Command::MoveNode(MoveNode {
        node: sid(node),
        new_parent: sid(parent),
        order_key: key(byte),
    })
}

fn records(state: &DocumentState) -> stellar_document::records::DocumentRecords {
    let mut records = state.model().to_records().unwrap();
    records.sort_canonical();
    records
}

/// 실행 결과의 위반 이유(성공이면 `None`). 거부면 문서·revision이 실행 전과 같은지도 확인합니다.
fn run(
    state: &mut DocumentState,
    table: &LeaseTable,
    origin: CommandOrigin,
    command: Command,
) -> Option<ViolationReason> {
    let transaction = tx(state, origin, command);
    let before = records(state);
    let revision = state.revision();
    match execute_leased(state, &transaction, table) {
        Ok(applied) => {
            assert_eq!(applied.changes.new_revision().0, revision.0 + 1);
            None
        }
        Err(LeasedExecutionError::Lease(violation)) => {
            assert_eq!(
                records(state),
                before,
                "거부 뒤 문서가 실행 전과 같아야 합니다"
            );
            assert_eq!(
                state.revision(),
                revision,
                "거부 뒤 revision이 같아야 합니다"
            );
            Some(violation.reason)
        }
        Err(other) => panic!("예상하지 않은 실패: {other}"),
    }
}

#[test]
fn user_operation_matrix_on_leased_subtree() {
    // 검증 기준(XIX.4.3): 잠긴 subtree(13 ─ 14)의 속성 변경, 자식 추가·삭제·이동, 상위(11) 삭제·이동을 거부합니다.
    let leased = ViolationReason::Leased(LeaseId(sid(900)));
    let ancestor = ViolationReason::AncestorOfLease(LeaseId(sid(900)));
    let cases: Vec<(&str, Command, Option<ViolationReason>)> = vec![
        ("root 속성", rename(13), Some(leased)),
        ("자손 속성", rename(14), Some(leased)),
        (
            "자손 표시",
            Command::set_visibility(sid(14), false),
            Some(leased),
        ),
        (
            "root 잠금 토글",
            Command::set_locked(sid(13), true),
            Some(leased),
        ),
        ("root에 자식 추가", insert(20, 13), Some(leased)),
        ("자손에 자식 추가", insert(20, 14), Some(leased)),
        ("자손 삭제", delete(14), Some(leased)),
        ("root 삭제", delete(13), Some(leased)),
        ("자손을 밖으로 이동", move_to(14, 12, 0x10), Some(leased)),
        (
            "밖의 node를 안으로 이동",
            move_to(12, 13, 0x20),
            Some(leased),
        ),
        ("root 이동", move_to(13, 12, 0x10), Some(leased)),
        ("상위 삭제", delete(11), Some(leased)),
        ("상위 이동", move_to(11, 12, 0x10), Some(ancestor)),
        ("상위 순서 변경", move_to(11, 10, 0x30), Some(ancestor)),
        ("상위 이름", rename(11), None),
        ("상위에 다른 자식 추가", insert(21, 11), None),
        ("밖의 node 속성", rename(12), None),
        ("밖의 node 삭제", delete(12), None),
    ];
    for (name, command, expected) in cases {
        let mut state = state();
        let table = table(&state, &[13]);
        assert_eq!(run(&mut state, &table, user(), command), expected, "{name}");
    }
}

#[test]
fn reading_and_selection_are_not_blocked() {
    // 조회·선택은 command가 아니므로 lease와 무관합니다. 잠긴 node도 읽고 판정 정보를 얻습니다.
    let state = state();
    let table = table(&state, &[13]);
    assert!(state.model().record(sid(14)).is_some());
    assert_eq!(
        table.holder(state.model(), sid(14)),
        Some(LeaseId(sid(900)))
    );
    assert_eq!(table.holder(state.model(), sid(11)), None);
    assert_eq!(
        crate::lock::edit_permission(state.model(), sid(14), crate::lock::EditIntent::Edit),
        Ok(()),
        "D8 잠금(locked 필드)과 lease는 별개입니다"
    );
}

#[test]
fn new_descendant_is_locked_automatically() {
    // 검증 기준(XIX.4.2): lease 뒤 추가된 자손도 같은 lease입니다.
    let mut state = state();
    let table = table(&state, &[13]);
    assert_eq!(run(&mut state, &table, agent(900), insert(30, 14)), None);
    assert_eq!(
        table.holder(state.model(), sid(30)),
        Some(LeaseId(sid(900)))
    );
    assert_eq!(
        run(&mut state, &table, user(), rename(30)),
        Some(ViolationReason::Leased(LeaseId(sid(900))))
    );
}

#[test]
fn agent_scope_escape_is_rejected() {
    // 검증 기준(XIX.5.2): lease 범위 밖 StableId를 바꾸는 proposal은 거부합니다.
    let cases: Vec<(&str, Command, Option<ViolationReason>)> = vec![
        ("범위 안 속성", rename(14), None),
        ("범위 안 추가", insert(20, 13), None),
        ("범위 안 자손 삭제", delete(14), None),
        (
            "범위 밖 속성",
            rename(12),
            Some(ViolationReason::OutsideLease),
        ),
        (
            "범위 밖 조상",
            rename(11),
            Some(ViolationReason::OutsideLease),
        ),
        (
            "범위 밖으로 이동",
            move_to(14, 12, 0x10),
            Some(ViolationReason::OutsideLease),
        ),
        (
            "범위 안으로 끌어옴",
            move_to(12, 13, 0x20),
            Some(ViolationReason::OutsideLease),
        ),
        (
            "자기 root 삭제",
            delete(13),
            Some(ViolationReason::RemovesLeaseRoot),
        ),
        (
            "범위 밖 추가",
            insert(20, 12),
            Some(ViolationReason::OutsideLease),
        ),
    ];
    for (name, command, expected) in cases {
        let mut state = state();
        let table = table(&state, &[13]);
        assert_eq!(
            run(&mut state, &table, agent(900), command),
            expected,
            "{name}"
        );
    }
    // 다른 lease 안, 없는 lease.
    let mut state = state();
    let mut table = table(&state, &[13]);
    table.insert(lease(901, state.model(), &[12])).unwrap();
    assert_eq!(
        run(&mut state, &table, agent(901), rename(14)),
        Some(ViolationReason::Leased(LeaseId(sid(900))))
    );
    assert_eq!(
        run(&mut state, &table, agent(999), rename(12)),
        Some(ViolationReason::UnknownLease(LeaseId(sid(999))))
    );
}

#[test]
fn system_origin_is_not_judged_but_reports_touched_leases() {
    let mut state = state();
    let table = table(&state, &[13]);
    let before = table.snapshot(state.model());
    let transaction = tx(&state, CommandOrigin::System, rename(14));
    let applied = execute_leased(&mut state, &transaction, &table).unwrap();
    assert_eq!(
        table.touched_leases(&before, state.model(), &applied.changes),
        [LeaseId(sid(900))].into_iter().collect()
    );
}

#[test]
fn user_edit_without_lease_uses_plain_execution() {
    let mut state = state();
    let table = LeaseTable::new();
    assert_eq!(run(&mut state, &table, user(), rename(13)), None);
}

#[test]
fn overlap_matrix() {
    // 검증 기준(XIX.4.6): 동일·조상·자손 요청은 거부, 겹치지 않는 subtree는 동시에 잠급니다.
    let state = state();
    let model = state.model();
    let cases = [
        (13, 13, true),
        (11, 13, true),
        (13, 11, true),
        (10, 14, true),
        (14, 10, true),
        (13, 12, false),
        (12, 14, false),
        (14, 12, false),
    ];
    for (held, requested, overlaps) in cases {
        let mut table = LeaseTable::new();
        table.insert(lease(900, model, &[held])).unwrap();
        let result = table.insert(lease(901, model, &[requested]));
        assert_eq!(result.is_err(), overlaps, "{held} / {requested}");
        if overlaps {
            assert!(matches!(
                result,
                Err(InsertError::Conflict(Conflict::Subtree { holder, .. })) if holder == LeaseId(sid(900))
            ));
        }
    }
    // 다른 문서의 같은 StableId는 겹치지 않습니다.
    let mut other = SubtreeLock::capture(model, sid(13)).unwrap();
    other.document = sid(77);
    assert!(!other.overlaps(&SubtreeLock::capture(model, sid(13)).unwrap()));
    // 한 요청 안의 겹침·빈 요청.
    assert_eq!(
        lease(902, model, &[11, 14]).check_shape(),
        Err(LeaseShapeError::SelfOverlap {
            first: sid(11),
            second: sid(14)
        })
    );
    assert_eq!(
        lease(902, model, &[]).check_shape(),
        Err(LeaseShapeError::Empty)
    );
}

#[test]
fn external_resources_need_their_own_lock() {
    // 검증 기준(XIX.4.4): 요청하지 않은 외부 resource 편집은 거부하고, node 잠금에서 자동으로 잇지 않습니다.
    let state = state();
    let component = ExternalResource {
        kind: ExternalKind::ComponentDefinition,
        id: sid(600),
    };
    let token = ExternalResource {
        kind: ExternalKind::Token,
        id: sid(601),
    };
    let mut table = table(&state, &[13]);
    assert_eq!(
        table.check_external(component, Actor::Agent(LeaseId(sid(900)))),
        Err(LeaseViolation {
            node: sid(600),
            reason: ViolationReason::ExternalNotLocked(ExternalKind::ComponentDefinition)
        })
    );
    assert_eq!(table.check_external(component, Actor::User), Ok(()));
    let mut with_external = lease(901, state.model(), &[12]);
    with_external.externals.push(component);
    table.insert(with_external).unwrap();
    assert_eq!(
        table.check_external(component, Actor::Agent(LeaseId(sid(901)))),
        Ok(())
    );
    assert_eq!(
        table.check_external(component, Actor::Agent(LeaseId(sid(900)))),
        Err(LeaseViolation {
            node: sid(600),
            reason: ViolationReason::ExternalLeased(LeaseId(sid(901)))
        })
    );
    assert_eq!(
        table.check_external(component, Actor::User),
        Err(LeaseViolation {
            node: sid(600),
            reason: ViolationReason::ExternalLeased(LeaseId(sid(901)))
        })
    );
    assert_eq!(
        table.check_external(token, Actor::Agent(LeaseId(sid(901)))),
        Err(LeaseViolation {
            node: sid(601),
            reason: ViolationReason::ExternalNotLocked(ExternalKind::Token)
        })
    );
    // 같은 resource를 다른 lease가 요청하면 겹침입니다.
    let mut again = lease(902, state.model(), &[]);
    again.externals.push(component);
    assert!(matches!(
        table.insert(again),
        Err(InsertError::Conflict(Conflict::External { .. }))
    ));
    for kind in ExternalKind::ALL {
        assert_eq!(ExternalKind::from_name(kind.name()), Some(kind));
    }
}

#[test]
fn preview_does_not_change_document_state() {
    // 검증 기준(XIX.5.3): 미리보기 전후 revision과 문서가 같습니다.
    let state = state();
    let table = table(&state, &[13]);
    let before = records(&state);
    let revision = state.revision();
    let preview = preview(&state, &tx(&state, agent(900), rename(14)), &table).unwrap();
    assert_eq!(state.revision(), revision);
    assert_eq!(records(&state), before);
    assert_eq!(preview.before, before);
    assert_ne!(preview.after, before);
    assert_eq!(preview.scope, Ok(()));
    assert_eq!(preview.changes.changed_nodes(), vec![sid(14)]);
    // 범위 밖이면 미리보기는 만들되 판정은 위반입니다.
    let escape = preview_fn(&state, &table, rename(12));
    assert_eq!(
        escape.scope.map_err(|v| v.reason),
        Err(ViolationReason::OutsideLease)
    );
    assert_eq!(state.revision(), revision);
}

fn preview_fn(state: &DocumentState, table: &LeaseTable, command: Command) -> LeasePreview {
    preview(state, &tx(state, agent(900), command), table).unwrap()
}
