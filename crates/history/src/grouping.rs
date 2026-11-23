//! history grouping 정책 (VII.4.3~VII.4.5).
//!
//! 도구별 병합 예외는 공통 [`HistoryGroupingPolicy`]만을 통해 정의합니다(기술 아키텍처 §23.3).
//! 시간 조건은 wall clock이 아니라 주입한 [`MonotonicClock`](stellar_core_types::clock::MonotonicClock)
//! 의 nanoseconds로 판단하므로 테스트는 `ManualClock`으로 결정론적으로 재현합니다.
//!
//! - **텍스트 입력(VII.4.3)**: 같은 Text node, 같은 입력 종류, 인접 caret(새 입력의 편집 전 caret
//!   = 직전 입력의 편집 후 caret), 직전 입력 뒤 [`GroupingConfig::text_merge_interval_nanos`] 이내.
//!
//! - **키보드 이동·크기 조절 반복(VII.4.4)**: 같은 [`CommandSignature`](종류·대상·속성), 직전 입력 뒤
//!   [`GroupingConfig::keyboard_repeat_interval_nanos`] 이내.
//!
//! 간격 값은 [`GroupingConfig`]에 둡니다. 기본값은 사용성 벤치마크 전 잠정값입니다.

use stellar_commands::command::Command;
use stellar_commands::text_edit::ReplaceText;
use stellar_commands::transaction::CommandTransaction;

use crate::entry::{CommandSignature, GroupingKey, TextInputKind};

impl CommandSignature {
    /// 요청 하나짜리 속성·이동 transaction의 signature. 그 밖의 transaction은 반복 병합 대상이
    /// 아니라 `None`입니다.
    pub fn of(transaction: &CommandTransaction) -> Option<CommandSignature> {
        let [request] = transaction.requests.as_slice() else {
            return None;
        };
        let kind = request.command.kind();
        match &request.command {
            Command::SetProperty(command) => Some(CommandSignature {
                kind,
                target: command.node,
                key: Some(command.key),
            }),
            Command::SetModifierProperty(command) => Some(CommandSignature {
                kind,
                target: command.modifier.stable_id(),
                key: Some(command.key),
            }),
            Command::MoveNode(command) => Some(CommandSignature {
                kind,
                target: command.node,
                key: None,
            }),
            _ => None,
        }
    }
}

impl GroupingKey {
    /// 키보드 반복 입력의 grouping key(signature가 없으면 `None`).
    pub fn keyboard(transaction: &CommandTransaction) -> Option<GroupingKey> {
        CommandSignature::of(transaction).map(GroupingKey::KeyboardRepeat)
    }

    /// 확정한 Text 편집 하나의 grouping key(XII.10.32). caret은 scalar index입니다.
    ///
    /// | 입력 종류 | 편집 전 caret | 편집 후 caret |
    /// |---|---|---|
    /// | `Insert`·`Replace` | 범위 끝 | 범위 시작 + 확정 문자열 길이 |
    /// | `DeleteBackward` | 범위 끝 | 범위 시작 |
    /// | `DeleteForward` | 범위 시작 | 범위 시작 |
    ///
    /// 빈 범위에 넣는 입력이면 `Insert`, 선택을 지우는 입력이면 삭제 종류, 선택을 다른 문자열로 바꾸면
    /// `Replace`를 씁니다. IME 조합 중 문자열은 command가 아니므로 확정 문자열만 넘깁니다.
    pub fn text_edit(edit: &ReplaceText, kind: TextInputKind) -> GroupingKey {
        let (start, end) = (edit.range.start.0, edit.range.end.0);
        let typed = stellar_commands::document::text::scalar_len(&edit.replacement);
        let (caret_before, caret_after) = match kind {
            TextInputKind::Insert | TextInputKind::Replace => (end, start.saturating_add(typed)),
            TextInputKind::DeleteBackward => (end, start),
            TextInputKind::DeleteForward => (start, start),
        };
        GroupingKey::TextInput {
            node: edit.node,
            kind,
            caret_before,
            caret_after,
        }
    }
}

/// grouping 시간 조건 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupingConfig {
    /// 텍스트 연속 입력 병합 간격(ns). 직전 입력 뒤 이 시간 안의 입력만 합칩니다.
    pub text_merge_interval_nanos: u64,
    /// 키보드 이동·크기 조절 반복 병합 간격(ns).
    pub keyboard_repeat_interval_nanos: u64,
}

impl GroupingConfig {
    /// 기본 텍스트 병합 간격: 1초(잠정값).
    pub const DEFAULT_TEXT_MERGE_INTERVAL_NANOS: u64 = 1_000_000_000;

    /// 기본 키보드 반복 병합 간격: 0.5초(잠정값, OS key repeat 간격보다 깁니다).
    pub const DEFAULT_KEYBOARD_REPEAT_INTERVAL_NANOS: u64 = 500_000_000;
}

impl Default for GroupingConfig {
    fn default() -> GroupingConfig {
        GroupingConfig {
            text_merge_interval_nanos: GroupingConfig::DEFAULT_TEXT_MERGE_INTERVAL_NANOS,
            keyboard_repeat_interval_nanos: GroupingConfig::DEFAULT_KEYBOARD_REPEAT_INTERVAL_NANOS,
        }
    }
}

/// 공통 grouping 정책.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistoryGroupingPolicy {
    config: GroupingConfig,
}

impl HistoryGroupingPolicy {
    /// 설정으로 정책을 만듭니다.
    pub fn new(config: GroupingConfig) -> HistoryGroupingPolicy {
        HistoryGroupingPolicy { config }
    }

    /// 설정.
    pub fn config(&self) -> GroupingConfig {
        self.config
    }

    /// 직전 항목(`previous`, 마지막 입력 시각 `previous_nanos`)에 새 입력(`next`, `now_nanos`)을
    /// 합칠지 판단합니다.
    pub fn should_merge(
        &self,
        previous: &GroupingKey,
        previous_nanos: u64,
        next: &GroupingKey,
        now_nanos: u64,
    ) -> bool {
        let Some(elapsed) = now_nanos.checked_sub(previous_nanos) else {
            return false;
        };
        match (previous, next) {
            (
                GroupingKey::TextInput {
                    node: prev_node,
                    kind: prev_kind,
                    caret_after: prev_after,
                    ..
                },
                GroupingKey::TextInput {
                    node,
                    kind,
                    caret_before,
                    ..
                },
            ) => {
                *kind != TextInputKind::Replace
                    && prev_node == node
                    && prev_kind == kind
                    && prev_after == caret_before
                    && elapsed <= self.config.text_merge_interval_nanos
            }
            (GroupingKey::KeyboardRepeat(previous), GroupingKey::KeyboardRepeat(next)) => {
                previous == next && elapsed <= self.config.keyboard_repeat_interval_nanos
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::TextInputKind;
    use crate::history::test_fixtures::sid;

    fn text(node: u64, kind: TextInputKind, before: u32, after: u32) -> GroupingKey {
        GroupingKey::TextInput {
            node: sid(node),
            kind,
            caret_before: before,
            caret_after: after,
        }
    }

    #[test]
    fn continuous_text_input_merges_into_one_history_entry() {
        // XII.10.32: 연속 입력 merge fixture와 한글 조합 확정 fixture.
        use crate::window::WindowHistory;
        use stellar_commands::command::Command;
        use stellar_commands::document::model::DocumentModel;
        use stellar_commands::document::records::{NodePayload, NodeRecord};
        use stellar_commands::document::text::TextPayload;
        use stellar_commands::id::{CommandId, CommandOrigin, WindowSessionId};
        use stellar_commands::state::DocumentState;
        use stellar_commands::transaction::CommandTransaction;
        use stellar_core_types::clock::ManualClock;
        use stellar_core_types::order_key::OrderKey;

        let mut model = DocumentModel::from_records(
            crate::history::test_fixtures::small_state()
                .model()
                .to_records()
                .unwrap(),
        )
        .unwrap()
        .0;
        model
            .insert_node(NodeRecord {
                id: sid(20),
                parent: Some(sid(10)),
                order_key: OrderKey::from_bytes(vec![0x30]).unwrap(),
                visible: true,
                locked: false,
                display_name: "text".into(),
                payload: NodePayload::Text(TextPayload::empty(0)),
            })
            .unwrap();
        let mut doc = DocumentState::new(model);
        let mut history = WindowHistory::new(WindowSessionId::from_stable_id(sid(820)));
        let mut clock = ManualClock::new(0);
        let ms = 1_000_000u64;
        let mut seq = 0u64;
        let mut type_text = |history: &mut WindowHistory,
                             doc: &mut DocumentState,
                             clock: &ManualClock,
                             start: u32,
                             end: u32,
                             text: &str,
                             kind: TextInputKind| {
            seq += 1;
            let edit = ReplaceText::new(sid(20), start, end, text.into()).unwrap();
            let key = GroupingKey::text_edit(&edit, kind);
            let transaction = CommandTransaction::new(
                CommandId::from_stable_id(sid(9_000 + seq)),
                CommandOrigin::User,
            )
            .with_command(doc.revision(), Command::ReplaceText(edit));
            history
                .record_grouped(doc, &transaction, key, clock)
                .unwrap();
        };
        let content = |doc: &DocumentState| match doc.model().payload(sid(20)) {
            Some(NodePayload::Text(text)) => text.content().to_string(),
            other => panic!("{other:?}"),
        };
        // 한글: 조합(ㅎ→하→한)은 transient이고 확정한 음절만 command입니다. "한" "글"은 인접 caret이라 합쳐집니다.
        type_text(
            &mut history,
            &mut doc,
            &clock,
            0,
            0,
            "한",
            TextInputKind::Insert,
        );
        clock.advance(200 * ms);
        type_text(
            &mut history,
            &mut doc,
            &clock,
            1,
            1,
            "글",
            TextInputKind::Insert,
        );
        clock.advance(200 * ms);
        type_text(
            &mut history,
            &mut doc,
            &clock,
            2,
            2,
            "!",
            TextInputKind::Insert,
        );
        assert_eq!(content(&doc), "한글!");
        assert_eq!(
            history.history().entries().len(),
            1,
            "연속 입력은 항목 하나"
        );
        // Backspace 두 번도 하나로 합쳐지고, 입력과는 다른 항목입니다.
        clock.advance(200 * ms);
        type_text(
            &mut history,
            &mut doc,
            &clock,
            2,
            3,
            "",
            TextInputKind::DeleteBackward,
        );
        clock.advance(200 * ms);
        type_text(
            &mut history,
            &mut doc,
            &clock,
            1,
            2,
            "",
            TextInputKind::DeleteBackward,
        );
        assert_eq!(content(&doc), "한");
        assert_eq!(history.history().entries().len(), 2);
        // 선택 교체는 앞 입력과 합치지 않습니다.
        clock.advance(200 * ms);
        type_text(
            &mut history,
            &mut doc,
            &clock,
            0,
            1,
            "韓",
            TextInputKind::Replace,
        );
        clock.advance(200 * ms);
        type_text(
            &mut history,
            &mut doc,
            &clock,
            1,
            1,
            "국",
            TextInputKind::Insert,
        );
        assert_eq!(content(&doc), "韓국");
        assert_eq!(history.history().entries().len(), 4);
        // 간격을 넘긴 입력은 새 항목입니다.
        clock.advance(2_000 * ms);
        type_text(
            &mut history,
            &mut doc,
            &clock,
            2,
            2,
            ".",
            TextInputKind::Insert,
        );
        assert_eq!(history.history().entries().len(), 5);
        // undo 한 번이 합쳐진 입력 전체를 되돌립니다.
        for _ in 0..4 {
            history.undo(&mut doc).unwrap().unwrap();
        }
        assert_eq!(content(&doc), "한글!");
        history.undo(&mut doc).unwrap().unwrap();
        assert_eq!(content(&doc), "");
        // grouping key는 journal에 그대로 기록됩니다(Replace tag 포함).
        let key = GroupingKey::text_edit(
            &ReplaceText::new(sid(20), 0, 1, "ab".into()).unwrap(),
            TextInputKind::Replace,
        );
        assert_eq!(
            key,
            GroupingKey::TextInput {
                node: sid(20),
                kind: TextInputKind::Replace,
                caret_before: 1,
                caret_after: 2
            }
        );
    }

    #[test]
    fn text_merge_conditions() {
        let policy = HistoryGroupingPolicy::default();
        let ms = 1_000_000u64;
        let typed = text(20, TextInputKind::Insert, 0, 1);
        // 인접 caret·같은 node·같은 종류·간격 이내 → 병합.
        assert!(policy.should_merge(&typed, 0, &text(20, TextInputKind::Insert, 1, 2), 300 * ms));
        // 경계: 간격과 같으면 병합, 넘으면 분리.
        assert!(policy.should_merge(
            &typed,
            0,
            &text(20, TextInputKind::Insert, 1, 2),
            1_000 * ms
        ));
        assert!(!policy.should_merge(
            &typed,
            0,
            &text(20, TextInputKind::Insert, 1, 2),
            1_000 * ms + 1
        ));
        // 다른 node, 다른 종류, 비인접 caret, 시간 역행 → 분리.
        assert!(!policy.should_merge(&typed, 0, &text(21, TextInputKind::Insert, 1, 2), ms));
        assert!(!policy.should_merge(
            &typed,
            0,
            &text(20, TextInputKind::DeleteBackward, 1, 0),
            ms
        ));
        assert!(!policy.should_merge(&typed, 0, &text(20, TextInputKind::Insert, 5, 6), ms));
        assert!(!policy.should_merge(&typed, 10, &text(20, TextInputKind::Insert, 1, 2), 5));
        // 텍스트가 아닌 key는 정책 병합 대상이 아닙니다.
        let gesture = GroupingKey::PointerGesture { gesture: 1 };
        assert!(!policy.should_merge(&gesture, 0, &gesture, 0));
        // 간격 값은 설정을 따릅니다.
        let strict = HistoryGroupingPolicy::new(GroupingConfig {
            text_merge_interval_nanos: 10 * ms,
            ..GroupingConfig::default()
        });
        assert!(!strict.should_merge(&typed, 0, &text(20, TextInputKind::Insert, 1, 2), 20 * ms));
        assert_eq!(strict.config().text_merge_interval_nanos, 10 * ms);
    }
}
