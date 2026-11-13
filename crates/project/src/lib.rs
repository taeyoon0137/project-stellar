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

/// 기본 프로젝트 scaffold 생성 (IV.4.3).
pub mod scaffold;

/// 프로젝트 열기 probe (IV.5.1).
pub mod probe;

/// 프로젝트 신뢰 상태 (IV.5.2).
pub mod trust;

/// 프로젝트 clone 요청과 민감 URL (IV.6.1).
pub mod clone;

/// 복구 스냅샷 저장소 (V.9.1).
pub mod recovery;

/// canonical recovery payload (V.9.2).
pub mod recovery_payload;

/// 복구 스냅샷 최신성 probe (V.9.3).
pub mod recovery_probe;

/// 정상 종료 marker와 스냅샷 cleanup (V.9.4).
pub mod recovery_shutdown;

/// symlink를 그대로 허용하는 프로젝트 경로 resolver (XIII.4.1).
pub mod path_resolver;

/// 프로젝트 신뢰 grant/revoke와 OS 사용자 데이터 저장소 (XVI.7.2).
pub mod trust_store;

/// 문서 ↔ chunk model 입출력 (VI.10.2).
pub mod document_io;

/// 프로젝트 신뢰 capability matrix (XVI.7.1).
pub mod capability;

/// 프로젝트 파일 감시기 (XIII.2).
pub mod watcher;

/// 외부 변경 반영 상태 (XIII.3).
pub mod external_change;

/// 지연 자동 저장 정책 (XIII.1.1).
pub mod autosave;

/// 디스크에 저장된 프로젝트 loader와 Git snapshot 연결 (XIV.2.7).
pub mod saved_project;

/// 비공개 임시 디렉터리와 정리 (XVI.7.5).
pub mod private_temp;
