//! 효과 window: pass마다 그리는 device 영역과 해상도 (ADR-0136).
//!
//! 그리기 목록([`Op`])의 좌표는 device pixel입니다. pass 하나는 [`Window`] 하나에 그립니다. window는 device 영역
//! `[x, x + width·step) × [y, y + height·step)`이고 texel 하나가 device `step` pixel(2의 거듭제곱)입니다. device 점 `p`는
//! texel 좌표 `(p − 원점) / step`입니다. CPU 기준([`super::raster`])과 GPU lowering([`super::lower`])이 같은 계획
//! ([`plan_layer`])을 씁니다.
//!
//! # 규칙
//!
//! - **출력 window**: 바깥 target은 출력 영역(`core`)만 덮습니다. 효과가 번지는 여백은 target에 들어가지 않습니다.
//! - **효과 없는 layer**: 내용 pass·합성 결과가 부모 window 그대로입니다.
//! - **layer blur**: 부모 window를 blur 번짐 거리([`reach`])만큼 넓힌 영역, ops 범위([`EffectSpace::canvas`]), layer 내용
//!   범위를 번짐만큼 넓힌 영역의 교집합에 내용과 shadow를 쌓습니다. 해상도는 blur 낮은 해상도 배수 `e`
//!   ([`super::blur::effect_downsample`])가 2 이상이면 `e / 2`(부모보다 거칠게만)이고, 그 window에서 `e`까지 2×2 상자
//!   평균으로 줄여 blur합니다. 결과는 부모 window에 bilinear로 되돌려 그립니다(resolve).
//! - **shadow**: 쌓는 window를 `−offset`만큼 옮겨 번짐만큼 넓힌 영역(원본 범위를 번짐만큼 넓힌 영역, ops 범위와 교집합)에
//!   원본을 그립니다. 그래서 offset이 커도 window는 offset만큼 커지지 않습니다. 해상도 규칙은 layer blur와 같습니다. 원본이
//!   내용이고 해상도가 같고 쌓는 window 안이면 내용 pass를 그대로 씁니다. 아니면 내용을 그 window에 따로 그립니다.
//! - **정렬**: 효과 window 원점은 최종 blur 해상도 `max(e, step)`의 배수(device 0 기준)입니다. 그래서 낮은 해상도
//!   texel 격자가 window와 무관하게 같습니다(전체 target 하나에 그리던 이전 결과와 같은 격자).
//! - **크기 상한**: window 한 변이 [`EffectSpace::max_side`] texel을 넘으면 해상도를 2배씩 낮춥니다. 최종 blur 해상도
//!   `max(e, 부모 step)`보다 거칠어져야 하면 [`WindowError::TooLarge`]입니다(근사하지 않음, 호출자가 supersampling을 낮춤).
//! - **정밀도**: `e ≥ 2`이면 device σ / e ≥ 32입니다. `e / 2` 해상도로 pixel 중심 표본을 뜬 뒤 한 번 상자 평균하면 가장자리
//!   위치 오차가 texel 1/4 이하라 blur 결과 차이가 약 `0.4 · 0.25 / 32` = 0.3%(8-bit 한 단계 미만)입니다(test).

use super::blur::REACH_PER_SIGMA;
use super::fixture::{ClipSpec, LayerOp, Mesh, Op, ShadowSource};

/// device pixel 영역 `[x0, x1) × [y0, y1)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// 왼쪽.
    pub x0: f32,
    /// 위.
    pub y0: f32,
    /// 오른쪽.
    pub x1: f32,
    /// 아래.
    pub y1: f32,
}

impl Bounds {
    /// `[0, width) × [0, height)`.
    pub fn of_size(width: u32, height: u32) -> Bounds {
        Bounds {
            x0: 0.0,
            y0: 0.0,
            x1: width as f32,
            y1: height as f32,
        }
    }

    /// 넓이가 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        !(self.x1 > self.x0 && self.y1 > self.y0)
    }

    /// 합집합 상자.
    pub fn union(self, other: Bounds) -> Bounds {
        Bounds {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    /// 교집합(비면 `None`).
    pub fn intersect(self, other: Bounds) -> Option<Bounds> {
        let out = Bounds {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };
        (!out.is_empty()).then_some(out)
    }

    /// 축마다 넓힙니다.
    pub fn expand(self, dx: f32, dy: f32) -> Bounds {
        Bounds {
            x0: self.x0 - dx,
            y0: self.y0 - dy,
            x1: self.x1 + dx,
            y1: self.y1 + dy,
        }
    }

    /// 옮깁니다.
    pub fn shift(self, [ox, oy]: [f32; 2]) -> Bounds {
        Bounds {
            x0: self.x0 + ox,
            y0: self.y0 + oy,
            x1: self.x1 + ox,
            y1: self.y1 + oy,
        }
    }

    /// `other`를 덮으면 참입니다.
    pub fn contains(&self, other: &Bounds) -> bool {
        self.x0 <= other.x0 && self.y0 <= other.y0 && self.x1 >= other.x1 && self.y1 >= other.y1
    }

    /// 점들의 경계 상자(비면 `None`).
    pub fn of_points<'a>(points: impl IntoIterator<Item = &'a [f32; 2]>) -> Option<Bounds> {
        let mut out: Option<Bounds> = None;
        for &[x, y] in points {
            let point = Bounds {
                x0: x,
                y0: y,
                x1: x,
                y1: y,
            };
            out = Some(out.map_or(point, |b| b.union(point)));
        }
        out
    }
}

/// pass 하나가 그리는 device 영역과 해상도(모듈 문서).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Window {
    /// 원점 x(device pixel).
    pub x: i64,
    /// 원점 y(device pixel).
    pub y: i64,
    /// 너비(texel).
    pub width: u32,
    /// 높이(texel).
    pub height: u32,
    /// texel 하나의 device pixel 수(2의 거듭제곱).
    pub step: u32,
}

impl Window {
    /// 원점 0, 해상도 1의 `width × height` window.
    pub const fn full(width: u32, height: u32) -> Window {
        Window {
            x: 0,
            y: 0,
            width,
            height,
            step: 1,
        }
    }

    /// texel 크기.
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 덮는 device 영역.
    pub fn bounds(&self) -> Bounds {
        let step = f64::from(self.step);
        Bounds {
            x0: self.x as f32,
            y0: self.y as f32,
            x1: (self.x as f64 + f64::from(self.width) * step) as f32,
            y1: (self.y as f64 + f64::from(self.height) * step) as f32,
        }
    }

    /// device 점 → texel 좌표.
    pub fn map(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let step = self.step as f32;
        [
            ((f64::from(x) - self.x as f64) as f32) / step,
            ((f64::from(y) - self.y as f64) as f32) / step,
        ]
    }

    /// texel `(tx, ty)` 중심의 device 좌표.
    pub fn center(&self, tx: u32, ty: u32) -> [f32; 2] {
        let step = f64::from(self.step);
        [
            (self.x as f64 + (f64::from(tx) + 0.5) * step) as f32,
            (self.y as f64 + (f64::from(ty) + 0.5) * step) as f32,
        ]
    }

    /// mesh를 texel 좌표로 옮깁니다.
    pub fn map_mesh(&self, mesh: &Mesh) -> Mesh {
        Mesh {
            positions: mesh.positions.iter().map(|p| self.map(*p)).collect(),
        }
    }

    /// device 사각형(x0, y0, x1, y1) → texel 사각형.
    pub fn map_rect(&self, [x0, y0, x1, y1]: [f32; 4]) -> [f32; 4] {
        let [a, b] = self.map([x0, y0]);
        let [c, d] = self.map([x1, y1]);
        [a, b, c, d]
    }

    /// `other` window가 덮는 영역의 이 window texel 사각형.
    pub fn rect_of(&self, other: &Window) -> [f32; 4] {
        let b = other.bounds();
        self.map_rect([b.x0, b.y0, b.x1, b.y1])
    }

    /// device scissor 사각형 → 이 window texel scissor(바깥 쪽으로 올림, window 안으로 자름). 비면 `None`.
    pub fn map_scissor(&self, x: u32, y: u32, width: u32, height: u32) -> Option<[u32; 4]> {
        let step = f64::from(self.step);
        let lo = |v: u32, origin: i64, n: u32| {
            ((f64::from(v) - origin as f64) / step)
                .floor()
                .clamp(0.0, f64::from(n)) as u32
        };
        let hi = |v: u64, origin: i64, n: u32| {
            ((v as f64 - origin as f64) / step)
                .ceil()
                .clamp(0.0, f64::from(n)) as u32
        };
        let x0 = lo(x, self.x, self.width);
        let y0 = lo(y, self.y, self.height);
        let x1 = hi(u64::from(x) + u64::from(width), self.x, self.width);
        let y1 = hi(u64::from(y) + u64::from(height), self.y, self.height);
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
    }

    /// `factor`배(2의 거듭제곱) 줄인 window(같은 원점, 한 단계마다 `ceil(n / 2)`).
    pub fn downsampled(&self, factor: u32) -> Window {
        Window {
            x: self.x,
            y: self.y,
            width: super::blur::downsampled_extent(self.width, factor),
            height: super::blur::downsampled_extent(self.height, factor),
            step: self.step * factor.max(1),
        }
    }

    /// `bounds`를 덮는 window. 원점은 `align`의 배수로 내리고 크기는 `step`으로 올립니다.
    fn covering(bounds: Bounds, align: u32, step: u32) -> Option<Window> {
        let align = f64::from(align.max(step));
        let step_f = f64::from(step);
        let x = ((f64::from(bounds.x0) / align).floor() * align) as i64;
        let y = ((f64::from(bounds.y0) / align).floor() * align) as i64;
        let width = ((f64::from(bounds.x1) - x as f64) / step_f).ceil().max(1.0);
        let height = ((f64::from(bounds.y1) - y as f64) / step_f).ceil().max(1.0);
        if !(width.is_finite() && height.is_finite()) || width > f64::from(u32::MAX) {
            return None;
        }
        if height > f64::from(u32::MAX) {
            return None;
        }
        Some(Window {
            x,
            y,
            width: width as u32,
            height: height as u32,
            step,
        })
    }
}

/// 그리기 목록 전체의 공간: ops 좌표가 뜻을 갖는 device 범위와 texture 한 변 상한.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectSpace {
    /// ops의 device 범위(fixture target, 내보내기 tile의 compile viewport). 효과 window는 이 범위를 넘지 않습니다(밖의
    /// 내용은 없는 것으로 봅니다).
    pub canvas: Bounds,
    /// 효과 window 한 변 상한(texel).
    pub max_side: u32,
}

impl EffectSpace {
    /// `width × height` target 하나에 그리는 fixture 공간(texture 상한 [`crate::render_graph::MAX_TEXTURE_DIMENSION`]).
    pub fn full(width: u32, height: u32) -> EffectSpace {
        EffectSpace {
            canvas: Bounds::of_size(width, height),
            max_side: crate::render_graph::MAX_TEXTURE_DIMENSION,
        }
    }
}

/// window 계획 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    /// 효과 window가 texture 한 변 상한을 넘습니다(최종 blur 해상도로도).
    TooLarge {
        /// 필요한 한 변(texel, 가장 거친 해상도에서).
        side: u64,
    },
    /// σ가 낮은 해상도 배수 상한 밖입니다(fixture 계약 위반).
    Sigma,
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WindowError::TooLarge { side } => {
                write!(
                    f,
                    "효과 window 한 변 {side} texel이 texture 상한을 넘습니다"
                )
            }
            WindowError::Sigma => f.write_str("blur σ가 범위 밖입니다"),
        }
    }
}

impl std::error::Error for WindowError {}

/// σ의 효과 번짐 거리(device pixel): [`REACH_PER_SIGMA`]·σ + bilinear 한 pixel.
pub fn reach(sigma: f32) -> f32 {
    if sigma > 0.0 {
        REACH_PER_SIGMA * sigma + 1.0
    } else {
        1.0
    }
}

/// shadow 하나의 계획.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowPlan {
    /// 원본 window.
    pub source: Window,
    /// 원본이 layer 내용 pass 그대로면 참(따로 그리지 않음).
    pub reuse_content: bool,
    /// 원본 → blur 해상도 줄이기 배수.
    pub factor: u32,
    /// blur 해상도 texel σ(가로, 세로).
    pub sigma: [f32; 2],
    /// blur 결과 window(`source`를 `factor`배 줄인 것).
    pub blurred: Window,
}

/// layer blur 계획.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurPlan {
    /// 쌓은 결과 → blur 해상도 줄이기 배수.
    pub factor: u32,
    /// blur 해상도 texel σ(가로, 세로).
    pub sigma: [f32; 2],
    /// blur 결과 window.
    pub blurred: Window,
    /// 결과를 부모 window에 다시 그려야 하면 참(`blurred`가 부모와 다름).
    pub resolve: bool,
}

/// layer 하나의 window 계획.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerPlan {
    /// 내용을 그리고 shadow·내용을 쌓는 window. blur가 없으면 부모 window입니다.
    pub group: Window,
    /// shadow마다 계획(`None`이면 이 window에 보이지 않음).
    pub shadows: Vec<Option<ShadowPlan>>,
    /// layer blur 계획.
    pub blur: Option<BlurPlan>,
}

/// 그리기 목록의 device 경계 상자(비면 `None`).
pub fn ops_bounds(ops: &[Op]) -> Option<Bounds> {
    let mut out: Option<Bounds> = None;
    for op in ops {
        let bounds = match op {
            Op::Fill(fill) => Bounds::of_points(&fill.mesh.positions),
            Op::Image(image) => {
                let [x0, y0, x1, y1] = image.rect;
                Some(Bounds {
                    x0: x0.min(x1),
                    y0: y0.min(y1),
                    x1: x0.max(x1),
                    y1: y0.max(y1),
                })
            }
            Op::Glyphs(glyph) => {
                let [x, y] = glyph.origin.map(|v| v as f32);
                Some(Bounds {
                    x0: x,
                    y0: y,
                    x1: x + glyph.mask.width as f32,
                    y1: y + glyph.mask.height as f32,
                })
            }
            Op::Layer(layer) => layer_bounds(layer),
        };
        if let Some(bounds) = bounds.filter(|b| !b.is_empty()) {
            out = Some(out.map_or(bounds, |b| b.union(bounds)));
        }
    }
    out
}

/// shadow 원본 범위.
fn source_bounds(layer: &LayerOp, source: &ShadowSource) -> Option<Bounds> {
    match source {
        ShadowSource::Content => ops_bounds(&layer.content),
        ShadowSource::Box(mesh) => Bounds::of_points(&mesh.positions),
    }
}

/// blur·shadow 전 layer 범위(내용 ∪ shadow 번짐).
fn stacked_bounds(layer: &LayerOp) -> Option<Bounds> {
    let mut out = ops_bounds(&layer.content);
    for shadow in &layer.shadows {
        let [sx, sy] = shadow.sigmas();
        if let Some(source) = source_bounds(layer, &shadow.source) {
            let footprint = source.shift(shadow.offset).expand(reach(sx), reach(sy));
            out = Some(out.map_or(footprint, |b| b.union(footprint)));
        }
    }
    out
}

/// layer 합성 결과의 device 경계 상자(효과 번짐 포함, clip·mask로 자름).
pub fn layer_bounds(layer: &LayerOp) -> Option<Bounds> {
    let mut out = stacked_bounds(layer)?;
    let [bx, by] = layer.blur_sigmas();
    if bx > 0.0 || by > 0.0 {
        out = out.expand(reach(bx), reach(by));
    }
    let clip = match &layer.clip {
        None => None,
        Some(ClipSpec::Scissor {
            x,
            y,
            width,
            height,
        }) => Some(Bounds {
            x0: *x as f32,
            y0: *y as f32,
            x1: (u64::from(*x) + u64::from(*width)) as f32,
            y1: (u64::from(*y) + u64::from(*height)) as f32,
        }),
        Some(ClipSpec::Stencil(mesh)) => {
            Some(Bounds::of_points(&mesh.positions).unwrap_or(Bounds {
                x0: 0.0,
                y0: 0.0,
                x1: 0.0,
                y1: 0.0,
            }))
        }
        Some(ClipSpec::Mask(shapes)) => Some(coverage_bounds(shapes)),
    };
    if let Some(clip) = clip {
        out = out.intersect(clip)?;
    }
    if !layer.mask.is_empty() {
        out = out.intersect(coverage_bounds(&layer.mask))?;
    }
    Some(out)
}

fn coverage_bounds(shapes: &[super::fixture::CoverageShape]) -> Bounds {
    shapes
        .iter()
        .filter_map(|shape| Bounds::of_points(&shape.mesh.positions))
        .reduce(Bounds::union)
        .unwrap_or(Bounds {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        })
}

/// 효과 원본 해상도: `e ≥ 2`이면 `max(parent, e / 2)`, 아니면 `parent`.
fn source_step(e: u32, parent: u32) -> u32 {
    if e >= 2 {
        parent.max(e / 2)
    } else {
        parent
    }
}

/// `needed`를 덮는 효과 window. 원점은 최종 blur 해상도의 배수이고, 한 변이 상한을 넘으면 해상도를 2배씩 낮춥니다
/// (최종 blur 해상도 `max(e, step)`까지).
fn fit(needed: Bounds, e: u32, step: u32, space: &EffectSpace) -> Result<Window, WindowError> {
    let mut step = step.max(1);
    let cap = e.max(step);
    loop {
        let lattice = e.max(step);
        let window = Window::covering(needed, lattice, step).ok_or(WindowError::TooLarge {
            side: u64::from(u32::MAX),
        })?;
        let side = window.width.max(window.height);
        if side <= space.max_side {
            return Ok(window);
        }
        if step * 2 > cap {
            return Err(WindowError::TooLarge {
                side: u64::from(side),
            });
        }
        step *= 2;
    }
}

/// `layer`를 `parent` window에 합성할 때의 window 계획(모듈 문서 규칙). 부모 window에 보이는 것이 없으면 `None`입니다.
pub fn plan_layer(
    layer: &LayerOp,
    parent: &Window,
    space: &EffectSpace,
) -> Result<Option<LayerPlan>, WindowError> {
    let Some(output) = layer_bounds(layer) else {
        return Ok(None);
    };
    if output.intersect(parent.bounds()).is_none() {
        return Ok(None);
    }
    let [bx, by] = layer.blur_sigmas();
    let mut blur = None;
    let group = if bx > 0.0 || by > 0.0 {
        let e = layer.blur_downsample().ok_or(WindowError::Sigma)?;
        let (rx, ry) = (reach(bx), reach(by));
        let Some(needed) = stacked_bounds(layer).and_then(|stacked| {
            parent
                .bounds()
                .expand(rx, ry)
                .intersect(space.canvas)?
                .intersect(stacked.expand(rx, ry))
        }) else {
            return Ok(None);
        };
        let group = fit(needed, e, source_step(e, parent.step), space)?;
        let blur_step = e.max(group.step);
        let factor = blur_step / group.step;
        let blurred = group.downsampled(factor);
        blur = Some(BlurPlan {
            factor,
            sigma: [bx / blur_step as f32, by / blur_step as f32],
            blurred,
            resolve: blurred != *parent,
        });
        group
    } else {
        *parent
    };
    let mut shadows = Vec::with_capacity(layer.shadows.len());
    for shadow in &layer.shadows {
        let e = shadow.downsample().ok_or(WindowError::Sigma)?;
        let [sx, sy] = shadow.sigmas();
        let (rx, ry) = (reach(sx), reach(sy));
        let [ox, oy] = shadow.offset;
        let needed = source_bounds(layer, &shadow.source).and_then(|source| {
            group
                .bounds()
                .shift([-ox, -oy])
                .expand(rx, ry)
                .intersect(space.canvas)?
                .intersect(source.expand(rx, ry))
        });
        let Some(needed) = needed else {
            shadows.push(None);
            continue;
        };
        let step = source_step(e, group.step);
        let reuse = matches!(shadow.source, ShadowSource::Content)
            && step == group.step
            && group.bounds().contains(&needed);
        let source = if reuse {
            group
        } else {
            fit(needed, e, step, space)?
        };
        let blur_step = e.max(source.step);
        let factor = blur_step / source.step;
        shadows.push(Some(ShadowPlan {
            source,
            reuse_content: reuse,
            factor,
            sigma: [sx / blur_step as f32, sy / blur_step as f32],
            blurred: source.downsampled(factor),
        }));
    }
    Ok(Some(LayerPlan {
        group,
        shadows,
        blur,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::fixture::{solid, srgb, Coverage, SceneBlendMode};

    fn card(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Op> {
        vec![solid(
            Mesh::rect(x0, y0, x1, y1),
            srgb(1.0, 1.0, 1.0, 1.0),
            Coverage::Disjoint,
        )]
    }

    #[test]
    fn layers_without_effects_draw_in_the_parent_window() {
        let parent = Window {
            x: 100,
            y: 50,
            width: 64,
            height: 64,
            step: 1,
        };
        let space = EffectSpace::full(1000, 1000);
        let layer = LayerOp::new(card(120.0, 60.0, 130.0, 70.0), SceneBlendMode::Multiply);
        let plan = plan_layer(&layer, &parent, &space).unwrap().unwrap();
        assert_eq!(plan.group, parent);
        assert!(plan.shadows.is_empty() && plan.blur.is_none());
        // 부모 window 밖 내용만 있으면 그리지 않습니다.
        let away = LayerOp::new(card(400.0, 400.0, 410.0, 410.0), SceneBlendMode::Multiply);
        assert_eq!(plan_layer(&away, &parent, &space).unwrap(), None);
    }

    #[test]
    fn large_offset_shadow_source_window_follows_the_offset_instead_of_growing() {
        // 출력 window 64×64, offset (600, 0), σ 2: 원본 window는 −600만큼 옮긴 자리 하나뿐이고 크기는 출력 + 번짐입니다.
        let parent = Window {
            x: 700,
            y: 0,
            width: 64,
            height: 64,
            step: 1,
        };
        let space = EffectSpace::full(2000, 200);
        let layer = LayerOp::new(card(110.0, 10.0, 150.0, 40.0), SceneBlendMode::Normal)
            .with_shadow([600, 0], 4.0, srgb(0.0, 0.0, 0.0, 0.5));
        let plan = plan_layer(&layer, &parent, &space).unwrap().unwrap();
        let shadow = plan.shadows[0].unwrap();
        assert!(!shadow.reuse_content);
        assert_eq!(shadow.factor, 1);
        assert_eq!(shadow.source.step, 1);
        let reach = reach(2.0);
        // 원본 window는 (출력 − offset) ⊕ 번짐과 내용 ⊕ 번짐의 교집합입니다.
        assert_eq!(shadow.source.x, (110.0 - reach).floor() as i64);
        assert!(shadow.source.width <= 64 + 2 * reach.ceil() as u32);
        assert!(shadow.source.bounds().x1 >= 150.0);
    }

    #[test]
    fn large_blur_draws_the_margin_at_half_the_blur_resolution() {
        // σ 300 → e 8: 내용·쌓기는 step 4, blur는 step 8, 원점은 8의 배수입니다.
        let parent = Window {
            x: 1000,
            y: 1000,
            width: 1024,
            height: 1024,
            step: 1,
        };
        let space = EffectSpace {
            canvas: Bounds::of_size(4000, 4000),
            max_side: 4096,
        };
        let layer = LayerOp::new(card(1200.0, 1200.0, 1600.0, 1500.0), SceneBlendMode::Normal)
            .with_blur(300.0);
        let plan = plan_layer(&layer, &parent, &space).unwrap().unwrap();
        assert_eq!(plan.group.step, 4);
        assert_eq!(plan.group.x % 8, 0);
        let blur = plan.blur.unwrap();
        assert_eq!((blur.factor, blur.blurred.step), (2, 8));
        assert!((blur.sigma[0] - 37.5).abs() < 1e-4);
        assert!(blur.resolve);
        // 한 변은 (내용 + 2·번짐) / 4 정도입니다(device 해상도면 2300을 넘음).
        assert!(plan.group.width < 700, "{}", plan.group.width);
        // 상한이 작으면 blur 해상도까지 낮추고, 그래도 넘으면 실패합니다.
        let tight = EffectSpace {
            max_side: 400,
            ..space
        };
        let plan = plan_layer(&layer, &parent, &tight).unwrap().unwrap();
        assert_eq!(plan.group.step, 8);
        let tighter = EffectSpace {
            max_side: 100,
            ..space
        };
        assert!(matches!(
            plan_layer(&layer, &parent, &tighter),
            Err(WindowError::TooLarge { .. })
        ));
    }

    #[test]
    fn content_shadow_reuses_the_content_pass_inside_the_group() {
        let parent = Window::full(64, 64);
        let space = EffectSpace::full(64, 64);
        let layer = LayerOp::new(card(10.0, 10.0, 30.0, 30.0), SceneBlendMode::Normal)
            .with_shadow([2, 2], 2.0, srgb(0.0, 0.0, 0.0, 0.5))
            .with_box_shadow(
                [2.0, 2.0],
                2.0,
                srgb(0.0, 0.0, 0.0, 0.5),
                Mesh::rect(10.0, 10.0, 30.0, 30.0),
            );
        let plan = plan_layer(&layer, &parent, &space).unwrap().unwrap();
        let [content, boxed] = [plan.shadows[0].unwrap(), plan.shadows[1].unwrap()];
        assert!(content.reuse_content && content.source == parent);
        assert!(!boxed.reuse_content && boxed.source.width < 64);
    }

    #[test]
    fn window_mapping_and_scissor() {
        let w = Window {
            x: 16,
            y: 8,
            width: 10,
            height: 10,
            step: 4,
        };
        assert_eq!(w.map([24.0, 12.0]), [2.0, 1.0]);
        assert_eq!(w.center(0, 0), [18.0, 10.0]);
        assert_eq!(w.map_scissor(17, 8, 4, 4), Some([0, 0, 2, 1]));
        assert_eq!(w.map_scissor(0, 0, 4, 4), None);
        assert_eq!(w.downsampled(2).size(), (5, 5));
        assert_eq!(w.downsampled(2).step, 8);
        let parent = Window::full(64, 64);
        assert_eq!(parent.rect_of(&w), [16.0, 8.0, 56.0, 48.0]);
    }
}
