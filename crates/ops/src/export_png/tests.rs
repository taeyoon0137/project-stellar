//! PNG 저장 테스트 (XX.2.3): 성공 저장, 취소·단계별 중단 시 최종 경로 불변, temp 정리.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use stellar_export::capture::{
    AssetRevision, EncodedImage, ExportAssetSource, ExportCapture, FontRevision, ResourcePins,
    SceneSource,
};
use stellar_export::png::{decode_png, DecodeLimits};
use stellar_export::preflight::{ExportPreflight, PreflightInput, TargetCapabilityReport};
use stellar_export::raster::software::SoftwareTarget;
use stellar_export::request::{
    ExportBackground, ExportFormat, ExportRequest, ExportScale, ExportSource, OutputPath,
    TargetProfileKey,
};
use stellar_export::svg::NoRasterizer;
use stellar_export::text::NoText;
use stellar_scene::builder::{FrameClipInput, FrameInput, NodeInput, SceneBuilder};
use stellar_scene::handle::SnapshotHandle;
use stellar_scene::node::CornerRadii;
use stellar_scene::revision::SceneRevisionMap;
use stellar_scene::{ColorSpace, Size2};

use super::*;
use crate::core_types::color::Color;
use crate::core_types::ids::StableId;
use crate::core_types::reference::AssetRef;
use crate::core_types::revision::DocumentRevision;
use crate::core_types::value::Transform2D;

struct NoAssets;

impl ExportAssetSource for NoAssets {
    fn encoded_image(&self, _: AssetRef, _: AssetRevision) -> Option<EncodedImage> {
        None
    }
}

fn view() -> StableId {
    StableId::new_v7(0x0195_c530_3acd, 0x7bc, 0x0414_c213_f61c_b965)
}

fn profile() -> TargetProfileKey {
    TargetProfileKey::new("web-modern").unwrap()
}

/// 테스트마다 다른 빈 디렉터리.
fn temp_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "stellar-ops-png-{}-{name}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn preflight_input() -> PreflightInput<'static> {
    static REPORT: std::sync::OnceLock<TargetCapabilityReport> = std::sync::OnceLock::new();
    PreflightInput {
        capabilities: REPORT.get_or_init(|| TargetCapabilityReport::Validated {
            profile: profile(),
            findings: Vec::new(),
        }),
        font_fallbacks: &[],
        assets: &NoAssets,
        text: &NoText,
        rasterizer: &NoRasterizer,
    }
}

/// 40×20 빨간 Frame scene의 capture.
fn capture(output: &Path, scale: f32, format: ExportFormat) -> ExportCapture {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let input = FrameInput {
        node: NodeInput::new(
            StableId::new_v7(0x0195_c530_3acd, 0x7bc, 0x1001),
            Transform2D::identity(),
        ),
        size: Size2::new(40.0, 20.0).unwrap(),
        corner_radii: CornerRadii::ZERO,
        background: Some(Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap()),
        border: None,
        clip: FrameClipInput::None,
    };
    let frame = builder.begin_frame(&root, &input).unwrap();
    builder.finish_frame(&mut root, frame).unwrap();
    let roots = builder.finish(root);
    let snapshot = SceneRevisionMap::new()
        .generate(DocumentRevision(1), roots)
        .unwrap();
    let request = ExportRequest::new(
        ExportSource::view(view()).unwrap(),
        profile(),
        ExportScale::new(scale).unwrap(),
        ExportBackground::Transparent,
        OutputPath::new(output).unwrap(),
        format,
    )
    .unwrap();
    ExportCapture::new(
        request,
        SceneSource {
            view: view(),
            target_profile: profile(),
            snapshot: SnapshotHandle::new(snapshot),
        },
        ResourcePins {
            fonts: FontRevision(1),
            assets: AssetRevision(1),
        },
    )
    .unwrap()
}

/// 사전 진단을 통과한 진행 허가.
fn confirmed(output: &Path, scale: f32) -> ConfirmedExport {
    ExportPreflight::run(
        capture(output, scale, ExportFormat::Png),
        &preflight_input(),
    )
    .unwrap()
    .proceed(None)
    .unwrap()
}

fn target() -> Arc<SoftwareTarget> {
    Arc::new(SoftwareTarget::new(Arc::new(NoAssets), Arc::new(NoText)))
}

#[test]
fn saves_decodable_png_without_leftovers() {
    let dir = temp_dir("ok");
    let path = dir.join("out.png");
    let export = PngExport::start(confirmed(&path, 2.0), target(), PngOptions::default()).unwrap();
    assert_eq!(export.path(), path);
    let outcome = export.finish().unwrap();
    assert_eq!((outcome.width, outcome.height), (80, 40));
    assert_eq!(outcome.profile, OutputProfile::Srgb);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(outcome.bytes, bytes.len() as u64);
    let decoded = decode_png(&bytes, DecodeLimits::default()).unwrap();
    assert!(decoded.rgba.chunks(4).all(|p| p == [255, 0, 0, 255]));
    assert_eq!(entries(&dir), vec!["out.png".to_string()]);
    // 같은 경로에 다시 내보내면 원자적으로 교체합니다.
    let outcome = PngExport::start(confirmed(&path, 1.0), target(), PngOptions::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!((outcome.width, outcome.height), (40, 20));
    assert_eq!(entries(&dir), vec!["out.png".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancel_before_save_writes_nothing() {
    let dir = temp_dir("cancel");
    let path = dir.join("out.png");
    let export = PngExport::start(confirmed(&path, 4.0), target(), PngOptions::default()).unwrap();
    export.cancel();
    assert!(matches!(export.finish(), Err(PngSaveError::Cancelled)));
    assert!(entries(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `stage`에서 취소를 요청하는 관찰자.
struct CancelAt {
    stage: WriteStage,
    token: CancelToken,
}

impl WriteObserver for CancelAt {
    fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
        if stage == self.stage {
            self.token.cancel();
        }
        Ok(())
    }
}

/// `stage` 직전에 강제 중단을 모사하는 관찰자.
struct FailAt(WriteStage);

impl WriteObserver for FailAt {
    fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
        if stage == self.0 {
            return Err(std::io::Error::other("주입한 중단"));
        }
        Ok(())
    }
}

#[test]
fn cancel_during_write_leaves_previous_file() {
    for stage in [
        WriteStage::WriteBytes,
        WriteStage::Flush,
        WriteStage::SyncFile,
    ] {
        let dir = temp_dir("cancel-write");
        let path = dir.join("out.png");
        std::fs::write(&path, b"previous").unwrap();
        let export =
            PngExport::start(confirmed(&path, 1.0), target(), PngOptions::default()).unwrap();
        let token = export.cancel_token();
        let result = export.finish_observed(&mut CancelAt { stage, token });
        assert!(matches!(result, Err(PngSaveError::Cancelled)), "{stage}");
        assert_eq!(std::fs::read(&path).unwrap(), b"previous", "{stage}");
        assert_eq!(
            entries(&dir),
            vec!["out.png".to_string()],
            "temp 정리 {stage}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn failure_at_any_stage_before_rename_keeps_final_path() {
    for stage in [
        WriteStage::CreateTemp,
        WriteStage::WriteBytes,
        WriteStage::Flush,
        WriteStage::SyncFile,
        WriteStage::Rename,
    ] {
        let dir = temp_dir("fail");
        let path = dir.join("out.png");
        let export =
            PngExport::start(confirmed(&path, 1.0), target(), PngOptions::default()).unwrap();
        let result = export.finish_observed(&mut FailAt(stage));
        assert!(matches!(result, Err(PngSaveError::Write { .. })), "{stage}");
        assert!(!path.exists(), "부분 파일 없음 {stage}");
        assert!(entries(&dir).is_empty(), "temp 정리 {stage}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn missing_directory_is_a_write_error() {
    let dir = temp_dir("missing");
    let path = dir.join("nested").join("out.png");
    let result = PngExport::start(confirmed(&path, 1.0), target(), PngOptions::default())
        .unwrap()
        .finish();
    assert!(matches!(result, Err(PngSaveError::Write { .. })));
    assert!(!dir.join("nested").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_png_requests_are_rejected_without_writing() {
    let dir = temp_dir("svg");
    let path = dir.join("out.svg");
    let svg = ExportPreflight::run(capture(&path, 1.0, ExportFormat::Svg), &preflight_input())
        .unwrap()
        .proceed(None)
        .unwrap();
    let error = PngExport::start(svg, target(), PngOptions::default()).unwrap_err();
    assert!(matches!(
        error,
        PngSaveError::Export(PngExportError::WrongFormat(ExportFormat::Svg))
    ));
    assert!(entries(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
