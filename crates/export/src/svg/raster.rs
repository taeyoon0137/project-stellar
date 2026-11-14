//! 부분 래스터화 경계 planner (XX.4.1).
//!
//! 아키텍처 §34: SVG로 정확히 표현할 수 없는 **최소** 하위 트리만 래스터화하고 나머지는 벡터로 두며,
//! 래스터 경계는 Blend·Filter·투명도 의존 범위를 포함합니다. [`plan_raster`]는 [`ExportCapture`]만 읽어
//! 래스터화할 영역([`RasterRegion`])과 그 이유([`RasterCause`])를 paint 순서로 계산합니다. 영역을
//! offscreen image로 렌더해 SVG `<image>`로 넣는 일은 XX.4.2, export report 기록은 XX.4.3입니다.
//!
//! ## 래스터 기능과 영향 범위
//!
//! 대상은 mapping table([`crate::svg::mapping`])에서 지원 상태가
//! [`SvgSupport::Rasterize`](crate::svg::mapping::SvgSupport::Rasterize)인 기능뿐입니다
//! ([`raster_scope`]가 기능마다 범위를 정하고, inventory test가 두 표의 일치를 고정합니다).
//!
//! - [`RasterScope::Content`] — drop shadow, image 색공간 override: node **자기 내용**만 그 node 위치에서
//!   래스터화하고 자식은 vector로 둡니다. scene `PaintStack`에서 shadow는 node 내용 layer 아래에만
//!   그리고, 색공간 override는 image 픽셀에만 걸립니다.
//! - [`RasterScope::Subtree`] — layer blur: node와 subtree를 합성한 결과 전체에 걸리므로 subtree를
//!   래스터화합니다.
//! - [`RasterScope::Backdrop`] — Normal이 아닌 blend: subtree 합성 결과를 **먼저 그린 내용(backdrop)**과
//!   섞으므로 결과에 영향을 주는 backdrop까지 함께 래스터화합니다(아래).
//!
//! 영역 image는 영역 안 node의 합성 효과(opacity·blend·blur)를 모두 포함하고, 영역 밖 조상의 효과는
//! 포함하지 않습니다. 단 [`RasterRegion::Content`]는 node 내용만이므로 그 node의 subtree 합성
//! 효과(group opacity)는 vector 그룹에 남습니다.
//!
//! ## blend backdrop 의존 범위
//!
//! blend node `N`의 결과는 가장 가까운 isolation group `G`(opacity < 1·Normal 외 blend·layer blur가
//! 있는 조상, 없으면 내보내기 canvas) 안에서 `N`보다 먼저 그린 내용 중 `N` footprint와 겹치는 부분에만
//! 의존합니다. 이는 투명한 source가 backdrop을 바꾸지 않는다는 합성 수식의 성질(모든 separable blend
//! mode가 만족, 수식 고정은 XI.10.2)을 전제로 합니다.
//!
//! `N`에서 `G`까지의 경로를 `p0 = N, p1, …, pm = G`라 하면 단계 `i`의 앞선 내용은 `p_i` 자기 내용과
//! `p_{i-1}`보다 앞선 `p_i`의 자식 subtree입니다(canvas 단계는 배경과 앞선 최상위 node). `N`
//! footprint와 겹치는 앞선 내용이 있는 **가장 높은 단계**를 `J`라 하면 영역은 다음과 같습니다.
//!
//! - 겹침 없음: `N` subtree 하나.
//! - 단계 `J`에서 `p_J` 자기 내용이 겹침: `p_J` subtree 하나(그 내용이 backdrop으로 들어가야 함).
//!   canvas 단계에서 배경이 겹치면 canvas 전체([`RasterRegion::Canvas`]).
//! - 단계 `J`에서 앞선 자식만 겹침: `p_J`의 자식 중 겹치는 첫 자식부터 `p_{J-1}`까지의 연속 범위.
//!
//! 영역은 paint 순서상 연속이라 그리는 순서를 바꾸지 않고, 영역 밖 앞선 내용은 `N` footprint와 겹치지
//! 않습니다. 그래서 영역을 투명 바탕에서 합성한 image를 영역 첫 위치에 Normal로 그리면 원래 합성과
//! 같고(Normal over의 결합 법칙), 겹치지 않는 형제·조상 내용은 vector로 남습니다.
//!
//! ## footprint
//!
//! node 자기 내용 footprint는 scene world render bounds(stroke·shadow·자기 blur 포함, 보수적)를 내보내기
//! pixel grid([`PixelGrid`])에서 바깥으로 정수화한 상자입니다. 아무것도 그리지 않는 내용(칠하기 layer와
//! shadow가 없는 사각형·path)은 footprint가 없습니다. subtree footprint는 자손 footprint의 합집합이고,
//! layer blur node는 자식 합집합을 blur 반경(world 변환 최대 배율 반영)만큼 넓힙니다.
//!
//! clip으로는 footprint를 줄이지 않습니다. 효과와 clip의 적용 순서는 XI.10.4가 고정하므로 지금은 넓게
//! 잡는 쪽(래스터가 커질 수는 있어도 결과는 틀리지 않음)을 택했습니다. 겹침은 canvas 안에서 양의 넓이를
//! 가진 pixel 교집합입니다. pixel 경계에 맞닿기만 하면 겹치지 않고, 한 pixel을 나눠 덮으면 겹칩니다.
//!
//! ## 병합
//!
//! 영역은 pre-order 구간으로 비교합니다. 다른 영역 안에 든 영역은 흡수하고, 같은 부모 아래 서로 겹치는
//! 형제 범위는 하나로 합칩니다. canvas 영역이 하나라도 있으면 전체가 canvas 영역 하나입니다. 결과 경계는
//! 서로 겹치지 않고 paint 순서로 정렬됩니다. 모든 순회는 명시적 stack·배열로 하며 재귀하지 않습니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_scene::bounds::Aabb;
use stellar_scene::node::{SceneContent, SceneNode, Visibility, ZOrder};
use stellar_scene::stack::{Opacity, SceneBlendMode};

use crate::capture::ExportCapture;
use crate::request::{ExportBackground, ExportScale};
use crate::svg::mapping::{node_features, SvgFeature};

/// pixel 경계 판정 오차(device pixel). 정수에서 이 거리 안의 경계는 그 정수로 붙입니다.
pub const PIXEL_EPSILON: f64 = 1.0 / 256.0;

/// device pixel 좌표 상한. 이 밖은 잘라 정수 연산이 넘치지 않게 합니다.
const COORD_LIMIT: i64 = 1 << 40;

/// 래스터 기능의 영향 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RasterScope {
    /// node 자기 내용만.
    Content,
    /// node subtree 합성 결과.
    Subtree,
    /// subtree와 그 결과가 섞이는 backdrop.
    Backdrop,
}

/// 기능의 래스터 영향 범위. 래스터 대상이 아닌 기능은 `None`입니다.
///
/// [`SvgFeature`]를 wildcard 없이 match하므로 기능이 추가되면 범위를 정하기 전까지 컴파일되지 않습니다.
pub const fn raster_scope(feature: SvgFeature) -> Option<RasterScope> {
    match feature {
        SvgFeature::DropShadow | SvgFeature::ImageColorSpaceOverride => Some(RasterScope::Content),
        SvgFeature::LayerBlur => Some(RasterScope::Subtree),
        SvgFeature::BlendMode => Some(RasterScope::Backdrop),
        SvgFeature::RectContent
        | SvgFeature::RoundedRectContent
        | SvgFeature::PathContent
        | SvgFeature::EvenOddFill
        | SvgFeature::TextContent
        | SvgFeature::ImageContent
        | SvgFeature::ImageFitFill
        | SvgFeature::ImageFitContain
        | SvgFeature::ImageFitCover
        | SvgFeature::ImageFitNone
        | SvgFeature::ImageCrop
        | SvgFeature::ImageNearestSampling
        | SvgFeature::SolidFill
        | SvgFeature::SrgbColor
        | SvgFeature::WideGamutColor
        | SvgFeature::TranslucentColor
        | SvgFeature::CenterStroke
        | SvgFeature::InsideStroke
        | SvgFeature::OutsideStroke
        | SvgFeature::RoundCap
        | SvgFeature::SquareCap
        | SvgFeature::GroupOpacity
        | SvgFeature::RectClip
        | SvgFeature::RoundedRectClip
        | SvgFeature::PathClip
        | SvgFeature::NestedClip
        | SvgFeature::WorldTransform
        | SvgFeature::HiddenNode
        | SvgFeature::LinearGradient
        | SvgFeature::RadialGradient => None,
    }
}

/// 내보내기 device pixel grid의 반열린 상자 `[x0, x1) × [y0, y1)`. 항상 넓이가 양수입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelRect {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl PixelRect {
    /// 네 경계로 만듭니다. 넓이가 0 이하면 `None`.
    pub const fn new(x0: i64, y0: i64, x1: i64, y1: i64) -> Option<PixelRect> {
        if x0 < x1 && y0 < y1 {
            Some(PixelRect { x0, y0, x1, y1 })
        } else {
            None
        }
    }

    /// 왼쪽 경계(포함).
    pub const fn x0(&self) -> i64 {
        self.x0
    }

    /// 위쪽 경계(포함).
    pub const fn y0(&self) -> i64 {
        self.y0
    }

    /// 오른쪽 경계(제외).
    pub const fn x1(&self) -> i64 {
        self.x1
    }

    /// 아래쪽 경계(제외).
    pub const fn y1(&self) -> i64 {
        self.y1
    }

    /// 너비(pixel).
    pub const fn width(&self) -> i64 {
        self.x1 - self.x0
    }

    /// 높이(pixel).
    pub const fn height(&self) -> i64 {
        self.y1 - self.y0
    }

    /// 교집합. 넓이가 0이면 `None`.
    pub fn intersection(&self, other: &PixelRect) -> Option<PixelRect> {
        PixelRect::new(
            self.x0.max(other.x0),
            self.y0.max(other.y0),
            self.x1.min(other.x1),
            self.y1.min(other.y1),
        )
    }

    /// 두 상자를 덮는 가장 작은 상자.
    pub fn union(&self, other: &PixelRect) -> PixelRect {
        PixelRect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    fn expanded(&self, amount: i64) -> PixelRect {
        let clamp = |v: i64| v.clamp(-COORD_LIMIT, COORD_LIMIT);
        PixelRect {
            x0: clamp(self.x0.saturating_sub(amount)),
            y0: clamp(self.y0.saturating_sub(amount)),
            x1: clamp(self.x1.saturating_add(amount)),
            y1: clamp(self.y1.saturating_add(amount)),
        }
    }

    /// 좌표 상한 전체(world bounds 계산이 넘친 node의 footprint).
    const UNBOUNDED: PixelRect = PixelRect {
        x0: -COORD_LIMIT,
        y0: -COORD_LIMIT,
        x1: COORD_LIMIT,
        y1: COORD_LIMIT,
    };
}

fn snap(value: f64) -> Option<f64> {
    let nearest = value.round();
    ((value - nearest).abs() <= PIXEL_EPSILON).then_some(nearest)
}

fn to_coord(value: f64) -> i64 {
    // f64 → i64 변환은 saturate하고, 상한으로 한 번 더 자릅니다.
    (value as i64).clamp(-COORD_LIMIT, COORD_LIMIT)
}

fn pixel_floor(value: f64) -> i64 {
    to_coord(snap(value).unwrap_or_else(|| value.floor()))
}

fn pixel_ceil(value: f64) -> i64 {
    to_coord(snap(value).unwrap_or_else(|| value.ceil()))
}

/// 내보내기 device pixel grid: world 점 `p`는 `(p - 내보내기 영역 원점) × scale` pixel입니다.
///
/// canvas는 `[0, ⌈너비 × scale⌉) × [0, ⌈높이 × scale⌉)`(각 변 최소 1 pixel)입니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelGrid {
    origin_x: f64,
    origin_y: f64,
    scale: f64,
    canvas: PixelRect,
}

impl PixelGrid {
    /// 내보내기 영역(world)과 배율로 grid를 만듭니다.
    pub fn new(bounds: Aabb, scale: ExportScale) -> PixelGrid {
        let scale = f64::from(scale.get());
        let size = |min: f32, max: f32| {
            let pixels = (f64::from(max) - f64::from(min)) * scale;
            pixel_ceil(pixels).max(1)
        };
        PixelGrid {
            origin_x: f64::from(bounds.min_x()),
            origin_y: f64::from(bounds.min_y()),
            scale,
            canvas: PixelRect {
                x0: 0,
                y0: 0,
                x1: size(bounds.min_x(), bounds.max_x()),
                y1: size(bounds.min_y(), bounds.max_y()),
            },
        }
    }

    /// capture의 내보내기 영역과 요청 배율로 만든 grid.
    pub fn of_capture(capture: &ExportCapture) -> PixelGrid {
        PixelGrid::new(capture.bounds(), capture.request().scale())
    }

    /// canvas 상자.
    pub const fn canvas(&self) -> PixelRect {
        self.canvas
    }

    /// world 1 단위가 몇 pixel인지.
    pub const fn scale(&self) -> f64 {
        self.scale
    }

    /// world 상자를 바깥으로 정수화한 pixel 상자(canvas로 자르지 않음). 넓이가 0이면 `None`.
    pub fn device_box(&self, world: &Aabb) -> Option<PixelRect> {
        let x = |v: f32| (f64::from(v) - self.origin_x) * self.scale;
        let y = |v: f32| (f64::from(v) - self.origin_y) * self.scale;
        PixelRect::new(
            pixel_floor(x(world.min_x())),
            pixel_floor(y(world.min_y())),
            pixel_ceil(x(world.max_x())),
            pixel_ceil(y(world.max_y())),
        )
    }

    fn in_canvas(&self, area: Option<PixelRect>) -> Option<PixelRect> {
        area.and_then(|a| a.intersection(&self.canvas))
    }

    /// 두 footprint가 canvas 안에서 양의 넓이로 겹치면 참입니다.
    fn overlaps(&self, a: Option<PixelRect>, b: &PixelRect) -> bool {
        a.and_then(|a| a.intersection(b))
            .and_then(|both| both.intersection(&self.canvas))
            .is_some()
    }
}

/// 계획에 나오는 scene node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RasterNode {
    /// paint 순서 key(내보내기 안에서 유일).
    pub z: ZOrder,
    /// node를 만든 document node.
    pub source: StableId,
}

impl RasterNode {
    fn of(node: &SceneNode) -> RasterNode {
        RasterNode {
            z: node.z(),
            source: node.source(),
        }
    }
}

/// 래스터화할 영역.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RasterRegion {
    /// node 자기 내용만(자식과 그 node의 subtree 합성 효과는 vector).
    Content {
        /// node.
        node: RasterNode,
    },
    /// 같은 부모 아래 연속한 자식 subtree `first..=last`(paint 순서).
    Subtrees {
        /// 부모. `None`이면 내보내기 최상위 node 목록입니다.
        parent: Option<RasterNode>,
        /// 첫 subtree root.
        first: RasterNode,
        /// 마지막 subtree root(`first`와 같으면 subtree 하나).
        last: RasterNode,
    },
    /// 배경을 포함한 내보내기 전체.
    Canvas,
}

/// blend backdrop에서 영역을 넓힌 내용.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackdropItem {
    /// 앞서 그린 node 내용(또는 blur로 퍼진 subtree root).
    Node(RasterNode),
    /// 내보내기 배경.
    Background,
}

/// blend의 외부 backdrop 의존.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlendDependency {
    /// blend가 합성되는 isolation group(가장 가까운 합성 효과 조상). `None`이면 내보내기 canvas.
    pub isolation: Option<RasterNode>,
    /// footprint가 겹치는 앞선 내용 중 가장 높은 단계의 첫 내용(paint 순서). `None`이면 겹치는 앞선
    /// 내용이 없어 blend node subtree만 래스터화합니다.
    pub widest: Option<BackdropItem>,
}

/// 래스터화 이유 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RasterCause {
    /// 래스터 기능을 쓰는 node.
    pub node: RasterNode,
    /// 기능(지원 상태 [`SvgSupport::Rasterize`](crate::svg::mapping::SvgSupport::Rasterize)).
    pub feature: SvgFeature,
    /// blend면 backdrop 의존(그 외 `None`).
    pub blend: Option<BlendDependency>,
}

/// 래스터 경계 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RasterBoundary {
    region: RasterRegion,
    causes: Vec<RasterCause>,
    pixels: Option<PixelRect>,
    nodes: u64,
    first_z: ZOrder,
    last_z: ZOrder,
}

impl RasterBoundary {
    /// 영역.
    pub const fn region(&self) -> &RasterRegion {
        &self.region
    }

    /// 이 영역으로 모인 이유(node paint 순서, 같은 node는 기능 선언 순서).
    pub fn causes(&self) -> &[RasterCause] {
        &self.causes
    }

    /// 영역이 그리는 pixel 범위(canvas로 자름). 아무것도 그리지 않으면 `None`.
    pub const fn pixels(&self) -> Option<PixelRect> {
        self.pixels
    }

    /// 내용이 래스터화되는 보이는 node 수.
    pub const fn rasterized_nodes(&self) -> u64 {
        self.nodes
    }

    fn covers(&self, z: ZOrder) -> bool {
        match self.region {
            RasterRegion::Content { node } => node.z == z,
            RasterRegion::Subtrees { .. } | RasterRegion::Canvas => {
                self.first_z <= z && z <= self.last_z
            }
        }
    }
}

/// 부분 래스터화 계획.
#[derive(Debug, Clone, PartialEq)]
pub struct RasterPlan {
    grid: PixelGrid,
    boundaries: Vec<RasterBoundary>,
    by_start: BTreeMap<ZOrder, usize>,
    visible_nodes: u64,
}

impl RasterPlan {
    /// 겹침 판정에 쓴 pixel grid.
    pub const fn grid(&self) -> &PixelGrid {
        &self.grid
    }

    /// 래스터 경계(paint 순서, 서로 겹치지 않음).
    pub fn boundaries(&self) -> &[RasterBoundary] {
        &self.boundaries
    }

    /// 래스터화할 것이 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.boundaries.is_empty()
    }

    /// 영역이 z의 node에서 시작하는 경계(Content node 또는 Subtrees 첫 root). canvas 경계는 첫
    /// 최상위 node에서 시작합니다.
    pub fn boundary_at(&self, z: ZOrder) -> Option<&RasterBoundary> {
        let index = *self.by_start.get(&z)?;
        self.boundaries.get(index)
    }

    /// z의 node 내용이 어떤 경계 안에서 래스터화되면 참입니다.
    pub fn rasterizes(&self, z: ZOrder) -> bool {
        self.by_start
            .range(..=z)
            .next_back()
            .and_then(|(_, index)| self.boundaries.get(*index))
            .is_some_and(|boundary| boundary.covers(z))
    }

    /// 보이는 내보내기 node 수.
    pub const fn visible_nodes(&self) -> u64 {
        self.visible_nodes
    }

    /// 내용이 래스터화되는 보이는 node 수.
    pub fn rasterized_nodes(&self) -> u64 {
        self.boundaries.iter().map(|b| b.nodes).sum()
    }

    /// vector로 남는 보이는 node 수.
    pub fn vector_nodes(&self) -> u64 {
        self.visible_nodes - self.rasterized_nodes()
    }
}

/// capture의 부분 래스터화 경계를 계산합니다(모듈 문서 규칙).
pub fn plan_raster(capture: &ExportCapture) -> RasterPlan {
    let background = match capture.request().background() {
        ExportBackground::Transparent => false,
        ExportBackground::Solid(color) => color.alpha() > 0.0,
    };
    let roots: Vec<&SceneNode> = capture.roots().iter().map(|root| &**root.node()).collect();
    plan_nodes(&roots, PixelGrid::of_capture(capture), background)
}

/// 최상위 node 목록(paint 순서)의 계획. `background`는 배경이 무언가를 칠하는지입니다.
pub(crate) fn plan_nodes(roots: &[&SceneNode], grid: PixelGrid, background: bool) -> RasterPlan {
    let arena = Arena::build(roots, grid);
    let mut candidates: Vec<Candidate> = Vec::new();
    for index in 0..arena.entries.len() {
        let node = arena.entries[index].node;
        for feature in node_features(node) {
            let Some(scope) = raster_scope(feature) else {
                continue;
            };
            let mut cause = RasterCause {
                node: RasterNode::of(node),
                feature,
                blend: None,
            };
            let kind = match scope {
                RasterScope::Content => Kind::Content(index),
                RasterScope::Subtree => arena.subtree_kind(index),
                RasterScope::Backdrop => {
                    let (kind, dependency) = arena.blend_region(index, background);
                    cause.blend = Some(dependency);
                    kind
                }
            };
            candidates.push(arena.candidate(kind, cause));
        }
    }
    arena.finish(candidates)
}

/// 평평하게 편 보이는 node(pre-order).
struct Entry<'a> {
    node: &'a SceneNode,
    parent: Option<usize>,
    /// subtree 끝(pre-order index, 배타).
    end: usize,
    /// 자기 내용 footprint.
    own: Option<PixelRect>,
    /// subtree footprint(blur 확장 포함).
    subtree: Option<PixelRect>,
    /// opacity < 1·Normal 외 blend·layer blur 중 하나가 있음.
    isolating: bool,
    /// layer blur가 subtree footprint를 넓히는 pixel 수(없으면 0).
    blur: i64,
}

/// 영역 종류(arena index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Content(usize),
    Range {
        parent: Option<usize>,
        first: usize,
        last: usize,
    },
    Canvas,
}

struct Candidate {
    /// pre-order 구간 `[lo, hi)`.
    lo: usize,
    hi: usize,
    kind: Kind,
    causes: Vec<RasterCause>,
}

struct Arena<'a> {
    grid: PixelGrid,
    entries: Vec<Entry<'a>>,
    roots: Vec<usize>,
}

/// node 자기 내용이 무언가를 그리면 참입니다(text·image는 고유 모습이 있어 항상 그림).
fn draws_content(node: &SceneNode) -> bool {
    match node.content() {
        SceneContent::Text(_) | SceneContent::Image(_) => true,
        SceneContent::Rect(_) | SceneContent::Path(_) => !node.paint().get().paints_nothing(),
    }
}

/// subtree를 따로 합성하는 효과가 있으면 참입니다.
fn is_isolating(node: &SceneNode) -> bool {
    let paint = node.paint().get();
    paint.opacity < Opacity::OPAQUE
        || paint.blend != SceneBlendMode::Normal
        || paint.layer_blur.get() > 0.0
}

/// layer blur가 subtree footprint를 넓히는 pixel 수.
///
/// 반경에 world 변환 선형부의 최대 특이값(가장 크게 늘리는 배율, 최소 1)과 grid 배율을 곱해
/// 올림합니다. 반경이 local 단위든 world 단위든 넓게 잡는 쪽입니다.
fn blur_pixels(node: &SceneNode, scale: f64) -> i64 {
    let radius = f64::from(node.paint().get().layer_blur.get());
    if radius <= 0.0 {
        return 0;
    }
    let (a, b, c, d, _, _) = node.world_transform().components();
    let (a, b, c, d) = (f64::from(a), f64::from(b), f64::from(c), f64::from(d));
    let sum = a * a + b * b + c * c + d * d;
    let det = a * d - b * c;
    let spread = (sum * sum - 4.0 * det * det).max(0.0).sqrt();
    let norm = ((sum + spread) / 2.0).sqrt().max(1.0);
    pixel_ceil(radius * norm * scale).max(1)
}

fn union(a: Option<PixelRect>, b: Option<PixelRect>) -> Option<PixelRect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(&b)),
        (a, None) => a,
        (None, b) => b,
    }
}

impl<'a> Arena<'a> {
    fn build(roots: &[&'a SceneNode], grid: PixelGrid) -> Arena<'a> {
        let mut entries: Vec<Entry<'a>> = Vec::new();
        let mut top = Vec::new();
        let mut stack: Vec<(&'a SceneNode, Option<usize>)> =
            roots.iter().rev().map(|node| (*node, None)).collect();
        while let Some((node, parent)) = stack.pop() {
            if node.hit().visibility == Visibility::Hidden {
                continue;
            }
            let index = entries.len();
            if parent.is_none() {
                top.push(index);
            }
            // world bounds 계산이 넘치면 어디든 그릴 수 있다고 봅니다.
            let own = if draws_content(node) {
                match node.world_render_bounds() {
                    Some(bounds) => grid.device_box(&bounds),
                    None => Some(PixelRect::UNBOUNDED),
                }
            } else {
                None
            };
            entries.push(Entry {
                node,
                parent,
                end: index + 1,
                own,
                // 역순 pass 전까지는 자식 footprint 합집합을 모읍니다.
                subtree: None,
                isolating: is_isolating(node),
                blur: blur_pixels(node, grid.scale()),
            });
            let start = stack.len();
            stack.extend(node.children().map(|child| (&**child, Some(index))));
            stack[start..].reverse();
        }
        // 자손은 조상보다 index가 크므로 역순 한 번으로 subtree 끝과 footprint가 확정됩니다.
        // 자기 내용 footprint는 scene render bounds가 이미 자기 blur를 포함하므로 자식 합집합만
        // blur만큼 넓힙니다.
        for index in (0..entries.len()).rev() {
            let entry = &mut entries[index];
            let children = if entry.blur > 0 {
                entry.subtree.map(|area| area.expanded(entry.blur))
            } else {
                entry.subtree
            };
            entry.subtree = union(entry.own, children);
            let (end, subtree, parent) = (entry.end, entry.subtree, entry.parent);
            if let Some(parent) = parent {
                let parent = &mut entries[parent];
                parent.end = parent.end.max(end);
                parent.subtree = union(parent.subtree, subtree);
            }
        }
        Arena {
            grid,
            entries,
            roots: top,
        }
    }

    /// `parent`의 자식(paint 순서).
    fn children(&self, parent: usize) -> impl Iterator<Item = usize> + '_ {
        let end = self.entries[parent].end;
        let mut next = parent + 1;
        std::iter::from_fn(move || {
            (next < end).then(|| {
                let child = next;
                next = self.entries[child].end;
                child
            })
        })
    }

    fn raster_node(&self, index: usize) -> RasterNode {
        RasterNode::of(self.entries[index].node)
    }

    fn subtree_kind(&self, index: usize) -> Kind {
        Kind::Range {
            parent: self.entries[index].parent,
            first: index,
            last: index,
        }
    }

    /// `start` subtree에서 `target`과 겹치는 첫 내용(pre-order). blur node는 subtree가 겹치면 그
    /// node 자체가 겹침입니다(합성 결과가 퍼짐).
    fn first_hit(&self, start: usize, target: &PixelRect) -> Option<usize> {
        let mut stack = vec![start];
        while let Some(index) = stack.pop() {
            let entry = &self.entries[index];
            if !self.grid.overlaps(entry.subtree, target) {
                continue;
            }
            if entry.blur > 0 || self.grid.overlaps(entry.own, target) {
                return Some(index);
            }
            let from = stack.len();
            stack.extend(self.children(index));
            stack[from..].reverse();
        }
        None
    }

    /// `siblings` 중 `before` 앞에서 `target`과 겹치는 첫 subtree와 그 안의 첫 겹침 내용.
    fn first_overlapping(
        &self,
        siblings: impl Iterator<Item = usize>,
        before: usize,
        target: &PixelRect,
    ) -> Option<(usize, usize)> {
        siblings
            .take_while(|&sibling| sibling != before)
            .find_map(|sibling| self.first_hit(sibling, target).map(|hit| (sibling, hit)))
    }

    /// blend node의 영역과 backdrop 의존(모듈 문서 규칙).
    fn blend_region(&self, index: usize, background: bool) -> (Kind, BlendDependency) {
        let mut kind = self.subtree_kind(index);
        let mut widest = None;
        let target = self.grid.in_canvas(self.entries[index].subtree);
        let mut child = index;
        let isolation = loop {
            match self.entries[child].parent {
                Some(parent) => {
                    if let Some(target) = &target {
                        if self.grid.overlaps(self.entries[parent].own, target) {
                            kind = self.subtree_kind(parent);
                            widest = Some(BackdropItem::Node(self.raster_node(parent)));
                        } else if let Some((first, hit)) =
                            self.first_overlapping(self.children(parent), child, target)
                        {
                            kind = Kind::Range {
                                parent: Some(parent),
                                first,
                                last: child,
                            };
                            widest = Some(BackdropItem::Node(self.raster_node(hit)));
                        }
                    }
                    if self.entries[parent].isolating {
                        break Some(self.raster_node(parent));
                    }
                    child = parent;
                }
                None => {
                    if let Some(target) = &target {
                        if background {
                            kind = Kind::Canvas;
                            widest = Some(BackdropItem::Background);
                        } else if let Some((first, hit)) =
                            self.first_overlapping(self.roots.iter().copied(), child, target)
                        {
                            kind = Kind::Range {
                                parent: None,
                                first,
                                last: child,
                            };
                            widest = Some(BackdropItem::Node(self.raster_node(hit)));
                        }
                    }
                    break None;
                }
            }
        };
        (kind, BlendDependency { isolation, widest })
    }

    fn candidate(&self, kind: Kind, cause: RasterCause) -> Candidate {
        let (lo, hi) = match kind {
            Kind::Content(index) => (index, index + 1),
            Kind::Range { first, last, .. } => (first, self.entries[last].end),
            Kind::Canvas => (0, self.entries.len()),
        };
        Candidate {
            lo,
            hi,
            kind,
            causes: vec![cause],
        }
    }

    /// 후보를 병합해 계획을 만듭니다(모듈 문서 병합 규칙).
    fn finish(self, mut candidates: Vec<Candidate>) -> RasterPlan {
        let mut canvas = candidates.iter().any(|c| c.kind == Kind::Canvas);
        candidates.sort_by(|a, b| a.lo.cmp(&b.lo).then(b.hi.cmp(&a.hi)));
        let mut merged: Vec<Candidate> = Vec::new();
        for candidate in candidates {
            if let Some(current) = merged.last_mut() {
                if candidate.lo < current.hi {
                    if candidate.hi > current.hi {
                        // tree 구조상 부분 겹침은 같은 부모의 형제 범위끼리만 생깁니다. 그 밖의 경우가
                        // 나오면 canvas 전체로 물러나 결과를 틀리게 만들지 않습니다.
                        match (&mut current.kind, candidate.kind) {
                            (
                                Kind::Range { parent, last, .. },
                                Kind::Range {
                                    parent: other,
                                    last: other_last,
                                    ..
                                },
                            ) if *parent == other => {
                                *last = other_last;
                                current.hi = candidate.hi;
                            }
                            _ => canvas = true,
                        }
                    }
                    current.causes.extend(candidate.causes);
                    continue;
                }
            }
            merged.push(candidate);
        }
        if canvas {
            let causes = merged.into_iter().flat_map(|c| c.causes).collect();
            merged = vec![Candidate {
                lo: 0,
                hi: self.entries.len(),
                kind: Kind::Canvas,
                causes,
            }];
        }
        let visible_nodes = self.entries.len() as u64;
        let mut boundaries = Vec::with_capacity(merged.len());
        let mut by_start = BTreeMap::new();
        for mut candidate in merged {
            candidate
                .causes
                .sort_by(|a, b| a.node.z.cmp(&b.node.z).then(a.feature.cmp(&b.feature)));
            candidate.causes.dedup();
            let boundary = self.boundary(candidate);
            by_start.insert(boundary.first_z, boundaries.len());
            boundaries.push(boundary);
        }
        RasterPlan {
            grid: self.grid,
            boundaries,
            by_start,
            visible_nodes,
        }
    }

    fn boundary(&self, candidate: Candidate) -> RasterBoundary {
        let first_z = self.entries[candidate.lo].node.z();
        // 숨긴 자손도 포함하도록 마지막 subtree root의 last z를 씁니다.
        let last_z = match candidate.kind {
            Kind::Content(index) => self.entries[index].node.z(),
            Kind::Range { last, .. } => self.entries[last].node.last_z(),
            Kind::Canvas => self
                .roots
                .last()
                .map_or(first_z, |root| self.entries[*root].node.last_z()),
        };
        let (region, pixels, nodes) = match candidate.kind {
            Kind::Content(index) => (
                RasterRegion::Content {
                    node: self.raster_node(index),
                },
                self.grid.in_canvas(self.entries[index].own),
                1,
            ),
            Kind::Range {
                parent,
                first,
                last,
            } => {
                let mut area = None;
                let mut sibling = first;
                loop {
                    area = union(area, self.entries[sibling].subtree);
                    if sibling == last {
                        break;
                    }
                    sibling = self.entries[sibling].end;
                }
                (
                    RasterRegion::Subtrees {
                        parent: parent.map(|p| self.raster_node(p)),
                        first: self.raster_node(first),
                        last: self.raster_node(last),
                    },
                    self.grid.in_canvas(area),
                    (candidate.hi - candidate.lo) as u64,
                )
            }
            Kind::Canvas => (
                RasterRegion::Canvas,
                Some(self.grid.canvas()),
                self.entries.len() as u64,
            ),
        };
        RasterBoundary {
            region,
            causes: candidate.causes,
            pixels,
            nodes,
            first_z,
            last_z,
        }
    }
}

#[cfg(test)]
mod tests;
