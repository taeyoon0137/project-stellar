//! 축 정렬 bounds와 scene node world bounds (XI.3.1).
//!
//! - [`Aabb`]는 finite·`min ≤ max`인 닫힌 축 정렬 사각형입니다. 교차·포함은 경계를 포함합니다
//!   (닿기만 해도 교차).
//! - [`SceneNode::local_hit_bounds`]: layout 상자와 content geometry의 합집합을 stroke가 경계 밖으로
//!   나가는 거리만큼 넓힌 node local bounds입니다. hit test(XI.3.3)의 기준입니다.
//! - [`SceneNode::local_render_bounds`]: hit bounds에 drop shadow(offset + blur 반경)와 layer blur
//!   반경을 더한 그리기 영역입니다. culling(XI.3.2)과 공간 인덱스의 기준입니다.
//! - world bounds는 local bounds 네 꼭짓점을 world transform으로 옮긴 AABB입니다.
//!
//! path geometry bounds는 control point의 볼록 껍질 bounds를 씁니다. 곡선은 항상 control point
//! 볼록 껍질 안에 있으므로 정확한 bounds(document VI.6.4)보다 크거나 같아 culling·hit test 후보
//! 판정에 안전합니다(보수적).

use stellar_core_types::value::{Rect, Transform2D, Vec2};

use crate::geometry::{PathCommand, PathGeometry};
use crate::node::{SceneContent, SceneNode};
use crate::stack::PaintLayer;

/// 닫힌 축 정렬 사각형(finite, `min ≤ max`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

/// 유한 범위로 saturate합니다(오버플로한 ±무한대만 들어옵니다).
fn saturate(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else if value > 0.0 {
        f32::MAX
    } else {
        f32::MIN
    }
}

impl Aabb {
    /// 네 경계로 만듭니다. 비유한이거나 `min > max`면 `None`.
    pub fn new(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Option<Aabb> {
        let finite = [min_x, min_y, max_x, max_y].iter().all(|v| v.is_finite());
        if !finite || min_x > max_x || min_y > max_y {
            return None;
        }
        // -0.0을 0.0으로 정규화해 비교·hash 결과를 고정합니다.
        let n = |v: f32| if v == 0.0 { 0.0 } else { v };
        Some(Aabb {
            min_x: n(min_x),
            min_y: n(min_y),
            max_x: n(max_x),
            max_y: n(max_y),
        })
    }

    /// 한 점.
    pub fn point(point: Vec2) -> Aabb {
        Aabb {
            min_x: point.x(),
            min_y: point.y(),
            max_x: point.x(),
            max_y: point.y(),
        }
    }

    /// core [`Rect`]. 끝 좌표가 오버플로하면 유한 범위로 saturate합니다.
    pub fn from_rect(rect: Rect) -> Aabb {
        let (x, y) = (rect.origin().x(), rect.origin().y());
        Aabb {
            min_x: x,
            min_y: y,
            max_x: saturate(x + rect.size().width()),
            max_y: saturate(y + rect.size().height()),
        }
    }

    /// 점들의 bounds. 점이 없으면 `None`.
    pub fn from_points(points: impl IntoIterator<Item = Vec2>) -> Option<Aabb> {
        points.into_iter().fold(None, |acc: Option<Aabb>, point| {
            let p = Aabb::point(point);
            Some(acc.map_or(p, |a| a.union(&p)))
        })
    }

    /// 최소 x.
    pub const fn min_x(&self) -> f32 {
        self.min_x
    }

    /// 최소 y.
    pub const fn min_y(&self) -> f32 {
        self.min_y
    }

    /// 최대 x.
    pub const fn max_x(&self) -> f32 {
        self.max_x
    }

    /// 최대 y.
    pub const fn max_y(&self) -> f32 {
        self.max_y
    }

    /// 너비(saturate).
    pub fn width(&self) -> f32 {
        saturate(self.max_x - self.min_x)
    }

    /// 높이(saturate).
    pub fn height(&self) -> f32 {
        saturate(self.max_y - self.min_y)
    }

    /// 합집합.
    pub fn union(&self, other: &Aabb) -> Aabb {
        Aabb {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    /// 교집합. 겹치지 않으면 `None`(경계만 닿으면 폭 0 bounds).
    pub fn intersection(&self, other: &Aabb) -> Option<Aabb> {
        Aabb::new(
            self.min_x.max(other.min_x),
            self.min_y.max(other.min_y),
            self.max_x.min(other.max_x),
            self.max_y.min(other.max_y),
        )
    }

    /// 겹치거나 닿으면 참입니다(닫힌 구간).
    pub fn intersects(&self, other: &Aabb) -> bool {
        self.min_x <= other.max_x
            && other.min_x <= self.max_x
            && self.min_y <= other.max_y
            && other.min_y <= self.max_y
    }

    /// 점이 안쪽이거나 경계 위면 참입니다.
    pub fn contains_point(&self, point: Vec2) -> bool {
        self.min_x <= point.x()
            && point.x() <= self.max_x
            && self.min_y <= point.y()
            && point.y() <= self.max_y
    }

    /// 사방으로 `amount`(비음수)만큼 넓힙니다(saturate).
    pub fn expanded(&self, amount: f32) -> Aabb {
        let d = if amount.is_finite() && amount > 0.0 {
            amount
        } else {
            0.0
        };
        Aabb {
            min_x: saturate(self.min_x - d),
            min_y: saturate(self.min_y - d),
            max_x: saturate(self.max_x + d),
            max_y: saturate(self.max_y + d),
        }
    }

    /// `(dx, dy)`만큼 옮깁니다(saturate).
    pub fn translated(&self, dx: f32, dy: f32) -> Aabb {
        Aabb {
            min_x: saturate(self.min_x + dx),
            min_y: saturate(self.min_y + dy),
            max_x: saturate(self.max_x + dx),
            max_y: saturate(self.max_y + dy),
        }
    }

    /// 네 꼭짓점을 `transform`으로 옮긴 AABB. 결과가 비유한이면 `None`.
    pub fn transformed(&self, transform: &Transform2D) -> Option<Aabb> {
        let corners = [
            (self.min_x, self.min_y),
            (self.max_x, self.min_y),
            (self.min_x, self.max_y),
            (self.max_x, self.max_y),
        ];
        let mut points = Vec::with_capacity(4);
        for (x, y) in corners {
            let point = Vec2::new(x, y).ok()?;
            points.push(transform.apply(point).ok()?);
        }
        Aabb::from_points(points)
    }
}

/// path geometry의 control point 볼록 껍질 bounds(보수적). 빈 path는 `None`.
pub fn path_control_bounds(geometry: &PathGeometry) -> Option<Aabb> {
    Aabb::from_points(geometry.commands().iter().flat_map(|command| {
        let points: [Option<Vec2>; 3] = match *command {
            PathCommand::MoveTo(p) | PathCommand::LineTo(p) => [Some(p), None, None],
            PathCommand::CubicTo { c1, c2, to } => [Some(c1), Some(c2), Some(to)],
            PathCommand::Close => [None, None, None],
        };
        points.into_iter().flatten()
    }))
}

impl SceneNode {
    /// content geometry의 local bounds(stroke·effect 제외).
    pub fn local_content_bounds(&self) -> Option<Aabb> {
        match self.content() {
            SceneContent::Rect(rect) => Some(Aabb::from_rect(rect.rect)),
            SceneContent::Path(path) => path_control_bounds(&path.geometry),
            SceneContent::Text(text) => Some(Aabb::from_rect(text.bounds)),
            SceneContent::Image(image) => Some(Aabb::from_rect(image.bounds)),
        }
    }

    /// hit test 기준 local bounds: layout 상자 ∪ content bounds, stroke outset 포함.
    pub fn local_hit_bounds(&self) -> Aabb {
        let layout = Aabb::from_rect(self.layout_rect());
        let base = self
            .local_content_bounds()
            .map_or(layout, |content| content.union(&layout));
        let outset = self
            .paint()
            .get()
            .layers
            .iter()
            .filter_map(|layer| match layer {
                PaintLayer::Stroke(stroke) => Some(stroke.outset()),
                PaintLayer::Fill(_) => None,
            })
            .fold(0.0f32, f32::max);
        base.expanded(outset)
    }

    /// 그리기 영역 local bounds: hit bounds ∪ drop shadow 영역, layer blur 반경 포함.
    pub fn local_render_bounds(&self) -> Aabb {
        let hit = self.local_hit_bounds();
        let paint = self.paint().get();
        let with_shadows = paint.shadows.iter().fold(hit, |acc, shadow| {
            let offset = shadow.offset();
            acc.union(
                &hit.translated(offset.x(), offset.y())
                    .expanded(shadow.blur().get()),
            )
        });
        with_shadows.expanded(paint.layer_blur.get())
    }

    /// world 좌표의 hit bounds. world transform 적용이 오버플로하면 `None`.
    pub fn world_hit_bounds(&self) -> Option<Aabb> {
        self.local_hit_bounds().transformed(&self.world_transform())
    }

    /// world 좌표의 그리기 영역. world transform 적용이 오버플로하면 `None`.
    pub fn world_render_bounds(&self) -> Option<Aabb> {
        self.local_render_bounds()
            .transformed(&self.world_transform())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{p, rect, solid, spec, translate};
    use crate::node::{PathPrimitive, SceneContent};
    use crate::stack::{PaintStack, PaintStackRef, StrokeAlign, StrokeCap, StrokeLayer};
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::shadow::{BlurRadius, Shadow};

    fn aabb(a: f32, b: f32, c: f32, d: f32) -> Aabb {
        Aabb::new(a, b, c, d).expect("aabb")
    }

    #[test]
    fn aabb_rules() {
        assert!(Aabb::new(1.0, 0.0, 0.0, 1.0).is_none());
        assert!(Aabb::new(f32::NAN, 0.0, 1.0, 1.0).is_none());
        let a = aabb(0.0, 0.0, 10.0, 10.0);
        // 닫힌 구간: 경계만 닿아도 교차·포함.
        assert!(a.intersects(&aabb(10.0, 10.0, 20.0, 20.0)));
        assert!(!a.intersects(&aabb(10.5, 0.0, 20.0, 1.0)));
        assert!(a.contains_point(p(10.0, 0.0)));
        assert!(!a.contains_point(p(10.01, 0.0)));
        assert_eq!(
            a.intersection(&aabb(5.0, -5.0, 15.0, 5.0)),
            Some(aabb(5.0, 0.0, 10.0, 5.0))
        );
        assert_eq!(a.intersection(&aabb(11.0, 0.0, 12.0, 1.0)), None);
        assert_eq!(
            a.union(&aabb(-1.0, 2.0, 3.0, 12.0)),
            aabb(-1.0, 0.0, 10.0, 12.0)
        );
        assert_eq!(a.expanded(2.0), aabb(-2.0, -2.0, 12.0, 12.0));
        assert_eq!(a.expanded(-1.0), a);
        // 90도 회전 후 AABB.
        let rotate = Transform2D::new(0.0, 1.0, -1.0, 0.0, 0.0, 0.0).unwrap();
        assert_eq!(
            aabb(0.0, 0.0, 4.0, 2.0).transformed(&rotate),
            Some(aabb(-2.0, 0.0, 0.0, 4.0))
        );
        let huge = Transform2D::new(f32::MAX, 0.0, 0.0, 1.0, 0.0, 0.0).unwrap();
        assert_eq!(aabb(0.0, 0.0, 4.0, 1.0).transformed(&huge), None);
        // Rect 끝 좌표 오버플로는 saturate.
        let big = Aabb::from_rect(rect(f32::MAX, 0.0, f32::MAX, 1.0));
        assert_eq!(big.max_x(), f32::MAX);
        assert_eq!(
            Aabb::new(-0.0, 0.0, 0.0, 0.0),
            Some(aabb(0.0, 0.0, 0.0, 0.0))
        );
    }

    #[test]
    fn path_bounds_use_control_point_hull() {
        let geometry = PathGeometry::new(vec![
            PathCommand::MoveTo(p(0.0, 0.0)),
            PathCommand::CubicTo {
                c1: p(0.0, 10.0),
                c2: p(10.0, 10.0),
                to: p(10.0, 0.0),
            },
            PathCommand::Close,
        ])
        .unwrap();
        // 곡선 실제 최대 y는 7.5지만 control point 껍질은 10까지(보수적).
        assert_eq!(
            path_control_bounds(&geometry),
            Some(aabb(0.0, 0.0, 10.0, 10.0))
        );
        assert_eq!(path_control_bounds(&PathGeometry::empty()), None);
    }

    #[test]
    fn node_bounds_include_stroke_shadow_and_blur() {
        let mut s = spec(1, crate::fixtures::rect_content(10.0, 10.0), 1);
        s.world_transform = translate(100.0, 0.0);
        s.paint = PaintStackRef::new(PaintStack {
            layers: vec![crate::stack::PaintLayer::Stroke(
                StrokeLayer::new(
                    solid(0.0, 0.0, 0.0, 1.0),
                    4.0,
                    StrokeAlign::Center,
                    StrokeCap::Butt,
                )
                .unwrap(),
            )],
            shadows: vec![Shadow::new(
                p(5.0, 0.0),
                BlurRadius::new(1.0).unwrap(),
                Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap(),
            )],
            layer_blur: BlurRadius::new(0.5).unwrap(),
            ..PaintStack::empty()
        });
        let node = SceneNode::leaf(s);
        assert_eq!(
            node.local_content_bounds(),
            Some(aabb(0.0, 0.0, 10.0, 10.0))
        );
        // stroke 중심 정렬 → 반폭 2.
        assert_eq!(node.local_hit_bounds(), aabb(-2.0, -2.0, 12.0, 12.0));
        // shadow: hit을 (5,0) 이동 + blur 1 → (2,-3)~(18,13); 합집합 후 layer blur 0.5.
        assert_eq!(node.local_render_bounds(), aabb(-2.5, -3.5, 18.5, 13.5));
        assert_eq!(node.world_hit_bounds(), Some(aabb(98.0, -2.0, 112.0, 12.0)));
        assert_eq!(
            node.world_render_bounds(),
            Some(aabb(97.5, -3.5, 118.5, 13.5))
        );

        // path는 layout 상자와 control 껍질의 합집합.
        let mut path_spec = spec(
            2,
            SceneContent::Path(PathPrimitive {
                geometry: PathGeometry::new(vec![
                    PathCommand::MoveTo(p(-5.0, 0.0)),
                    PathCommand::LineTo(p(3.0, 4.0)),
                ])
                .unwrap(),
                fill_rule: crate::geometry::FillRule::NonZero,
            }),
            2,
        );
        path_spec.layout_rect = rect(0.0, 0.0, 2.0, 8.0);
        let path = SceneNode::leaf(path_spec);
        assert_eq!(path.local_hit_bounds(), aabb(-5.0, 0.0, 3.0, 8.0));
    }
}
