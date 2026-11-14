//! stellar-accessibility — 시각 layer와 독립된 접근성 의미 모델(기술 아키텍처 §18). 의존: core-types.
//!
//! crate 그래프(`docs/architecture/crate-dependency-graph.md`)상 이 crate는 core-types에만 의존합니다. 문서
//! 모델(document)·binding(bindings) 값을 직접 읽지 않고 StableId·PropertyPath·TypedValue로 출처를 가리킵니다.

/// canonical binary 도우미 (IX.8.1).
pub mod codec;

/// 접근성 의미 node (IX.8.1).
pub mod node;

#[cfg(test)]
pub(crate) mod test_support;
