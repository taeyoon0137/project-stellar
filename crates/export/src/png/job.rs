//! PNG 내보내기 worker (XX.2.3).
//!
//! [`PngJob::spawn`]은 렌더와 encode를 이름 있는 worker thread(`stellar-export-png`)에서 실행합니다. 호출 thread(UI·
//! document thread)는 기다리지 않습니다. 취소([`PngJob::cancel`])는 렌더 draw 사이와 encode 행·압축 block 사이에서
//! 확인하며, 취소된 job은 bytes를 돌려주지 않습니다([`PngExportError::Cancelled`]). 결과 bytes를 파일로 옮기는 저장은
//! host가 temp + atomic rename으로 하므로(모듈 [`crate::png`]) 취소·실패 시 최종 경로에 부분 파일이 생기지 않습니다.
//!
//! [`render_png`]는 같은 일을 호출 thread에서 하는 본체입니다(worker가 이 함수를 부릅니다).

use std::fmt;
use std::sync::Arc;
use std::thread::JoinHandle;

use stellar_core_types::ids::StableId;
use stellar_renderer_core::output_color::{GamutStatus, OutputColor};
use stellar_scene::node::{SceneNode, Visibility};
use stellar_scene::stack::PaintLayer;

use super::encode::encode_png;
use super::PngOptions;
use crate::preflight::ConfirmedExport;
use crate::raster::{
    check_image, CancelToken, OffscreenError, OffscreenRequest, OffscreenTarget, OutputProfile,
};
use crate::request::ExportFormat;

/// gamut 요약에 기록하는 node 수 상한.
pub const MAX_GAMUT_NODES: usize = 64;

/// 출력 profile로 옮길 때 잘린 색 요약(내보내기 범위의 보이는 node, XI.10.5·XI.10.6 판정).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GamutSummary {
    /// 출력 gamut 밖이라 잘린 색 수.
    pub clipped_colors: u64,
    /// 잘린 색이 있는 node(paint 순서, 최대 [`MAX_GAMUT_NODES`]개).
    pub clipped_nodes: Vec<StableId>,
    /// Display P3로도 표현할 수 없는 확장 범위 색 수.
    pub extended_colors: u64,
    /// 확장 범위 색이 있는 node(최대 [`MAX_GAMUT_NODES`]개).
    pub extended_nodes: Vec<StableId>,
}

impl GamutSummary {
    /// 잘린 색이 없으면 참입니다.
    pub fn is_clean(&self) -> bool {
        self.clipped_colors == 0 && self.extended_colors == 0
    }

    /// 내보내기 범위의 단색 paint(fill·stroke·shadow)를 `profile`로 옮겨 판정합니다(renderer
    /// [`OutputColorPlan::convert`](stellar_renderer_core::output_color::OutputColorPlan::convert)).
    pub fn of(export: &ConfirmedExport, profile: OutputProfile) -> GamutSummary {
        let plan = profile.color_plan();
        let mut summary = GamutSummary::default();
        for root in export.capture().roots() {
            let mut stack: Vec<&SceneNode> = vec![root.node()];
            while let Some(node) = stack.pop() {
                if node.hit().visibility == Visibility::Hidden {
                    continue;
                }
                let paint = node.paint().get();
                let mut colors: Vec<OutputColor> = paint
                    .layers
                    .iter()
                    .map(|layer| match layer {
                        PaintLayer::Fill(paint) => OutputColor::from_paint(*paint),
                        PaintLayer::Stroke(stroke) => OutputColor::from_paint(stroke.paint()),
                    })
                    .collect();
                colors.extend(paint.shadows.iter().map(|shadow| {
                    let color = shadow.color();
                    OutputColor {
                        space: color.space(),
                        rgb: [color.r(), color.g(), color.b()],
                        alpha: color.alpha(),
                    }
                }));
                let (mut clipped, mut extended) = (false, false);
                for color in colors {
                    match plan.convert(color).gamut {
                        GamutStatus::InGamut => {}
                        GamutStatus::Clipped => {
                            summary.clipped_colors += 1;
                            clipped = true;
                        }
                        GamutStatus::ExtendedRange => {
                            summary.extended_colors += 1;
                            extended = true;
                        }
                    }
                }
                if clipped && summary.clipped_nodes.len() < MAX_GAMUT_NODES {
                    summary.clipped_nodes.push(node.source());
                }
                if extended && summary.extended_nodes.len() < MAX_GAMUT_NODES {
                    summary.extended_nodes.push(node.source());
                }
                let start = stack.len();
                stack.extend(node.children().map(|child| &**child));
                stack[start..].reverse();
            }
        }
        summary
    }
}

/// encode한 PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngFile {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    profile: OutputProfile,
    gamut: GamutSummary,
}

impl PngFile {
    /// PNG bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// bytes를 꺼냅니다.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// 너비.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// 높이.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// 출력 profile.
    pub const fn profile(&self) -> OutputProfile {
        self.profile
    }

    /// gamut 요약.
    pub const fn gamut(&self) -> &GamutSummary {
        &self.gamut
    }
}

/// PNG 내보내기 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum PngExportError {
    /// 요청 형식이 PNG가 아닙니다.
    WrongFormat(ExportFormat),
    /// 취소되었습니다.
    Cancelled,
    /// offscreen 렌더 실패.
    Render(OffscreenError),
    /// worker thread를 만들거나 결과를 받지 못했습니다.
    Worker {
        /// 설명.
        message: String,
    },
}

impl fmt::Display for PngExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngExportError::WrongFormat(format) => {
                write!(f, "PNG 내보내기에 {} 요청을 넘겼습니다", format.name())
            }
            PngExportError::Cancelled => f.write_str("PNG 내보내기가 취소되었습니다"),
            PngExportError::Render(error) => write!(f, "{error}"),
            PngExportError::Worker { message } => write!(f, "PNG 내보내기 worker 실패: {message}"),
        }
    }
}

impl std::error::Error for PngExportError {}

impl From<OffscreenError> for PngExportError {
    fn from(error: OffscreenError) -> PngExportError {
        match error {
            OffscreenError::Cancelled => PngExportError::Cancelled,
            other => PngExportError::Render(other),
        }
    }
}

/// 렌더와 encode를 호출 thread에서 실행합니다(모듈 문서).
pub fn render_png(
    export: &ConfirmedExport,
    target: &dyn OffscreenTarget,
    options: PngOptions,
    cancel: &CancelToken,
) -> Result<PngFile, PngExportError> {
    let format = export.capture().request().format();
    if format != ExportFormat::Png {
        return Err(PngExportError::WrongFormat(format));
    }
    let request = OffscreenRequest::new(export, options.profile());
    request.size()?;
    let image = target.render(&request, cancel)?;
    check_image(&image, &request)?;
    if cancel.is_cancelled() {
        return Err(PngExportError::Cancelled);
    }
    let scale = export.capture().request().scale().get();
    let bytes = encode_png(&image, scale, cancel).map_err(|_| PngExportError::Cancelled)?;
    if cancel.is_cancelled() {
        return Err(PngExportError::Cancelled);
    }
    Ok(PngFile {
        bytes,
        width: image.width(),
        height: image.height(),
        profile: image.profile(),
        gamut: GamutSummary::of(export, options.profile()),
    })
}

/// worker에서 도는 PNG 내보내기.
#[derive(Debug)]
pub struct PngJob {
    cancel: CancelToken,
    handle: JoinHandle<Result<PngFile, PngExportError>>,
}

impl PngJob {
    /// worker를 시작합니다. 요청 형식이 PNG가 아니면 thread를 만들지 않고 거부합니다.
    pub fn spawn(
        export: ConfirmedExport,
        target: Arc<dyn OffscreenTarget + Send + Sync>,
        options: PngOptions,
    ) -> Result<PngJob, PngExportError> {
        let format = export.capture().request().format();
        if format != ExportFormat::Png {
            return Err(PngExportError::WrongFormat(format));
        }
        let cancel = CancelToken::new();
        let worker_cancel = cancel.clone();
        let handle = std::thread::Builder::new()
            .name("stellar-export-png".to_string())
            .spawn(move || render_png(&export, target.as_ref(), options, &worker_cancel))
            .map_err(|error| PngExportError::Worker {
                message: error.to_string(),
            })?;
        Ok(PngJob { cancel, handle })
    }

    /// 취소 신호(복제본). 진행 정도도 이 값으로 읽습니다.
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// 취소를 요청합니다.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// worker가 끝났으면 참입니다.
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// 결과를 기다립니다. 결과가 나온 뒤 취소를 요청했어도 취소로 봅니다(취소한 내보내기는 저장하지 않음).
    pub fn wait(self) -> Result<PngFile, PngExportError> {
        let result = self.handle.join().map_err(|_| PngExportError::Worker {
            message: "worker가 비정상 종료했습니다".to_string(),
        })?;
        if self.cancel.is_cancelled() {
            return Err(PngExportError::Cancelled);
        }
        result
    }
}
