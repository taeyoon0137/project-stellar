//! stellar-codegen — 단방향 플랫폼 코드 생성(Phase XXII, ADR-0049). 의존: core-types, target-profile,
//! accessibility, layout.
//!
//! resolved View를 플랫폼 중립 IR([`ir::CodegenIr`])로 바꾸고, Web·Apple·Android adapter가 IR만 읽어 코드를
//! 만듭니다. 문서 모델·binding graph는 직접 읽지 않고 하위 crate의 지역 모델(ViewModel, LayoutInput,
//! AccessibilitySnapshot)을 입력으로 받습니다. 생성 코드를 IR이나 문서로 되읽는 API는 두지 않습니다
//! (기술 아키텍처 §17.3).

/// 중립 코드 생성 IR (XXII.1.1).
pub mod ir;

/// 결정론 텍스트 도우미.
pub mod text;

/// adapter 노트가 가리키는 접근성 의미 facet.
pub mod facet;

/// modifier stack → CSS 고정 적용 순서 선형화 계획 (IX.11.18).
pub mod modifier_plan;

/// Web adapter (XXII.2).
pub mod web;

/// Apple adapter (XXII.3).
pub mod apple;

#[cfg(test)]
pub(crate) mod fixture;

#[cfg(test)]
mod scale_tests;
