//! stellar-project — 프로젝트 수명주기 골격. 의존: document, storage, assets, dependencies.

/// 제한 YAML 지원 (IV.1).
pub mod yaml;

/// `stellar.yaml` 매니페스트 데이터 모델 (IV.1.1).
pub mod manifest;

/// `stellar.lock.yaml` 잠금 파일 데이터 모델 (IV.1.6).
pub mod lock;

/// 플랫폼 공통 폴더 이름 검증기 (IV.2.1).
pub mod folder_name;

/// 프로젝트 root 검증기 (IV.2.2).
pub mod root;

/// 프로젝트 식별 (IV.2.3).
pub mod identity;

/// 새 프로젝트 생성 요청과 사전 검증 (IV.4.1).
pub mod create;

/// 폴더 생성 트랜잭션 (IV.4.2).
pub mod fs_transaction;
