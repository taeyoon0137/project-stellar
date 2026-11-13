//! 창 하나의 history stack (VII.4).
//!
//! [`History`]는 확정된 [`HistoryEntry`]를 시간순으로 쌓습니다. 문서 편집은 transaction을
//! 실행한 뒤 정방향·역방향을 함께 기록합니다.
//!
//! **preview(VII.4.2)**: pointer gesture(drag·resize·rotate)는 `begin → update* → end`로 한
//! 항목을 만듭니다. `update`는 직전 preview transaction을 되돌리고 새 transaction을 적용하는 일을
//! **한 원자 transaction**으로 수행하므로 문서에는 항상 마지막 preview만 반영되고 history에는
//! 아무것도 쌓이지 않습니다. `end`에서 마지막 preview를 한 항목으로 확정합니다.

use stellar_commands::change::ChangeSet;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::MonotonicClock;

use crate::entry::{
    GroupingKey, HistoryEntry, HistoryError, HistoryPayload, HistorySequence, SequenceAllocator,
};
use crate::grouping::HistoryGroupingPolicy;

/// 적용 중인 preview transaction과 그 역(gesture 시작 전 상태로 되돌림).
#[derive(Debug, Clone)]
struct AppliedPreview {
    forward: CommandTransaction,
    inverse: CommandTransaction,
}

/// 마지막 항목에 이어 합칠 수 있는 입력 묶음(VII.4.3~).
#[derive(Debug, Clone, Copy)]
struct OpenGroup {
    key: GroupingKey,
    last_nanos: u64,
}

/// 진행 중인 preview 조작.
#[derive(Debug, Clone)]
struct Preview {
    key: GroupingKey,
    applied: Option<AppliedPreview>,
}

/// 창 하나의 history.
#[derive(Debug, Clone, Default)]
pub struct History {
    entries: Vec<HistoryEntry>,
    sequences: SequenceAllocator,
    preview: Option<Preview>,
    policy: HistoryGroupingPolicy,
    open_group: Option<OpenGroup>,
}

impl History {
    /// 빈 history.
    pub fn new() -> History {
        History::default()
    }

    /// grouping 정책을 지정한 빈 history.
    pub fn with_policy(policy: HistoryGroupingPolicy) -> History {
        History {
            policy,
            ..History::default()
        }
    }

    /// grouping 정책.
    pub fn policy(&self) -> HistoryGroupingPolicy {
        self.policy
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
        if self.preview.is_some() {
            return Err(HistoryError::PreviewActive);
        }
        let mut sequences = self.sequences.clone();
        let sequence = sequences.allocate()?;
        let applied = doc.execute_transaction(transaction)?;
        self.sequences = sequences;
        self.open_group = None;
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

    /// grouping key를 가진 연속 입력을 실행하고, 정책([`HistoryGroupingPolicy`])이 허락하면 마지막
    /// 항목에 합칩니다(VII.4.3~). 합치면 정방향은 뒤에 이어 붙이고 역방향은 앞에 붙여, 항목 하나의
    /// undo가 묶음 전체를 되돌립니다.
    pub fn record_grouped<C: MonotonicClock>(
        &mut self,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
        key: GroupingKey,
        clock: &C,
    ) -> Result<ChangeSet, HistoryError> {
        if self.preview.is_some() {
            return Err(HistoryError::PreviewActive);
        }
        let now = clock.now_nanos();
        let merge = self.open_group.is_some_and(|open| {
            self.policy
                .should_merge(&open.key, open.last_nanos, &key, now)
        }) && self.entries.last().is_some_and(HistoryEntry::is_document);
        if !merge {
            let changes = self.record_document(doc, transaction, Some(key))?;
            self.open_group = Some(OpenGroup {
                key,
                last_nanos: now,
            });
            return Ok(changes);
        }
        let applied = doc.execute_transaction(transaction)?;
        if let Some(HistoryEntry {
            grouping,
            payload: HistoryPayload::Document { forward, inverse },
            ..
        }) = self.entries.last_mut()
        {
            forward
                .requests
                .extend(transaction.requests.iter().cloned());
            let mut merged = applied.inverse.requests;
            merged.append(&mut inverse.requests);
            inverse.requests = merged;
            *grouping = Some(key);
        }
        self.open_group = Some(OpenGroup {
            key,
            last_nanos: now,
        });
        Ok(applied.changes)
    }

    /// preview가 진행 중인지.
    pub fn preview_active(&self) -> bool {
        self.preview.is_some()
    }

    /// preview 조작을 시작합니다(pointer down). 문서는 바뀌지 않습니다.
    pub fn begin_preview(&mut self, key: GroupingKey) -> Result<(), HistoryError> {
        if self.preview.is_some() {
            return Err(HistoryError::PreviewActive);
        }
        self.preview = Some(Preview { key, applied: None });
        Ok(())
    }

    /// pointer gesture(drag·resize·rotate)를 시작합니다(VII.4.2).
    pub fn begin_pointer_gesture(&mut self, gesture: u64) -> Result<(), HistoryError> {
        self.begin_preview(GroupingKey::PointerGesture { gesture })
    }

    /// preview transaction을 교체합니다(pointer move).
    ///
    /// 직전 preview의 역과 새 transaction을 **한 transaction**으로 실행하므로, 실패하면 문서와
    /// preview가 모두 이전 상태로 남습니다. 새 transaction의 base revision은 현재 문서 revision이어야
    /// 합니다.
    pub fn update_preview(
        &mut self,
        doc: &mut DocumentState,
        transaction: &CommandTransaction,
    ) -> Result<ChangeSet, HistoryError> {
        let preview = self.preview.as_mut().ok_or(HistoryError::NoPreview)?;
        let mut combined = CommandTransaction::new(transaction.id, transaction.origin);
        if let Some(previous) = &preview.applied {
            combined
                .requests
                .extend(previous.inverse.rebased(doc.revision()).requests);
        }
        combined
            .requests
            .extend(transaction.requests.iter().cloned());
        let applied = doc.execute_transaction(&combined)?;
        // 결합 역 transaction의 앞쪽 n개가 새 transaction의 역입니다(역순 실행).
        let mut inverse = applied.inverse;
        inverse.requests.truncate(transaction.requests.len());
        preview.applied = Some(AppliedPreview {
            forward: transaction.clone(),
            inverse,
        });
        Ok(applied.changes)
    }

    /// preview를 끝내고(pointer up) 마지막 preview를 **한 항목**으로 확정합니다. update가 한 번도
    /// 없었으면 기록할 것이 없어 `None`입니다.
    pub fn end_preview(&mut self) -> Result<Option<HistorySequence>, HistoryError> {
        let preview = self.preview.as_ref().ok_or(HistoryError::NoPreview)?;
        let Some(applied) = preview.applied.clone() else {
            self.preview = None;
            return Ok(None);
        };
        let sequence = self.sequences.allocate()?;
        let key = preview.key;
        self.preview = None;
        self.open_group = None;
        self.entries.push(HistoryEntry {
            sequence,
            origin: applied.forward.origin,
            grouping: Some(key),
            payload: HistoryPayload::Document {
                forward: applied.forward,
                inverse: applied.inverse,
            },
        });
        Ok(Some(sequence))
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

    fn transform(x: f32, y: f32) -> stellar_commands::property::PropertyValue {
        stellar_commands::property::PropertyValue::Transform(
            stellar_core_types::value::Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).unwrap(),
        )
    }

    fn drag(
        doc: &DocumentState,
        step: u64,
        x: f32,
    ) -> stellar_commands::transaction::CommandTransaction {
        tx(
            doc,
            1_000 + step,
            Command::SetProperty(stellar_commands::property::SetProperty {
                node: sid(11),
                key: stellar_commands::document::frame::keys::TRANSFORM,
                value: transform(x, x / 2.0),
            }),
        )
    }

    fn snapshot(doc: &DocumentState) -> String {
        stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json(
            doc.model(),
        )
        .unwrap()
    }

    #[test]
    fn drag_with_500_updates_records_one_entry() {
        // 검증 기준(VII.4.2): drag 500 update가 history 1건.
        let mut doc = small_state();
        let before = snapshot(&doc);
        let mut history = History::new();
        history.begin_pointer_gesture(1).unwrap();
        for step in 0..500u64 {
            let t = drag(&doc, step, step as f32);
            history.update_preview(&mut doc, &t).unwrap();
            assert!(history.is_empty(), "update 중에는 항목 없음");
        }
        let sequence = history.end_preview().unwrap().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(sequence.get(), 1);
        let entry = &history.entries()[0];
        assert_eq!(
            entry.grouping,
            Some(GroupingKey::PointerGesture { gesture: 1 })
        );
        // 문서에는 마지막 preview만 반영됩니다.
        let record = doc.model().record(sid(11)).unwrap();
        let stellar_commands::document::records::NodePayload::Frame(frame) = record.payload else {
            panic!("Frame");
        };
        assert_eq!(frame.transform.components().4, 499.0);
        // 항목의 역은 gesture 시작 전 상태로 되돌립니다.
        let HistoryPayload::Document { inverse, forward } = &entry.payload else {
            panic!("문서 항목");
        };
        assert_eq!(forward.requests.len(), 1);
        assert_eq!(inverse.requests.len(), 1);
        doc.execute_transaction(&inverse.rebased(doc.revision()))
            .unwrap();
        assert_eq!(snapshot(&doc), before);
    }

    #[test]
    fn update_replaces_previous_preview_instead_of_accumulating() {
        use stellar_commands::node_ops::InsertNode;
        let mut doc = small_state();
        let mut history = History::new();
        history.begin_pointer_gesture(7).unwrap();
        for step in 0..20u64 {
            // 매 update가 다른 node를 넣는 preview: 교체가 아니라 누적이면 node가 쌓입니다.
            let t = tx(
                &doc,
                2_000 + step,
                Command::InsertNode(InsertNode::single(frame(100 + step, Some(12), 0x10))),
            );
            history.update_preview(&mut doc, &t).unwrap();
            assert_eq!(doc.model().node_count(), 5, "항상 마지막 preview 하나만");
            assert!(doc.model().contains(sid(100 + step)));
        }
        // 실패한 update는 이전 preview를 유지합니다.
        let bad = tx(&doc, 3_000, Command::rename_node(sid(99), "x"));
        assert!(history.update_preview(&mut doc, &bad).is_err());
        assert!(doc.model().contains(sid(119)));
        // preview 중에는 일반 기록·새 preview를 시작할 수 없습니다.
        let other = tx(&doc, 3_001, Command::rename_node(sid(11), "x"));
        assert_eq!(
            history.record_document(&mut doc, &other, None),
            Err(HistoryError::PreviewActive)
        );
        assert_eq!(
            history.begin_pointer_gesture(8),
            Err(HistoryError::PreviewActive)
        );
        history.end_preview().unwrap();
        assert_eq!(history.len(), 1);
        // update 없는 gesture는 기록하지 않습니다.
        history.begin_pointer_gesture(9).unwrap();
        assert_eq!(history.end_preview().unwrap(), None);
        assert_eq!(history.len(), 1);
        assert_eq!(history.end_preview(), Err(HistoryError::NoPreview));
    }

    mod text {
        //! 텍스트 입력 grouping fixture(VII.4.3).

        use super::super::test_fixtures::*;
        use super::super::*;
        use crate::entry::TextInputKind;
        use crate::grouping::GroupingConfig;
        use stellar_commands::command::Command;
        use stellar_commands::convert::RestoreNodeRecords;
        use stellar_commands::document::records::NodePayload;
        use stellar_commands::document::text::{edit_text, scalar_len, TextPayload, TextRange};
        use stellar_commands::node_ops::InsertNode;
        use stellar_core_types::clock::ManualClock;

        const MS: u64 = 1_000_000;

        fn text_node(doc: &mut DocumentState, id: u64) {
            let mut record = frame(id, Some(12), 0x10 + id as u8);
            record.payload = NodePayload::Text(TextPayload::empty(0));
            let t = tx(
                doc,
                5_000 + id,
                Command::InsertNode(InsertNode::single(record)),
            );
            doc.execute_transaction(&t).unwrap();
        }

        fn content(doc: &DocumentState, id: u64) -> String {
            match doc.model().payload(sid(id)) {
                Some(NodePayload::Text(text)) => text.content().to_string(),
                _ => panic!("Text node"),
            }
        }

        /// `[start, end)`를 `replacement`로 바꾸는 편집 transaction과 grouping key.
        fn text_edit(
            doc: &DocumentState,
            id: u64,
            start: u32,
            end: u32,
            replacement: &str,
            kind: TextInputKind,
            caret_before: u32,
        ) -> (CommandTransaction, GroupingKey) {
            let mut record = doc.model().record(sid(id)).unwrap();
            let NodePayload::Text(text) = &record.payload else {
                panic!("Text");
            };
            let edited =
                edit_text(text, TextRange::new(start, end).unwrap(), replacement, &[]).unwrap();
            record.payload = NodePayload::Text(edited);
            let caret_after = start + scalar_len(replacement);
            let command = Command::RestoreNodeRecords(RestoreNodeRecords {
                records: vec![record],
            });
            let key = GroupingKey::TextInput {
                node: sid(id),
                kind,
                caret_before,
                caret_after,
            };
            (tx(doc, 6_000 + u64::from(start), command), key)
        }

        fn type_char(
            history: &mut History,
            doc: &mut DocumentState,
            clock: &ManualClock,
            id: u64,
            caret: u32,
            ch: &str,
        ) {
            let (t, key) = text_edit(doc, id, caret, caret, ch, TextInputKind::Insert, caret);
            history.record_grouped(doc, &t, key, clock).unwrap();
        }

        #[test]
        fn consecutive_typing_merges_into_one_entry_and_undoes_together() {
            // 검증 기준(VII.4.3): 조건별 merge/split fixture 통과 — merge.
            let mut doc = small_state();
            text_node(&mut doc, 20);
            let mut history = History::new();
            let mut clock = ManualClock::new(0);
            for (i, ch) in ["가", "b", "😀"].iter().enumerate() {
                type_char(&mut history, &mut doc, &clock, 20, i as u32, ch);
                clock.advance(200 * MS);
            }
            assert_eq!(content(&doc, 20), "가b😀");
            assert_eq!(history.len(), 1, "연속 입력 1건");
            let entry = &history.entries()[0];
            assert_eq!(
                entry.grouping,
                Some(GroupingKey::TextInput {
                    node: sid(20),
                    kind: TextInputKind::Insert,
                    caret_before: 2,
                    caret_after: 3
                })
            );
            let HistoryPayload::Document { forward, inverse } = &entry.payload else {
                panic!("문서 항목");
            };
            assert_eq!(forward.requests.len(), 3);
            doc.execute_transaction(&inverse.rebased(doc.revision()))
                .unwrap();
            assert_eq!(content(&doc, 20), "", "항목 하나의 역이 묶음 전체를 되돌림");
        }

        #[test]
        fn split_conditions() {
            // 검증 기준(VII.4.3): split — 간격 초과, 다른 node, 비인접 caret, 입력 종류 변경.
            let mut doc = small_state();
            text_node(&mut doc, 20);
            text_node(&mut doc, 21);
            let mut history = History::new();
            let mut clock = ManualClock::new(0);
            type_char(&mut history, &mut doc, &clock, 20, 0, "a");
            clock.advance(1_500 * MS); // 기본 간격 1초 초과
            type_char(&mut history, &mut doc, &clock, 20, 1, "b");
            assert_eq!(history.len(), 2, "간격 초과 → 분리");
            clock.advance(10 * MS);
            type_char(&mut history, &mut doc, &clock, 21, 0, "x");
            assert_eq!(history.len(), 3, "다른 node → 분리");
            clock.advance(10 * MS);
            type_char(&mut history, &mut doc, &clock, 20, 0, "_");
            assert_eq!(history.len(), 4, "다른 node 뒤 비인접 caret → 분리");
            clock.advance(10 * MS);
            type_char(&mut history, &mut doc, &clock, 20, 3, "c");
            assert_eq!(history.len(), 5, "비인접 caret(1 → 3) → 분리");
            clock.advance(10 * MS);
            // Backspace: caret 4 → 3, 다음 Backspace는 caret 3 → 2(인접).
            let (t, key) = text_edit(&doc, 20, 3, 4, "", TextInputKind::DeleteBackward, 4);
            history.record_grouped(&mut doc, &t, key, &clock).unwrap();
            assert_eq!(history.len(), 6, "입력 종류 변경 → 분리");
            clock.advance(10 * MS);
            let (t, key) = text_edit(&doc, 20, 2, 3, "", TextInputKind::DeleteBackward, 3);
            history.record_grouped(&mut doc, &t, key, &clock).unwrap();
            assert_eq!(history.len(), 6, "연속 Backspace → 병합");
            assert_eq!(content(&doc, 20), "_a");
            // 일반 기록이 끼면 묶음이 닫힙니다.
            let t = tx(&doc, 7_000, Command::rename_node(sid(11), "x"));
            history.record_document(&mut doc, &t, None).unwrap();
            clock.advance(10 * MS);
            let (t, key) = text_edit(&doc, 20, 1, 2, "", TextInputKind::DeleteBackward, 2);
            history.record_grouped(&mut doc, &t, key, &clock).unwrap();
            assert_eq!(history.len(), 8);
        }

        #[test]
        fn interval_comes_from_config() {
            let mut doc = small_state();
            text_node(&mut doc, 20);
            let mut history = History::with_policy(HistoryGroupingPolicy::new(GroupingConfig {
                text_merge_interval_nanos: 50 * MS,
            }));
            let mut clock = ManualClock::new(0);
            type_char(&mut history, &mut doc, &clock, 20, 0, "a");
            clock.advance(40 * MS);
            type_char(&mut history, &mut doc, &clock, 20, 1, "b");
            clock.advance(60 * MS);
            type_char(&mut history, &mut doc, &clock, 20, 2, "c");
            assert_eq!(history.len(), 2, "50ms 설정: 40ms 병합, 60ms 분리");
        }
    }
}
