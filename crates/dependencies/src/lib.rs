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

#[cfg(test)]
pub(crate) mod test_support;
