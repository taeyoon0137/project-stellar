//! 창 하나의 history stack (VII.4).
//!
//! [`History`]는 확정된 [`HistoryEntry`]를 시간순으로 쌓습니다. 문서 편집은 transaction을
//! 실행한 뒤 정방향·역방향을 함께 기록합니다.
//!
//! **preview(VII.4.2)**: pointer gesture(drag·resize·rotate)는 `begin → update* → end`로 한
//! 항목을 만듭니다. `update`는 직전 preview transaction을 되돌리고 새 transaction을 적용하는 일을
//! **한 원자 transaction**으로 수행하므로 문서에는 항상 마지막 preview만 반영되고 history에는
//! 아무것도 쌓이지 않습니다. `end`에서 마지막 preview를 한 항목으로 확정합니다. slider·scrub
//! (VII.4.5)도 같은 경로이며, `cancel`은 preview 전 상태로 되돌리고 항목을 만들지 않습니다(§23.3:
//! 취소된 조작은 기록하지 않음).

use stellar_commands::change::ChangeSet;
use stellar_commands::id::CommandOrigin;
use stellar_commands::selection::{SelectionPolicy, SetSelection, WindowViewState};
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::MonotonicClock;

use crate::entry::{
    GroupingKey, HistoryEntry, HistoryError, HistoryPayload, HistorySequence, SelectionRestore,
    SequenceAllocator,
};
use crate::grouping::HistoryGroupingPolicy;
use stellar_commands::selection::Selection;

/// 적용 중인 preview transaction과 그 역(gesture 시작 전 상태로 되돌림).
#[derive(Debug, Clone)]
struct AppliedPreview {
    forward: CommandTransaction,
    inverse: CommandTransaction,
}

/// 재생 방향.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Undo,
    Redo,
}

/// undo 한 번의 결과(VII.4.6).
#[derive(Debug, Clone, PartialEq)]
pub struct UndoOutcome {
    /// 되돌린 항목.
    pub entry: HistoryEntry,
    /// 문서 항목이면 역 transaction의 change set, 선택 항목이면 `None`.
    pub changes: Option<ChangeSet>,
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
    redo: Vec<HistoryEntry>,
    sequences: SequenceAllocator,
    preview: Option<Preview>,
    policy: HistoryGroupingPolicy,
    open_group: Option<OpenGroup>,
    /// undo·redo 재생 중인지(VII.5.5 reentrancy guard).
    replaying: bool,
    /// 이 sequence 미만 항목은 journal에 기록하지 않습니다(XXV.1.4 기준점 변경 이전 항목).
    journal_floor: u64,
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

    /// journal에서 복원한 stack으로 history를 만듭니다(XXV.1.3). 열린 grouping·preview는 없고, 새 항목은
    /// `next_sequence`부터 이어 할당합니다. sequence 무결성은 journal 디코딩이 이미 확인했습니다.
    pub(crate) fn from_parts(
        policy: HistoryGroupingPolicy,
        undo: Vec<HistoryEntry>,
        redo: Vec<HistoryEntry>,
        next_sequence: u64,
    ) -> History {
        History {
            entries: undo,
            redo,
            sequences: SequenceAllocator::starting_at(next_sequence),
            policy,
            ..History::default()
        }
    }

    /// grouping 정책.
    pub fn policy(&self) -> HistoryGroupingPolicy {
        self.policy
    }

    /// 확정된 항목(오래된 것부터). undo stack입니다.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// redo stack(다음에 redo할 항목이 마지막).
    pub fn redo_entries(&self) -> &[HistoryEntry] {
        &self.redo
    }

    /// redo 가능한 항목 수.
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// 다음 항목에 할당할 sequence 값(journal 기록용, XXV.1.1).
    pub fn next_sequence(&self) -> u64 {
        self.sequences.peek()
    }

    /// journal 기록 하한(XXV.1.4). 이 값 미만 sequence의 항목은 마지막 기준점 변경 이전 항목이라 journal에
    /// 기록하지 않습니다. 기준점 변경이 없었으면 0입니다.
    pub fn journal_floor(&self) -> u64 {
        self.journal_floor
    }

    /// journal에 기록할 undo·redo stack(XXV.1.4). 메모리 history는 그대로 두고, 기준점 변경 이후 항목만
    /// 고릅니다.
    ///
    /// - undo: sequence가 하한 이상인 항목(undo는 sequence 증가 순이라 항상 뒤쪽 연속 구간).
    /// - redo: 다음 redo(맨 위)가 하한 이상일 때만 전부. 맨 위가 하한 미만이면 현재 위치가 기준점 변경
    ///   이전이라 하한 이상 redo 항목도 그 앞 항목 없이는 다시 적용할 수 없으므로 기록하지 않습니다.
    ///
    /// 두 경우 모두 기록한 항목을 전부 undo하면 기준점 변경 직후 상태 또는 현재 상태에 머물러, 이전 기준점의
    /// history와 섞이지 않습니다.
    pub fn journal_stacks(&self) -> (&[HistoryEntry], &[HistoryEntry]) {
        let floor = self.journal_floor;
        let start = self.entries.partition_point(|e| e.sequence.get() < floor);
        let redo = match self.redo.last() {
            Some(next) if next.sequence.get() < floor => &self.redo[..0],
            _ => &self.redo[..],
        };
        (&self.entries[start..], redo)
    }

    /// 기준점 변경(checkout·merge·rebase 등) 뒤 새 journal session을 시작합니다(XXV.1.4). 이후 journal에는
    /// 지금의 다음 sequence 이상 항목만 기록되고, 열린 grouping을 닫아 새 입력이 이전 항목에 합쳐지지
    /// 않게 합니다. 메모리 undo·redo stack은 그대로입니다(§22.5 synthetic command undo). preview·재생
    /// 중에는 완료된 상태가 아니라 거부합니다. 새 하한을 돌려줍니다.
    pub(crate) fn begin_journal_session(&mut self) -> Result<u64, HistoryError> {
        self.check_journal_session()?;
        self.open_group = None;
        self.journal_floor = self.sequences.peek();
        Ok(self.journal_floor)
    }

    /// [`History::begin_journal_session`]을 할 수 있는지(상태는 바꾸지 않음).
    pub(crate) fn check_journal_session(&self) -> Result<(), HistoryError> {
        if self.replaying {
            return Err(HistoryError::Replaying);
        }
        if self.preview.is_some() {
            return Err(HistoryError::PreviewActive);
        }
        Ok(())
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
        if self.replaying {
            return Err(HistoryError::Replaying);
        }
        if self.preview.is_some() {
            return Err(HistoryError::PreviewActive);
        }
        let mut sequences = self.sequences.clone();
        let sequence = sequences.allocate()?;
        let applied = doc.execute_transaction(transaction)?;
        self.sequences = sequences;
        self.open_group = None;
        // 같은 창에서 새 command가 확정되면 redo 갈래는 폐기됩니다(VII.5.3).
        self.redo.clear();
        self.entries.push(HistoryEntry {
            sequence,
            origin: transaction.origin,
            grouping,
            payload: HistoryPayload::Document {
                forward: transaction.clone(),
                inverse: applied.inverse,
            },
            selection: None,
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
        if self.replaying {
            return Err(HistoryError::Replaying);
        }
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
        self.redo.clear();
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

    /// 선택 변경 event를 view 상태에 적용하고 문서 편집과 **같은 시간순 stack**에 한 항목으로
    /// 기록합니다(VII.4.6). 걸러진 결과가 현재 선택과 같으면 기록하지 않고 `None`입니다.
    pub fn record_selection(
        &mut self,
        view: &mut WindowViewState,
        doc: &DocumentState,
        event: &SetSelection,
        policy: SelectionPolicy,
        origin: CommandOrigin,
    ) -> Result<Option<HistorySequence>, HistoryError> {
        // undo·redo가 복원한 선택에 반응한 재진입 기록은 새 항목을 만들지 않습니다(VII.5.5).
        if self.replaying {
            return Ok(None);
        }
        if self.preview.is_some() {
            return Err(HistoryError::PreviewActive);
        }
        if event.filtered(doc.model(), policy) == *view.selection() {
            return Ok(None);
        }
        let sequence = self.sequences.allocate()?;
        let applied = event.apply(view, doc.model(), policy);
        self.open_group = None;
        self.redo.clear();
        self.entries.push(HistoryEntry {
            sequence,
            origin,
            grouping: None,
            payload: HistoryPayload::Selection {
                forward: applied.forward,
                inverse: applied.inverse,
            },
            selection: None,
        });
        Ok(Some(sequence))
    }

    /// 마지막 항목 하나를 종류와 무관하게 되돌립니다(VII.4.6).
    ///
    /// 문서 항목은 역 transaction을 현재 revision으로 rebase해 실행하고, 그 결과 사라진 node를 선택에서
    /// 정리합니다(system event, 항목 없음). 선택 복원 정보가 있으면 편집 직전 선택을 복원합니다(VII.5.5).
    /// 선택 항목은 이전 선택으로 되돌립니다. 되돌릴 항목이 없으면 `None`, 역 실행이 실패하면 항목을
    /// stack에 그대로 두고 오류를 반환합니다.
    pub fn undo(
        &mut self,
        doc: &mut DocumentState,
        view: &mut WindowViewState,
    ) -> Result<Option<UndoOutcome>, HistoryError> {
        self.undo_with(doc, view, |_, _, _| {})
    }

    /// [`History::undo`]와 같고, 선택을 복원한 직후 `on_selection_restored`를 **reentrancy guard 안에서**
    /// 부릅니다(VII.5.5). UI의 선택 변경 알림을 본뜬 hook으로, 그 안에서 `record_selection`을 불러도 새
    /// 항목이 생기지 않고 문서 기록은 [`HistoryError::Replaying`]으로 거부됩니다.
    pub fn undo_with<F>(
        &mut self,
        doc: &mut DocumentState,
        view: &mut WindowViewState,
        on_selection_restored: F,
    ) -> Result<Option<UndoOutcome>, HistoryError>
    where
        F: FnMut(&mut History, &mut WindowViewState, &DocumentState),
    {
        self.replay(doc, view, Direction::Undo, on_selection_restored)
    }

    /// 마지막으로 되돌린 항목을 다시 적용합니다(VII.5.3). 문서 항목은 정방향을 현재 revision으로
    /// rebase해 실행하고(선택 복원 정보가 있으면 편집 직후 선택 복원), 선택 항목은 정방향 선택을
    /// 적용합니다. 다시 적용할 항목이 없으면 `None`, 실행이 실패하면 항목을 redo stack에 그대로 두고
    /// 오류를 반환합니다.
    pub fn redo(
        &mut self,
        doc: &mut DocumentState,
        view: &mut WindowViewState,
    ) -> Result<Option<UndoOutcome>, HistoryError> {
        self.redo_with(doc, view, |_, _, _| {})
    }

    /// [`History::redo`]와 같고 선택 복원 직후 hook을 reentrancy guard 안에서 부릅니다(VII.5.5).
    pub fn redo_with<F>(
        &mut self,
        doc: &mut DocumentState,
        view: &mut WindowViewState,
        on_selection_restored: F,
    ) -> Result<Option<UndoOutcome>, HistoryError>
    where
        F: FnMut(&mut History, &mut WindowViewState, &DocumentState),
    {
        self.replay(doc, view, Direction::Redo, on_selection_restored)
    }

    /// undo·redo 재생 중인지(VII.5.5).
    pub fn is_replaying(&self) -> bool {
        self.replaying
    }

    /// 마지막 문서 항목의 선택 복원 정보를 기록합니다(창 history 전용). 이미 있으면(병합) 편집 직전
    /// 선택은 처음 값을 유지하고 직후 선택만 갱신합니다.
    pub(crate) fn note_selection(&mut self, before: Selection, after: Selection) {
        if let Some(entry) = self.entries.last_mut() {
            if !entry.is_document() {
                return;
            }
            match &mut entry.selection {
                Some(restore) => restore.after = after,
                None => entry.selection = Some(SelectionRestore { before, after }),
            }
        }
    }

    fn replay<F>(
        &mut self,
        doc: &mut DocumentState,
        view: &mut WindowViewState,
        direction: Direction,
        mut on_selection_restored: F,
    ) -> Result<Option<UndoOutcome>, HistoryError>
    where
        F: FnMut(&mut History, &mut WindowViewState, &DocumentState),
    {
        if self.preview.is_some() {
            return Err(HistoryError::PreviewActive);
        }
        if self.replaying {
            return Err(HistoryError::Replaying);
        }
        let source = match direction {
            Direction::Undo => &mut self.entries,
            Direction::Redo => &mut self.redo,
        };
        let Some(entry) = source.pop() else {
            return Ok(None);
        };
        self.open_group = None;
        let changes = match &entry.payload {
            HistoryPayload::Document { forward, inverse } => {
                let transaction = match direction {
                    Direction::Undo => inverse,
                    Direction::Redo => forward,
                };
                match doc.execute_transaction(&transaction.rebased(doc.revision())) {
                    Ok(applied) => {
                        view.prune_removed(&applied.changes);
                        Some(applied.changes)
                    }
                    Err(error) => {
                        match direction {
                            Direction::Undo => self.entries.push(entry),
                            Direction::Redo => self.redo.push(entry),
                        }
                        return Err(error.into());
                    }
                }
            }
            HistoryPayload::Selection { forward, inverse } => {
                // 기록 당시의 선택 상태를 그대로 복원합니다(숨김 정책 재적용 없음, VII.5.5).
                let event = match direction {
                    Direction::Undo => inverse,
                    Direction::Redo => forward,
                };
                view.restore_selection(&event.selection, doc.model());
                None
            }
        };
        // 선택 복원: 문서 항목의 복원 정보만 적용하며 history에 기록하지 않습니다.
        if let Some(restore) = &entry.selection {
            let selection = match direction {
                Direction::Undo => restore.before.clone(),
                Direction::Redo => restore.after.clone(),
            };
            view.restore_selection(&selection, doc.model());
        }
        self.replaying = true;
        on_selection_restored(self, view, doc);
        self.replaying = false;
        match direction {
            Direction::Undo => self.redo.push(entry.clone()),
            Direction::Redo => self.entries.push(entry.clone()),
        }
        Ok(Some(UndoOutcome { entry, changes }))
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

    /// slider·scrub 조작을 시작합니다(VII.4.5). update·end는 pointer gesture와 같고, 취소는
    /// [`History::cancel_preview`]입니다.
    pub fn begin_scrub(&mut self, control: u64) -> Result<(), HistoryError> {
        self.begin_preview(GroupingKey::Scrub { control })
    }

    /// preview를 취소합니다(VII.4.5): 적용 중인 preview를 되돌려 시작 전 상태로 돌아가고 history
    /// 항목을 만들지 않습니다. 되돌리기가 실패하면 문서는 그대로이고 preview도 유지되어 다시 시도할
    /// 수 있습니다.
    pub fn cancel_preview(&mut self, doc: &mut DocumentState) -> Result<(), HistoryError> {
        let preview = self.preview.as_ref().ok_or(HistoryError::NoPreview)?;
        if let Some(applied) = &preview.applied {
            doc.execute_transaction(&applied.inverse.rebased(doc.revision()))?;
        }
        self.preview = None;
        Ok(())
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
        if transaction.requests.is_empty() {
            return Err(stellar_commands::transaction::TransactionError {
                index: 0,
                error: stellar_commands::command::CommandError::EmptyTransaction,
            }
            .into());
        }
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
        self.redo.clear();
        self.entries.push(HistoryEntry {
            sequence,
            origin: applied.forward.origin,
            grouping: Some(key),
            payload: HistoryPayload::Document {
                forward: applied.forward,
                inverse: applied.inverse,
            },
            selection: None,
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
        // 저장 형식 계층(storage·document)은 history에 의존하지 않습니다.
        for (crate_name, manifest) in [
            ("storage", include_str!("../../storage/Cargo.toml")),
            ("document", include_str!("../../document/Cargo.toml")),
        ] {
            assert!(
                !manifest.contains("stellar-history"),
                "{crate_name}는 history에 의존하지 않아야 합니다"
            );
        }
        // project는 창 session·autosave dirty 판정(XIII.1.2·XIII.3.1)에 history를 쓰지만(crate graph 허용
        // edge), 문서 직렬화·적재 모듈(`document_io`)은 history를 참조하지 않습니다.
        for (module, source) in [
            (
                "document_io",
                include_str!("../../project/src/document_io/mod.rs"),
            ),
            (
                "document_io::serialize",
                include_str!("../../project/src/document_io/serialize.rs"),
            ),
            (
                "document_io::load",
                include_str!("../../project/src/document_io/load.rs"),
            ),
            (
                "document_io::manifest",
                include_str!("../../project/src/document_io/manifest.rs"),
            ),
            (
                "document_io::residency",
                include_str!("../../project/src/document_io/residency.rs"),
            ),
        ] {
            assert!(
                !source.contains("stellar_history"),
                "project {module}는 history를 참조하지 않아야 합니다"
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
    fn empty_update_preserves_the_last_preview_for_cancel_or_undo() {
        use stellar_commands::command::CommandError;
        use stellar_commands::transaction::TransactionError;
        for cancel in [true, false] {
            let mut doc = small_state();
            let mut history = History::new();
            let start = snapshot(&doc);
            history.begin_pointer_gesture(3).unwrap();
            let movement = drag(&doc, 1, 20.0);
            history.update_preview(&mut doc, &movement).unwrap();
            let last = snapshot(&doc);
            let revision = doc.revision();
            let empty = CommandTransaction::new(movement.id, movement.origin);
            assert_eq!(
                history.update_preview(&mut doc, &empty),
                Err(HistoryError::Transaction(TransactionError {
                    index: 0,
                    error: CommandError::EmptyTransaction
                }))
            );
            assert_eq!(doc.revision(), revision);
            assert_eq!(snapshot(&doc), last);
            if cancel {
                history.cancel_preview(&mut doc).unwrap();
                assert!(history.is_empty());
                assert_eq!(snapshot(&doc), start);
            } else {
                history.end_preview().unwrap().unwrap();
                let mut view = WindowViewState::new(
                    stellar_commands::id::WindowSessionId::from_stable_id(sid(500)),
                );
                history.undo(&mut doc, &mut view).unwrap().unwrap();
                assert_eq!(snapshot(&doc), start);
                history.redo(&mut doc, &mut view).unwrap().unwrap();
                assert_eq!(snapshot(&doc), last);
            }
        }
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
                ..GroupingConfig::default()
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

    mod keyboard {
        //! 키보드 반복 grouping fixture(VII.4.4).

        use super::super::test_fixtures::*;
        use super::super::*;
        use crate::entry::CommandSignature;
        use stellar_commands::command::Command;
        use stellar_commands::document::frame::keys as frame_keys;
        use stellar_commands::document::records::NodePayload;
        use stellar_commands::kind::CommandKind;
        use stellar_commands::property::{PropertyValue, SetProperty};
        use stellar_core_types::clock::ManualClock;
        use stellar_core_types::value::{Size2, Transform2D};

        const MS: u64 = 1_000_000;

        fn x_of(doc: &DocumentState, id: u64) -> f32 {
            match doc.model().payload(sid(id)) {
                Some(NodePayload::Frame(frame)) => frame.transform.components().4,
                _ => panic!("Frame"),
            }
        }

        fn nudge(doc: &DocumentState, id: u64, step: u64) -> CommandTransaction {
            let x = x_of(doc, id) + 1.0;
            tx(
                doc,
                8_000 + step,
                Command::SetProperty(SetProperty {
                    node: sid(id),
                    key: frame_keys::TRANSFORM,
                    value: PropertyValue::Transform(
                        Transform2D::new(1.0, 0.0, 0.0, 1.0, x, 0.0).unwrap(),
                    ),
                }),
            )
        }

        fn press(
            history: &mut History,
            doc: &mut DocumentState,
            clock: &ManualClock,
            t: &CommandTransaction,
        ) {
            let key = GroupingKey::keyboard(t).expect("signature");
            history.record_grouped(doc, t, key, clock).unwrap();
        }

        #[test]
        fn key_repeat_twenty_times_is_one_entry() {
            // 검증 기준(VII.4.4): key repeat 20회 history 1건.
            let mut doc = small_state();
            let before = doc.model().to_records().unwrap();
            let mut history = History::new();
            let mut clock = ManualClock::new(0);
            for step in 0..20u64 {
                let t = nudge(&doc, 11, step);
                press(&mut history, &mut doc, &clock, &t);
                clock.advance(33 * MS);
            }
            assert_eq!(history.len(), 1);
            assert_eq!(x_of(&doc, 11), 20.0);
            assert_eq!(
                history.entries()[0].grouping,
                Some(GroupingKey::KeyboardRepeat(CommandSignature {
                    kind: CommandKind::SetProperty,
                    target: sid(11),
                    key: Some(frame_keys::TRANSFORM)
                }))
            );
            let HistoryPayload::Document { inverse, .. } = &history.entries()[0].payload else {
                panic!("문서 항목");
            };
            doc.execute_transaction(&inverse.rebased(doc.revision()))
                .unwrap();
            assert_eq!(
                doc.model().to_records().unwrap(),
                before,
                "한 번의 역으로 20회 전부 원복"
            );
        }

        #[test]
        fn different_signature_or_pause_splits() {
            let mut doc = small_state();
            let mut history = History::new();
            let mut clock = ManualClock::new(0);
            for step in 0..3u64 {
                let t = nudge(&doc, 11, step);
                press(&mut history, &mut doc, &clock, &t);
                clock.advance(30 * MS);
            }
            // 다른 대상(12) → 분리.
            let t = nudge(&doc, 12, 10);
            press(&mut history, &mut doc, &clock, &t);
            assert_eq!(history.len(), 2);
            // 같은 대상이라도 다른 속성(크기 조절) → 분리.
            clock.advance(30 * MS);
            let resize = tx(
                &doc,
                9_000,
                Command::SetProperty(SetProperty {
                    node: sid(12),
                    key: frame_keys::SIZE,
                    value: PropertyValue::Size(Size2::new(10.0, 10.0).unwrap()),
                }),
            );
            press(&mut history, &mut doc, &clock, &resize);
            assert_eq!(history.len(), 3);
            // 간격 초과(0.5초) → 분리.
            clock.advance(600 * MS);
            let t = nudge(&doc, 11, 20);
            press(&mut history, &mut doc, &clock, &t);
            assert_eq!(history.len(), 4);
            // signature: 요청이 둘 이상이거나 구조 command면 없음.
            let two = nudge(&doc, 11, 30)
                .with_command(doc.revision(), Command::rename_node(sid(11), "x"));
            assert_eq!(CommandSignature::of(&two), None);
            let rename = tx(&doc, 31, Command::rename_node(sid(11), "y"));
            assert!(
                CommandSignature::of(&rename).is_some(),
                "이름 변경도 SetProperty signature"
            );
            let delete = tx(
                &doc,
                32,
                Command::DeleteSubtree(stellar_commands::node_ops::DeleteSubtree { root: sid(13) }),
            );
            assert_eq!(GroupingKey::keyboard(&delete), None);
        }
    }

    mod scrub {
        //! slider·scrub commit/cancel fixture(VII.4.5).

        use super::super::test_fixtures::*;
        use super::super::*;
        use stellar_commands::command::Command;
        use stellar_commands::document::frame::{keys as frame_keys, CornerRadius};
        use stellar_commands::property::{PropertyValue, SetProperty};

        fn radius(doc: &DocumentState, value: f32) -> CommandTransaction {
            tx(
                doc,
                10_000 + value as u64,
                Command::SetProperty(SetProperty {
                    node: sid(12),
                    key: frame_keys::CORNER_RADIUS,
                    value: PropertyValue::CornerRadius(
                        CornerRadius::new(value, value, value, value).unwrap(),
                    ),
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
        fn scrub_commit_records_one_entry() {
            // 검증 기준(VII.4.5): commit fixture.
            let mut doc = small_state();
            let before = snapshot(&doc);
            let mut history = History::new();
            history.begin_scrub(3).unwrap();
            for value in 1..=50 {
                let t = radius(&doc, value as f32);
                history.update_preview(&mut doc, &t).unwrap();
            }
            history.end_preview().unwrap();
            assert_eq!(history.len(), 1);
            assert_eq!(
                history.entries()[0].grouping,
                Some(GroupingKey::Scrub { control: 3 })
            );
            let HistoryPayload::Document { inverse, .. } = &history.entries()[0].payload else {
                panic!("문서 항목");
            };
            doc.execute_transaction(&inverse.rebased(doc.revision()))
                .unwrap();
            assert_eq!(snapshot(&doc), before);
        }

        #[test]
        fn scrub_cancel_restores_pre_preview_state_without_history() {
            // 검증 기준(VII.4.5): cancel fixture.
            let mut doc = small_state();
            let mut history = History::new();
            // 앞선 기록 하나.
            let t = tx(&doc, 11_000, Command::rename_node(sid(11), "앞선 편집"));
            history.record_document(&mut doc, &t, None).unwrap();
            let before = snapshot(&doc);
            history.begin_scrub(4).unwrap();
            for value in 1..=30 {
                let t = radius(&doc, value as f32);
                history.update_preview(&mut doc, &t).unwrap();
            }
            assert_ne!(snapshot(&doc), before, "preview 반영 중");
            history.cancel_preview(&mut doc).unwrap();
            assert_eq!(snapshot(&doc), before, "취소 → preview 전 상태");
            assert_eq!(history.len(), 1, "취소는 항목을 만들지 않음");
            assert!(!history.preview_active());
            // update 없는 취소, 비활성 취소.
            history.begin_scrub(5).unwrap();
            history.cancel_preview(&mut doc).unwrap();
            assert_eq!(snapshot(&doc), before);
            assert_eq!(
                history.cancel_preview(&mut doc),
                Err(HistoryError::NoPreview)
            );
            // 취소 뒤 일반 기록은 정상입니다.
            let t = tx(&doc, 11_001, Command::rename_node(sid(12), "다음"));
            history.record_document(&mut doc, &t, None).unwrap();
            assert_eq!(history.len(), 2);
        }
    }

    mod unified {
        //! 선택·문서 편집 통합 stack fixture(VII.4.6).

        use super::super::test_fixtures::*;
        use super::super::*;
        use stellar_commands::command::Command;
        use stellar_commands::id::WindowSessionId;
        use stellar_commands::node_ops::InsertNode;
        use stellar_commands::selection::Selection;

        fn select(
            history: &mut History,
            view: &mut WindowViewState,
            doc: &DocumentState,
            ids: &[u64],
        ) -> Option<HistorySequence> {
            let event = SetSelection::new(Selection::new(ids.iter().map(|i| sid(*i)), None));
            history
                .record_selection(
                    view,
                    doc,
                    &event,
                    SelectionPolicy::default(),
                    CommandOrigin::User,
                )
                .unwrap()
        }

        #[test]
        fn selection_edit_selection_undo_in_order() {
            // 검증 기준(VII.4.6): selection→edit→selection 순서 undo fixture 통과.
            let mut doc = small_state();
            let mut view = WindowViewState::new(WindowSessionId::from_stable_id(sid(500)));
            let mut history = History::new();
            let original_name = doc.model().record(sid(11)).unwrap().display_name;

            select(&mut history, &mut view, &doc, &[11]);
            let t = tx(&doc, 12_000, Command::rename_node(sid(11), "편집"));
            history.record_document(&mut doc, &t, None).unwrap();
            select(&mut history, &mut view, &doc, &[12, 13]);
            let kinds: Vec<bool> = history
                .entries()
                .iter()
                .map(HistoryEntry::is_selection)
                .collect();
            assert_eq!(kinds, vec![true, false, true], "같은 시간순 stack");
            let sequences: Vec<u64> = history.entries().iter().map(|e| e.sequence.get()).collect();
            assert_eq!(sequences, vec![1, 2, 3]);

            // 1) 마지막 선택을 되돌림 → [11], 문서 그대로.
            let outcome = history.undo(&mut doc, &mut view).unwrap().unwrap();
            assert!(outcome.entry.is_selection() && outcome.changes.is_none());
            assert_eq!(view.selection().ids(), &[sid(11)]);
            assert_eq!(doc.model().record(sid(11)).unwrap().display_name, "편집");
            // 2) 편집을 되돌림 → 이름 원복, 선택 그대로.
            let outcome = history.undo(&mut doc, &mut view).unwrap().unwrap();
            assert!(outcome.entry.is_document());
            assert_eq!(
                doc.model().record(sid(11)).unwrap().display_name,
                original_name
            );
            assert_eq!(view.selection().ids(), &[sid(11)]);
            // 3) 첫 선택을 되돌림 → 빈 선택.
            history.undo(&mut doc, &mut view).unwrap().unwrap();
            assert!(view.selection().is_empty());
            assert!(history.is_empty());
            assert_eq!(history.undo(&mut doc, &mut view).unwrap(), None);
        }

        #[test]
        fn unchanged_selection_is_not_recorded_and_undo_prunes_removed_nodes() {
            let mut doc = small_state();
            let mut view = WindowViewState::new(WindowSessionId::from_stable_id(sid(500)));
            let mut history = History::new();
            let policy = SelectionPolicy::default();
            // 삽입한 node를 선택한 뒤 삽입을 undo하면 선택에서도 사라집니다(system 정리, 항목 없음).
            let t = tx(
                &doc,
                13_000,
                Command::InsertNode(InsertNode::single(frame(20, Some(12), 0x10))),
            );
            history.record_document(&mut doc, &t, None).unwrap();
            select(&mut history, &mut view, &doc, &[20]);
            assert_eq!(
                select(&mut history, &mut view, &doc, &[20]),
                None,
                "같은 선택은 기록 안 함"
            );
            assert_eq!(
                select(&mut history, &mut view, &doc, &[99]),
                Some(HistorySequence::from_u64(3))
            );
            assert!(view.selection().is_empty(), "없는 id는 걸러짐");
            history.undo(&mut doc, &mut view).unwrap();
            assert_eq!(view.selection().ids(), &[sid(20)]);
            history.undo(&mut doc, &mut view).unwrap(); // 선택 [20] 기록 되돌림
            assert!(view.selection().is_empty());
            select(&mut history, &mut view, &doc, &[20, 12]);
            let entries_before = history.len();
            // 선택 기록을 되돌리고, 삽입을 되돌리면 20은 문서·선택에서 사라집니다.
            history.undo(&mut doc, &mut view).unwrap();
            SetSelection::new(Selection::new([sid(20), sid(12)], None)).apply(
                &mut view,
                doc.model(),
                policy,
            );
            history.undo(&mut doc, &mut view).unwrap();
            assert!(!doc.model().contains(sid(20)));
            assert_eq!(view.selection().ids(), &[sid(12)], "삭제된 id만 정리");
            assert_eq!(
                history.len(),
                entries_before - 2,
                "정리는 항목을 만들지 않음"
            );
        }
    }
}
