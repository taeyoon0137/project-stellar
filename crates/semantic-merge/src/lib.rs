//! stellar-semantic-merge — Project Stellar 3-way Semantic Merge (Phase XIV.10). 의존: semantic-diff.
//!
//! base·ours·theirs [`SemanticSnapshot`](stellar_semantic_diff::snapshot::SemanticSnapshot)을
//! `StableId + PropertyPath` 단위로 비교해 겹치지 않는 변경을 자동 병합하고, 겹치는 변경은
//! base·ours·theirs 상태를 모두 담은 conflict로 남깁니다(아키텍처 §27).

/// merge 입력·결과·conflict 모델 (XIV.10.1).
pub mod model;

/// 3-way merge 엔진 (XIV.10.2~).
pub mod merge;

/// 독립 CandidateSnapshot C와 해결 모델 (XXIII.2, XIV.12).
pub mod candidate;

/// Semantic Diff/Merge conformance corpus runner (XXVIII.2.3).
pub mod conformance;
