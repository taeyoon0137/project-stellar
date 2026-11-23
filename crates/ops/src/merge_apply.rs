//! 병합 결과 적용 (XIII.3.4, XXIII.3.1~XXIII.3.4, ADR-0110).
//!
//! 두 흐름이 같은 병합 판정([`stellar_semantic_merge::apply::reconcile`])과 같은 역투영(semantic snapshot →
//! 문서 레코드)을 씁니다.
//!
//! - [`document`]: 열린 창 문서 하나의 base(LoadedBase)·ours(CurrentMemory)·theirs(ExternalDisk) 3-way. runtime
//!   창 session이 외부 writer 변경과 로컬 편집이 겹칠 때 부릅니다(XIII.3.4). 파일을 쓰지 않습니다.
//! - [`target`]: Merge Composer 결과 target(current worktree·existing worktree·new branch+worktree)의 정의와 검증,
//!   새 branch+worktree 생성(시스템 Git만, XXIII.3.1·XXIII.3.2).
//! - `composer`: Source A·B·Base snapshot에서 C를 만들고(XXIII.1·XXIII.2), 적용 전에 source revision·OID를 다시
//!   검사한 뒤(XXIII.3.4) C를 target WorkingTree에 generation save합니다(XXIII.3.3). merge commit·commit·
//!   continue는 만들지 않습니다.

/// 창 문서 하나의 3-way reconciliation(XIII.3.4).
pub mod document;

/// 결과 target 정의·검증·생성(XXIII.3.1·XXIII.3.2).
pub mod target;

#[cfg(test)]
mod tests;
