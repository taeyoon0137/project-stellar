//! history 항목 (VII.4.1).
//!
//! [`HistoryEntry`]는 창 history의 undo 단위입니다:
//!
//! - **origin**: 편집 출처([`CommandOrigin`]). agent 편집은 provenance를 그대로 보존합니다.
//! - **forward / inverse**: 문서 편집이면 정방향·역방향 [`CommandTransaction`], 선택 변경이면
//!   정방향·역방향 [`SetSelection`](VII.3.2).
//! - **grouping key**: 연속 입력을 한 항목으로 합칠지 판단하는 key([`GroupingKey`]).
//! - **sequence**: wall-clock timestamp 대신 창 안에서 단조 증가하는 [`HistorySequence`]입니다
//!   (II.3.3: 시스템 시계는 history 순서에 쓰지 않음).
//!
//! 항목은 **세션 메모리 전용**입니다. project serializer(문서 chunk·manifest)는 이 crate에
//! 의존하지 않으며 항목을 기록하지 않습니다(§23.1). 세션 간 복원용 journal 형식은 로드맵
//! XXV.1의 별도 schema입니다.

use std::fmt;

use stellar_commands::id::CommandOrigin;
use stellar_commands::kind::CommandKind;
use stellar_commands::selection::{Selection, SetSelection};
use stellar_commands::transaction::{CommandTransaction, TransactionError};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;

/// 창 history 안 항목 순서. 창마다 1부터 단조 증가합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistorySequence(u64);

impl HistorySequence {
    /// 순서 값.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// 값으로 만듭니다(journal 복원용).
    pub const fn from_u64(value: u64) -> HistorySequence {
        HistorySequence(value)
    }
}

/// 단조 sequence 할당기.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceAllocator {
    next: u64,
}

impl Default for SequenceAllocator {
    fn default() -> SequenceAllocator {
        SequenceAllocator::new()
    }
}

impl SequenceAllocator {
    /// 1부터 시작하는 할당기.
    pub const fn new() -> SequenceAllocator {
        SequenceAllocator { next: 1 }
    }

    /// `next`부터 이어 할당하는 할당기(복원한 history에 이어 기록할 때).
    pub const fn starting_at(next: u64) -> SequenceAllocator {
        SequenceAllocator { next }
    }

    /// 다음에 할당할 값(할당하지 않음, journal 기록용).
    pub const fn peek(&self) -> u64 {
        self.next
    }

    /// 다음 sequence. 한계에 도달하면 오류입니다(되돌아가지 않음).
    pub fn allocate(&mut self) -> Result<HistorySequence, HistoryError> {
        let value = self.next;
        self.next = value.checked_add(1).ok_or(HistoryError::SequenceOverflow)?;
        Ok(HistorySequence(value))
    }
}

/// 텍스트 입력 종류(VII.4.3 grouping 조건).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextInputKind {
    /// 문자 삽입(타이핑).
    Insert,
    /// caret 앞 문자 삭제(Backspace).
    DeleteBackward,
    /// caret 뒤 문자 삭제(Delete).
    DeleteForward,
}

/// 반복 입력 병합에 쓰는 command signature(VII.4.4): 같은 종류·대상·속성이면 같은 signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandSignature {
    /// command 종류.
    pub kind: CommandKind,
    /// 대상 node(modifier 속성이면 modifier id).
    pub target: StableId,
    /// 속성 key(속성 command가 아니면 `None`).
    pub key: Option<PropertyKey>,
}

/// grouping key: 연속 입력을 한 항목으로 합칠지 판단합니다(§23.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupingKey {
    /// pointer down부터 up까지의 drag·resize·rotate(VII.4.2). `gesture`는 조작 세션 번호입니다.
    PointerGesture {
        /// 조작 세션 번호.
        gesture: u64,
    },
    /// 같은 Text node의 연속 입력(VII.4.3). caret은 편집 전·후 위치(scalar index)입니다.
    TextInput {
        /// 대상 Text node.
        node: StableId,
        /// 입력 종류.
        kind: TextInputKind,
        /// 편집 전 caret.
        caret_before: u32,
        /// 편집 후 caret.
        caret_after: u32,
    },
    /// 키보드 이동·크기 조절 반복(VII.4.4).
    KeyboardRepeat(CommandSignature),
    /// slider·scrub 조작(VII.4.5). `control`은 조작 세션 번호입니다.
    Scrub {
        /// 조작 세션 번호.
        control: u64,
    },
    /// 영역 선택(VII.3.3): 최종 선택 하나만 기록합니다.
    AreaSelection,
}

/// 항목 내용.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryPayload {
    /// 문서 편집.
    Document {
        /// 정방향 transaction.
        forward: CommandTransaction,
        /// 역방향 transaction.
        inverse: CommandTransaction,
    },
    /// 선택 변경.
    Selection {
        /// 정방향 선택 event(걸러진 결과).
        forward: SetSelection,
        /// 이전 선택으로의 event.
        inverse: SetSelection,
    },
}

/// 문서 항목의 선택 복원 정보(VII.5.5): 편집 직전·직후 창 선택.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionRestore {
    /// undo 시 복원할 선택(편집 직전).
    pub before: Selection,
    /// redo 시 복원할 선택(편집 직후, 삭제 정리 반영).
    pub after: Selection,
}

/// history 항목(세션 메모리 전용).
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    /// 창 안 단조 순서.
    pub sequence: HistorySequence,
    /// 편집 출처.
    pub origin: CommandOrigin,
    /// grouping key(병합 대상이 아니면 `None`).
    pub grouping: Option<GroupingKey>,
    /// 내용.
    pub payload: HistoryPayload,
    /// 문서 항목의 선택 복원 정보(창 history가 기록, 없으면 복원하지 않음).
    pub selection: Option<SelectionRestore>,
}

impl HistoryEntry {
    /// Undo·Redo 메뉴의 동작 이름(코어가 정합니다, ADR-0079). 외부 변경 반영을 담은 항목은 "외부 변경
    /// 반영", 그 밖의 문서 항목은 첫 command 종류의 이름, 선택 항목은 "선택 변경"입니다.
    pub fn action_name(&self) -> &'static str {
        match &self.payload {
            HistoryPayload::Document { forward, .. } => {
                let kinds = || forward.requests.iter().map(|r| r.command.kind());
                if kinds().any(|kind| kind == CommandKind::ApplyExternalSnapshot) {
                    CommandKind::ApplyExternalSnapshot.action_name()
                } else {
                    kinds().next().map_or("편집", CommandKind::action_name)
                }
            }
            HistoryPayload::Selection { .. } => "선택 변경",
        }
    }

    /// 문서 편집 항목인지.
    pub fn is_document(&self) -> bool {
        matches!(self.payload, HistoryPayload::Document { .. })
    }

    /// 선택 변경 항목인지.
    pub fn is_selection(&self) -> bool {
        matches!(self.payload, HistoryPayload::Selection { .. })
    }
}

/// history 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryError {
    /// 문서 transaction 실행 실패(문서는 실행 전 상태).
    Transaction(TransactionError),
    /// sequence 한계.
    SequenceOverflow,
    /// preview(gesture·scrub)가 진행 중이라 다른 기록·preview를 시작할 수 없습니다.
    PreviewActive,
    /// 진행 중인 preview가 없습니다.
    NoPreview,
    /// undo·redo 재생 중이라 새 문서 편집을 기록할 수 없습니다(VII.5.5 reentrancy guard).
    Replaying,
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryError::Transaction(error) => write!(f, "{error}"),
            HistoryError::SequenceOverflow => f.write_str("history sequence가 한계에 도달했습니다"),
            HistoryError::PreviewActive => f.write_str("진행 중인 preview가 있습니다"),
            HistoryError::NoPreview => f.write_str("진행 중인 preview가 없습니다"),
            HistoryError::Replaying => {
                f.write_str("undo·redo 재생 중에는 새 편집을 기록할 수 없습니다")
            }
        }
    }
}

impl std::error::Error for HistoryError {}

impl From<TransactionError> for HistoryError {
    fn from(error: TransactionError) -> HistoryError {
        HistoryError::Transaction(error)
    }
}

#[cfg(test)]
mod tests {
    use crate::history::test_fixtures::{sid, small_state, tx};
    use crate::journal::HistoryJournal;
    use crate::window::WindowHistory;
    use stellar_commands::command::Command;
    use stellar_commands::document::definitions::ViewDefinition;
    use stellar_commands::document::model::DocumentModel;
    use stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_commands::id::{CommandId, WindowSessionId};
    use stellar_commands::snapshot_apply::ApplyExternalSnapshot;
    use stellar_commands::state::DocumentState;

    fn snapshot(doc: &DocumentState) -> String {
        model_snapshot_json(doc.model()).unwrap()
    }

    #[test]
    fn external_snapshot_is_one_history_entry_named_by_the_core() {
        // UX.4.1 코어: 외부 원본 반영이 history 항목 하나이고, Undo 한 번으로 되돌아가며, 메뉴 이름은 코어가
        // 정합니다. journal에도 그대로 기록·복원됩니다.
        let mut history = WindowHistory::new(WindowSessionId::from_stable_id(sid(800)));
        let mut doc = small_state();
        let rename = tx(&doc, 1, Command::rename_node(sid(11), "앱 편집"));
        history.record_document(&mut doc, &rename, None).unwrap();
        assert_eq!(history.history().entries()[0].action_name(), "속성 변경");
        let before = snapshot(&doc);
        // 다른 프로세스가 쓴 새 원본: 12 삭제, 이름 변경, 새 View.
        let mut incoming = doc.model().to_records().unwrap();
        incoming
            .nodes
            .retain(|n| n.id != sid(12) && n.id != sid(13));
        incoming
            .nodes
            .iter_mut()
            .for_each(|n| n.display_name = "외부".into());
        incoming.definition.pages[0]
            .add_view(ViewDefinition::new(sid(30), "새".into(), sid(31)))
            .unwrap();
        let mut root = incoming.nodes[0].clone();
        root.id = sid(31);
        incoming.nodes.push(root);
        let command = ApplyExternalSnapshot::between(doc.model(), &incoming).unwrap();
        let transaction =
            command.into_transaction(CommandId::from_stable_id(sid(901)), doc.revision());
        history
            .record_document(&mut doc, &transaction, None)
            .unwrap();
        let expected =
            model_snapshot_json(&DocumentModel::from_records(incoming).unwrap().0).unwrap();
        assert_eq!(snapshot(&doc), expected);
        let entries = history.history().entries();
        assert_eq!(entries.len(), 2, "외부 반영은 항목 하나");
        assert_eq!(entries[1].action_name(), "외부 변경 반영");
        assert_eq!(entries[1].origin, ApplyExternalSnapshot::ORIGIN);
        // journal 왕복.
        let journal = HistoryJournal::capture(
            history.window(),
            crate::journal::tests::base(3),
            history.history(),
        );
        assert_eq!(HistoryJournal::decode(&journal.encode()).unwrap(), journal);
        history.undo(&mut doc).unwrap().unwrap();
        assert_eq!(snapshot(&doc), before, "Undo 한 번");
        history.redo(&mut doc).unwrap().unwrap();
        assert_eq!(snapshot(&doc), expected, "Redo");
    }
}
