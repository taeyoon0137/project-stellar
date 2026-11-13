//! Merge Composer source 선택과 snapshot (XXIII.1.*).
//!
//! 아키텍처 27.4의 로드맵 기능입니다. 임의의 Source A·B(branch·commit·tag·worktree·HEAD·index·작업
//! 트리)를 시스템 Git으로 해석하고, 이후 단계(공통 이력 base, snapshot materialize)가 그 결과를 씁니다.
//! UI는 이 crate에 없습니다.

/// source 정의와 해석(XXIII.1.1).
pub mod source;

pub use source::{resolve_source, ResolvedSource, SourceError, SourceRef, WorktreeState};

/// 공통 이력 base와 비교 방식(XXIII.1.2).
pub mod base;

pub use base::{comparison_mode, ComparisonMode, TwoWayReason};

/// Source A·B·Base snapshot materialize(XXIII.1.3).
pub mod snapshot;

pub use snapshot::{
    materialize_source, materialize_sources, ComposerSnapshotError, ComposerSnapshots,
    SourceSnapshot,
};
