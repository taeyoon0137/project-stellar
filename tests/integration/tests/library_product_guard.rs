//! 제품 적용 경로의 library 읽기 전용 사전 검사 종단 시험 (XV.5.3, ADR-0127 결정 3).
//!
//! fixture(`library_common::product`): 사용 프로젝트 문서에 library ds의 Card instance가 있고, 제품 OS 기본 library cache에
//! ds가 있습니다. Card instance의 override를 바꾸는 실제 command(`SwapInstance`, 캔버스 override 편집이 쓰는 command)와 override를
//! 가진 새 Card instance 삽입(`InsertNode`)을
//! headless 적용(`stellar_ops::apply::apply`: CLI `stellar apply`·MCP 편집 경로)과 창 session 적용
//! (`ProjectWindowSession::apply`)에 넣어, 비공개·타입이 다른 property override는 적용 전에 거부되고(아무것도 쓰거나 기록하지
//! 않음) 공개 property override는 성공하는지 확인합니다.

#![cfg(unix)] // Windows 경로는 보류 상태라 검증하지 않습니다(ADR-0006).

mod e2e_common;
mod library_common;

use e2e_common::TempDir;
use library_common::product::*;
use library_common::*;
use stellar_commands::codec::encode_command;
use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::node_ops::InsertNode;
use stellar_commands::request::CommandRequest;
use stellar_commands::swap::SwapInstance;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::order_key::OrderKey;
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::model::DocumentModel;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_ops::apply::{apply, ApplyError, ApplyMode, ApplyRequest, CommandInput};
use stellar_ops::library::LibraryGuardRejection;
use stellar_ops::lock::LockPolicy;
use stellar_ops::snapshot::open_snapshot;
use stellar_runtime::window_session::WindowSessionEditError;

fn swap(payload: ComponentInstancePayload) -> Command {
    Command::SwapInstance(SwapInstance {
        node: sid(SPACE, INSTANCE),
        payload,
    })
}

fn swap_input(payload: ComponentInstancePayload) -> CommandInput {
    let command = swap(payload);
    CommandInput::Canonical {
        kind: command.kind().name().to_string(),
        bytes: encode_command(&command),
    }
}

/// root frame(fixture `903`)의 마지막 자식 뒤에 `payload` instance 하나를 넣는 command.
fn insert(model: &DocumentModel, id: u64, payload: ComponentInstancePayload) -> CommandInput {
    let parent = sid(SPACE, 903);
    let last = model
        .children_in_order(parent)
        .last()
        .and_then(|child| model.record(*child))
        .map(|record| record.order_key);
    let command = Command::InsertNode(InsertNode::single(NodeRecord {
        id: sid(SPACE, id),
        parent: Some(parent),
        order_key: OrderKey::between(last.as_ref(), None),
        visible: true,
        locked: false,
        display_name: "Card".into(),
        payload: NodePayload::ComponentInstance(payload),
    }));
    CommandInput::Canonical {
        kind: command.kind().name().to_string(),
        bytes: encode_command(&command),
    }
}

#[test]
fn library_override_edits_are_judged_before_headless_and_window_apply() {
    let dir = TempDir::new("library-product-guard");
    let Some(project) = product_project(&dir) else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    // 창 session(열기가 로컬 cache를 맞춰 쓸 수 있으므로 먼저 엽니다).
    let mut session = open_window(&project);
    assert_eq!(session.libraries().instance().snapshots().count(), 1);

    let snapshot = open_snapshot(&project).unwrap();
    let document = snapshot.documents()[0].id;
    let base = snapshot.revision();
    let request = |payload| ApplyRequest {
        base_revision: base,
        document,
        commands: vec![swap_input(payload)],
    };

    // headless: 비공개 property override는 적용 전에 거부되고 아무것도 쓰지 않습니다.
    match apply(
        &project,
        &request(with_override(SECRET, text_value("몰래"))),
        ApplyMode::Write,
        &LockPolicy::None,
    ) {
        Err(ApplyError::Rejected { index, reason }) => {
            assert_eq!(index, 0);
            assert!(reason.contains("공개 property가 아니라"), "{reason}");
        }
        other => panic!("비공개 override는 거부되어야 함: {other:?}"),
    }
    assert_eq!(
        open_snapshot(&project).unwrap().revision(),
        base,
        "쓰지 않음"
    );
    assert!(instance_overrides(&project, document).is_empty());
    // 타입이 다른 공개 property override도 거부됩니다(dry-run도 같은 판정).
    assert!(matches!(
        apply(
            &project,
            &request(with_override(TITLE, number_value(3.0))),
            ApplyMode::DryRun,
            &LockPolicy::None
        ),
        Err(ApplyError::Rejected { index: 0, .. })
    ));
    // override를 가진 새 library instance 삽입(InsertNode)도 같은 판정입니다: 비공개 override는 거부, 공개 override는 통과.
    let model = snapshot.document_model(document).unwrap();
    let inserted = |payload| ApplyRequest {
        base_revision: base,
        document,
        commands: vec![insert(&model, 500, payload)],
    };
    match apply(
        &project,
        &inserted(with_override(SECRET, text_value("몰래"))),
        ApplyMode::DryRun,
        &LockPolicy::None,
    ) {
        Err(ApplyError::Rejected { index, reason }) => {
            assert_eq!(index, 0);
            assert!(reason.contains("공개 property가 아니라"), "{reason}");
        }
        other => panic!("비공개 override instance 삽입은 거부되어야 함: {other:?}"),
    }
    apply(
        &project,
        &inserted(with_override(TITLE, text_value("삽입"))),
        ApplyMode::DryRun,
        &LockPolicy::None,
    )
    .expect("공개 override instance 삽입은 통과");

    // 공개 property override는 성공하고 저장됩니다.
    let outcome = apply(
        &project,
        &request(with_override(TITLE, text_value("저장"))),
        ApplyMode::Write,
        &LockPolicy::None,
    )
    .unwrap();
    assert_ne!(outcome.revision, base);
    assert_eq!(
        instance_overrides(&project, document),
        [(sid(SPACE, TITLE), format!("{:?}", text_value("저장")))]
    );

    // 창 session: 같은 판정이 history 기록 전에 거부합니다. 문서·history는 그대로입니다.
    let before = session.revision();
    let transaction = |payload| CommandTransaction {
        id: CommandId::from_stable_id(sid(SPACE, 990)),
        origin: CommandOrigin::User,
        requests: vec![CommandRequest::new(before, swap(payload))],
    };
    let refused = session
        .apply(&transaction(with_override(SECRET, text_value("몰래"))), 0)
        .unwrap_err();
    match &refused {
        WindowSessionEditError::Library(refused) => {
            assert_eq!(refused.index, 0);
            assert!(matches!(
                refused.rejection,
                LibraryGuardRejection::Library(_)
            ));
        }
        other => panic!("library 거부여야 함: {other:?}"),
    }
    assert_eq!(session.revision(), before, "거부는 문서를 바꾸지 않음");
    assert!(session.history().history().entries().is_empty());
    // 공개 property override는 기록됩니다.
    let changes = session
        .apply(&transaction(with_override(TITLE, text_value("창"))), 0)
        .unwrap();
    assert!(!changes.is_empty());
    assert_ne!(session.revision(), before);
    assert_eq!(session.history().history().entries().len(), 1);
}
