//! surface 출력 색공간 협상과 gamut 경고 (XI.10.5).
//!
//! 아키텍처 §10.3: 초기 지원 색공간은 sRGB와 Display P3이며 Target Profile로 변환할 때 gamut 경고를 줍니다.
//!
//! - **capability 감지**: Display P3 surface 지원 여부는 backend `init`이 돌려준
//!   [`DeviceCapabilities::display_p3_surface`]입니다(Metal: 연결된 display·layer 지원, mock: 설정값).
//!   [`OutputColorPlan::negotiate`]가 요청 색공간(canvas·Target Profile)과 capability로 실제 surface 색공간을
//!   정합니다.
//! - **unsupported display**: Display P3를 요청했는데 지원하지 않으면 surface를 sRGB로 두고
//!   [`OutputColorWarning::DisplayP3Unsupported`]를 냅니다. 이때 Display P3 색은 선형광에서 D65 primaries 행렬로
//!   sRGB로 변환하며, sRGB 범위 `[0, 1]` 밖으로 나가는 색은 잘라 표시하고 그 사실을 gamut 경고
//!   ([`OutputColorWarning::GamutClipped`])로 남깁니다(조용히 자르지 않음).
//! - **P3 surface**: sRGB 색은 P3 안에 들어가므로 행렬 변환만 하고 경고가 없습니다.
//!
//! 변환식(행 우선, 선형광, D65): transfer decode(ADR-0002, sRGB·P3 공통 곡선) → 행렬 → surface 색공간
//! encode. 행렬은 expressions crate 색 함수(VIII.4.4)와 같은 상수입니다(crate 의존 정책상 각자 보관).
//! gamut 판정 허용 오차는 [`GAMUT_EPSILON`]입니다(행렬 반올림이 경계 색을 잘못 경고하지 않게).
//!
//! 경고에는 정적 종류와 개수, node의 안정 ID만 담습니다(이름·내용 없음).

use stellar_scene::stack::{Paint, PaintLayer};
use stellar_scene::{ColorSpace, SceneSnapshot};

use crate::backend::{DeviceCapabilities, SurfaceConfig};

/// 선형 Display P3 → 선형 sRGB (D65). 행 우선.
const P3_TO_SRGB: [[f32; 3]; 3] = [
    [1.224_940_1, -0.224_940_4, 0.0],
    [-0.042_056_9, 1.042_057_1, 0.0],
    [-0.019_637_6, -0.078_636_1, 1.098_273_5],
];

/// 선형 sRGB → 선형 Display P3 (D65). 행 우선.
const SRGB_TO_P3: [[f32; 3]; 3] = [
    [0.822_462_1, 0.177_538, 0.0],
    [0.033_194_1, 0.966_805_8, 0.0],
    [0.017_082_7, 0.072_397_4, 0.910_519_9],
];

/// 선형 채널이 `[-ε, 1 + ε]` 안이면 gamut 안으로 봅니다.
pub const GAMUT_EPSILON: f32 = 1.0e-5;

/// gamut 경고에 기록하는 node 수 상한(개수는 모두 셉니다).
pub const MAX_REPORTED_NODES: usize = 64;

/// renderer 색 값: 색공간 + 비연관 RGBA(인코딩 값).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputColor {
    /// 색공간.
    pub space: ColorSpace,
    /// 인코딩된 R, G, B(범위 제한 없음).
    pub rgb: [f32; 3],
    /// 알파 `[0, 1]`.
    pub alpha: f32,
}

impl OutputColor {
    /// scene paint의 색.
    pub fn from_paint(paint: Paint) -> OutputColor {
        match paint {
            Paint::Solid(color) => OutputColor {
                space: color.space(),
                rgb: [color.r(), color.g(), color.b()],
                alpha: color.alpha(),
            },
        }
    }
}

/// gamut 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GamutStatus {
    /// surface 색공간 안입니다.
    InGamut,
    /// 색공간 변환 결과가 surface 범위 밖이라 잘랐습니다.
    Clipped,
}

/// surface 색공간으로 변환한 색.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceColor {
    /// surface 색공간 선형광 RGB(`[0, 1]`로 자른 값).
    pub linear: [f32; 3],
    /// surface 색공간 인코딩 RGB.
    pub encoded: [f32; 3],
    /// 알파.
    pub alpha: f32,
    /// gamut 판정.
    pub gamut: GamutStatus,
}

/// 출력 색 경고.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutputColorWarning {
    /// Display P3를 요청했지만 display·device가 지원하지 않아 sRGB surface로 렌더링합니다.
    DisplayP3Unsupported,
    /// 색공간 변환 결과가 surface gamut 밖이라 잘라 표시합니다.
    GamutClipped {
        /// 잘린 색 수.
        colors: u64,
        /// 잘린 색이 있는 node 수.
        nodes: u64,
        /// 해당 node 안정 ID(앞에서부터 최대 [`MAX_REPORTED_NODES`]개, pre-order 순서).
        node_ids: Vec<[u8; 16]>,
    },
}

impl OutputColorWarning {
    /// 정적 진단 코드.
    pub const fn code(&self) -> &'static str {
        match self {
            OutputColorWarning::DisplayP3Unsupported => "render.color.display_p3_unsupported",
            OutputColorWarning::GamutClipped { .. } => "render.color.gamut_clipped",
        }
    }
}

/// surface 색공간 결정과 변환 규칙.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputColorPlan {
    requested: ColorSpace,
    surface: ColorSpace,
}

fn multiply(matrix: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    let row = |r: &[f32; 3]| r[0] * v[0] + r[1] * v[1] + r[2] * v[2];
    [row(&matrix[0]), row(&matrix[1]), row(&matrix[2])]
}

impl OutputColorPlan {
    /// 요청 색공간과 device capability로 surface 색공간을 정합니다.
    pub fn negotiate(requested: ColorSpace, caps: &DeviceCapabilities) -> OutputColorPlan {
        let surface = match requested {
            ColorSpace::DisplayP3 if caps.display_p3_surface => ColorSpace::DisplayP3,
            ColorSpace::DisplayP3 | ColorSpace::Srgb => ColorSpace::Srgb,
        };
        OutputColorPlan { requested, surface }
    }

    /// 요청 색공간.
    pub const fn requested(&self) -> ColorSpace {
        self.requested
    }

    /// 실제 surface 색공간.
    pub const fn surface(&self) -> ColorSpace {
        self.surface
    }

    /// 요청보다 좁은 surface로 떨어졌으면 참입니다.
    pub fn is_fallback(&self) -> bool {
        self.requested != self.surface
    }

    /// surface 설정에 결정된 색공간을 넣습니다.
    pub fn surface_config(&self, base: SurfaceConfig) -> SurfaceConfig {
        SurfaceConfig {
            color_space: self.surface,
            ..base
        }
    }

    /// 협상 경고(fallback이면 [`OutputColorWarning::DisplayP3Unsupported`]).
    pub fn negotiation_warnings(&self) -> Vec<OutputColorWarning> {
        if self.is_fallback() {
            vec![OutputColorWarning::DisplayP3Unsupported]
        } else {
            Vec::new()
        }
    }

    /// 색을 surface 색공간으로 변환합니다(모듈 문서 변환식).
    pub fn convert(&self, color: OutputColor) -> SurfaceColor {
        let source = color.space;
        let linear = color.rgb.map(|c| source.decode_channel(c));
        let converted = match (source, self.surface) {
            (ColorSpace::DisplayP3, ColorSpace::Srgb) => multiply(&P3_TO_SRGB, linear),
            (ColorSpace::Srgb, ColorSpace::DisplayP3) => multiply(&SRGB_TO_P3, linear),
            (ColorSpace::Srgb, ColorSpace::Srgb)
            | (ColorSpace::DisplayP3, ColorSpace::DisplayP3) => linear,
        };
        let in_gamut = converted
            .iter()
            .all(|c| (-GAMUT_EPSILON..=1.0 + GAMUT_EPSILON).contains(c));
        let clipped = converted.map(|c| c.clamp(0.0, 1.0));
        SurfaceColor {
            linear: clipped,
            encoded: clipped.map(|c| self.surface.encode_channel(c)),
            alpha: color.alpha,
            gamut: if in_gamut {
                GamutStatus::InGamut
            } else {
                GamutStatus::Clipped
            },
        }
    }

    /// snapshot의 모든 단색 paint(fill·stroke·shadow)를 변환해 gamut 경고를 만듭니다.
    ///
    /// 협상 경고를 앞에 두고, 잘린 색이 있으면 [`OutputColorWarning::GamutClipped`]를 덧붙입니다.
    pub fn scan(&self, snapshot: &SceneSnapshot) -> GamutReport {
        let mut report = GamutReport {
            checked: 0,
            clipped_colors: 0,
            clipped_nodes: 0,
            node_ids: Vec::new(),
        };
        for node in snapshot.iter_preorder() {
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
            let mut node_clipped = false;
            for color in colors {
                report.checked += 1;
                if self.convert(color).gamut == GamutStatus::Clipped {
                    report.clipped_colors += 1;
                    node_clipped = true;
                }
            }
            if node_clipped {
                report.clipped_nodes += 1;
                if report.node_ids.len() < MAX_REPORTED_NODES {
                    report.node_ids.push(node.source().to_bytes());
                }
            }
        }
        report
    }

    /// 협상 경고와 snapshot gamut 경고를 합칩니다.
    pub fn warnings(&self, snapshot: &SceneSnapshot) -> Vec<OutputColorWarning> {
        let mut warnings = self.negotiation_warnings();
        if let Some(warning) = self.scan(snapshot).warning() {
            warnings.push(warning);
        }
        warnings
    }
}

/// snapshot gamut 검사 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamutReport {
    /// 검사한 색 수.
    pub checked: u64,
    /// 잘린 색 수.
    pub clipped_colors: u64,
    /// 잘린 색이 있는 node 수.
    pub clipped_nodes: u64,
    /// 해당 node 안정 ID(최대 [`MAX_REPORTED_NODES`]개).
    pub node_ids: Vec<[u8; 16]>,
}

impl GamutReport {
    /// 잘린 색이 있으면 gamut 경고.
    pub fn warning(&self) -> Option<OutputColorWarning> {
        (self.clipped_colors > 0).then(|| OutputColorWarning::GamutClipped {
            colors: self.clipped_colors,
            nodes: self.clipped_nodes,
            node_ids: self.node_ids.clone(),
        })
    }
}

#[cfg(test)]
mod tests;
