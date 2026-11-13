//! Clip stack과 paint stack (XI.1.2).
//!
//! scene node는 자신에게 적용되는 clip과 paint를 **참조**로 들고 있습니다.
//!
//! - [`ClipStackRef`]: 조상들이 건 clip의 불변 linked list입니다. 자식 scope는 부모 stack에
//!   항목 하나를 덧붙인 새 참조를 만들고, 나머지는 `Arc`로 공유합니다. node는 자신을 자른 모든
//!   clip을 들고 있어, 가시성·hit test가 조상 node를 거슬러 올라가지 않아도 됩니다.
//! - [`PaintStackRef`]: node 내용에 칠하는 fill·stroke layer, 그 아래 drop shadow, node 합성
//!   결과 전체에 적용하는 opacity·blend·layer blur입니다. 같은 paint를 쓰는 node끼리 `Arc`로
//!   공유할 수 있습니다.
//!
//! 두 참조 모두 `Arc` 동일성이 아니라 **값**으로 비교합니다.

use std::fmt;
use std::sync::Arc;

use stellar_core_types::color::Color;
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::value::{FiniteF32, Rect, Transform2D};

use crate::geometry::{FillRule, PathGeometry};
use crate::node::{finite, non_negative, CornerRadii, SceneValueError};

/// clip 하나. clip 좌표계는 `transform`으로 world에 놓입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipShape {
    /// 모서리 둥근 사각형 clip(Frame bounds clip).
    Rect {
        /// clip local 사각형.
        rect: Rect,
        /// 실효 모서리 radius.
        radii: CornerRadii,
        /// clip local → world 변환(clip을 건 node의 world transform).
        transform: Transform2D,
    },
    /// path clip(Frame PathRef clip).
    Path {
        /// clip local path.
        geometry: PathGeometry,
        /// 채우기 규칙.
        fill_rule: FillRule,
        /// clip local → world 변환.
        transform: Transform2D,
    },
}

#[derive(Debug)]
struct ClipEntry {
    parent: ClipStackRef,
    clip: ClipShape,
    depth: usize,
}

/// 불변 clip stack 참조. 비어 있으면 clip이 없습니다.
#[derive(Debug, Clone, Default)]
pub struct ClipStackRef(Option<Arc<ClipEntry>>);

impl ClipStackRef {
    /// 빈 stack(clip 없음).
    pub const fn none() -> ClipStackRef {
        ClipStackRef(None)
    }

    /// `clip`을 가장 안쪽에 덧붙인 새 stack. 기존 항목은 공유합니다.
    pub fn push(&self, clip: ClipShape) -> ClipStackRef {
        ClipStackRef(Some(Arc::new(ClipEntry {
            parent: self.clone(),
            clip,
            depth: self.depth() + 1,
        })))
    }

    /// clip 개수.
    pub fn depth(&self) -> usize {
        self.0.as_ref().map_or(0, |entry| entry.depth)
    }

    /// clip이 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// 가장 안쪽 clip.
    pub fn innermost(&self) -> Option<&ClipShape> {
        self.0.as_ref().map(|entry| &entry.clip)
    }

    /// clip을 안쪽 → 바깥쪽 순으로 순회합니다.
    pub fn iter_inner_first(&self) -> ClipIter<'_> {
        ClipIter {
            next: self.0.as_deref(),
        }
    }

    /// clip을 바깥쪽 → 안쪽 순으로 모읍니다.
    pub fn to_outer_first(&self) -> Vec<&ClipShape> {
        let mut clips: Vec<&ClipShape> = self.iter_inner_first().collect();
        clips.reverse();
        clips
    }

    /// 같은 저장소를 가리키면 참입니다(구조 공유 확인용).
    pub fn ptr_eq(&self, other: &ClipStackRef) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl PartialEq for ClipStackRef {
    fn eq(&self, other: &ClipStackRef) -> bool {
        self.ptr_eq(other)
            || (self.depth() == other.depth()
                && self
                    .iter_inner_first()
                    .zip(other.iter_inner_first())
                    .all(|(a, b)| a == b))
    }
}

impl Eq for ClipStackRef {}

/// [`ClipStackRef::iter_inner_first`] 반복자.
#[derive(Debug)]
pub struct ClipIter<'a> {
    next: Option<&'a ClipEntry>,
}

impl<'a> Iterator for ClipIter<'a> {
    type Item = &'a ClipShape;

    fn next(&mut self) -> Option<&'a ClipShape> {
        let entry = self.next?;
        self.next = entry.parent.0.as_deref();
        Some(&entry.clip)
    }
}

/// 칠하기 원천.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Paint {
    /// 단색(색공간 포함, 비연관 알파).
    Solid(Color),
}

/// stroke를 geometry 경계 기준 어디에 놓을지.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StrokeAlign {
    /// 경계 중심선 양쪽으로 폭의 절반씩(Shape·Path 기본, document Line 중심선 규칙과 같음).
    #[default]
    Center,
    /// 경계 안쪽(Frame 테두리: 크기 상자 안에 그림).
    Inside,
    /// 경계 바깥쪽.
    Outside,
}

impl StrokeAlign {
    /// 안정 numeric tag(Center=0, Inside=1, Outside=2).
    pub const fn tag(self) -> u8 {
        match self {
            StrokeAlign::Center => 0,
            StrokeAlign::Inside => 1,
            StrokeAlign::Outside => 2,
        }
    }

    /// tag에서 복원합니다. 미지 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<StrokeAlign> {
        match tag {
            0 => Some(StrokeAlign::Center),
            1 => Some(StrokeAlign::Inside),
            2 => Some(StrokeAlign::Outside),
            _ => None,
        }
    }
}

/// 열린 subpath 끝 모양. document `LineCap`과 같은 의미·tag입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StrokeCap {
    /// 끝점에서 자름(기본).
    #[default]
    Butt,
    /// 반원 끝.
    Round,
    /// 반폭만큼 연장한 사각 끝.
    Square,
}

impl StrokeCap {
    /// 안정 numeric tag(Butt=0, Round=1, Square=2).
    pub const fn tag(self) -> u8 {
        match self {
            StrokeCap::Butt => 0,
            StrokeCap::Round => 1,
            StrokeCap::Square => 2,
        }
    }

    /// tag에서 복원합니다. 미지 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<StrokeCap> {
        match tag {
            0 => Some(StrokeCap::Butt),
            1 => Some(StrokeCap::Round),
            2 => Some(StrokeCap::Square),
            _ => None,
        }
    }
}

/// stroke layer. join·dash는 stroke tessellation(XI.6.2)이 정의합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StrokeLayer {
    paint: Paint,
    width: FiniteF32,
    align: StrokeAlign,
    cap: StrokeCap,
}

impl StrokeLayer {
    /// finite·비음수 폭의 stroke를 만듭니다.
    pub fn new(
        paint: Paint,
        width: f32,
        align: StrokeAlign,
        cap: StrokeCap,
    ) -> Result<StrokeLayer, SceneValueError> {
        Ok(StrokeLayer {
            paint,
            width: non_negative(width, "stroke width")?,
            align,
            cap,
        })
    }

    /// 칠하기 원천.
    pub const fn paint(&self) -> Paint {
        self.paint
    }

    /// 폭.
    pub const fn width(&self) -> f32 {
        self.width.get()
    }

    /// 정렬.
    pub const fn align(&self) -> StrokeAlign {
        self.align
    }

    /// 끝 모양.
    pub const fn cap(&self) -> StrokeCap {
        self.cap
    }

    /// geometry 경계 밖으로 나가는 최대 거리(culling·hit test bounds 확장용).
    ///
    /// join은 아직 정의되지 않아(XI.6.2) miter join의 뾰족한 확장은 반영하지 않습니다.
    pub fn outset(&self) -> f32 {
        let width = self.width.get();
        let along_edge = match self.align {
            StrokeAlign::Center => width / 2.0,
            StrokeAlign::Inside => 0.0,
            StrokeAlign::Outside => width,
        };
        // Square cap은 끝점 너머로 반폭만큼 연장되므로 대각 방향으로 반폭의 √2배까지 나갑니다.
        match self.cap {
            StrokeCap::Square => along_edge.max(width / 2.0 * std::f32::consts::SQRT_2),
            StrokeCap::Butt | StrokeCap::Round => along_edge,
        }
    }
}

/// node 내용에 칠하는 layer 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaintLayer {
    /// 채우기.
    Fill(Paint),
    /// 선.
    Stroke(StrokeLayer),
}

/// blend mode. document `BlendMode`와 같은 의미·tag입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SceneBlendMode {
    /// 일반(기본).
    #[default]
    Normal,
    /// 곱하기.
    Multiply,
    /// 스크린.
    Screen,
    /// 오버레이.
    Overlay,
    /// 어둡게.
    Darken,
    /// 밝게.
    Lighten,
}

impl SceneBlendMode {
    /// 정의된 모든 mode.
    pub const ALL: [SceneBlendMode; 6] = [
        SceneBlendMode::Normal,
        SceneBlendMode::Multiply,
        SceneBlendMode::Screen,
        SceneBlendMode::Overlay,
        SceneBlendMode::Darken,
        SceneBlendMode::Lighten,
    ];

    /// 안정 numeric tag(document `BlendMode`와 동일: Normal=0 … Lighten=5).
    pub const fn tag(self) -> u8 {
        match self {
            SceneBlendMode::Normal => 0,
            SceneBlendMode::Multiply => 1,
            SceneBlendMode::Screen => 2,
            SceneBlendMode::Overlay => 3,
            SceneBlendMode::Darken => 4,
            SceneBlendMode::Lighten => 5,
        }
    }

    /// tag에서 복원합니다. 미지 tag는 `None`.
    pub fn from_tag(tag: u8) -> Option<SceneBlendMode> {
        SceneBlendMode::ALL
            .into_iter()
            .find(|mode| mode.tag() == tag)
    }
}

/// 불투명도(`[0, 1]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Opacity(FiniteF32);

impl Opacity {
    /// 완전 불투명.
    pub const OPAQUE: Opacity = Opacity(FiniteF32::ONE);

    /// `[0, 1]` 범위 불투명도를 만듭니다.
    pub fn new(value: f32) -> Result<Opacity, SceneValueError> {
        let v = finite(value, "opacity")?;
        if v.get() < 0.0 || v.get() > 1.0 {
            return Err(SceneValueError::OutOfRange { field: "opacity" });
        }
        Ok(Opacity(v))
    }

    /// 값.
    pub const fn get(&self) -> f32 {
        self.0.get()
    }
}

impl Default for Opacity {
    fn default() -> Opacity {
        Opacity::OPAQUE
    }
}

/// node의 paint stack.
///
/// - `layers`: node 내용에 아래 → 위 순서로 칠합니다.
/// - `shadows`: 모든 layer **아래**에 목록 순서대로 그리는 drop shadow입니다.
/// - `opacity`·`blend`·`layer_blur`: node와 그 subtree를 합성한 결과 전체에 적용합니다
///   (offscreen 경계 판단은 render graph, XI.4.4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PaintStack {
    /// fill·stroke layer(아래 → 위).
    pub layers: Vec<PaintLayer>,
    /// drop shadow.
    pub shadows: Vec<Shadow>,
    /// 합성 불투명도.
    pub opacity: Opacity,
    /// 합성 blend mode.
    pub blend: SceneBlendMode,
    /// 합성 결과 blur 반경(0이면 없음).
    pub layer_blur: BlurRadius,
}

impl PaintStack {
    /// 아무것도 칠하지 않는 불투명·Normal stack.
    pub fn empty() -> PaintStack {
        PaintStack {
            layers: Vec::new(),
            shadows: Vec::new(),
            opacity: Opacity::OPAQUE,
            blend: SceneBlendMode::Normal,
            layer_blur: BlurRadius::ZERO,
        }
    }

    /// 칠하는 layer와 shadow가 모두 없으면 참입니다(합성 효과는 별개).
    pub fn paints_nothing(&self) -> bool {
        self.layers.is_empty() && self.shadows.is_empty()
    }
}

impl Default for PaintStack {
    fn default() -> PaintStack {
        PaintStack::empty()
    }
}

/// 공유 가능한 불변 paint stack 참조.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PaintStackRef(Arc<PaintStack>);

impl PaintStackRef {
    /// stack을 참조로 감쌉니다.
    pub fn new(stack: PaintStack) -> PaintStackRef {
        PaintStackRef(Arc::new(stack))
    }

    /// 빈 stack 참조.
    pub fn empty() -> PaintStackRef {
        PaintStackRef::new(PaintStack::empty())
    }

    /// stack.
    pub fn get(&self) -> &PaintStack {
        &self.0
    }

    /// 같은 저장소를 가리키면 참입니다(구조 공유 확인용).
    pub fn ptr_eq(&self, other: &PaintStackRef) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Default for PaintStackRef {
    fn default() -> PaintStackRef {
        PaintStackRef::empty()
    }
}

impl fmt::Display for SceneBlendMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SceneBlendMode::Normal => "normal",
            SceneBlendMode::Multiply => "multiply",
            SceneBlendMode::Screen => "screen",
            SceneBlendMode::Overlay => "overlay",
            SceneBlendMode::Darken => "darken",
            SceneBlendMode::Lighten => "lighten",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::ColorSpace;
    use stellar_core_types::value::{Size2, Vec2};

    fn rect_clip(w: f32) -> ClipShape {
        ClipShape::Rect {
            rect: Rect::new(Vec2::ZERO, Size2::new(w, w).unwrap()),
            radii: CornerRadii::ZERO,
            transform: Transform2D::identity(),
        }
    }

    #[test]
    fn clip_stack_shares_parent_entries() {
        let outer = ClipStackRef::none().push(rect_clip(100.0));
        let a = outer.push(rect_clip(10.0));
        let b = outer.push(rect_clip(20.0));
        assert_eq!(outer.depth(), 1);
        assert_eq!(a.depth(), 2);
        assert_eq!(
            a.to_outer_first(),
            vec![&rect_clip(100.0), &rect_clip(10.0)]
        );
        assert_eq!(a.innermost(), Some(&rect_clip(10.0)));
        assert_ne!(a, b);
        // 독립 생성한 같은 값은 같음(값 비교), 저장소는 다름.
        let a2 = ClipStackRef::none()
            .push(rect_clip(100.0))
            .push(rect_clip(10.0));
        assert_eq!(a, a2);
        assert!(!a.ptr_eq(&a2));
        assert!(ClipStackRef::none().is_empty());
        assert_eq!(ClipStackRef::none(), ClipStackRef::default());
    }

    #[test]
    fn paint_values_are_validated() {
        let red = Paint::Solid(Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap());
        assert_eq!(
            StrokeLayer::new(red, -1.0, StrokeAlign::Center, StrokeCap::Butt),
            Err(SceneValueError::Negative {
                field: "stroke width"
            })
        );
        assert_eq!(
            Opacity::new(1.5),
            Err(SceneValueError::OutOfRange { field: "opacity" })
        );
        assert_eq!(
            Opacity::new(f32::NAN),
            Err(SceneValueError::NonFinite { field: "opacity" })
        );
        assert_eq!(Opacity::new(0.25).unwrap().get(), 0.25);
        let center = StrokeLayer::new(red, 4.0, StrokeAlign::Center, StrokeCap::Butt).unwrap();
        let inside = StrokeLayer::new(red, 4.0, StrokeAlign::Inside, StrokeCap::Butt).unwrap();
        let outside = StrokeLayer::new(red, 4.0, StrokeAlign::Outside, StrokeCap::Round).unwrap();
        let square = StrokeLayer::new(red, 4.0, StrokeAlign::Center, StrokeCap::Square).unwrap();
        assert_eq!(center.outset(), 2.0);
        assert_eq!(inside.outset(), 0.0);
        assert_eq!(outside.outset(), 4.0);
        assert!(square.outset() > 2.0 && square.outset() < 3.0);
        assert!(PaintStack::empty().paints_nothing());
        assert_eq!(PaintStackRef::empty(), PaintStackRef::default());
    }

    #[test]
    fn enum_tags_match_document_and_round_trip() {
        // document BlendMode tag(Normal=0 … Lighten=5), LineCap tag(Butt=0, Round=1, Square=2).
        for (index, mode) in SceneBlendMode::ALL.into_iter().enumerate() {
            assert_eq!(mode.tag() as usize, index);
            assert_eq!(SceneBlendMode::from_tag(mode.tag()), Some(mode));
        }
        assert_eq!(SceneBlendMode::from_tag(6), None);
        for (cap, tag) in [
            (StrokeCap::Butt, 0),
            (StrokeCap::Round, 1),
            (StrokeCap::Square, 2),
        ] {
            assert_eq!(cap.tag(), tag);
            assert_eq!(StrokeCap::from_tag(tag), Some(cap));
        }
        for (align, tag) in [
            (StrokeAlign::Center, 0),
            (StrokeAlign::Inside, 1),
            (StrokeAlign::Outside, 2),
        ] {
            assert_eq!(align.tag(), tag);
            assert_eq!(StrokeAlign::from_tag(tag), Some(align));
        }
        assert_eq!(StrokeCap::from_tag(3), None);
        assert_eq!(StrokeAlign::from_tag(3), None);
        assert_eq!(SceneBlendMode::Overlay.to_string(), "overlay");
    }
}
