//! clip stack → scissor / stencil / offscreen mask 계획 (XI.4.3).
//!
//! scene node의 clip stack(XI.1.2)을 backend 중립 [`ClipPlan`] 하나로 바꿉니다. 판정은 두 단계입니다.
//!
//! 1. **분석**([`analyze_clip`], device capability와 무관): clip마다 world → device 변환 결과를 봅니다.
//!    모서리 없는 사각형이 축을 보존하는 변환(이동·축척·반전·90° 회전)으로 놓이고 네 변이 device
//!    pixel 경계에 정확히 있으면 **scissor 가능**입니다. 그 밖(모서리 radius, 축이 아닌 회전, 소수 pixel
//!    가장자리, path)은 가장자리 coverage가 필요한 **복합 clip**입니다. 모든 clip의 device AABB
//!    교집합이 scissor bounds가 됩니다.
//! 2. **결정**([`decide_clip`], decision table):
//!
//! | 조건 | 결과 |
//! |---|---|
//! | 고려할 clip 없음 | [`ClipPlan::None`] |
//! | bounds 교집합이 비었거나 surface 밖 | [`ClipPlan::Culled`] |
//! | 복합 clip 없음(모두 scissor 가능) | [`ClipPlan::Scissor`]: 교집합 그대로(정확) |
//! | 복합 clip 있음, 4x MSAA, stencil bit > 0, 복합 clip 수 ≤ 2^bit − 1 | [`ClipPlan::Stencil`]: 복합 clip을 stencil에 차례로 증가 기록, scissor로 범위 제한 |
//! | 그 밖의 복합 clip | [`ClipPlan::Mask`]: 복합 clip coverage를 offscreen mask(R8)에 그려 곱함 |
//!
//! stencil은 sample 단위 coverage라 MSAA가 있어야 가장자리 AA가 생깁니다. MSAA가 없거나 stencil 깊이가
//! 모자라면 AA mask로 갑니다. 단순 축 정렬 사각형 clip은 항상 scissor입니다.
//!
//! `from_depth`는 clip stack에서 고려를 시작할 위치(바깥 → 안 index)입니다. offscreen layer 안 draw는
//! layer 경계 node 바깥 clip을 layer 합성 단계에서 적용하므로 경계 안쪽 clip만 봅니다(blur 등 효과
//! 뒤에 바깥 clip을 적용해야 하기 때문).

use std::collections::HashMap;
use std::sync::Arc;

use stellar_scene::bounds::Aabb;
use stellar_scene::stack::{ClipShape, ClipStackRef};
use stellar_scene::visibility::{clip_shape_world_bounds, Viewport};

use crate::backend::{DeviceCapabilities, ScissorRect};
use crate::pass_compiler::AddrBuild;
use crate::render_graph::SampleCount;

/// pixel 경계 판정 허용 오차(device pixel).
pub const PIXEL_EPSILON: f32 = 1.0 / 256.0;

/// clip 결정에 쓰는 device capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClipCapabilities {
    /// main target stencil bit 수.
    pub stencil_bits: u8,
    /// main target multisample 수.
    pub samples: SampleCount,
}

impl ClipCapabilities {
    /// device capability와 사용할 sample 수로 만듭니다(4x는 device가 지원할 때만).
    pub const fn from_device(caps: &DeviceCapabilities, samples: SampleCount) -> ClipCapabilities {
        let samples = match samples {
            SampleCount::X4 if caps.msaa_x4 => SampleCount::X4,
            _ => SampleCount::X1,
        };
        ClipCapabilities {
            stencil_bits: caps.stencil_bits,
            samples,
        }
    }

    /// stencil로 중첩할 수 있는 복합 clip 최대 수.
    pub const fn max_stencil_depth(&self) -> u32 {
        if self.stencil_bits >= 8 {
            255
        } else {
            (1u32 << self.stencil_bits) - 1
        }
    }
}

/// device pixel 좌표 사각형(좌상단 원점, 실수).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceRect {
    /// 왼쪽.
    pub min_x: f32,
    /// 위.
    pub min_y: f32,
    /// 오른쪽.
    pub max_x: f32,
    /// 아래.
    pub max_y: f32,
}

fn near_integer(value: f32) -> bool {
    (value - value.round()).abs() <= PIXEL_EPSILON
}

impl DeviceRect {
    fn from_aabb(aabb: &Aabb) -> DeviceRect {
        DeviceRect {
            min_x: aabb.min_x(),
            min_y: aabb.min_y(),
            max_x: aabb.max_x(),
            max_y: aabb.max_y(),
        }
    }

    /// 네 변이 pixel 경계에 있으면 참입니다.
    pub fn is_pixel_aligned(&self) -> bool {
        [self.min_x, self.min_y, self.max_x, self.max_y]
            .into_iter()
            .all(near_integer)
    }

    /// 교집합. 비면 `None`.
    pub fn intersection(&self, other: &DeviceRect) -> Option<DeviceRect> {
        let rect = DeviceRect {
            min_x: self.min_x.max(other.min_x),
            min_y: self.min_y.max(other.min_y),
            max_x: self.max_x.min(other.max_x),
            max_y: self.max_y.min(other.max_y),
        };
        (rect.min_x < rect.max_x && rect.min_y < rect.max_y).then_some(rect)
    }

    /// 바깥쪽으로 정수화하고 surface(`width × height`)로 자른 scissor. 비면 `None`.
    pub fn to_scissor(&self, width: u32, height: u32) -> Option<ScissorRect> {
        let clamp = |value: f32, limit: u32| -> u32 {
            if value <= 0.0 {
                0
            } else if value >= limit as f32 {
                limit
            } else {
                value as u32
            }
        };
        let snap_down = |value: f32| {
            if near_integer(value) {
                value.round()
            } else {
                value.floor()
            }
        };
        let snap_up = |value: f32| {
            if near_integer(value) {
                value.round()
            } else {
                value.ceil()
            }
        };
        let x0 = clamp(snap_down(self.min_x), width);
        let y0 = clamp(snap_down(self.min_y), height);
        let x1 = clamp(snap_up(self.max_x), width);
        let y1 = clamp(snap_up(self.max_y), height);
        (x0 < x1 && y0 < y1).then_some(ScissorRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    }
}

/// 사각형 `(x, y, w, h)`를 변환 성분 `(a, b, c, d, tx, ty)`로 옮긴 결과가 축 정렬 사각형이면 그
/// device 사각형. 축을 보존하지 않거나(임의 회전·기울임) 비유한이면 `None`.
pub fn axis_aligned_rect(
    (x, y, width, height): (f32, f32, f32, f32),
    (a, b, c, d, tx, ty): (f32, f32, f32, f32, f32, f32),
) -> Option<DeviceRect> {
    let preserves_axes = (b == 0.0 && c == 0.0) || (a == 0.0 && d == 0.0);
    if !preserves_axes {
        return None;
    }
    let corners = [
        (x, y),
        (x + width, y),
        (x, y + height),
        (x + width, y + height),
    ]
    .map(|(px, py)| (a * px + c * py + tx, b * px + d * py + ty));
    let min_x = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max);
    [min_x, min_y, max_x, max_y]
        .iter()
        .all(|v| v.is_finite())
        .then_some(DeviceRect {
            min_x,
            min_y,
            max_x,
            max_y,
        })
}

/// 복합 clip인 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComplexReason {
    /// 모서리 radius.
    RoundedCorners,
    /// 축을 보존하지 않는 변환(임의 회전·기울임).
    NotAxisAligned,
    /// 가장자리가 pixel 경계에 있지 않음.
    FractionalEdges,
    /// path clip.
    Path,
}

/// scissor로 표현할 수 없는 clip 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComplexClip {
    /// clip stack 바깥 → 안 index.
    pub index: usize,
    /// 이유.
    pub reason: ComplexReason,
}

/// capability와 무관한 clip 분석.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipAnalysis {
    /// 고려한 clip 수.
    pub considered: usize,
    /// 모든 clip device AABB의 교집합(보수적). 교집합이 비면 `None`.
    pub bounds: Option<DeviceRect>,
    /// 복합 clip(바깥 → 안).
    pub complex: Vec<ComplexClip>,
}

impl ClipAnalysis {
    /// clip이 없거나 모두 scissor로 정확히 표현되면 참입니다.
    pub fn is_scissor_exact(&self) -> bool {
        self.complex.is_empty() && (self.considered == 0 || self.bounds.is_some())
    }
}

/// clip 결정 결과.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClipPlan {
    /// clip 없음.
    None,
    /// 보이는 영역이 없어 그리지 않음.
    Culled,
    /// scissor만으로 정확함.
    Scissor(ScissorRect),
    /// scissor로 범위를 줄이고 복합 clip을 stencil에 차례로 증가 기록한 뒤 `reference`와 같은
    /// stencil에서만 그림.
    Stencil {
        /// 범위 제한 scissor.
        scissor: ScissorRect,
        /// stencil에 기록할 clip(바깥 → 안 index).
        clips: Arc<[usize]>,
        /// 그리기 stencil reference(= 복합 clip 수).
        reference: u8,
    },
    /// scissor로 범위를 줄이고 복합 clip coverage를 offscreen mask에 그려 곱함.
    Mask {
        /// 범위 제한 scissor.
        scissor: ScissorRect,
        /// mask에 그릴 clip(바깥 → 안 index).
        clips: Arc<[usize]>,
    },
}

impl ClipPlan {
    /// scissor rect(있으면).
    pub fn scissor(&self) -> Option<ScissorRect> {
        match self {
            ClipPlan::None | ClipPlan::Culled => None,
            ClipPlan::Scissor(scissor)
            | ClipPlan::Stencil { scissor, .. }
            | ClipPlan::Mask { scissor, .. } => Some(*scissor),
        }
    }
}

impl ClipAnalysis {
    /// 고려한 clip이 없는 분석.
    const fn empty() -> ClipAnalysis {
        ClipAnalysis {
            considered: 0,
            bounds: None,
            complex: Vec::new(),
        }
    }

    /// 바깥 → 안 index `index`의 clip 하나를 더 고려합니다. [`analyze_clip`]은 이 단계를 바깥부터 차례로
    /// 적용한 결과입니다.
    fn push_clip(&mut self, index: usize, clip: &ClipShape, viewport: &Viewport) {
        let view = &viewport.view();
        let device_bounds = clip_shape_world_bounds(clip)
            .and_then(|world| world.transformed(view))
            .map(|aabb| DeviceRect::from_aabb(&aabb));
        let reason = match clip {
            ClipShape::Rect {
                rect,
                radii,
                transform,
            } => {
                let device = transform.then(view).ok().and_then(|to_device| {
                    let components = to_device.components();
                    axis_aligned_rect(
                        (
                            rect.origin().x(),
                            rect.origin().y(),
                            rect.size().width(),
                            rect.size().height(),
                        ),
                        components,
                    )
                });
                match device {
                    _ if !radii.is_zero() => Some(ComplexReason::RoundedCorners),
                    None => Some(ComplexReason::NotAxisAligned),
                    Some(rect) if !rect.is_pixel_aligned() => Some(ComplexReason::FractionalEdges),
                    Some(_) => None,
                }
            }
            ClipShape::Path { .. } => Some(ComplexReason::Path),
        };
        if let Some(reason) = reason {
            self.complex.push(ComplexClip { index, reason });
        }
        // 첫 clip은 그 bounds, 이후는 교집합. 한 번 비거나(교집합 없음) 비유한이면 끝까지 비어 있습니다.
        self.bounds = match (self.considered, self.bounds, device_bounds) {
            (0, _, bounds) => bounds,
            (_, Some(acc), Some(rect)) => acc.intersection(&rect),
            _ => None,
        };
        self.considered += 1;
    }
}

/// clip stack의 `from_depth`(바깥 → 안 index)부터를 분석합니다.
pub fn analyze_clip(stack: &ClipStackRef, from_depth: usize, viewport: &Viewport) -> ClipAnalysis {
    let mut analysis = ClipAnalysis::empty();
    for (index, clip) in stack
        .to_outer_first()
        .into_iter()
        .enumerate()
        .skip(from_depth)
    {
        analysis.push_clip(index, clip, viewport);
    }
    analysis
}

/// prefix memo를 쓰는 clip stack 최대 깊이. 더 깊으면 [`analyze_clip`]으로 바로 분석합니다(memo 항목마다
/// 복합 clip 목록을 복제하므로 깊이 제곱 비용을 막음).
const MEMO_MAX_DEPTH: usize = 32;

/// 한 viewport에서 clip stack 분석을 prefix 단위로 다시 쓰는 memo.
///
/// clip stack은 구조 공유 목록이라 형제 node는 같은 stack을, 자식은 부모 stack에 clip 하나를 더한 stack을
/// 가집니다. `(stack 저장소 주소, from_depth)`별 분석을 남기면 node마다 가장 안쪽 clip 하나만 새로 분석합니다.
/// 결과는 [`analyze_clip`]과 같습니다. 주소는 stack이 살아 있는 동안만 식별자이므로 memo는 snapshot 하나를
/// compile하는 동안만 씁니다.
#[derive(Debug, Default)]
pub(crate) struct ClipAnalysisMemo {
    analyses: HashMap<(usize, usize), ClipAnalysis, AddrBuild>,
}

impl ClipAnalysisMemo {
    /// [`analyze_clip`]과 같은 분석.
    pub(crate) fn analyze(
        &mut self,
        stack: &ClipStackRef,
        from_depth: usize,
        viewport: &Viewport,
    ) -> ClipAnalysis {
        if stack.depth() > MEMO_MAX_DEPTH {
            return analyze_clip(stack, from_depth, viewport);
        }
        // 기록이 있는 prefix(또는 고려 시작 depth)까지 올라갑니다.
        let mut chain: Vec<&ClipStackRef> = Vec::new();
        let mut current = stack;
        let mut analysis = loop {
            if current.depth() <= from_depth {
                break ClipAnalysis::empty();
            }
            if let Some(hit) = self.analyses.get(&(current.storage_addr(), from_depth)) {
                break hit.clone();
            }
            chain.push(current);
            match current.outer() {
                Some(outer) => current = outer,
                None => break ClipAnalysis::empty(),
            }
        };
        for level in chain.into_iter().rev() {
            if let Some(clip) = level.innermost() {
                analysis.push_clip(level.depth() - 1, clip, viewport);
            }
            self.analyses
                .insert((level.storage_addr(), from_depth), analysis.clone());
        }
        analysis
    }
}

/// 분석 결과를 decision table(모듈 문서)로 결정합니다.
pub fn decide_clip(
    analysis: &ClipAnalysis,
    surface_width: u32,
    surface_height: u32,
    caps: ClipCapabilities,
) -> ClipPlan {
    if analysis.considered == 0 {
        return ClipPlan::None;
    }
    let Some(scissor) = analysis
        .bounds
        .and_then(|bounds| bounds.to_scissor(surface_width, surface_height))
    else {
        return ClipPlan::Culled;
    };
    if analysis.complex.is_empty() {
        return ClipPlan::Scissor(scissor);
    }
    let clips: Arc<[usize]> = analysis.complex.iter().map(|clip| clip.index).collect();
    let stencil_ok = caps.samples == SampleCount::X4
        && caps.stencil_bits > 0
        && (clips.len() as u64) <= u64::from(caps.max_stencil_depth());
    if stencil_ok {
        ClipPlan::Stencil {
            reference: clips.len() as u8,
            scissor,
            clips,
        }
    } else {
        ClipPlan::Mask { scissor, clips }
    }
}

/// surface 크기(viewport size, 반올림).
pub fn surface_extent(viewport: &Viewport) -> (u32, u32) {
    let size = viewport.size();
    let to_pixels = |value: f32| {
        if value <= 0.0 {
            0
        } else if value >= u32::MAX as f32 {
            u32::MAX
        } else {
            value.round() as u32
        }
    };
    (to_pixels(size.width()), to_pixels(size.height()))
}

/// clip stack을 분석·결정합니다.
pub fn plan_clip(
    stack: &ClipStackRef,
    from_depth: usize,
    viewport: &Viewport,
    caps: ClipCapabilities,
) -> ClipPlan {
    let (width, height) = surface_extent(viewport);
    decide_clip(
        &analyze_clip(stack, from_depth, viewport),
        width,
        height,
        caps,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass_compiler::tests::viewport_with;
    use crate::test_scene::{
        nodes, opaque, path_clip, rect, rect_clip, snapshot, ClipFx, IDENTITY,
    };

    const MSAA: ClipCapabilities = ClipCapabilities {
        stencil_bits: 8,
        samples: SampleCount::X4,
    };
    const NO_MSAA: ClipCapabilities = ClipCapabilities {
        stencil_bits: 8,
        samples: SampleCount::X1,
    };

    /// clip을 가진 node 하나를 만들고 그 clip stack을 계획합니다.
    fn plan(clips: Vec<ClipFx>, view: [f32; 6], caps: ClipCapabilities) -> ClipPlan {
        let mut node = rect(1, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0));
        node.clips = clips;
        let scene = snapshot(&[node]);
        let viewport = viewport_with(100.0, 100.0, view);
        plan_clip(nodes(&scene)[0].clip(), 0, &viewport, caps)
    }

    fn scissor(x: u32, y: u32, width: u32, height: u32) -> ScissorRect {
        ScissorRect {
            x,
            y,
            width,
            height,
        }
    }

    fn rotated(mut clip: ClipFx, transform: [f32; 6]) -> ClipFx {
        clip.transform = transform;
        clip
    }

    fn rounded(mut clip: ClipFx) -> ClipFx {
        clip.radii = [2.0; 4];
        clip
    }

    #[test]
    fn simple_rect_clip_selects_scissor() {
        // 검증 기준(XI.4.3): simple rect clip은 scissor 선택.
        let simple = plan(vec![rect_clip([10.0, 20.0, 30.0, 40.0])], IDENTITY, MSAA);
        assert_eq!(simple, ClipPlan::Scissor(scissor(10, 20, 30, 40)));
        // capability와 무관합니다.
        assert_eq!(
            plan(vec![rect_clip([10.0, 20.0, 30.0, 40.0])], IDENTITY, NO_MSAA),
            simple
        );
        // 중첩 사각형은 교집합 scissor.
        assert_eq!(
            plan(
                vec![
                    rect_clip([0.0, 0.0, 50.0, 50.0]),
                    rect_clip([25.0, 10.0, 50.0, 20.0])
                ],
                IDENTITY,
                MSAA
            ),
            ClipPlan::Scissor(scissor(25, 10, 25, 20))
        );
        // zoom·pan·90° 회전도 축을 보존하면 scissor입니다.
        assert_eq!(
            plan(
                vec![rect_clip([1.0, 2.0, 3.0, 4.0])],
                [2.0, 0.0, 0.0, 2.0, 10.0, 0.0],
                MSAA
            ),
            ClipPlan::Scissor(scissor(12, 4, 6, 8))
        );
        assert_eq!(
            plan(
                vec![rotated(
                    rect_clip([0.0, 0.0, 10.0, 20.0]),
                    [0.0, 1.0, -1.0, 0.0, 50.0, 0.0]
                )],
                IDENTITY,
                MSAA
            ),
            ClipPlan::Scissor(scissor(30, 0, 20, 10))
        );
        // clip 없음.
        assert_eq!(plan(Vec::new(), IDENTITY, MSAA), ClipPlan::None);
    }

    #[test]
    fn empty_or_offscreen_clips_cull() {
        assert_eq!(
            plan(
                vec![
                    rect_clip([0.0, 0.0, 10.0, 10.0]),
                    rect_clip([20.0, 20.0, 10.0, 10.0])
                ],
                IDENTITY,
                MSAA
            ),
            ClipPlan::Culled
        );
        assert_eq!(
            plan(vec![rect_clip([200.0, 0.0, 10.0, 10.0])], IDENTITY, MSAA),
            ClipPlan::Culled
        );
        // surface 밖으로 나간 부분은 잘립니다.
        assert_eq!(
            plan(vec![rect_clip([90.0, -5.0, 20.0, 10.0])], IDENTITY, MSAA),
            ClipPlan::Scissor(scissor(90, 0, 10, 5))
        );
    }

    #[test]
    fn complex_clips_use_stencil_with_msaa_and_mask_otherwise() {
        let cases = [
            (
                rounded(rect_clip([10.0, 10.0, 20.0, 20.0])),
                ComplexReason::RoundedCorners,
            ),
            (
                rect_clip([10.5, 10.0, 20.0, 20.0]),
                ComplexReason::FractionalEdges,
            ),
            (
                rotated(
                    rect_clip([0.0, 0.0, 10.0, 10.0]),
                    [0.866, 0.5, -0.5, 0.866, 50.0, 50.0],
                ),
                ComplexReason::NotAxisAligned,
            ),
            (
                path_clip(&[[10.0, 10.0], [30.0, 10.0], [20.0, 30.0]]),
                ComplexReason::Path,
            ),
        ];
        for (clip, reason) in cases {
            let mut node = rect(1, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0));
            node.clips = vec![clip.clone()];
            let scene = snapshot(&[node]);
            let viewport = viewport_with(100.0, 100.0, IDENTITY);
            let analysis = analyze_clip(nodes(&scene)[0].clip(), 0, &viewport);
            assert_eq!(analysis.complex, vec![ComplexClip { index: 0, reason }]);
            assert!(!analysis.is_scissor_exact());

            let stencil = plan(vec![clip.clone()], IDENTITY, MSAA);
            let ClipPlan::Stencil {
                scissor: bound,
                clips,
                reference,
            } = stencil
            else {
                panic!("{reason:?}: MSAA에서는 stencil이어야 합니다: {stencil:?}");
            };
            assert_eq!((&*clips, reference), (&[0][..], 1));
            // scissor는 clip을 보수적으로 감쌉니다.
            assert!(bound.width > 0 && bound.height > 0);
            assert_eq!(
                plan(vec![clip], IDENTITY, NO_MSAA),
                ClipPlan::Mask {
                    scissor: bound,
                    clips: vec![0].into()
                }
            );
        }
    }

    #[test]
    fn scissor_and_complex_clips_combine() {
        // 바깥 정렬 사각형 + 안쪽 둥근 사각형: 둥근 것만 stencil, 범위는 교집합.
        let combined = plan(
            vec![
                rect_clip([0.0, 0.0, 20.0, 100.0]),
                rounded(rect_clip([10.0, 10.0, 30.0, 30.0])),
            ],
            IDENTITY,
            MSAA,
        );
        assert_eq!(
            combined,
            ClipPlan::Stencil {
                scissor: scissor(10, 10, 10, 30),
                clips: vec![1].into(),
                reference: 1
            }
        );
        // 소수 가장자리 bound는 바깥쪽으로 정수화합니다.
        assert_eq!(
            plan(vec![rect_clip([10.25, 10.0, 5.5, 5.0])], IDENTITY, MSAA).scissor(),
            Some(scissor(10, 10, 6, 5))
        );
        // stencil 깊이가 모자라면 mask.
        let shallow = ClipCapabilities {
            stencil_bits: 1,
            samples: SampleCount::X4,
        };
        assert_eq!(shallow.max_stencil_depth(), 1);
        let two_rounded = vec![
            rounded(rect_clip([0.0, 0.0, 50.0, 50.0])),
            rounded(rect_clip([10.0, 10.0, 50.0, 50.0])),
        ];
        assert!(matches!(
            plan(two_rounded.clone(), IDENTITY, shallow),
            ClipPlan::Mask { ref clips, .. } if **clips == [0, 1]
        ));
        assert!(matches!(
            plan(two_rounded, IDENTITY, MSAA),
            ClipPlan::Stencil { reference: 2, .. }
        ));
        let no_stencil = ClipCapabilities {
            stencil_bits: 0,
            samples: SampleCount::X4,
        };
        assert!(matches!(
            plan(
                vec![rounded(rect_clip([0.0, 0.0, 9.0, 9.0]))],
                IDENTITY,
                no_stencil
            ),
            ClipPlan::Mask { .. }
        ));
    }

    #[test]
    fn from_depth_skips_outer_clips() {
        let mut node = rect(1, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0));
        node.clips = vec![
            rounded(rect_clip([0.0, 0.0, 50.0, 50.0])),
            rect_clip([5.0, 5.0, 10.0, 10.0]),
        ];
        let scene = snapshot(&[node]);
        let viewport = viewport_with(100.0, 100.0, IDENTITY);
        let stack = nodes(&scene)[0].clip().clone();
        assert!(matches!(
            plan_clip(&stack, 0, &viewport, MSAA),
            ClipPlan::Stencil { .. }
        ));
        // layer 안쪽(바깥 clip 1개 건너뜀)에서는 정렬 사각형만 남아 scissor입니다.
        assert_eq!(
            plan_clip(&stack, 1, &viewport, MSAA),
            ClipPlan::Scissor(scissor(5, 5, 10, 10))
        );
        assert_eq!(plan_clip(&stack, 2, &viewport, MSAA), ClipPlan::None);
        assert_eq!(plan_clip(&stack, 9, &viewport, MSAA), ClipPlan::None);
    }

    #[test]
    fn capabilities_follow_device() {
        let mut device = crate::backend::DeviceCapabilities {
            backend: crate::backend::BackendKind::Mock,
            max_texture_dimension: 4096,
            display_p3_surface: false,
            timestamp_query: false,
            msaa_x4: false,
            stencil_bits: 8,
        };
        assert_eq!(
            ClipCapabilities::from_device(&device, SampleCount::X4).samples,
            SampleCount::X1
        );
        device.msaa_x4 = true;
        assert_eq!(
            ClipCapabilities::from_device(&device, SampleCount::X4),
            MSAA
        );
        assert_eq!(MSAA.max_stencil_depth(), 255);
    }

    #[test]
    fn compiled_draws_carry_clip_plans() {
        use crate::pass_compiler::tests::{id_of, input};
        use crate::pass_compiler::{compile_frame, DrawSource, EffectBoundaries};
        use crate::test_scene::group;

        let scene = snapshot(&[
            // 단순 사각형 clip → scissor.
            rect(1, [0.0, 0.0, 50.0, 50.0], opaque(1.0, 0.0, 0.0))
                .with_clip(rect_clip([0.0, 0.0, 20.0, 20.0])),
            // 서로 겹치지 않는 clip → 그리지 않음.
            rect(2, [0.0, 0.0, 50.0, 50.0], opaque(0.0, 1.0, 0.0))
                .with_clip(rect_clip([0.0, 0.0, 5.0, 5.0]))
                .with_clip(rect_clip([10.0, 10.0, 5.0, 5.0])),
            // 바깥 둥근 clip에 잘린 effect group과, 그 안쪽 정렬 clip을 가진 자식.
            group(
                3,
                [0.0, 0.0, 60.0, 60.0],
                vec![rect(4, [0.0, 0.0, 30.0, 30.0], opaque(0.0, 0.0, 1.0))
                    .with_clip(rounded(rect_clip([0.0, 0.0, 60.0, 60.0])))
                    .with_clip(rect_clip([5.0, 5.0, 10.0, 10.0]))],
            )
            .with_clip(rounded(rect_clip([0.0, 0.0, 60.0, 60.0])))
            .with_opacity(0.5),
            // clip이 비어 합성이 culled인 effect group은 layer를 만들지 않습니다.
            group(
                5,
                [0.0, 0.0, 10.0, 10.0],
                vec![rect(6, [0.0, 0.0, 5.0, 5.0], opaque(1.0, 1.0, 1.0))
                    .with_clip(rect_clip([500.0, 0.0, 5.0, 5.0]))],
            )
            .with_clip(rect_clip([500.0, 0.0, 5.0, 5.0]))
            .with_opacity(0.5),
        ]);
        let viewport = viewport_with(100.0, 100.0, IDENTITY);
        let plan =
            compile_frame(&input(&scene, &viewport, EffectBoundaries::Conservative)).unwrap();
        let draws: Vec<(u8, bool, ClipPlan)> = plan
            .passes_in_order()
            .flat_map(|pass| {
                pass.draws.iter().map(|draw| {
                    (
                        id_of(draw.source.node()),
                        matches!(draw.source, DrawSource::Layer { .. }),
                        draw.clip.clone(),
                    )
                })
            })
            .collect();
        assert_eq!(
            draws,
            vec![
                // layer(3) 안 자식 4: 바깥 둥근 clip(depth 0)은 빼고 안쪽 정렬 clip만 → scissor.
                (4, false, ClipPlan::Scissor(scissor(5, 5, 10, 10))),
                // opaque: node 1.
                (1, false, ClipPlan::Scissor(scissor(0, 0, 20, 20))),
                // transparent: layer 3 합성에 바깥 둥근 clip(stencil).
                (
                    3,
                    true,
                    ClipPlan::Stencil {
                        scissor: scissor(0, 0, 60, 60),
                        clips: vec![0].into(),
                        reference: 1
                    }
                ),
            ]
        );
        assert_eq!(plan.layers.len(), 1);
        assert_eq!(id_of(&plan.layers[0].node), 3);
    }

    #[test]
    fn prefix_memo_matches_direct_analysis_on_shared_stacks() {
        // 구조 공유 stack(형제는 같은 stack, 자식은 부모 stack + clip 하나)에서 memo 분석이 직접 분석과 같습니다.
        let shapes: Vec<ClipShape> = [
            rect_clip([0.0, 0.0, 80.0, 80.0]),
            rounded(rect_clip([4.0, 4.0, 40.0, 40.0])),
            rect_clip([10.25, 8.0, 20.0, 20.0]),
            path_clip(&[[0.0, 0.0], [30.0, 0.0], [0.0, 30.0]]),
            rect_clip([200.0, 200.0, 5.0, 5.0]),
        ]
        .into_iter()
        .map(|clip| {
            let mut node = rect(1, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0));
            node.clips = vec![clip];
            let scene = snapshot(&[node]);
            nodes(&scene)[0].clip().innermost().unwrap().clone()
        })
        .collect();
        let mut stacks = vec![ClipStackRef::none()];
        for (index, shape) in shapes.iter().enumerate() {
            // 같은 바깥 stack에서 두 갈래(마지막 stack과 그 바깥 stack)로 자랍니다.
            let last = stacks.last().unwrap().clone();
            stacks.push(last.push(shape.clone()));
            if index > 0 {
                let outer = last.outer().unwrap().clone();
                stacks.push(outer.push(shape.clone()));
            }
        }
        // 깊이 상한을 넘는 stack은 직접 분석 경로를 탑니다.
        let mut deep = stacks.last().unwrap().clone();
        for index in 0..(MEMO_MAX_DEPTH + 3) {
            deep = deep.push(shapes[index % shapes.len()].clone());
        }
        stacks.push(deep);
        for view in [
            IDENTITY,
            [2.0, 0.0, 0.0, 2.0, 0.5, 0.0],
            [0.0, 1.0, -1.0, 0.0, 90.0, 0.0],
        ] {
            let viewport = viewport_with(100.0, 100.0, view);
            let mut memo = ClipAnalysisMemo::default();
            // 깊은 stack부터 물어도, 얕은 stack부터 물어도 같습니다.
            for stack in stacks.iter().rev().chain(stacks.iter()) {
                for from in 0..=stack.depth() + 1 {
                    assert_eq!(
                        memo.analyze(stack, from, &viewport),
                        analyze_clip(stack, from, &viewport),
                        "depth {} from {from}",
                        stack.depth()
                    );
                }
            }
        }
    }
}
