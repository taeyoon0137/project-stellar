//! selection handle·modifier control point transient hit-test layer (XI.3.5).
//!
//! 선택 handle과 modifier control point는 문서 내용이 아니라 **창의 상호작용 상태**(선택은
//! `WindowViewState`, VII.3.1)에서 만든 일시 객체입니다. 그래서 [`SceneSnapshot`](crate::snapshot::SceneSnapshot)·
//! 공간 인덱스·scene codec 어디에도 들어가지 않고 별도 [`TransientLayer`]로 존재합니다. layer는 직렬화
//! 경로가 없어 project 저장·semantic hash에 포함될 수 없습니다.
//!
//! - **좌표**: handle 위치는 world 좌표(선택 상자 꼭짓점 등)로, 크기는 view(화면) px로 둡니다. zoom과
//!   관계없이 화면에서 같은 크기로 잡히도록 hit test 때 viewport로 위치만 view로 옮깁니다.
//! - **우선순위**: handle은 항상 scene node보다 먼저 판정합니다. handle끼리는 종류 우선순위
//!   ([`HandleKind::priority`]: modifier control point > 크기 조절 모서리 > 크기 조절 변 > 회전 영역)가
//!   높은 쪽, 같으면 view 거리가 가까운 쪽, 그것도 같으면 나중에 추가한(위에 그린) 쪽이 이깁니다.
//! - 맞은 handle이 없으면 narrow-phase scene hit test(XI.3.4)의 topmost node가 대상입니다
//!   ([`hit_test_canvas`]).

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Vec2};

use crate::hit_test::HitCandidate;
use crate::narrow_phase::{hit_test_view_narrow, NarrowPhaseOptions, TextHitGeometry};
use crate::node::{non_negative, SceneValueError};
use crate::spatial::SceneSpatialIndex;
use crate::visibility::Viewport;

/// 선택 상자 모서리(상자 꼭짓점 순서 `[top_left, top_right, bottom_right, bottom_left]`의 index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Corner {
    /// 왼쪽 위.
    TopLeft,
    /// 오른쪽 위.
    TopRight,
    /// 오른쪽 아래.
    BottomRight,
    /// 왼쪽 아래.
    BottomLeft,
}

impl Corner {
    /// 꼭짓점 순서대로 네 모서리.
    pub const ALL: [Corner; 4] = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomRight,
        Corner::BottomLeft,
    ];

    const fn index(self) -> usize {
        match self {
            Corner::TopLeft => 0,
            Corner::TopRight => 1,
            Corner::BottomRight => 2,
            Corner::BottomLeft => 3,
        }
    }
}

/// 선택 상자 변. 변 `i`는 꼭짓점 `i`에서 `i + 1`(마지막은 0)로 갑니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Edge {
    /// 위(왼쪽 위 → 오른쪽 위).
    Top,
    /// 오른쪽(오른쪽 위 → 오른쪽 아래).
    Right,
    /// 아래(오른쪽 아래 → 왼쪽 아래).
    Bottom,
    /// 왼쪽(왼쪽 아래 → 왼쪽 위).
    Left,
}

impl Edge {
    /// 꼭짓점 순서대로 네 변.
    pub const ALL: [Edge; 4] = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];

    const fn index(self) -> usize {
        match self {
            Edge::Top => 0,
            Edge::Right => 1,
            Edge::Bottom => 2,
            Edge::Left => 3,
        }
    }
}

/// modifier control point 식별자. 의미(어느 modifier의 몇 번째 점인지)는 layer를 만드는 쪽이 정합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ControlPointId(
    /// 값.
    pub u64,
);

/// transient handle 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandleKind {
    /// modifier control point(모서리 radius 점, gradient 끝점 등).
    ModifierControl(ControlPointId),
    /// 크기 조절 모서리.
    ResizeCorner(Corner),
    /// 크기 조절 변.
    ResizeEdge(Edge),
    /// 모서리 바깥 회전 영역.
    Rotate(Corner),
}

impl HandleKind {
    /// 겹칠 때의 우선순위(클수록 먼저). 작은 control point가 크기 조절보다, 모서리가 변보다, 크기
    /// 조절이 상자 바깥 회전 영역보다 앞섭니다.
    pub const fn priority(&self) -> u8 {
        match self {
            HandleKind::ModifierControl(_) => 4,
            HandleKind::ResizeCorner(_) => 3,
            HandleKind::ResizeEdge(_) => 2,
            HandleKind::Rotate(_) => 1,
        }
    }
}

/// handle 판정 영역. 위치는 world, 크기는 view px입니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandleShape(Shape);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    Circle {
        center: Vec2,
        radius: FiniteF32,
    },
    Square {
        center: Vec2,
        half: FiniteF32,
    },
    Segment {
        from: Vec2,
        to: Vec2,
        half_width: FiniteF32,
    },
    OutsideQuad {
        quad: [Vec2; 4],
        corner: usize,
        radius: FiniteF32,
    },
}

impl HandleShape {
    /// world 점 `center`에서 view 반지름 `radius`(finite·비음수) 이내.
    pub fn circle(center: Vec2, radius: f32) -> Result<HandleShape, SceneValueError> {
        Ok(HandleShape(Shape::Circle {
            center,
            radius: non_negative(radius, "handle radius")?,
        }))
    }

    /// world 점 `center` 중심, view 반변 `half`(finite·비음수)인 화면 축 정렬 정사각형.
    pub fn square(center: Vec2, half: f32) -> Result<HandleShape, SceneValueError> {
        Ok(HandleShape(Shape::Square {
            center,
            half: non_negative(half, "handle size")?,
        }))
    }

    /// world 선분 `from`→`to`에서 view 거리 `half_width`(finite·비음수) 이내인 띠.
    pub fn segment(from: Vec2, to: Vec2, half_width: f32) -> Result<HandleShape, SceneValueError> {
        Ok(HandleShape(Shape::Segment {
            from,
            to,
            half_width: non_negative(half_width, "handle width")?,
        }))
    }

    /// world 사각형 `quad`(꼭짓점 순서)의 `corner`에서 view 반지름 `radius` 이내이면서 사각형 **밖**인
    /// 영역(회전 영역).
    pub fn outside_corner(
        quad: [Vec2; 4],
        corner: Corner,
        radius: f32,
    ) -> Result<HandleShape, SceneValueError> {
        Ok(HandleShape(Shape::OutsideQuad {
            quad,
            corner: corner.index(),
            radius: non_negative(radius, "handle radius")?,
        }))
    }

    /// view 점 `point`가 영역 안이면 비교용 view 거리(작을수록 가까움)를 돌려줍니다.
    fn view_distance(&self, viewport: &Viewport, point: Vec2) -> Option<f32> {
        let view = viewport.view();
        let to_view = |world: Vec2| view.apply(world).ok();
        let (px, py) = (point.x(), point.y());
        match self.0 {
            Shape::Circle { center, radius } => {
                let c = to_view(center)?;
                let distance = (px - c.x()).hypot(py - c.y());
                (distance <= radius.get()).then_some(distance)
            }
            Shape::Square { center, half } => {
                let c = to_view(center)?;
                let distance = (px - c.x()).abs().max((py - c.y()).abs());
                (distance <= half.get()).then_some(distance)
            }
            Shape::Segment {
                from,
                to,
                half_width,
            } => {
                let (a, b) = (to_view(from)?, to_view(to)?);
                let distance = segment_distance(a, b, point);
                (distance <= half_width.get()).then_some(distance)
            }
            Shape::OutsideQuad {
                quad,
                corner,
                radius,
            } => {
                let mut view_quad = [Vec2::ZERO; 4];
                for (slot, world) in view_quad.iter_mut().zip(quad) {
                    *slot = to_view(world)?;
                }
                let c = view_quad[corner];
                let distance = (px - c.x()).hypot(py - c.y());
                (distance <= radius.get() && !quad_contains(&view_quad, point)).then_some(distance)
            }
        }
    }
}

/// 선분 `a→b`에서 `p`까지 거리.
fn segment_distance(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    let (abx, aby) = (b.x() - a.x(), b.y() - a.y());
    let (apx, apy) = (p.x() - a.x(), p.y() - a.y());
    let len2 = abx * abx + aby * aby;
    let t = if len2 > 0.0 {
        ((apx * abx + apy * aby) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (apx - abx * t).hypot(apy - aby * t)
}

/// 볼록 사각형(꼭짓점 순서, 방향 무관) 안(경계 포함)이면 참입니다. 면적 0이면 거짓입니다.
fn quad_contains(quad: &[Vec2; 4], p: Vec2) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let (a, b) = (quad[i], quad[(i + 1) % 4]);
        let cross = (b.x() - a.x()) * (p.y() - a.y()) - (b.y() - a.y()) * (p.x() - a.x());
        if cross != 0.0 {
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
    }
    sign != 0.0
}

/// transient handle 하나.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransientHandle {
    /// 종류.
    pub kind: HandleKind,
    /// handle이 조작하는 node(선택 primary 또는 control point를 가진 node).
    pub target: StableId,
    /// 판정 영역.
    pub shape: HandleShape,
}

/// 선택 상자 handle 크기(view px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandleMetrics {
    corner_half: FiniteF32,
    edge_half_width: FiniteF32,
    rotate_radius: FiniteF32,
}

impl HandleMetrics {
    /// 모서리 정사각형 반변, 변 띠 반폭, 회전 영역 반지름(모두 finite·비음수 view px).
    pub fn new(
        corner_half: f32,
        edge_half_width: f32,
        rotate_radius: f32,
    ) -> Result<HandleMetrics, SceneValueError> {
        Ok(HandleMetrics {
            corner_half: non_negative(corner_half, "handle size")?,
            edge_half_width: non_negative(edge_half_width, "handle width")?,
            rotate_radius: non_negative(rotate_radius, "handle radius")?,
        })
    }
}

/// handle hit 결과.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransientHit {
    /// 맞은 handle.
    pub handle: TransientHandle,
    /// layer 안 순서(추가 순).
    pub index: usize,
    /// 비교에 쓴 view 거리.
    pub distance: f32,
}

/// 창마다 다시 만드는 transient hit-test layer. scene snapshot·저장과 무관합니다.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransientLayer {
    handles: Vec<TransientHandle>,
}

impl TransientLayer {
    /// 빈 layer.
    pub const fn new() -> TransientLayer {
        TransientLayer {
            handles: Vec::new(),
        }
    }

    /// handle을 추가합니다. 나중에 추가한 handle이 위에 그려집니다.
    pub fn push(&mut self, handle: TransientHandle) {
        self.handles.push(handle);
    }

    /// 선택 상자(world 꼭짓점 `[top_left, top_right, bottom_right, bottom_left]`)의 회전 영역 4개,
    /// 변 4개, 모서리 4개 handle을 추가합니다. 회전한 node의 상자도 그대로 받습니다.
    pub fn push_selection_box(
        &mut self,
        target: StableId,
        quad: [Vec2; 4],
        metrics: HandleMetrics,
    ) {
        for corner in Corner::ALL {
            self.push(TransientHandle {
                kind: HandleKind::Rotate(corner),
                target,
                shape: HandleShape(Shape::OutsideQuad {
                    quad,
                    corner: corner.index(),
                    radius: metrics.rotate_radius,
                }),
            });
        }
        for edge in Edge::ALL {
            let i = edge.index();
            self.push(TransientHandle {
                kind: HandleKind::ResizeEdge(edge),
                target,
                shape: HandleShape(Shape::Segment {
                    from: quad[i],
                    to: quad[(i + 1) % 4],
                    half_width: metrics.edge_half_width,
                }),
            });
        }
        for corner in Corner::ALL {
            self.push(TransientHandle {
                kind: HandleKind::ResizeCorner(corner),
                target,
                shape: HandleShape(Shape::Square {
                    center: quad[corner.index()],
                    half: metrics.corner_half,
                }),
            });
        }
    }

    /// handle 수.
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    /// handle이 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// handle들(추가 순).
    pub fn handles(&self) -> &[TransientHandle] {
        &self.handles
    }

    /// 모든 handle을 지웁니다(선택이 바뀌어 다시 만들 때).
    pub fn clear(&mut self) {
        self.handles.clear();
    }

    /// view 점 `view_point`에 맞는 handle(모듈 문서의 우선순위). 없으면 `None`.
    pub fn hit(&self, viewport: &Viewport, view_point: Vec2) -> Option<TransientHit> {
        let mut best: Option<TransientHit> = None;
        for (index, handle) in self.handles.iter().enumerate() {
            let Some(distance) = handle.shape.view_distance(viewport, view_point) else {
                continue;
            };
            let wins = best.as_ref().is_none_or(|current| {
                let (p, q) = (handle.kind.priority(), current.handle.kind.priority());
                // 같은 우선순위·같은 거리면 나중 handle(index가 큼)이 이깁니다.
                p > q || (p == q && distance <= current.distance)
            });
            if wins {
                best = Some(TransientHit {
                    handle: *handle,
                    index,
                    distance,
                });
            }
        }
        best
    }
}

/// canvas hit 결과.
#[derive(Debug, Clone)]
pub enum CanvasHit {
    /// transient handle.
    Handle(TransientHit),
    /// scene node(narrow-phase topmost).
    Node(HitCandidate),
    /// 아무것도 맞지 않음.
    Empty,
}

/// view 점의 canvas hit test: transient layer handle을 먼저, 없으면 narrow-phase scene hit test
/// (XI.3.4)의 topmost node를 돌려줍니다. `options.tolerance`는 view 단위입니다.
pub fn hit_test_canvas(
    layer: &TransientLayer,
    index: &SceneSpatialIndex,
    viewport: &Viewport,
    view_point: Vec2,
    options: &NarrowPhaseOptions,
    text: &dyn TextHitGeometry,
) -> CanvasHit {
    if let Some(hit) = layer.hit(viewport, view_point) {
        return CanvasHit::Handle(hit);
    }
    match hit_test_view_narrow(index, viewport, view_point, options, text)
        .hits
        .into_iter()
        .next()
    {
        Some(node) => CanvasHit::Node(node),
        None => CanvasHit::Empty,
    }
}

#[cfg(test)]
mod tests;
