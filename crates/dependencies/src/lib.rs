//! stellar-dependencies — Git 기반 외부 Component·Token library 의존성 모델 (Phase XV).
//!
//! 의존: core-types만. `stellar.yaml`·`stellar.lock.yaml` 텍스트(제한 YAML), 시스템 Git 실행,
//! 프로젝트 probe·저장소 검증은 project crate가 이 crate의 trait과 형식 중립 트리([`node`])를
//! 구현·연결합니다.

/// 형식 중립 선언 트리와 스키마 오류 (XV.1.1).
pub mod node;

/// `stellar.yaml` 의존성 선언 모델 (XV.1.1).
pub mod declaration;

/// version 선택자 해석 계약과 lock pin (XV.1.2).
pub mod selection;

/// 의존성 선언 충돌 validation (XV.1.3).
pub mod validation;

/// 의존성 선언 민감값 분류와 redaction (XV.1.4).
pub mod sensitivity;

/// 의존성 resolver 그래프 (XV.2).
pub mod graph;

/// 전이 의존성 resolver (XV.2.2).
pub mod resolver;

/// 시스템 Git 실행 경계 (XV.2.4).
pub mod git;

/// 시스템 Git 기반 version 선택자 해석 (XV.2.4).
pub mod git_refs;

/// library cache entry content hash 문자열 (XV.3.1).
pub mod content_hash;

/// 사용자별 library cache root (XV.3.1).
pub mod cache_root;

/// library cache index (XV.3.1).
pub mod cache_index;

/// repository별 bare mirror cache (XV.3.2).
pub mod mirror;

/// content-addressed immutable library cache entry (XV.3.3).
pub mod cache_entry;

/// cache read lock과 fetch/write lock 분리 (XV.3.6).
pub mod cache_lock;

/// library cache entry 검증 (XV.3.4).
pub mod entry_validation;

/// cache index privacy mode와 local source config (XV.3.5).
pub mod cache_privacy;

/// resolver 그래프 → lock resolved entry 변환 (XV.4.1)과 entry validator (XV.4.2).
pub mod lock;

/// lockfile content hash와 cache entry 무결성 검사 (XV.4.4).
pub mod lock_integrity;

/// local path override 매핑 (XV.6.1)과 대상 검증 (XV.6.2).
pub mod local_override;

/// 읽기 전용 library snapshot과 resource key (XV.5.1).
pub mod library_snapshot;

/// 외부 library component·token 참조 (XV.5.2).
pub mod external_ref;

/// 사용 프로젝트의 library resource 읽기 전용 정책 (XV.5.3).
pub mod read_only;

/// 외부 참조 scope 해석과 전이 resource ref (XV.5.4).
pub mod ref_resolution;

/// lock 기준 library 로드와 부분 열기 (XV.5.5).
pub mod library_load;

/// 다중 버전 경고 상세: 경로·예상 추가 용량·UI/telemetry 분리 (XV.8.3).
pub mod duplicate_report;

#[cfg(test)]
pub(crate) mod test_support;
