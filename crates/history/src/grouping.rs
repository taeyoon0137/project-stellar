//! history grouping 정책 (VII.4.3~VII.4.5).
//!
//! 도구별 병합 예외는 공통 [`HistoryGroupingPolicy`]만을 통해 정의합니다(기술 아키텍처 §23.3).
//! 시간 조건은 wall clock이 아니라 주입한 [`MonotonicClock`](stellar_core_types::clock::MonotonicClock)
//! 의 nanoseconds로 판단하므로 테스트는 `ManualClock`으로 결정론적으로 재현합니다.
//!
//! - **텍스트 입력(VII.4.3)**: 같은 Text node, 같은 입력 종류, 인접 caret(새 입력의 편집 전 caret
//!   = 직전 입력의 편집 후 caret), 직전 입력 뒤 [`GroupingConfig::text_merge_interval_nanos`] 이내.
//!
//! 간격 값은 [`GroupingConfig`]에 둡니다. 기본값은 사용성 벤치마크 전 잠정값입니다.

use crate::entry::GroupingKey;

/// grouping 시간 조건 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupingConfig {
    /// 텍스트 연속 입력 병합 간격(ns). 직전 입력 뒤 이 시간 안의 입력만 합칩니다.
    pub text_merge_interval_nanos: u64,
}

impl GroupingConfig {
    /// 기본 텍스트 병합 간격: 1초(잠정값).
    pub const DEFAULT_TEXT_MERGE_INTERVAL_NANOS: u64 = 1_000_000_000;
}

impl Default for GroupingConfig {
    fn default() -> GroupingConfig {
        GroupingConfig {
            text_merge_interval_nanos: GroupingConfig::DEFAULT_TEXT_MERGE_INTERVAL_NANOS,
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
                prev_node == node
                    && prev_kind == kind
                    && prev_after == caret_before
                    && elapsed <= self.config.text_merge_interval_nanos
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
        });
        assert!(!strict.should_merge(&typed, 0, &text(20, TextInputKind::Insert, 1, 2), 20 * ms));
        assert_eq!(strict.config().text_merge_interval_nanos, 10 * ms);
    }
}
