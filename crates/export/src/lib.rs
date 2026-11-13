//! stellar-export — PNG·SVG 내보내기(Phase XX, ADR-0019). 의존: core-types, scene.
//!
//! 내보내기는 문서 모델과 GPU 객체를 직접 다루지 않고 불변 Scene Snapshot을 입력으로 받습니다
//! (아키텍처 §8, §34).

/// 내보내기 요청과 field schema (XX.1.1).
pub mod request;

pub use request::{
    ExportBackground, ExportFormat, ExportRequest, ExportRequestError, ExportScale, ExportSource,
    OutputPath, TargetProfileKey,
};
