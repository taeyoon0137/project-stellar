//! stellar-accessibility — 시각 layer와 독립된 접근성 의미 모델(기술 아키텍처 §18). 의존: core-types.
//!
//! crate 그래프(`docs/architecture/crate-dependency-graph.md`)상 이 crate는 core-types에만 의존합니다. 문서
//! 모델(document)·binding(bindings) 값을 직접 읽지 않고 StableId·PropertyPath·TypedValue로 출처를 가리킵니다.

/// canonical binary 도우미 (IX.8.1).
pub mod codec;

/// 접근성 의미 node (IX.8.1).
pub mod node;

/// 시각 tree와 별도 parent/child 관계를 갖는 접근성 트리 (IX.8.2).
pub mod tree;

/// 생성기 입력: 중립 시각 source tree (IX.8.3).
pub mod source;

/// 기본 접근성 트리 생성기 (IX.8.3).
pub mod generate;

/// Control contract → role·state·action 매핑 (IX.8.4).
pub mod control;

/// 접근성 진단: label 누락, 잘못된 중첩, focus·동작 불일치, 읽기 순서 중복 (IX.8.5).
pub mod diagnostics;

/// Target adapter용 중립 접근성 snapshot (IX.8.6).
pub mod snapshot;

#[cfg(test)]
pub(crate) mod test_support;
