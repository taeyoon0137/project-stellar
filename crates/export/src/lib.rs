//! stellar-export — PNG·SVG 내보내기(Phase XX, ADR-0019). 의존: core-types, scene.
//!
//! 내보내기는 문서 모델과 GPU 객체를 직접 다루지 않고 불변 Scene Snapshot을 입력으로 받습니다
//! (아키텍처 §8, §34).

/// 내보내기 요청과 field schema (XX.1.1).
pub mod request;

/// 내보내기 입력 고정: scene snapshot·resource revision (XX.1.2).
pub mod capture;

/// 내보내기 사전 진단과 사용자 확인 (XX.1.3).
pub mod preflight;

/// 텍스트 glyph 입력: glyph run과 outline 조회 (XX.3.2).
pub mod text;

/// SVG 벡터 변환 (XX.3).
pub mod svg;

/// 테스트 전용 scene fixture.
#[cfg(test)]
mod test_fixtures;

pub use capture::{
    AssetRevision, CaptureError, EncodedImage, ExportAssetSource, ExportCapture, ExportRoot,
    FontRevision, ImageMedia, ResourcePins, SceneSource,
};
pub use preflight::{
    ConfirmedExport, ExportConfirmation, ExportPreflight, GlyphBlockReason, GlyphBlocker,
    PreflightInput, PreflightIssue, PreflightRejection,
};
pub use request::{
    ExportBackground, ExportFormat, ExportRequest, ExportRequestError, ExportScale, ExportSource,
    OutputPath, TargetProfileKey,
};
pub use text::{
    ExportFontFace, ExportTextSource, GlyphOutline, GlyphOutlineError, GlyphOutlineKey, GlyphRun,
    GlyphSize, GlyphVariations, PlacedGlyph, RunSize, TextGlyphLayout,
};
