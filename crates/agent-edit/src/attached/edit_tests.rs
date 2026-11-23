//! attached 편집 경로 test (XIX.5.1~XIX.5.3).

use super::*;
use stellar_ops::apply::CommandInput;
use stellar_ops::commands::document::definitions::{
    DocumentDefinition, PageDefinition, ViewDefinition,
};
use stellar_ops::commands::document::frame::FramePayload;
use stellar_ops::commands::document::model::DocumentModel;
use stellar_ops::commands::document::records::{NodePayload, NodeRecord};
use stellar_ops::commands::lock::lease::{Lease, SubtreeLock, ViolationReason};
use stellar_ops::core_types::order_key::OrderKey;
use stellar_ops::revision::ProjectRevision;

fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

/// 문서 sid(1), View root sid(10). 10 ─ 11 ─ 13 ─ 14, 10 ─ 12.
fn model() -> DocumentModel {
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

const LEASE: u64 = 900;

fn state() -> DocumentState {
    DocumentState::with_revision(model(), DocumentRevision(4))
}

fn table(state: &DocumentState) -> LeaseTable {
    let mut table = LeaseTable::new();
    table
        .insert(Lease {
            id: LeaseId(sid(LEASE)),
            subtrees: vec![SubtreeLock::capture(state.model(), sid(13)).unwrap()],
            externals: Vec::new(),
        })
        .unwrap();
    table
}

fn request(document: StableId, commands: Vec<CommandInput>) -> ApplyRequest {
    ApplyRequest {
        base_revision: ProjectRevision::Unsaved,
        document,
        commands,
    }
}

fn proposal(
    state: &DocumentState,
    commands: Vec<CommandInput>,
) -> Result<CommandTransaction, AttachedEditError> {
    proposal_transaction(
        &request(sid(1), commands),
        state,
        LeaseId(sid(LEASE)),
        CommandId::from_stable_id(sid(5000)),
    )
}

fn rename(node: u64) -> Vec<CommandInput> {
    vec![CommandInput::Rename {
        node: sid(node),
        name: "새 이름".into(),
    }]
}

fn records(state: &DocumentState) -> stellar_ops::commands::document::records::DocumentRecords {
    let mut records = state.model().to_records().unwrap();
    records.sort_canonical();
    records
}

fn no_definitions<'a>() -> Definitions<'a> {
    Definitions::Failed("정의 없음")
}

#[test]
fn proposal_uses_closed_enum_and_lease_origin() {
    // XIX.5.1: 해석 결과는 닫힌 enum 변환(`decode_commands`)만 거치고, 결과는 lease 편집 세션 출처의 transaction입니다.
    // 모르는 command 종류는 브리지 해석기(`stellar_mcp::proposal`)가 `unknown-command`로 거부합니다.
    let state = state();
    let error = proposal(&state, Vec::new()).unwrap_err();
    assert_eq!(
        (
            error.kind(),
            matches!(
                error,
                AttachedEditError::Input {
                    code: "invalid-transaction",
                    ..
                }
            )
        ),
        (ResultKind::InvalidArguments, true)
    );
    let transaction = proposal(&state, rename(14)).unwrap();
    assert_eq!(
        transaction.origin,
        CommandOrigin::FutureAgent(AgentProvenance::new(sid(LEASE)))
    );
    assert_eq!(transaction.requests[0].base_revision, state.revision());
    let other = request(sid(77), rename(14));
    assert_eq!(
        proposal_transaction(
            &other,
            &state,
            LeaseId(sid(LEASE)),
            CommandId::from_stable_id(sid(1)),
        )
        .unwrap_err(),
        AttachedEditError::ForeignDocument(sid(77))
    );
}

#[test]
fn stale_base_revision_is_refused() {
    // ADR-0093 Consequences: attached proposal의 base_revision(저장 generation 기준)은 앱이 창의 generation과 대조합니다.
    let decoded = decode_proposal(&request(sid(1), rename(14))).unwrap();
    assert_eq!(decoded.check_base(ProjectRevision::Unsaved), Ok(()));
    let saved = ProjectRevision::Saved {
        generation: 2,
        hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .parse()
            .unwrap(),
    };
    let error = decoded.check_base(saved).unwrap_err();
    assert_eq!(
        error,
        AttachedEditError::StaleBase {
            expected: ProjectRevision::Unsaved,
            found: saved,
        }
    );
    assert_eq!(error.kind(), ResultKind::Stale);
}

#[test]
fn preview_has_semantic_diff_and_validation_without_changing_state() {
    // 검증 기준(XIX.5.3): preview 전후 revision 동일.
    let state = state();
    let table = table(&state);
    let before = records(&state);
    let transaction = proposal(&state, rename(14)).unwrap();
    let preview = preview(&state, &transaction, &table, no_definitions).unwrap();
    assert_eq!(state.revision(), DocumentRevision(4));
    assert_eq!(preview.revision, DocumentRevision(4));
    assert_eq!(records(&state), before);
    assert_eq!(preview.diff.entries.len(), 1);
    assert_eq!(preview.diff.entries[0].object, sid(14));
    assert!(preview.findings.is_empty(), "{:?}", preview.findings);
    assert_eq!(preview.scope, Ok(()));
    assert_eq!(preview.changes.changed_nodes(), vec![sid(14)]);
}

#[test]
fn scope_escape_fails_in_preview_and_apply() {
    // 검증 기준(XIX.5.2): scope escape fixture 실패.
    let mut state = state();
    let table = table(&state);
    let before = records(&state);
    let transaction = proposal(&state, rename(12)).unwrap();
    let preview = preview(&state, &transaction, &table, no_definitions).unwrap();
    assert_eq!(
        preview.scope.map_err(|v| v.reason),
        Err(ViolationReason::OutsideLease)
    );
    let error = apply_approved(&mut state, &transaction, &table).unwrap_err();
    assert_eq!(error.kind(), ResultKind::OutOfScope);
    assert_eq!(records(&state), before);
    assert_eq!(state.revision(), DocumentRevision(4));
    // 범위 안 proposal은 적용됩니다.
    let inside = proposal(&state, rename(14)).unwrap();
    let applied = apply_approved(&mut state, &inside, &table).unwrap();
    assert_eq!(applied.changes.new_revision(), DocumentRevision(5));
    // lease가 끝난 뒤의 proposal은 거부합니다.
    let ended = LeaseTable::new();
    let late = proposal(&state, rename(14)).unwrap();
    assert!(matches!(
        apply_approved(&mut state, &late, &ended),
        Err(AttachedEditError::OutOfScope(LeaseViolation {
            reason: ViolationReason::UnknownLease(_),
            ..
        }))
    ));
}
