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

/// 부분 래스터화 경계 planner (XX.4.1).
pub mod raster;

/// 래스터 경계 offscreen 렌더 요청과 host rasterizer 연결 지점 (XX.4.2).
pub mod rasterize;

/// 래스터화한 subtree 목록과 이유 report (XX.4.3).
pub mod report;

/// Text glyph run → outline path 변환 (XX.3.2).
pub mod text;

pub use raster::{plan_raster, RasterBoundary, RasterCause, RasterPlan, RasterRegion};
pub use rasterize::{ExportRasterizer, NoRasterizer, RasterRequest, RasterRoot, RasterizeError};
pub use report::{
    BackdropReport, BackdropSource, RasterReason, RasterizedKind, RasterizedRegion, SvgExportReport,
};
pub use writer::{write_svg, write_svg_with, SvgDocument, SvgExportError, SvgWriteOptions};
