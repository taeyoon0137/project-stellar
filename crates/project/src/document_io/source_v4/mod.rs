//! 텍스트 원본 쓰기 v4(ADR-0077).
//!
//! - 영속 속성 하나 = 키 하나·한 줄. 키는 document 속성 registry 이름이고 순서는 registry 순서입니다([`keys`]).
//! - 값은 CSS 문법 그대로이며 float은 정확히 왕복합니다([`css`]).
//! - 모르는 키·값은 거부하고, 모델 의미 가운데 텍스트로 정확히 복원할 수 없는 것이 하나라도 있으면 쓰기를
//!   거부합니다. 손실 저장은 없습니다.
//! - View·컴포넌트 원본 v1~v3은 읽기 전용이고, 실제 편집 때만 v4로 다시 씁니다.

/// 텍스트 asset catalog(TS-ASSET).
pub mod catalog;
/// CSS 값 문법.
pub(crate) mod css;
/// registry 기반 파일 키.
pub(crate) mod keys;
/// node·modifier 레코드.
pub(crate) mod node;
/// 프로젝트 전체 의미의 다중 파일 원본(V.10.4).
pub mod project;
/// 파일 이름 slug.
pub(crate) mod slug;
/// node → 파일·줄 범위(TS-MAP).
pub mod span;
/// 문서 Text style 표.
pub(crate) mod styles;
/// token set 원본.
pub mod token;
/// 단일 View 원본.
pub(crate) mod view;

/// 텍스트 원본의 쓰기 버전.
pub const VERSION: &str = "4";

#[cfg(test)]
mod tests;
