//! ring buffer 기반 로컬 trace store (XVI.1.5, ADR-0084 결정 8).
//!
//! 완료 trace([`CompletedTrace`])를 고정 용량 ring buffer에 보관합니다. 용량을 넘으면 가장 오래된 trace부터
//! 지우고 지운 수를 셉니다. 모든 항목이 고정 크기 값이라 메모리 상한은 `capacity × size_of::<CompletedTrace>()`
//! (+ 고정 overhead)입니다. store는 로컬 진단 전용이며 전송하지 않습니다(아키텍처 §39).

use std::collections::VecDeque;

use crate::interaction::CompletedTrace;

/// 기본 용량(trace 수). 120 Hz로 약 34초 분량입니다.
pub const DEFAULT_TRACE_CAPACITY: usize = 4_096;

/// 용량 상한(오입력 방지).
pub const MAX_TRACE_CAPACITY: usize = 1 << 20;

/// 고정 용량 ring buffer trace store.
#[derive(Debug, Clone)]
pub struct TraceStore {
    capacity: usize,
    traces: VecDeque<CompletedTrace>,
    dropped: u64,
}

impl TraceStore {
    /// `capacity`개를 보관하는 store. 0은 1로, [`MAX_TRACE_CAPACITY`]보다 크면 상한으로 맞춥니다.
    pub fn new(capacity: usize) -> TraceStore {
        let capacity = capacity.clamp(1, MAX_TRACE_CAPACITY);
        TraceStore {
            capacity,
            traces: VecDeque::with_capacity(capacity),
            dropped: 0,
        }
    }

    /// trace를 넣습니다. 가득 차 있으면 가장 오래된 trace를 지우고 돌려줍니다.
    pub fn push(&mut self, trace: CompletedTrace) -> Option<CompletedTrace> {
        let evicted = if self.traces.len() == self.capacity {
            self.dropped = self.dropped.saturating_add(1);
            self.traces.pop_front()
        } else {
            None
        };
        self.traces.push_back(trace);
        evicted
    }

    /// 용량.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// 보관한 trace 수.
    pub fn len(&self) -> usize {
        self.traces.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.traces.is_empty()
    }

    /// 용량 초과로 지운 trace 수.
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// 오래된 것부터 순회합니다.
    pub fn iter(&self) -> impl Iterator<Item = &CompletedTrace> + '_ {
        self.traces.iter()
    }

    /// 보관 bytes 상한(항목 크기 × 용량).
    pub const fn memory_bound_bytes(&self) -> usize {
        self.capacity * std::mem::size_of::<CompletedTrace>()
    }

    /// 지금 할당한 항목 bytes.
    pub fn allocated_bytes(&self) -> usize {
        self.traces.capacity() * std::mem::size_of::<CompletedTrace>()
    }
}

impl Default for TraceStore {
    fn default() -> TraceStore {
        TraceStore::new(DEFAULT_TRACE_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::{
        CacheState, DocumentSizeBucket, InteractionKind, InteractionTrace, InteractionTraceIds,
        Stage, TraceDimensions, VisibleNodeBucket,
    };

    fn completed(ids: &mut InteractionTraceIds, at: u64) -> CompletedTrace {
        let mut trace = InteractionTrace::begin(
            ids.issue(),
            TraceDimensions {
                kind: InteractionKind::Pan,
                document: DocumentSizeBucket::UpTo1k,
                visible: VisibleNodeBucket::UpTo1k,
                cache: CacheState::Warm,
            },
            at,
        );
        for stage in &Stage::ALL[1..] {
            trace.mark(*stage, at + 1).unwrap();
        }
        trace.validate().unwrap()
    }

    #[test]
    fn oldest_traces_are_evicted_and_memory_stays_bounded() {
        // 검증 기준(XVI.1.5): memory bound 유지.
        let mut ids = InteractionTraceIds::new();
        let mut store = TraceStore::new(64);
        let bound = store.memory_bound_bytes();
        let mut evicted = Vec::new();
        for n in 0..640u64 {
            if let Some(old) = store.push(completed(&mut ids, n)) {
                evicted.push(old.id().get());
            }
            assert!(store.len() <= 64);
            // VecDeque는 처음 잡은 용량을 넘겨 자라지 않습니다.
            assert!(
                store.allocated_bytes() <= bound * 2,
                "{}",
                store.allocated_bytes()
            );
        }
        assert_eq!(store.len(), 64);
        assert_eq!(store.dropped(), 576);
        // 가장 오래된 것부터 지웠습니다.
        assert_eq!(evicted.first(), Some(&1));
        assert_eq!(evicted.last(), Some(&576));
        let kept: Vec<u64> = store.iter().map(|t| t.id().get()).collect();
        assert_eq!(kept, (577..=640).collect::<Vec<_>>());
    }

    #[test]
    fn capacity_is_clamped() {
        assert_eq!(TraceStore::new(0).capacity(), 1);
        assert_eq!(TraceStore::new(usize::MAX).capacity(), MAX_TRACE_CAPACITY);
        assert_eq!(TraceStore::default().capacity(), DEFAULT_TRACE_CAPACITY);
    }
}
