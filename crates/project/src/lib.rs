//! stellar-project — 프로젝트 수명주기 골격. 의존: document, storage, assets, dependencies.

/// 제한 YAML 지원 (IV.1).
pub mod yaml;

/// `stellar.yaml` 매니페스트 데이터 모델 (IV.1.1).
pub mod manifest;

/// 제한 YAML ↔ 의존성 형식 중립 트리 변환 (XV.1.1·XV.4.1 연결).
pub mod manifest_node;

/// exact commit의 library `stellar.yaml` 읽기 (XV.2.2 resolver 연결).
pub mod manifest_source;

/// 기본값 세트 선택과 사용자 세트 파일 (SD.3, ADR-0082, ADR-0086).
pub mod manifest_style_defaults;

/// `stellar.lock.yaml` 잠금 파일 데이터 모델 (IV.1.6).
pub mod lock;

/// resolve 결과 `stellar.lock.yaml` 기록 transaction (XV.4.5).
pub mod lockfile_write;

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

/// 로컬 chunk cache 관리: 옛 배치 이동·다시 만들기·GC·Git index 정리 (V.10.7, ADR-0122).
pub mod cache;

/// 새 프로젝트·정의·노드에 사용할 UUIDv7.
pub mod new_ids;

/// 프로젝트 신뢰 capability matrix (XVI.7.1).
pub mod capability;

/// 프로젝트 파일 감시기 (XIII.2).
pub mod watcher;

/// 외부 변경 반영 상태 (XIII.3).
pub mod external_change;

/// 창 문서 세션과 clean 창의 외부 generation 자동 reload (XIII.3.1).
pub mod window_session;

/// 지연 자동 저장 정책 (XIII.1.1).
pub mod autosave;

/// 디스크에 저장된 프로젝트 loader와 Git snapshot 연결 (XIV.2.7).
pub mod saved_project;

/// 비공개 임시 디렉터리와 정리 (XVI.7.5).
pub mod private_temp;

/// conflict 세 stage virtual project 열기 (XIV.11.2).
pub mod conflict_open;

/// 프로젝트·시스템·외부 폰트 (XIII.6).
pub mod fonts;

/// 제품 문서 migration registry와 고정 decoder (IX.11.21).
pub mod migration;

/// library root 검증기: ProjectProbe·storage validator 제품 구현 (XV.3.4, XV.6.2).
pub mod library_entry;

/// library 텍스트 원본 export index 공급자 (XV.5.1 제품 공급자).
pub mod library_exports;

/// library 의존성 엔진의 시스템 Git·SHA-256 연결 (XV.3).
pub mod library_git;

/// 사용 프로젝트 local override 대상 검증 (XV.6.2).
pub mod library_overrides;

/// 프로젝트 열기 때 lock 기준 library 로드와 부분 열기 (XV.5.5).
pub mod library_open;

/// 사용 프로젝트 instance의 외부 component 해석과 외부 참조 사용 수집 (XV.5.3·XV.7.3).
pub mod library_refs;

/// cache를 먼저 보는 library 매니페스트 공급자와 `stellar.yaml` 의존성 편집 (XV.4.3·XV.7.4, ADR-0137).
pub mod library_resolve;

/// 사용 프로젝트 component command의 library 읽기 전용 사전 검사 (XV.5.3).
pub mod library_guard;
