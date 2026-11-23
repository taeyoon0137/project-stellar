//! 에이전트 출처·세션·되돌리기 test 공용 fixture (XIX.6, XIX.7).
//!
//! 이 crate는 history crate에 직접 의존하지 않으므로 창 history 연결 지점은 같은 trait를 구현한 test 대역
//! [`TestHistory`]로 대신합니다. 실제 창 history 구현(`stellar_history::agent::WindowAgentHistory`)은 history crate
//! test가 검증합니다.

use std::path::PathBuf;

use stellar_ops::commands::agent_sink::{
    AgentGroup, AgentGroupOutcome, AgentHistorySink, AgentSinkError, AppliedRevert,
};
use stellar_ops::commands::command::Command;
use stellar_ops::commands::document::definitions::{
    DocumentDefinition, PageDefinition, ViewDefinition,
};
use stellar_ops::commands::document::frame::FramePayload;
use stellar_ops::commands::document::model::DocumentModel;
use stellar_ops::commands::document::records::{NodePayload, NodeRecord};
use stellar_ops::commands::id::{AgentProvenance, CommandId, CommandOrigin};
use stellar_ops::commands::lock::lease::{
    execute_leased, LeaseId, LeaseTable, LeasedExecutionError,
};
use stellar_ops::commands::request::CommandRequest;
use stellar_ops::commands::state::DocumentState;
use stellar_ops::commands::transaction::CommandTransaction;
use stellar_ops::core_types::ids::StableId;
use stellar_ops::core_types::order_key::OrderKey;
use stellar_ops::core_types::revision::DocumentRevision;
use stellar_ops::test_support::TempDir;

use crate::lease::{open_project_coordinator, AgentEditLeases, LeaseTargets};
use crate::permission::{AgentSessionId, PermissionKey};

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

pub(crate) fn name(state: &DocumentState, node: u64) -> String {
    state.model().record(sid(node)).unwrap().display_name
}

pub(crate) fn session(n: u64) -> AgentSessionId {
    AgentSessionId(sid(n))
}

pub(crate) fn key(n: u64) -> PermissionKey {
    PermissionKey::new(sid(500), PathBuf::from("/project"), session(n))
}

pub(crate) fn lease_id(n: u64) -> LeaseId {
    LeaseId(sid(n))
}

pub(crate) fn id(n: u64) -> CommandId {
    CommandId::from_stable_id(sid(n))
}

pub(crate) fn leases(temp: &TempDir) -> AgentEditLeases {
    AgentEditLeases::new(open_project_coordinator(&temp.path, sid(500), &temp.path).unwrap())
}

/// `targets`를 잠그는 lease를 요청하고 승인합니다.
pub(crate) fn grant(
    leases: &mut AgentEditLeases,
    model: &DocumentModel,
    lease: LeaseId,
    targets: &[u64],
) {
    let request = LeaseTargets {
        document: sid(1),
        targets: targets.iter().map(|t| sid(*t)).collect(),
        externals: Vec::new(),
    };
    leases.submit(model, lease, &request).unwrap();
    leases.approve(model, lease).unwrap();
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

/// 창 history 연결 지점 test 대역.
#[derive(Debug)]
pub(crate) struct TestHistory {
    pub(crate) doc: DocumentState,
    pub(crate) entries: Vec<TestEntry>,
    pub(crate) refuse: bool,
    next: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct TestEntry {
    pub(crate) sequence: u64,
    pub(crate) kind: &'static str,
    pub(crate) origin: CommandOrigin,
    pub(crate) forward: CommandTransaction,
    pub(crate) inverse: CommandTransaction,
}

impl TestHistory {
    pub(crate) fn new(doc: DocumentState) -> TestHistory {
        TestHistory {
            doc,
            entries: Vec::new(),
            refuse: false,
            next: 1,
        }
    }

    fn push(
        &mut self,
        kind: &'static str,
        origin: CommandOrigin,
        forward: CommandTransaction,
        inverse: CommandTransaction,
    ) -> u64 {
        let sequence = self.next;
        self.next += 1;
        self.entries.push(TestEntry {
            sequence,
            kind,
            origin,
            forward,
            inverse,
        });
        sequence
    }

    /// 창 Undo 대역: 마지막 entry의 역을 실행하고 (sequence, change set).
    pub(crate) fn undo(&mut self) -> (u64, stellar_ops::commands::change::ChangeSet, TestEntry) {
        let entry = self.entries.pop().unwrap();
        let applied = self
            .doc
            .execute_transaction(&entry.inverse.rebased(self.doc.revision()))
            .unwrap();
        (entry.sequence, applied.changes, entry)
    }

    /// 창 Redo 대역: undo가 돌려준 entry를 다시 적용합니다.
    pub(crate) fn redo(&mut self, entry: TestEntry) -> stellar_ops::commands::change::ChangeSet {
        let applied = self
            .doc
            .execute_transaction(&entry.forward.rebased(self.doc.revision()))
            .unwrap();
        self.entries.push(entry);
        applied.changes
    }
}

impl AgentHistorySink for TestHistory {
    fn state(&self) -> &DocumentState {
        &self.doc
    }

    fn confirm_group(
        &mut self,
        group: AgentGroup,
        outcome: AgentGroupOutcome,
    ) -> Result<u64, AgentSinkError> {
        if self.refuse {
            return Err(AgentSinkError::History("preview 중".to_string()));
        }
        let kind = match outcome {
            AgentGroupOutcome::Completed => "completed",
            AgentGroupOutcome::Interrupted => "interrupted",
        };
        Ok(self.push(kind, group.origin, group.forward, group.inverse))
    }

    fn apply_revert(
        &mut self,
        transaction: &CommandTransaction,
        table: &LeaseTable,
    ) -> Result<AppliedRevert, AgentSinkError> {
        if self.refuse {
            return Err(AgentSinkError::History("preview 중".to_string()));
        }
        if transaction
            .requests
            .iter()
            .any(|r| r.base_revision != self.doc.revision())
        {
            return Err(AgentSinkError::Stale);
        }
        let applied = execute_leased(&mut self.doc, transaction, table).map_err(|e| match e {
            LeasedExecutionError::Transaction(e) => AgentSinkError::Transaction(e),
            LeasedExecutionError::Lease(v) => AgentSinkError::Lease(v),
            LeasedExecutionError::Restore { violation, .. } => AgentSinkError::Lease(violation),
        })?;
        let sequence = self.push(
            "revert",
            transaction.origin,
            transaction.clone(),
            applied.inverse,
        );
        Ok(AppliedRevert {
            sequence,
            changes: applied.changes,
        })
    }
}

/// 세션·lease·권한·출처·history 대역을 묶은 test 세계.
pub(crate) mod world {
    use super::*;
    use crate::agent_history::{
        AgentEditSessions, EndReason, SessionContext, SessionEndReport, SessionOrigin,
    };
    use crate::permission::PermissionStore;
    use crate::provenance::AgentChangeProvenance;
    use stellar_ops::commands::agent_batch::AgentBatchId;

    pub(crate) const S: u64 = 700;
    pub(crate) const L: u64 = 900;

    pub(crate) struct World {
        _temp: TempDir,
        pub(crate) leases: AgentEditLeases,
        pub(crate) permissions: PermissionStore,
        pub(crate) provenance: AgentChangeProvenance,
        pub(crate) sessions: AgentEditSessions,
        pub(crate) history: TestHistory,
    }

    impl World {
        pub(crate) fn new(origin: SessionOrigin) -> World {
            let temp = TempDir::new("agent-history");
            let mut world = World {
                leases: leases(&temp),
                _temp: temp,
                permissions: PermissionStore::new(),
                provenance: AgentChangeProvenance::new(),
                sessions: AgentEditSessions::new(),
                history: TestHistory::new(state()),
            };
            world.permissions.connect(key(S));
            world.permissions.respond_connection(&key(S), true).unwrap();
            world
                .sessions
                .start(session(S), key(S), origin, "디자인 정리", 1_000)
                .unwrap();
            let model = world.history.doc.model();
            grant(&mut world.leases, model, lease_id(L), &[13]);
            world.sessions.begin_edit(session(S), lease_id(L)).unwrap();
            world
        }

        pub(crate) fn ctx(
            &mut self,
        ) -> (SessionContext<'_>, &mut AgentEditSessions, &mut TestHistory) {
            (
                SessionContext {
                    leases: &mut self.leases,
                    permissions: &mut self.permissions,
                    provenance: &mut self.provenance,
                },
                &mut self.sessions,
                &mut self.history,
            )
        }

        pub(crate) fn table(&self) -> LeaseTable {
            self.leases.table().unwrap()
        }

        pub(crate) fn apply(&mut self, n: u64, commands: Vec<Command>) -> AgentBatchId {
            let tx = proposal(&self.history.doc, lease_id(L), n, commands);
            let proposal_id = self.sessions.queue_proposal(session(S), tx).unwrap();
            let table = self.table();
            self.sessions
                .apply_proposal(
                    session(S),
                    proposal_id,
                    &mut self.history.doc,
                    &table,
                    &mut self.provenance,
                )
                .unwrap()
        }

        pub(crate) fn end(&mut self, reason: EndReason, now: u64) -> SessionEndReport {
            let (mut ctx, sessions, history) = self.ctx();
            sessions
                .end(session(S), reason, &mut ctx, history, id(4000), now)
                .unwrap()
        }
    }
}
