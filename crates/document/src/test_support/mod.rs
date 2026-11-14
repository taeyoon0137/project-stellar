//! **테스트 전용** 지원 코드 (VI.10.4~).
//!
//! 이 모듈은 테스트(이 crate의 unit test와 다른 crate의 통합 테스트)에서만 사용합니다.
//! 프로덕션 저장·로드 경로는 이 모듈을 호출하지 않습니다 — 저장 형식은 chunk model
//! (VI.10.2)뿐이고 semantic snapshot JSON은 비교·diff 용도입니다.
//!
//! 프로덕션 빌드에는 **컴파일되지 않습니다**: `cfg(test)` 또는 기본값이 꺼진 `test-support`
//! feature에서만 존재합니다. 다른 crate의 통합 테스트는 `[dev-dependencies]`에서 그 feature를
//! 켭니다. `semantic_snapshot` 테스트가 (1) 이 crate와 workspace의 모든 다른 crate의 비테스트
//! 소스가 이 모듈을 참조하지 않는지, (2) 어떤 manifest도 `[dev-dependencies]` 밖에서 feature를
//! 켜지 않는지 검사해 고정합니다. 문서에서는 숨깁니다(`#[doc(hidden)]`). 새 의존성 없이 직접
//! 구현합니다.

/// 결정론적 JSON 값과 출력기.
pub mod json;

/// canonical semantic snapshot JSON (VI.10.4).
pub mod semantic_snapshot;

/// seed 기반 random valid document 생성기 (VI.10.5).
pub mod random_document;

/// compact/error/dark nullable Variant 조합 fixture (IX.9.3).
pub mod variant_fixtures;

/// Button·TextInput·Modal·List·Radio 합성 component library fixture (IX.9.1).
pub mod component_fixtures;

/// document loader/validator fuzz harness (XVI.6.3).
pub mod loader_fuzz;
