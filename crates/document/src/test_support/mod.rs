//! **테스트 전용** 지원 코드 (VI.10.4~).
//!
//! 이 모듈은 테스트(이 crate의 unit test와 다른 crate의 통합 테스트)에서만 사용합니다.
//! 프로덕션 저장·로드 경로는 이 모듈을 호출하지 않습니다 — 저장 형식은 chunk model
//! (VI.10.2)뿐이고 semantic snapshot JSON은 비교·diff 용도입니다. 이 crate의 비테스트
//! 코드가 이 모듈을 참조하지 않는다는 사실은 `semantic_snapshot` 테스트가 소스를 검사해
//! 고정합니다.
//!
//! 다른 crate의 통합 테스트도 쓸 수 있도록 `cfg(test)`가 아닌 공개 모듈로 두되 문서에서는
//! 숨깁니다(`#[doc(hidden)]`). 새 의존성 없이 직접 구현합니다.

/// 결정론적 JSON 값과 출력기.
pub mod json;

/// canonical semantic snapshot JSON (VI.10.4).
pub mod semantic_snapshot;
