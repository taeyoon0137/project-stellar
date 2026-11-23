//! 에이전트 출처·세션·되돌리기 test 공용 fixture (XIX.6, XIX.7).
//!
//! mcp-core는 history crate에 닿지 않으므로(crate graph) 창 history 연결 지점은 같은 trait를 구현한 test 대역
//! [`TestHistory`]로 대신합니다. 실제 창 history 구현(`stellar_history::agent::WindowAgentHistory`)은 history crate
//! test가 검증합니다.

use stellar_ops::commands::command::Command;
use stellar_ops::commands::document::definitions::{
    DocumentDefinition, PageDefinition, ViewDefinition,
};
use stellar_ops::commands::document::frame::FramePayload;
use stellar_ops::commands::document::model::DocumentModel;
use stellar_ops::commands::document::records::{NodePayload, NodeRecord};
use stellar_ops::commands::id::{AgentProvenance, CommandId, CommandOrigin};
use stellar_ops::commands::lock::lease::LeaseId;
use stellar_ops::commands::request::CommandRequest;
use stellar_ops::commands::state::DocumentState;
use stellar_ops::commands::transaction::CommandTransaction;
use stellar_ops::core_types::ids::StableId;
use stellar_ops::core_types::order_key::OrderKey;
use stellar_ops::core_types::revision::DocumentRevision;

use crate::permission::AgentSessionId;

pub(crate) fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

pub(crate) fn frame(id: u64, parent: Option<u64>, key: u8) -> NodeRecord {
    NodeRecord {
        id: sid(id),
        parent: parent.map(sid),
        order_key: OrderKey::from_bytes(vec![key]).unwrap(),
        visible: true,
        locked: false,
        display_name: format!("n{id}"),
        payload: NodePayload::Frame(FramePayload::default()),
    }
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
        model.insert_node(frame(id, parent, key)).unwrap();
    }
    model
}

pub(crate) fn state() -> DocumentState {
    DocumentState::with_revision(model(), DocumentRevision(4))
}

pub(crate) fn session(n: u64) -> AgentSessionId {
    AgentSessionId(sid(n))
}

pub(crate) fn lease_id(n: u64) -> LeaseId {
    LeaseId(sid(n))
}

pub(crate) fn id(n: u64) -> CommandId {
    CommandId::from_stable_id(sid(n))
}

/// lease 편집 세션 출처의 제안 transaction.
pub(crate) fn proposal(
    state: &DocumentState,
    lease: LeaseId,
    n: u64,
    commands: Vec<Command>,
) -> CommandTransaction {
    CommandTransaction {
        id: id(n),
        origin: CommandOrigin::FutureAgent(AgentProvenance::new(lease.0)),
        requests: commands
            .into_iter()
            .map(|c| CommandRequest::new(state.revision(), c))
            .collect(),
    }
}
