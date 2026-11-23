//! 실제 index → semantic 선택 → 임시 저장 후보 → 기존 loader 재열기(XIV.5.5)와 Git 상태 worker(UX.5.1).
//!
//! 선택 staging 파이프라인은 CLI와 같은 구현을 쓰도록 `stellar_ops::git`에 있고 여기서 재노출합니다(ADR-0080).
//! 동기식 API이며 호출자가 실행 문맥을 고릅니다. prepare는 읽기 전용이고, apply는 gated Git으로 index만
//! 적용·검증·조건부 복원합니다. 작업 트리·문서는 쓰지 않습니다.
//!
//! Git 상태 조회는 [`submit_git_status`]로 worker pool에서만 실행합니다. 결과에는 계산 revision과 요청
//! 순번이 붙으므로 받는 쪽은 더 새 순번의 결과만 반영합니다.

pub use stellar_ops::git::{
    apply_stage_candidate, prepare_selection_with_units, prepare_stage_candidate,
    prepare_stage_candidate_with_preview, prepare_unstage_candidate, ApplyStageError,
    PrepareStageError, PreparedStageCandidate,
};

mod status_worker;

pub use status_worker::{submit_git_status, GitStatusJob, GitStatusResult, RejectedGitStatus};

#[cfg(all(test, unix))]
mod tests;

