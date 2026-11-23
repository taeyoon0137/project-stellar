//! 저장 View → PNG 내보내기 (XX.2.3, ADR-0116 결정 5).
//!
//! [`prepare_view_png`]는 저장 generation의 문서 View를 앱과 같은 변환([`document_scene_source`], ADR-0116 결정 1)으로
//! scene snapshot으로 만들고, 내보내기 입력 고정([`ExportCapture`]) → 사전 진단([`ExportPreflight`]) → 진행 허가
//! ([`ConfirmedExport`])까지 진행합니다. [`export_view_png`]는 그 결과를 worker([`PngExport`])로 렌더·저장하고,
//! [`render_view_png`]는 파일을 쓰지 않고 PNG bytes만 돌려줍니다(MCP).
//!
//! # 규칙
//!
//! - **입력은 저장 generation뿐입니다.** 앱의 미저장 편집·활성 View Variant는 입력이 아닙니다.
//! - **target**: CPU target([`SoftwareTarget`])입니다. headless 도구는 GPU backend에 닿지 않습니다(ADR-0005 §8).
//!   CPU target이 아직 그리지 않는 효과(drop shadow, layer blur, Normal이 아닌 blend)는 근사하지 않고 렌더 오류입니다.
//! - **빠진 node**: 변환이 넣지 못한 node([`SkipReason`], 예: Text layout hook 없음·layout Frame)는 결과에 빠진 채로
//!   그리게 됩니다. 기본은 거부([`ViewPngError::SkippedNodes`])이고, 호출자가 `omit_skipped`로 확인해야 진행합니다.
//! - **사전 진단**: 확인이 필요한 진단(없는 이미지, 검사하지 못한 target profile)은 기본 거부
//!   ([`ViewPngError::ConfirmationRequired`])이고 `accept_issues`로 확인합니다. headless scene은 target profile을
//!   평가하지 않으므로 profile key는 [`BASE_PROFILE`]이며, `stellar.yaml`에 target profile이 있으면 검사하지 못한
//!   profile로 진단합니다.
//! - **이미지**: generation의 asset metadata chunk와 asset content store에서 PNG 내용을 읽습니다(symlink를 따르지
//!   않는 저장소 읽기). PNG가 아닌 이미지는 CPU target이 해석하지 못하므로 진단 없이 그리지 않고
//!   [`ViewPngError::UnsupportedImage`]로 거부합니다.
//! - asset·font revision은 generation 번호로 고정합니다. 저장된 font는 headless Text layout이 없어 쓰지 않습니다.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::core_types::clock::ManualClock;
use crate::core_types::color::Color;
use crate::core_types::reference::AssetRef;
use crate::core_types::revision::DocumentRevision;
use stellar_assets::record::AssetRecord;
use stellar_assets::store::AssetStore;
use stellar_export::capture::{
    AssetRevision, CaptureError, EncodedImage, ExportAssetSource, ExportCapture, FontRevision,
    ImageMedia, ResourcePins, SceneSource,
};
use stellar_export::png::{decode_png, render_png, DecodeLimits, PngFile, PngOptions};
use stellar_export::preflight::{
    ConfirmedExport, ExportPreflight, PreflightInput, PreflightRejection, TargetCapabilityReport,
};
use stellar_export::raster::software::SoftwareTarget;
use stellar_export::raster::{CancelToken, OffscreenTarget};
use stellar_export::request::{
    ExportBackground, ExportFormat, ExportRequest, ExportScale, ExportSource, OutputPath,
    TargetProfileKey,
};
use stellar_export::svg::NoRasterizer;
use stellar_export::text::NoText;
use stellar_scene::document::{document_scene_source, SceneSourceOptions, SkipReason};
use stellar_scene::handle::SnapshotHandle;
use stellar_scene::incremental::IncrementalScene;
use stellar_scene::SceneResourceRef;
use stellar_storage::chunk::kinds::KIND_ASSET_METADATA;
use stellar_storage::chunk::store::ChunkStore;
use stellar_storage::hash::ContentHash;

use super::{PngExport, PngSaveError, PngSaveOutcome};
use crate::core_types::ids::StableId;
use crate::error::DocumentReadError;
use crate::snapshot::ProjectSnapshot;

/// headless scene의 target profile key. 변환은 target profile을 평가하지 않습니다.
pub const BASE_PROFILE: &str = "base";

/// asset metadata chunk 하나의 읽기 상한.
const MAX_ASSET_RECORD_BYTES: usize = 64 * 1024;

/// PNG asset 내용 읽기 상한.
const MAX_PNG_ASSET_BYTES: u64 = 32 * 1024 * 1024;

/// 저장 View PNG 내보내기 요청.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewPngRequest {
    /// 문서.
    pub document: StableId,
    /// View.
    pub view: StableId,
    /// 출력 배율(1 = scene 단위 1 → 1 pixel).
    pub scale: f32,
    /// 출력 색 profile 이름(`srgb`·`display-p3`).
    pub profile: String,
    /// 배경 CSS 색(없으면 투명).
    pub background: Option<String>,
    /// 출력 경로(절대 경로, `.png`). [`render_view_png`]는 이 경로에 쓰지 않습니다.
    pub output: PathBuf,
    /// 확인이 필요한 사전 진단을 확인하고 진행합니다.
    pub accept_issues: bool,
    /// 변환이 넣지 못한 node를 빼고 그리는 것을 확인합니다.
    pub omit_skipped: bool,
}

/// 변환이 넣지 못한 node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SkippedNode {
    /// 문서 node(가상 instance node면 그 scene key id).
    pub node: StableId,
    /// 이유.
    pub reason: SkipReason,
}

/// 내보내기 준비 결과.
#[derive(Debug, Clone)]
pub struct PreparedViewPng {
    /// 진행 허가.
    pub export: ConfirmedExport,
    /// encoder 옵션.
    pub options: PngOptions,
    /// 빠진 node(확인함).
    pub skipped: Vec<SkippedNode>,
    /// 확인한 사전 진단(표시 문장).
    pub accepted: Vec<String>,
    /// 적용하지 않은 View Variant patch 수(저장 View는 Variant를 쓰지 않으므로 보통 0).
    pub unapplied_variant_patches: usize,
    target: Arc<SoftwareTarget>,
}

impl PreparedViewPng {
    /// 렌더 target.
    pub fn target(&self) -> Arc<dyn OffscreenTarget + Send + Sync> {
        self.target.clone()
    }
}

/// 저장 View PNG 내보내기 실패.
#[derive(Debug)]
pub enum ViewPngError {
    /// 저장된 generation이 없습니다.
    Unsaved,
    /// 문서를 읽지 못했습니다.
    Document(DocumentReadError),
    /// 문서에 View가 없습니다.
    UnknownView(StableId),
    /// 요청 값이 잘못되었습니다.
    InvalidRequest(String),
    /// scene을 만들지 못했습니다.
    Scene(String),
    /// 변환이 넣지 못한 node가 있습니다(`omit_skipped`로 확인).
    SkippedNodes(Vec<SkippedNode>),
    /// PNG가 아닌 이미지 asset.
    UnsupportedImage {
        /// asset.
        asset: StableId,
        /// media type.
        media: String,
    },
    /// 내보내기 입력 고정 실패(보이는 내용 없음 등).
    Capture(CaptureError),
    /// 확인이 필요한 사전 진단이 있습니다(`accept_issues`로 확인).
    ConfirmationRequired(Vec<String>),
    /// 진행할 수 없는 사전 진단.
    Rejected(String),
    /// 렌더·encode·저장 실패.
    Save(PngSaveError),
}

impl fmt::Display for ViewPngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViewPngError::Unsaved => f.write_str("저장된 generation이 없습니다"),
            ViewPngError::Document(error) => write!(f, "문서를 읽지 못했습니다: {error}"),
            ViewPngError::UnknownView(view) => write!(f, "문서에 View {view}가 없습니다"),
            ViewPngError::InvalidRequest(message) => f.write_str(message),
            ViewPngError::Scene(message) => write!(f, "scene을 만들지 못했습니다: {message}"),
            ViewPngError::SkippedNodes(nodes) => write!(
                f,
                "그리지 못하는 node {}개가 있습니다(빼고 그리려면 확인이 필요합니다)",
                nodes.len()
            ),
            ViewPngError::UnsupportedImage { asset, media } => write!(
                f,
                "이미지 asset {asset}의 형식 {media}를 PNG 내보내기가 해석하지 못합니다"
            ),
            ViewPngError::Capture(error) => write!(f, "내보내기 범위를 정하지 못했습니다: {error}"),
            ViewPngError::ConfirmationRequired(issues) => {
                write!(f, "확인이 필요한 사전 진단 {}개가 있습니다", issues.len())
            }
            ViewPngError::Rejected(message) => write!(f, "내보낼 수 없습니다: {message}"),
            ViewPngError::Save(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ViewPngError {}

/// 렌더·encode·저장 실패([`ViewPngError::Save`])의 종류. CLI 종료 code와 MCP 실패 종류가 같은 분류를 씁니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFailure {
    /// target이 그리지 못하는 기능(예: CPU target의 drop shadow·layer blur·Normal이 아닌 blend, 이미지 형식).
    Unsupported,
    /// 출력 pixel 크기가 상한을 넘습니다.
    TooLarge,
    /// 출력 파일을 쓰지 못했습니다.
    Write,
    /// 취소되었습니다.
    Cancelled,
    /// 그 밖의 렌더·encode 실패.
    Render,
}

impl SaveFailure {
    /// 안정 code(CLI·MCP 보고용).
    pub const fn code(self) -> &'static str {
        match self {
            SaveFailure::Unsupported => "png-unsupported",
            SaveFailure::TooLarge => "png-too-large",
            SaveFailure::Write => "png-write-failed",
            SaveFailure::Cancelled => "png-cancelled",
            SaveFailure::Render => "png-render-failed",
        }
    }
}

impl ViewPngError {
    /// 렌더·encode·저장 실패의 종류. [`ViewPngError::Save`]가 아니면 `None`입니다.
    pub fn save_failure(&self) -> Option<SaveFailure> {
        use stellar_export::png::PngExportError;
        use stellar_export::raster::OffscreenError;
        let ViewPngError::Save(error) = self else {
            return None;
        };
        Some(match error {
            PngSaveError::Write { .. } => SaveFailure::Write,
            PngSaveError::Cancelled => SaveFailure::Cancelled,
            PngSaveError::Export(PngExportError::Cancelled)
            | PngSaveError::Export(PngExportError::Render(OffscreenError::Cancelled)) => {
                SaveFailure::Cancelled
            }
            PngSaveError::Export(PngExportError::Render(OffscreenError::Unsupported {
                ..
            })) => SaveFailure::Unsupported,
            PngSaveError::Export(PngExportError::Render(OffscreenError::TooLarge { .. })) => {
                SaveFailure::TooLarge
            }
            PngSaveError::Export(_) => SaveFailure::Render,
        })
    }
}

/// generation의 이미지 asset 내용(asset id → metadata chunk hash).
struct GenerationAssets {
    root: PathBuf,
    revision: AssetRevision,
    chunks: BTreeMap<StableId, ContentHash>,
}

impl GenerationAssets {
    fn record(&self, asset: StableId) -> Option<AssetRecord> {
        let hash = self.chunks.get(&asset)?;
        let bytes = ChunkStore::new(&self.root)
            .read_chunk(hash, KIND_ASSET_METADATA)
            .ok()?;
        if bytes.len() > MAX_ASSET_RECORD_BYTES {
            return None;
        }
        AssetRecord::decode(&bytes).ok()
    }
}

impl ExportAssetSource for GenerationAssets {
    fn encoded_image(&self, asset: AssetRef, revision: AssetRevision) -> Option<EncodedImage> {
        if revision != self.revision {
            return None;
        }
        let record = self.record(asset.id())?;
        if record.media_type.as_str() != ImageMedia::Png.mime() {
            return None;
        }
        let bytes = AssetStore::new(&self.root)
            .read_content(&record.content_hash, record.size, MAX_PNG_ASSET_BYTES)
            .ok()??;
        let decoded = decode_png(&bytes, DecodeLimits::default()).ok()?;
        EncodedImage::new(
            ImageMedia::Png,
            Arc::from(bytes),
            decoded.width,
            decoded.height,
        )
        .ok()
    }
}

/// 저장 View를 내보내기 진행 허가까지 준비합니다(모듈 문서 규칙).
pub fn prepare_view_png(
    snapshot: &ProjectSnapshot,
    request: &ViewPngRequest,
) -> Result<PreparedViewPng, ViewPngError> {
    let generation = snapshot.generation().ok_or(ViewPngError::Unsaved)?;
    let generation_number = generation.manifest().generation;
    let options = PngOptions::from_names(&request.profile, 8)
        .map_err(|error| ViewPngError::InvalidRequest(error.to_string()))?;
    let scale = ExportScale::new(request.scale)
        .map_err(|error| ViewPngError::InvalidRequest(format!("배율이 잘못되었습니다: {error}")))?;
    let background = match &request.background {
        None => ExportBackground::Transparent,
        Some(text) => ExportBackground::Solid(parse_background(text)?),
    };
    let output = OutputPath::new(&request.output).map_err(|error| {
        ViewPngError::InvalidRequest(format!("출력 경로가 잘못되었습니다: {error}"))
    })?;
    let profile = TargetProfileKey::new(BASE_PROFILE).expect("고정 key");
    let export_request = ExportRequest::new(
        ExportSource::view(request.view)
            .map_err(|error| ViewPngError::InvalidRequest(error.to_string()))?,
        profile.clone(),
        scale,
        background,
        output,
        ExportFormat::Png,
    )
    .map_err(|error| ViewPngError::InvalidRequest(error.to_string()))?;

    let model = snapshot
        .document_model(request.document)
        .map_err(ViewPngError::Document)?;
    let root = model
        .definition()
        .pages
        .iter()
        .flat_map(|page| &page.views)
        .find(|view| view.id == request.view)
        .map(|view| view.root_frame)
        .ok_or(ViewPngError::UnknownView(request.view))?;
    // 컴포넌트 원본을 읽지 못하면 instance는 변환이 skip으로 남깁니다(빠진 node 보고).
    let library = crate::component_source::project_library(snapshot)
        .ok()
        .map(|(library, _)| library);
    let built = document_scene_source(
        &model,
        root,
        SceneSourceOptions {
            library: library.as_ref(),
            ..SceneSourceOptions::default()
        },
    )
    .map_err(|error| ViewPngError::Scene(format!("{error:?}")))?;
    let skipped: Vec<SkippedNode> = built
        .skipped
        .iter()
        .map(|(key, reason)| SkippedNode {
            node: key.id(),
            reason: *reason,
        })
        .collect();
    if !skipped.is_empty() && !request.omit_skipped {
        return Err(ViewPngError::SkippedNodes(skipped));
    }
    let unapplied_variant_patches = built.unapplied_variant_patches;
    let (scene, _) = IncrementalScene::build(
        built.source,
        DocumentRevision(generation_number),
        &ManualClock::new(0),
    )
    .map_err(|error| ViewPngError::Scene(format!("{error:?}")))?;
    let assets = GenerationAssets {
        root: snapshot.root().to_path_buf(),
        revision: AssetRevision(generation_number),
        chunks: generation
            .manifest()
            .chunk_pointers
            .iter()
            .filter(|p| p.kind == KIND_ASSET_METADATA && p.schema_version == 1)
            .map(|p| (p.chunk_id.stable_id(), p.content_hash))
            .collect(),
    };
    let capture = ExportCapture::new(
        export_request,
        SceneSource {
            view: request.view,
            target_profile: profile.clone(),
            snapshot: SnapshotHandle::new(scene.snapshot().clone()),
        },
        ResourcePins {
            fonts: FontRevision(generation_number),
            assets: AssetRevision(generation_number),
        },
    )
    .map_err(ViewPngError::Capture)?;
    for resource in capture.resources() {
        if let SceneResourceRef::Image(asset) = resource {
            if let Some(record) = assets.record(asset.id()) {
                if record.media_type.as_str() != ImageMedia::Png.mime() {
                    return Err(ViewPngError::UnsupportedImage {
                        asset: asset.id(),
                        media: record.media_type.as_str().to_string(),
                    });
                }
            }
        }
    }
    let capabilities = if snapshot.manifest().target_profiles.is_empty() {
        TargetCapabilityReport::Validated {
            profile,
            findings: Vec::new(),
        }
    } else {
        TargetCapabilityReport::RegistryUnavailable { profile }
    };
    let preflight = ExportPreflight::run(
        capture,
        &PreflightInput {
            capabilities: &capabilities,
            font_fallbacks: &[],
            assets: &assets,
            text: &NoText,
            rasterizer: &NoRasterizer,
        },
    )
    .map_err(|error| ViewPngError::Rejected(format!("{error:?}")))?;
    let accepted: Vec<String> = preflight.issues().iter().map(ToString::to_string).collect();
    let confirmation = if preflight.requires_confirmation() {
        if !request.accept_issues {
            return Err(ViewPngError::ConfirmationRequired(accepted));
        }
        Some(preflight.confirm())
    } else {
        None
    };
    let export = preflight
        .proceed(confirmation.as_ref())
        .map_err(|rejection| ViewPngError::Rejected(rejection_text(&rejection)))?;
    let target = Arc::new(SoftwareTarget::new(Arc::new(assets), Arc::new(NoText)));
    Ok(PreparedViewPng {
        export,
        options,
        skipped,
        accepted,
        unapplied_variant_patches,
        target,
    })
}

fn rejection_text(rejection: &PreflightRejection) -> String {
    match rejection {
        PreflightRejection::Blocked(blockers) => format!("형식 차단 {}개", blockers.len()),
        PreflightRejection::GlyphsBlocked(blockers) => format!("glyph 차단 {}개", blockers.len()),
        PreflightRejection::ConfirmationRequired(issues) => {
            format!("확인이 필요한 진단 {}개", issues.len())
        }
        PreflightRejection::ConfirmationMismatch => "확인한 진단이 바뀌었습니다".to_string(),
    }
}

fn parse_background(text: &str) -> Result<Color, ViewPngError> {
    stellar_document::css::value::parse_color(text)
        .map_err(|error| ViewPngError::InvalidRequest(format!("배경 색이 잘못되었습니다: {error}")))
}

/// 저장한 PNG와 준비 정보.
#[derive(Debug, Clone)]
pub struct ViewPngOutcome {
    /// 저장 결과.
    pub saved: PngSaveOutcome,
    /// 빠진 node(확인함).
    pub skipped: Vec<SkippedNode>,
    /// 확인한 사전 진단.
    pub accepted: Vec<String>,
}

/// 저장 View를 PNG로 렌더해 요청 경로에 저장합니다(worker 렌더, atomic 저장).
pub fn export_view_png(
    snapshot: &ProjectSnapshot,
    request: &ViewPngRequest,
) -> Result<ViewPngOutcome, ViewPngError> {
    let prepared = prepare_view_png(snapshot, request)?;
    let job = PngExport::start(prepared.export.clone(), prepared.target(), prepared.options)
        .map_err(ViewPngError::Save)?;
    let saved = job.finish().map_err(ViewPngError::Save)?;
    Ok(ViewPngOutcome {
        saved,
        skipped: prepared.skipped,
        accepted: prepared.accepted,
    })
}

/// 렌더한 PNG bytes와 준비 정보(파일을 쓰지 않음).
#[derive(Debug, Clone)]
pub struct RenderedViewPng {
    /// PNG.
    pub file: PngFile,
    /// 출력 profile 이름.
    pub profile: &'static str,
    /// 빠진 node(확인함).
    pub skipped: Vec<SkippedNode>,
    /// 확인한 사전 진단.
    pub accepted: Vec<String>,
}

/// 저장 View를 PNG bytes로 렌더합니다. 출력 경로에는 쓰지 않습니다.
pub fn render_view_png(
    snapshot: &ProjectSnapshot,
    request: &ViewPngRequest,
) -> Result<RenderedViewPng, ViewPngError> {
    let prepared = prepare_view_png(snapshot, request)?;
    let file = render_png(
        &prepared.export,
        prepared.target.as_ref(),
        prepared.options,
        &CancelToken::new(),
    )
    .map_err(|error| ViewPngError::Save(error.into()))?;
    Ok(RenderedViewPng {
        file,
        profile: prepared.options.profile().name(),
        skipped: prepared.skipped,
        accepted: prepared.accepted,
    })
}

/// [`render_view_png`]가 쓰지 않는 출력 경로(요청 계약상 절대 `.png` 경로가 필요해 프로젝트 root 아래 이름을 씁니다).
pub fn unwritten_output_path(root: &Path) -> PathBuf {
    root.join("stellar-export-preview.png")
}

/// PNG bytes의 base64(RFC 4648, SVG data URI와 같은 encoder).
pub fn png_base64(file: &PngFile) -> String {
    let mut out = String::new();
    stellar_export::svg::format::push_base64(&mut out, file.bytes());
    out
}

#[cfg(test)]
mod tests;
