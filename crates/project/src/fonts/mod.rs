//! 프로젝트·시스템·외부 폰트 (XIII.6).
//!
//! 폰트 face 정체성은 텍스트 엔진(X.8)의 규칙을 따릅니다: 폰트 파일 콘텐츠 해시 + face index
//! ([`stellar_text::registry::FontFaceKey`]). 이 모듈은 출처별로 실제 파일을 찾아 hash·face를
//! 계산하고 [`stellar_text::registry::FontRegistry`]에 등록합니다.
//!
//! - [`sfnt`]: 폰트 파일 container probe(형식, face 수, face 이름). 신뢰하지 않는 입력 기준으로
//!   모든 offset·개수를 제한합니다.
//! - [`license`]: 사용자 입력 라이선스 metadata.
//! - [`project_font`]: Project Font import(AssetStore 연결)와 record.
//! - [`system`]: 시스템 폰트 discovery(원래 위치에서 hash·probe 후 등록, 복사 없음). macOS
//!   adapter는 `system_macos`(CoreText)입니다.
//! - [`external`]: External Font — 관리형 패키지의 고정 hash/face 참조 인터페이스(다운로드 없음).

/// 폰트 파일 container probe (XIII.6.1).
pub mod sfnt;

/// 프로젝트 폰트 라이선스 metadata (XIII.6.1).
pub mod license;

/// Project Font import와 record (XIII.6.1).
pub mod project_font;

/// 시스템 폰트 discovery (XIII.6.2).
pub mod system;

/// macOS CoreText 시스템 폰트 locator (XIII.6.2).
#[cfg(target_os = "macos")]
pub mod system_macos;

/// External Font 패키지 참조 인터페이스 (XIII.6.4).
pub mod external;

#[cfg(test)]
pub(crate) mod test_fixtures;
