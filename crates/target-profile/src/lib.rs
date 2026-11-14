//! stellar-target-profile — Target Profile과 Capability Registry (Phase IX.10).
//!
//! 의존: core-types만(crate graph). `stellar.yaml` 텍스트(제한 YAML)는 project crate가 형식 중립 트리
//! ([`node`])로 바꿔 주고받습니다. 문서·command·binding 계층의 입력은 이 crate의 지역 타입과 trait으로
//! 표현하며, 상위 crate가 구현·연결합니다.

/// 형식 중립 매니페스트 트리와 스키마 오류 (IX.10.1).
pub mod node;

/// Target Profile 식별자·종류·schema version (IX.10.1).
pub mod profile;

/// profile 종류별 필드와 Target Profile entry (IX.10.2).
pub mod spec;

/// `stellar.yaml` 프로젝트 profile 목록과 기본 profile (IX.10.3).
pub mod manifest;

/// profile registry 버전과 프로젝트 pin (IX.10.4).
pub mod registry;

/// capability 상태와 element·property·contract별 registry 표 (IX.10.5).
pub mod capability;

/// 앱 내장 profile registry release (IX.10.5).
pub mod builtin;

/// 선택 profile의 생성 가능 여부와 비활성화 사유 조회 (IX.10.6).
pub mod query;

/// 선택 profile의 Binding 환경 입력 (IX.10.7).
pub mod environment;

/// 공통 구조 + profile 희소 override resolver (IX.10.7).
pub mod overrides;
