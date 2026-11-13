//! SVG 벡터 변환 (XX.3).
//!
//! 아키텍처 §34 SVG 정책: 모든 텍스트는 Path로 변환하고 폰트·편집 가능한 텍스트 객체를 넣지 않으며,
//! SVG로 정확히 표현할 수 없는 최소 하위 트리만 래스터화하고 나머지는 벡터로 유지합니다.

/// Scene primitive → SVG element mapping table (XX.3.1).
pub mod mapping;

/// SVG 속성 값 표기 (XX.3.1).
pub mod format;

/// Scene primitive별 SVG 요소 표기 (XX.3.1).
pub mod element;

/// StableId 기반 SVG id 생성과 충돌 검사 (XX.3.3).
pub mod id;

/// SVG 문서 writer (XX.3.3).
pub mod writer;

pub use writer::{write_svg, SvgDocument, SvgExportError, SvgWriteOptions};
