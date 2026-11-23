//! stellar-integrity — 프로젝트 무결성 진단 집계 (Phase XXV.2·XXV.3 코어, ADR-0095).
//!
//! 진단 생산자(문서 validator, binding orphan, missing font, 접근성, library 부분 열기, Target Profile capability,
//! schema migration)의 결과를 공통 [`index::DiagnosticIndex`]로 모읍니다.
//!
//! - [`diagnostic`]: 진단 한 건(종류·심각도·source StableId·property path·remediation action ID)과 정체성 key.
//! - [`sources`]: 생산자 결과 변환과 문서 레코드만 읽는 내장 checker. 판정은 생산자 함수를 그대로 씁니다.
//! - [`index`]: 집계·필터·pagination 코어 API(XXV.3.1). 셸 FFI는 이 crate에 두지 않습니다(ADR-0095 결정 6).
//! - [`incremental`]: command change set으로 영향받는 scope만 다시 검사합니다(XXV.2.3).
//! - [`remediation`]: action inventory와 계획. 일반 CommandTransaction 또는 명시 navigation만 있습니다(XXV.3.3).
//! - [`codec`]: 진단 목록 저장 schema v1(XXV.2.2 round-trip).
//!
//! 진단은 Git 원본이 아니라 원본에서 다시 계산하는 파생 데이터입니다. 텍스트 내용·파일 경로·repository URL을 담지
//! 않습니다.

/// 진단 한 건과 정체성 (XXV.2.1, XXV.2.2).
pub mod diagnostic;

/// 진단 목록 저장 schema v1 (XXV.2.2).
pub mod codec;

/// remediation action inventory와 계획 (XXV.3.3).
pub mod remediation;

/// 공통 DiagnosticIndex: 집계·필터·pagination (XXV.2.1, XXV.3.1).
pub mod index;

/// 증분 재검사 (XXV.2.3).
pub mod incremental;

/// 생산자 결과 변환과 내장 checker (XXV.2.1).
pub mod sources;

#[cfg(test)]
mod tests;
