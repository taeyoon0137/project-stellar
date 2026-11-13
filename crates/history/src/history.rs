//! 창 하나의 history stack (VII.4).
//!
//! [`History`]는 확정된 [`HistoryEntry`]를 시간순으로 쌓습니다. 문서 편집은 transaction을
//! 실행한 뒤 정방향·역방향을 함께 기록합니다.

use stellar_commands::change::ChangeSet;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;

use crate::entry::{GroupingKey, HistoryEntry, HistoryError, HistoryPayload, SequenceAllocator};

/// 창 하나의 history.
#[derive(Debug, Clone, Default)]
pub struct History {
    entries: Vec<HistoryEntry>,
    sequences: SequenceAllocator,
}

impl History {
    /// 빈 history.
    pub fn new() -> History {
        History::default()
    }

    /// 확정된 항목(오래된 것부터).
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// 항목 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 문서 transaction을 실행하고 한 항목으로 기록합니다. 실패하면 문서와 history 모두
    /// 실행 전 상태입니다.
    pub fn record_document(
        &mut self,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
        grouping: Option<GroupingKey>,
    ) -> Result<ChangeSet, HistoryError> {
        let mut sequences = self.sequences.clone();
        let sequence = sequences.allocate()?;
        let applied = doc.execute_transaction(transaction)?;
        self.sequences = sequences;
        self.entries.push(HistoryEntry {
            sequence,
            origin: transaction.origin,
            grouping,
            payload: HistoryPayload::Document {
                forward: transaction.clone(),
                inverse: applied.inverse,
            },
        });
        Ok(applied.changes)
    }
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    //! history unit test 공용 fixture.

    use stellar_commands::command::Command;
    use stellar_commands::document::definitions::{
        DocumentDefinition, PageDefinition, ViewDefinition,
    };
    use stellar_commands::document::frame::FramePayload;
    use stellar_commands::document::model::DocumentModel;
    use stellar_commands::document::records::{NodePayload, NodeRecord};
    use stellar_commands::id::{CommandId, CommandOrigin};
    use stellar_commands::state::DocumentState;
    use stellar_commands::transaction::CommandTransaction;
    use stellar_core_types::ids::StableId;
    use stellar_core_types::order_key::OrderKey;

    /// 결정론적 id.
    pub fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    /// Frame 레코드.
    pub fn frame(id: u64, parent: Option<u64>, key: u8) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![key]).expect("key"),
            visible: true,
            locked: false,
            display_name: format!("n{id}"),
            payload: NodePayload::Frame(FramePayload::default()),
        }
    }

    /// root(10) 아래 11·12, 11 아래 13.
    pub fn small_state() -> DocumentState {
        let mut definition = DocumentDefinition::new(sid(1), String::new());
        let mut page = PageDefinition::new(sid(2), String::new());
        page.add_view(ViewDefinition::new(sid(3), String::new(), sid(10)))
            .expect("view");
        definition.add_page(page).expect("page");
        let mut model = DocumentModel::new(definition);
        model.insert_node(frame(10, None, 0x10)).expect("root");
        model.insert_node(frame(11, Some(10), 0x10)).expect("11");
        model.insert_node(frame(12, Some(10), 0x20)).expect("12");
        model.insert_node(frame(13, Some(11), 0x10)).expect("13");
        DocumentState::new(model)
    }

    /// 현재 revision 기준 command 하나짜리 사용자 transaction.
    pub fn tx(doc: &DocumentState, id: u64, command: Command) -> CommandTransaction {
        CommandTransaction::new(CommandId::from_stable_id(sid(id)), CommandOrigin::User)
            .with_command(doc.revision(), command)
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::*;
    use super::*;
    use stellar_commands::command::Command;
    use stellar_commands::id::{AgentProvenance, CommandId, CommandOrigin};

    #[test]
    fn entries_have_origin_transactions_grouping_and_monotonic_sequence() {
        let mut doc = small_state();
        let mut history = History::new();
        for i in 0..5u64 {
            let command = Command::rename_node(sid(11), format!("이름 {i}"));
            let grouping = (i % 2 == 0).then_some(GroupingKey::PointerGesture { gesture: i });
            {
                let t = tx(&doc, 900 + i, command);
                history.record_document(&mut doc, &t, grouping)
            }
            .unwrap();
        }
        let sequences: Vec<u64> = history.entries().iter().map(|e| e.sequence.get()).collect();
        assert_eq!(
            sequences,
            vec![1, 2, 3, 4, 5],
            "timestamp 대신 단조 sequence"
        );
        let entry = &history.entries()[2];
        assert_eq!(entry.origin, CommandOrigin::User);
        assert_eq!(
            entry.grouping,
            Some(GroupingKey::PointerGesture { gesture: 2 })
        );
        let HistoryPayload::Document { forward, inverse } = &entry.payload else {
            panic!("문서 항목");
        };
        assert_eq!(forward.id, CommandId::from_stable_id(sid(902)));
        assert_eq!(inverse.id, forward.id);
        assert_eq!(
            inverse.requests[0].command,
            Command::rename_node(sid(11), "이름 1")
        );
        assert!(entry.is_document() && !entry.is_selection());

        // agent provenance도 보존됩니다.
        let agent = CommandOrigin::FutureAgent(AgentProvenance::new(sid(77)));
        let mut agent_tx = tx(&doc, 950, Command::rename_node(sid(12), "agent"));
        agent_tx.origin = agent;
        history.record_document(&mut doc, &agent_tx, None).unwrap();
        assert_eq!(
            history.entries()[5].origin.agent_provenance(),
            Some(AgentProvenance::new(sid(77)))
        );
    }

    #[test]
    fn failed_transaction_records_nothing() {
        let mut doc = small_state();
        let mut history = History::new();
        let bad = tx(&doc, 900, Command::rename_node(sid(99), "x"));
        assert!(history.record_document(&mut doc, &bad, None).is_err());
        assert!(history.is_empty());
        {
            let t = tx(&doc, 901, Command::rename_node(sid(11), "a"));
            history.record_document(&mut doc, &t, None)
        }
        .unwrap();
        assert_eq!(
            history.entries()[0].sequence.get(),
            1,
            "실패는 sequence를 쓰지 않음"
        );
    }

    #[test]
    fn sequence_allocator_refuses_overflow() {
        let mut allocator = SequenceAllocator::new();
        assert_eq!(allocator.allocate().unwrap().get(), 1);
        assert_eq!(allocator.allocate().unwrap().get(), 2);
        let mut last = SequenceAllocator::starting_at(u64::MAX);
        assert_eq!(last.allocate(), Err(HistoryError::SequenceOverflow));
    }

    #[test]
    fn history_is_excluded_from_project_serializer() {
        // 검증 기준(VII.4.1): 직렬화는 세션 메모리 전용이며 project serializer에서 제외.
        for (crate_name, manifest) in [
            ("project", include_str!("../../project/Cargo.toml")),
            ("storage", include_str!("../../storage/Cargo.toml")),
            ("document", include_str!("../../document/Cargo.toml")),
        ] {
            assert!(
                !manifest.contains("stellar-history"),
                "{crate_name}는 history에 의존하지 않아야 합니다"
            );
        }
        // 문서 저장 레코드에는 history 항목이 들어갈 자리가 없습니다: 기록 전후 레코드 비교.
        let mut doc = small_state();
        let before = doc.model().to_records().unwrap();
        let mut history = History::new();
        {
            let t = tx(&doc, 900, Command::rename_node(sid(11), "a"));
            history.record_document(&mut doc, &t, None)
        }
        .unwrap();
        let mut after = doc.model().to_records().unwrap();
        assert_ne!(after, before, "편집은 반영");
        // 편집 내용을 되돌리면 history가 남아 있어도 레코드는 원래와 같습니다.
        for node in &mut after.nodes {
            if node.id == sid(11) {
                node.display_name = "n11".into();
            }
        }
        assert_eq!(after, before);
        assert_eq!(history.len(), 1);
    }
}
