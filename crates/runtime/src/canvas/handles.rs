//! 변형 핸들 hit layer (XII.10.8, ADR-0104 결정 4).
//!
//! - 우선순위: 반경 > 크기 조절 모서리 > 크기 조절 변 > 회전 > 본체([`CanvasHandle::priority`]). ADR-0074 결정 3으로
//!   padding·gap 핸들 자리는 없습니다. 반경 핸들은 XI.3.5 transient layer의 modifier control point(가장 높은 우선순위)로
//!   넣어 같은 hit 규칙(우선순위 → 화면 거리 → 나중 핸들)을 씁니다.
//! - 위치: 단일 선택은 node local 사각형 꼭짓점을 world로 옮긴 사각형이라 회전한 node도 자기 축을 따릅니다. 다중 선택은
//!   선택 node world bounds의 합(축 정렬)입니다. 반경 핸들은 local 모서리에서 안쪽으로 `max(실효 반경, 안쪽 거리)`만큼
//!   들어간 점이며 변 길이 절반을 넘지 않습니다. 안쪽 거리는 화면 pt를 local 단위로 바꿔 씁니다.
//! - 숨김: 선택 상자 화면 짧은 변이 [`HIDE_RESIZE_BELOW_PT`] 미만이면 크기·반경 핸들이 없고, [`HIDE_EDGES_BELOW_PT`]
//!   미만이면 변 핸들이 없습니다. 반경 핸들은 Frame·Rectangle·Polygon 단일 선택이고 짧은 변이 [`RADIUS_MIN_SIDE_PT`]
//!   이상일 때만 있습니다. 길이 0인 축의 크기 핸들은 XI.3.5처럼 만들지 않습니다.
//! - 핸들 layer는 scene snapshot·저장과 무관한 transient 값입니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Rect, Transform2D, Vec2};
use stellar_document::records::NodePayload;
use stellar_document::shape::{RectangleParams, ShapeGeometry};
use stellar_scene::bounds::Aabb;
use stellar_scene::transient::{
    ControlPointId, Corner, Edge, HandleKind, HandleShape, ResizeAxes, TransientHandle,
    TransientLayer,
};
use stellar_scene::visibility::Viewport;

use super::hit::CanvasGeometry;
use super::select::SelectTree;
use super::tool_units::{
    CORNER_HALF_PT, EDGE_HALF_WIDTH_PT, HIDE_EDGES_BELOW_PT, HIDE_RESIZE_BELOW_PT,
    RADIUS_HANDLE_PT, RADIUS_INSET_PT, RADIUS_MIN_SIDE_PT, ROTATE_RADIUS_PT,
};

/// 캔버스 변형 핸들 종류(본체 포함).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanvasHandle {
    /// 모서리 안쪽 반경 핸들.
    Radius(Corner),
    /// 크기 조절 모서리.
    ResizeCorner(Corner),
    /// 크기 조절 변.
    ResizeEdge(Edge),
    /// 모서리 바깥 회전 영역.
    Rotate(Corner),
    /// 선택 node 본체(이동).
    Body,
}

impl CanvasHandle {
    /// 겹칠 때의 우선순위(클수록 먼저).
    pub const fn priority(self) -> u8 {
        match self {
            CanvasHandle::Radius(_) => 4,
            CanvasHandle::ResizeCorner(_) => 3,
            CanvasHandle::ResizeEdge(_) => 2,
            CanvasHandle::Rotate(_) => 1,
            CanvasHandle::Body => 0,
        }
    }
}

/// 핸들을 만들 선택 node 하나.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandleTarget {
    /// 문서 node.
    pub node: StableId,
    /// local → world.
    pub world: Transform2D,
    /// local 상자.
    pub rect: Rect,
    /// 반경 핸들을 두는 node의 실효 반경 `(top_left, top_right, bottom_right, bottom_left)`(local 단위).
    pub radius: Option<(f32, f32, f32, f32)>,
}

impl HandleTarget {
    /// 문서와 node 상자로 만듭니다. 상자가 없으면 `None`입니다. 반경 핸들은 Frame·Rectangle·Polygon만 둡니다.
    pub fn of(
        tree: &SelectTree<'_>,
        geometry: &CanvasGeometry,
        node: StableId,
    ) -> Option<HandleTarget> {
        let (world, rect) = match geometry.boxes(node) {
            [single] => (single.world, single.rect),
            _ => {
                let b = geometry.bounds(tree, node)?;
                (Transform2D::identity(), aabb_rect(&b)?)
            }
        };
        let size = rect.size();
        let radius = match tree.model().payload(node) {
            Some(NodePayload::Frame(frame)) => Some(
                RectangleParams {
                    size: frame.size,
                    corner_radius: frame.corner_radius,
                }
                .effective_corner_radius(),
            ),
            Some(NodePayload::Shape(ShapeGeometry::Rectangle(params))) => {
                Some(params.effective_corner_radius())
            }
            Some(NodePayload::Shape(ShapeGeometry::Polygon(params))) => {
                let r = params
                    .corner_radius()
                    .min(size.width().min(size.height()) / 2.0);
                Some((r, r, r, r))
            }
            _ => None,
        };
        Some(HandleTarget {
            node,
            world,
            rect,
            radius,
        })
    }

    fn quad(&self) -> Option<[Vec2; 4]> {
        local_quad(&self.rect)
            .iter()
            .map(|p| self.world.apply(*p).ok())
            .collect::<Option<Vec<_>>>()
            .and_then(|v| v.try_into().ok())
    }
}

fn aabb_rect(b: &Aabb) -> Option<Rect> {
    Some(Rect::new(
        Vec2::new(b.min_x(), b.min_y()).ok()?,
        stellar_core_types::value::Size2::new(b.max_x() - b.min_x(), b.max_y() - b.min_y()).ok()?,
    ))
}

fn local_quad(rect: &Rect) -> [Vec2; 4] {
    let (x, y) = (rect.origin().x(), rect.origin().y());
    let (w, h) = (rect.size().width(), rect.size().height());
    let p = |px: f32, py: f32| Vec2::new(px, py).unwrap_or(Vec2::ZERO);
    [p(x, y), p(x + w, y), p(x + w, y + h), p(x, y + h)]
}

/// 만든 핸들 layer.
#[derive(Debug, Clone, PartialEq)]
pub struct HandleLayer {
    layer: TransientLayer,
    /// 핸들을 만든 선택 상자(world 꼭짓점).
    selection_box: Option<[Vec2; 4]>,
    /// 본체 hit에 쓰는 선택 node별 world 사각형.
    bodies: Vec<(StableId, [Vec2; 4])>,
    /// 반경 핸들 world 위치(모서리 순서).
    radius_points: [Option<Vec2>; 4],
}

/// 핸들 hit 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandleHit {
    /// 맞은 핸들(본체 포함).
    pub handle: CanvasHandle,
    /// 핸들이 조작하는 node(다중 선택 상자는 primary).
    pub node: StableId,
}

impl HandleLayer {
    /// XI.3.5 transient layer(셸 overlay·hit 공용).
    pub fn transient(&self) -> &TransientLayer {
        &self.layer
    }

    /// 선택 상자 world 꼭짓점.
    pub fn selection_box(&self) -> Option<[Vec2; 4]> {
        self.selection_box
    }

    /// 반경 핸들 world 위치(없으면 `None`, 모서리 순서 `[top_left, top_right, bottom_right, bottom_left]`).
    pub fn radius_points(&self) -> [Option<Vec2>; 4] {
        self.radius_points
    }

    /// 보이는 핸들 종류(추가 순).
    pub fn kinds(&self) -> Vec<CanvasHandle> {
        self.layer
            .handles()
            .iter()
            .map(|h| canvas_handle(h.kind))
            .collect()
    }

    /// view 점의 hit. 핸들이 없으면 선택 node 본체, 그것도 아니면 `None`입니다.
    pub fn hit(&self, viewport: &Viewport, view_point: Vec2) -> Option<HandleHit> {
        if let Some(hit) = self.layer.hit(viewport, view_point) {
            return Some(HandleHit {
                handle: canvas_handle(hit.handle.kind),
                node: hit.handle.target,
            });
        }
        let world = viewport.view().inverse()?.apply(view_point).ok()?;
        self.bodies
            .iter()
            .rev()
            .find(|(_, quad)| quad_contains(quad, world))
            .map(|(node, _)| HandleHit {
                handle: CanvasHandle::Body,
                node: *node,
            })
    }
}

fn canvas_handle(kind: HandleKind) -> CanvasHandle {
    match kind {
        HandleKind::ModifierControl(ControlPointId(index)) => {
            CanvasHandle::Radius(Corner::ALL[(index as usize).min(3)])
        }
        HandleKind::ResizeCorner(corner) => CanvasHandle::ResizeCorner(corner),
        HandleKind::ResizeEdge(edge) => CanvasHandle::ResizeEdge(edge),
        HandleKind::Rotate(corner) => CanvasHandle::Rotate(corner),
    }
}

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

fn view_length(viewport: &Viewport, a: Vec2, b: Vec2) -> Option<f32> {
    let view = viewport.view();
    let (a, b) = (view.apply(a).ok()?, view.apply(b).ok()?);
    Some((b.x() - a.x()).hypot(b.y() - a.y()))
}

/// 선택 node들의 핸들 layer. `primary`는 다중 선택 상자 핸들이 조작할 node입니다.
pub fn build_handles(
    targets: &[HandleTarget],
    primary: Option<StableId>,
    viewport: &Viewport,
) -> HandleLayer {
    let mut out = HandleLayer {
        layer: TransientLayer::new(),
        selection_box: None,
        bodies: targets
            .iter()
            .filter_map(|t| Some((t.node, t.quad()?)))
            .collect(),
        radius_points: [None; 4],
    };
    let (quad, owner, single) = match targets {
        [] => return out,
        [only] => match only.quad() {
            Some(quad) => (quad, only.node, Some(only)),
            None => return out,
        },
        many => {
            let Some(bounds) = many
                .iter()
                .filter_map(|t| Aabb::from_points(t.quad()?))
                .reduce(|a, b| a.union(&b))
            else {
                return out;
            };
            let Some(rect) = aabb_rect(&bounds) else {
                return out;
            };
            let owner = primary
                .filter(|p| many.iter().any(|t| t.node == *p))
                .unwrap_or(many[many.len() - 1].node);
            (local_quad(&rect), owner, None)
        }
    };
    out.selection_box = Some(quad);
    let (Some(width), Some(height)) = (
        view_length(viewport, quad[0], quad[1]),
        view_length(viewport, quad[0], quad[3]),
    ) else {
        return out;
    };
    let short = width.min(height);
    let axes = ResizeAxes::of(&quad);
    let push = |layer: &mut TransientLayer, kind: HandleKind, shape: Option<HandleShape>| {
        if let Some(shape) = shape {
            layer.push(TransientHandle {
                kind,
                target: owner,
                shape,
            });
        }
    };
    for corner in Corner::ALL {
        push(
            &mut out.layer,
            HandleKind::Rotate(corner),
            HandleShape::outside_corner(quad, corner, ROTATE_RADIUS_PT).ok(),
        );
    }
    if short >= HIDE_RESIZE_BELOW_PT {
        if short >= HIDE_EDGES_BELOW_PT {
            for (i, edge) in Edge::ALL.into_iter().enumerate() {
                if axes.edge(edge) {
                    push(
                        &mut out.layer,
                        HandleKind::ResizeEdge(edge),
                        HandleShape::segment(quad[i], quad[(i + 1) % 4], EDGE_HALF_WIDTH_PT).ok(),
                    );
                }
            }
        }
        if axes.corner() {
            for (i, corner) in Corner::ALL.into_iter().enumerate() {
                push(
                    &mut out.layer,
                    HandleKind::ResizeCorner(corner),
                    HandleShape::square(quad[i], CORNER_HALF_PT).ok(),
                );
            }
        }
    }
    if let Some(target) = single.filter(|_| short >= RADIUS_MIN_SIDE_PT) {
        if let Some(radius) = target.radius {
            let points = radius_points(target, radius, width, height);
            for (i, point) in points.iter().enumerate() {
                let world = point.and_then(|local| target.world.apply(local).ok());
                out.radius_points[i] = world;
                push(
                    &mut out.layer,
                    HandleKind::ModifierControl(ControlPointId(i as u64)),
                    world.and_then(|w| HandleShape::circle(w, RADIUS_HANDLE_PT).ok()),
                );
            }
        }
    }
    out
}

/// 반경 핸들 local 위치. `width`·`height`는 상자 변의 화면 길이입니다.
fn radius_points(
    target: &HandleTarget,
    radius: (f32, f32, f32, f32),
    width: f32,
    height: f32,
) -> [Option<Vec2>; 4] {
    let (x, y) = (target.rect.origin().x(), target.rect.origin().y());
    let (w, h) = (target.rect.size().width(), target.rect.size().height());
    if w <= 0.0 || h <= 0.0 || width <= 0.0 || height <= 0.0 {
        return [None; 4];
    }
    // 화면 pt → local 단위.
    let (inset_x, inset_y) = (RADIUS_INSET_PT * w / width, RADIUS_INSET_PT * h / height);
    let radii = [radius.0, radius.1, radius.2, radius.3];
    let corners = [
        (x, y, 1.0),
        (x + w, y, -1.0),
        (x + w, y + h, -1.0),
        (x, y + h, 1.0),
    ];
    let mut out = [None; 4];
    for (i, ((cx, cy, sx), r)) in corners.into_iter().zip(radii).enumerate() {
        let sy = if i < 2 { 1.0 } else { -1.0 };
        let dx = r.max(inset_x).min(w / 2.0);
        let dy = r.max(inset_y).min(h / 2.0);
        out[i] = Vec2::new(cx + sx * dx, cy + sy * dy).ok();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::select_fixtures::{
        add_rectangle, nested, save_bytes, scene_of, sid, view_point,
    };
    use stellar_commands::state::DocumentState;
    use stellar_core_types::value::Size2;

    fn viewport(zoom: f32) -> Viewport {
        Viewport::new(
            Size2::new(800., 600.).unwrap(),
            Transform2D::new(zoom, 0., 0., zoom, 0., 0.).unwrap(),
        )
        .unwrap()
    }

    fn target(node: u64, world: Transform2D, w: f32, h: f32, radius: Option<f32>) -> HandleTarget {
        HandleTarget {
            node: sid(node),
            world,
            rect: Rect::new(Vec2::ZERO, Size2::new(w, h).unwrap()),
            radius: radius.map(|r| (r, r, r, r)),
        }
    }

    fn hit(layer: &HandleLayer, vp: &Viewport, x: f32, y: f32) -> Option<CanvasHandle> {
        layer.hit(vp, view_point(x, y)).map(|h| h.handle)
    }

    fn translate(x: f32, y: f32) -> Transform2D {
        Transform2D::new(1., 0., 0., 1., x, y).unwrap()
    }

    #[test]
    fn priority_order_is_radius_resize_corner_edge_rotate_body() {
        // 검증 기준(XII.10.8): 우선순위 fixture. 200×200 상자 (100,100), 반경 0.
        let vp = viewport(1.0);
        let layer = build_handles(
            &[target(1, translate(100., 100.), 200., 200., Some(0.))],
            None,
            &vp,
        );
        assert_eq!(
            layer.radius_points()[0].map(|p| (p.x(), p.y())),
            Some((112., 112.))
        );
        // 반경 핸들(112,112)은 모서리 정사각형(±4)과 겹치지 않고 반경이 이깁니다.
        assert_eq!(
            hit(&layer, &vp, 112., 112.),
            Some(CanvasHandle::Radius(Corner::TopLeft))
        );
        assert_eq!(
            hit(&layer, &vp, 101., 101.),
            Some(CanvasHandle::ResizeCorner(Corner::TopLeft))
        );
        assert_eq!(
            hit(&layer, &vp, 200., 102.),
            Some(CanvasHandle::ResizeEdge(Edge::Top))
        );
        assert_eq!(
            hit(&layer, &vp, 92., 92.),
            Some(CanvasHandle::Rotate(Corner::TopLeft))
        );
        assert_eq!(hit(&layer, &vp, 200., 200.), Some(CanvasHandle::Body));
        assert_eq!(hit(&layer, &vp, 50., 50.), None);
        // 큰 반경은 반경 위치에 핸들을 둡니다(변 절반에서 멈춤).
        let rounded = build_handles(
            &[target(1, translate(100., 100.), 200., 200., Some(150.))],
            None,
            &vp,
        );
        assert_eq!(
            rounded.radius_points()[2].map(|p| (p.x(), p.y())),
            Some((200., 200.))
        );
        for (a, b) in [
            (
                CanvasHandle::Radius(Corner::TopLeft),
                CanvasHandle::ResizeCorner(Corner::TopLeft),
            ),
            (
                CanvasHandle::ResizeCorner(Corner::TopLeft),
                CanvasHandle::ResizeEdge(Edge::Top),
            ),
            (
                CanvasHandle::ResizeEdge(Edge::Top),
                CanvasHandle::Rotate(Corner::TopLeft),
            ),
            (CanvasHandle::Rotate(Corner::TopLeft), CanvasHandle::Body),
        ] {
            assert!(a.priority() > b.priority());
        }
    }

    #[test]
    fn small_selections_hide_handles_by_screen_size() {
        // 검증 기준(XII.10.8): 숨김 경계 fixture(화면 pt 기준이라 줌에 따라 바뀜).
        let count = |kinds: &[CanvasHandle], f: fn(&CanvasHandle) -> bool| {
            kinds.iter().filter(|k| f(k)).count()
        };
        let radius = |k: &CanvasHandle| matches!(k, CanvasHandle::Radius(_));
        let corner = |k: &CanvasHandle| matches!(k, CanvasHandle::ResizeCorner(_));
        let edge = |k: &CanvasHandle| matches!(k, CanvasHandle::ResizeEdge(_));
        let rotate = |k: &CanvasHandle| matches!(k, CanvasHandle::Rotate(_));
        // 40×40 상자: 줌 1에서 짧은 변 40pt.
        let t = [target(1, translate(10., 10.), 40., 40., Some(0.))];
        let cases = [
            // (줌, 반경, 모서리, 변, 회전)
            (1.0, 4, 4, 4, 4),  // 40pt: 모두
            (0.99, 0, 4, 4, 4), // 39.6pt: 반경 숨김
            (0.59, 0, 4, 0, 4), // 23.6pt: 변 핸들 숨김
            (0.19, 0, 0, 0, 4), // 7.6pt: 크기 핸들 숨김
        ];
        for (zoom, r, c, e, rot) in cases {
            let kinds = build_handles(&t, None, &viewport(zoom)).kinds();
            assert_eq!(
                (
                    count(&kinds, radius),
                    count(&kinds, corner),
                    count(&kinds, edge),
                    count(&kinds, rotate)
                ),
                (r, c, e, rot),
                "줌 {zoom}"
            );
        }
        // 경계값은 포함: 24pt에서 변 핸들, 8pt에서 모서리 핸들.
        assert_eq!(
            count(&build_handles(&t, None, &viewport(0.6)).kinds(), edge),
            4
        );
        assert_eq!(
            count(&build_handles(&t, None, &viewport(0.2)).kinds(), corner),
            4
        );
        // 다중 선택은 반경 핸들이 없고 상자는 선택 bounds, 핸들은 primary를 조작.
        let many = [
            target(1, translate(0., 0.), 100., 100., Some(0.)),
            target(2, translate(200., 50.), 100., 100., Some(0.)),
        ];
        let layer = build_handles(&many, Some(sid(1)), &viewport(1.0));
        assert_eq!(count(&layer.kinds(), radius), 0);
        let quad = layer.selection_box().unwrap();
        assert_eq!(
            (quad[0].x(), quad[0].y(), quad[2].x(), quad[2].y()),
            (0., 0., 300., 150.)
        );
        let corner_hit = layer.hit(&viewport(1.0), view_point(300., 150.)).unwrap();
        assert_eq!(
            (corner_hit.handle, corner_hit.node),
            (CanvasHandle::ResizeCorner(Corner::BottomRight), sid(1))
        );
        let body = layer.hit(&viewport(1.0), view_point(250., 100.)).unwrap();
        assert_eq!((body.handle, body.node), (CanvasHandle::Body, sid(2)));
        // 반경이 없는 node(Ellipse·Text 등)는 반경 핸들이 없습니다.
        let plain = build_handles(
            &[target(3, translate(0., 0.), 100., 100., None)],
            None,
            &viewport(1.0),
        );
        assert_eq!(count(&plain.kinds(), radius), 0);
    }

    #[test]
    fn rotated_node_handles_follow_local_axes() {
        // 검증 기준(XII.10.8): 45도 회전 node 핸들 위치.
        let (s, c) = std::f32::consts::FRAC_PI_4.sin_cos();
        let world = Transform2D::new(c, s, -s, c, 400., 100.).unwrap();
        let vp = viewport(1.0);
        let layer = build_handles(&[target(1, world, 100., 100., Some(0.))], None, &vp);
        let quad = layer.selection_box().unwrap();
        let expect = [
            (400., 100.),
            (400. + 100. * c, 100. + 100. * s),
            (400., 100. + 200. * s),
            (400. - 100. * s, 100. + 100. * c),
        ];
        for (p, (x, y)) in quad.iter().zip(expect) {
            assert!(
                (p.x() - x).abs() < 1e-3 && (p.y() - y).abs() < 1e-3,
                "{p:?} vs ({x}, {y})"
            );
        }
        // 회전한 오른쪽 위 모서리 위에서 크기 모서리, AABB 모서리(사각형 밖)는 회전 영역 또는 없음.
        let tr = quad[1];
        assert_eq!(
            hit(&layer, &vp, tr.x(), tr.y()),
            Some(CanvasHandle::ResizeCorner(Corner::TopRight))
        );
        assert!(!matches!(
            hit(&layer, &vp, 400. - 70., 100.),
            Some(CanvasHandle::ResizeCorner(_)) | Some(CanvasHandle::Body)
        ));
        // 반경 핸들은 local (12,12)을 world로 옮긴 점.
        let r = layer.radius_points()[0].unwrap();
        let expected = world.apply(Vec2::new(12., 12.).unwrap()).unwrap();
        assert!((r.x() - expected.x()).abs() < 1e-3 && (r.y() - expected.y()).abs() < 1e-3);
        assert_eq!(
            hit(&layer, &vp, r.x(), r.y()),
            Some(CanvasHandle::Radius(Corner::TopLeft))
        );
        // 변 중점은 local 축 방향 띠.
        let top_mid = world.apply(Vec2::new(50., 0.).unwrap()).unwrap();
        assert_eq!(
            hit(&layer, &vp, top_mid.x(), top_mid.y()),
            Some(CanvasHandle::ResizeEdge(Edge::Top))
        );
    }

    #[test]
    fn handles_come_from_the_document_and_stay_out_of_the_snapshot() {
        // 검증 기준(XII.10.8): overlay가 project snapshot에 없음.
        let mut fixture = nested();
        add_rectangle(&mut fixture.model, 40, 10, 0x40, (500., 50., 120., 80.));
        let root = fixture.root;
        let doc = DocumentState::new(fixture.model);
        let before = save_bytes(&doc);
        let tree = SelectTree::new(doc.model(), root);
        let scene = scene_of(doc.model(), root);
        let hash = scene.snapshot.semantic_hash();
        let count = scene.snapshot.node_count();
        let geometry = CanvasGeometry::from_snapshot(&scene.snapshot);
        let vp = viewport(1.0);
        // Frame 11: 반경 있음(0). Rectangle 40도 반경 있음. Fragment 12는 자손 bounds 상자, 반경 없음.
        let frame = HandleTarget::of(&tree, &geometry, sid(11)).unwrap();
        assert_eq!(frame.radius, Some((0., 0., 0., 0.)));
        let rect = HandleTarget::of(&tree, &geometry, sid(40)).unwrap();
        assert!(rect.radius.is_some());
        let fragment = HandleTarget::of(&tree, &geometry, sid(12)).unwrap();
        assert_eq!(fragment.radius, None);
        assert_eq!(
            (fragment.rect.origin().x(), fragment.rect.size().width()),
            (300., 120.)
        );
        let layer = build_handles(&[rect], None, &vp);
        let quad = layer.selection_box().unwrap();
        assert_eq!(
            (quad[0].x(), quad[0].y(), quad[2].x(), quad[2].y()),
            (500., 50., 620., 130.)
        );
        assert_eq!(
            layer.radius_points().iter().filter(|p| p.is_some()).count(),
            4
        );
        assert_eq!(scene.snapshot.semantic_hash(), hash);
        assert_eq!(scene.snapshot.node_count(), count);
        assert_eq!(save_bytes(&doc), before);
    }
}
