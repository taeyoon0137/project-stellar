//! Scene node와 resource 참조 (XI.1.1).
//!
//! Scene은 document 계층이 binding·layout 평가를 끝낸 결과를 **값으로** 담는 불변 모델입니다.
//!
//! - **Document Node를 담지 않습니다.** arena handle, payload, modifier stack 같은 document
//!   객체는 scene에 들어오지 않고, 원본 node는 [`StableId`] 값([`SceneNode::source`])으로만
//!   가리킵니다. crate 의존 정책(II.2.8)상 scene은 document를 import할 수도 없습니다.
//! - **GPU 객체를 담지 않습니다.** buffer·texture·pipeline·native pointer 대신, 별도 cache가
//!   해석하는 데이터(이미지 픽셀, text layout 결과)는 [`SceneResourceRef`] 식별자로만
//!   가리킵니다. GPU resource 수명은 renderer backend 소관입니다(아키텍처 §10.1).
//! - 모든 float은 finite 전용 타입을 지나므로 NaN·±Infinity가 scene에 들어올 수 없습니다.
//! - node는 생성 후 바뀌지 않습니다. 자식은 `Arc`로 보관해 snapshot 간에 공유할 수 있습니다.
//!
//! ## node가 들고 있는 평가 결과 (XI.1.2)
//!
//! | field | 의미 |
//! |---|---|
//! | [`SceneNode::world_transform`] | node local → world 변환(조상 변환을 모두 합성한 값) |
//! | [`SceneNode::layout_rect`] | node local 좌표계의 layout 상자 |
//! | [`SceneNode::clip`] | 이 node를 자르는 조상 clip 전체([`ClipStackRef`]) |
//! | [`SceneNode::paint`] | fill·stroke·shadow·합성 효과([`PaintStackRef`]) |
//! | [`SceneNode::z`] | 전역 paint 순서 key([`ZOrder`]) |
//! | [`SceneNode::hit`] | 실효 visibility·pointer-events([`HitTestMeta`]) |
//!
//! **z-order 불변식**: snapshot 전체를 pre-order(부모 → 자식, 자식은 paint 순서)로 걸으면
//! [`ZOrder`]가 엄격히 증가합니다. 값이 연속일 필요는 없습니다. [`SceneNode::new`]가 직접
//! 자식 사이의 순서를, [`SceneSnapshot::new`](crate::snapshot::SceneSnapshot::new)가 최상위
//! node 사이의 순서를 검사하므로, 생성된 모든 tree에서 불변식이 지역 검사만으로 성립합니다.

use std::fmt;
use std::sync::Arc;

use stellar_core_types::color::ColorSpace;
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::value::{FiniteF32, Rect, Transform2D};

use crate::geometry::{FillRule, PathGeometry};
use crate::persistent::PersistentVec;
use crate::stack::{ClipStackRef, PaintStackRef};

/// scene 값 생성 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneValueError {
    /// 비유한 값(NaN·±무한대).
    NonFinite {
        /// 문제 field.
        field: &'static str,
    },
    /// 음수일 수 없는 값이 음수입니다.
    Negative {
        /// 문제 field.
        field: &'static str,
    },
    /// 허용 범위를 벗어났습니다.
    OutOfRange {
        /// 문제 field.
        field: &'static str,
    },
}

impl fmt::Display for SceneValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneValueError::NonFinite { field } => write!(f, "{field} 값이 유한하지 않습니다"),
            SceneValueError::Negative { field } => write!(f, "{field} 값은 음수일 수 없습니다"),
            SceneValueError::OutOfRange { field } => {
                write!(f, "{field} 값이 허용 범위를 벗어났습니다")
            }
        }
    }
}

impl std::error::Error for SceneValueError {}

/// finite 값을 만듭니다.
pub(crate) fn finite(value: f32, field: &'static str) -> Result<FiniteF32, SceneValueError> {
    FiniteF32::new(value).map_err(|_| SceneValueError::NonFinite { field })
}

/// finite·비음수 값을 만듭니다.
pub(crate) fn non_negative(value: f32, field: &'static str) -> Result<FiniteF32, SceneValueError> {
    let v = finite(value, field)?;
    if v.get() < 0.0 {
        return Err(SceneValueError::Negative { field });
    }
    Ok(v)
}

/// text layout 결과 식별자.
///
/// shaping·line layout 결과(glyph run 묶음)는 text layout cache(X.10.5)가 소유하고, scene은
/// 그 cache가 발급한 16바이트 key만 들고 있습니다. key의 의미(입력 hash 구성)는 text 계층이
/// 정하며 scene은 비교·전달만 합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextLayoutKey([u8; 16]);

impl TextLayoutKey {
    /// 16바이트로부터 만듭니다.
    pub const fn from_bytes(bytes: [u8; 16]) -> TextLayoutKey {
        TextLayoutKey(bytes)
    }

    /// 16바이트 표현.
    pub const fn to_bytes(&self) -> [u8; 16] {
        self.0
    }
}

/// scene이 값으로 들고 있지 않고 별도 cache가 해석하는 resource의 식별자.
///
/// GPU 객체가 아니라 **값 식별자**입니다. renderer는 이 참조로 자기 cache(texture, glyph
/// atlas 등)를 조회하고, 없으면 placeholder 정책(XI.7.4)을 적용합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SceneResourceRef {
    /// 이미지 asset. decode·texture 생성은 renderer cache(XI.7)가 담당합니다.
    Image(AssetRef),
    /// text layout 결과. glyph rasterization·atlas는 renderer cache(XI.6.4)가 담당합니다.
    TextLayout(TextLayoutKey),
}

/// 개별 모서리 radius(finite·비음수).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CornerRadii {
    top_left: FiniteF32,
    top_right: FiniteF32,
    bottom_right: FiniteF32,
    bottom_left: FiniteF32,
}

impl CornerRadii {
    /// 모두 0인 radius(직각).
    pub const ZERO: CornerRadii = CornerRadii {
        top_left: FiniteF32::ZERO,
        top_right: FiniteF32::ZERO,
        bottom_right: FiniteF32::ZERO,
        bottom_left: FiniteF32::ZERO,
    };

    /// finite·비음수 radius를 만듭니다.
    ///
    /// scene에는 **평가가 끝난 실효 radius**(document `effective_corner_radius`처럼 변 길이에
    /// 맞춰 줄인 값)를 넣습니다.
    pub fn new(
        top_left: f32,
        top_right: f32,
        bottom_right: f32,
        bottom_left: f32,
    ) -> Result<CornerRadii, SceneValueError> {
        Ok(CornerRadii {
            top_left: non_negative(top_left, "corner radius")?,
            top_right: non_negative(top_right, "corner radius")?,
            bottom_right: non_negative(bottom_right, "corner radius")?,
            bottom_left: non_negative(bottom_left, "corner radius")?,
        })
    }

    /// (top_left, top_right, bottom_right, bottom_left).
    pub const fn components(&self) -> (f32, f32, f32, f32) {
        (
            self.top_left.get(),
            self.top_right.get(),
            self.bottom_right.get(),
            self.bottom_left.get(),
        )
    }

    /// 모든 radius가 0이면 참입니다.
    pub fn is_zero(&self) -> bool {
        *self == CornerRadii::ZERO
    }
}

impl Default for CornerRadii {
    fn default() -> CornerRadii {
        CornerRadii::ZERO
    }
}

/// 이미지를 상자에 배치하는 방식. document `FitMode`와 같은 의미·tag입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImageFit {
    /// 비율을 무시하고 상자를 채웁니다.
    Fill,
    /// 비율 유지, 전체가 보이게(기본).
    #[default]
    Contain,
    /// 비율 유지, 상자를 덮게(넘치는 부분은 상자로 잘림).
    Cover,
    /// 원본 크기.
    None,
}

impl ImageFit {
    /// 안정 numeric tag(document `FitMode`와 동일: Fill=0, Contain=1, Cover=2, None=3).
    pub const fn tag(self) -> u8 {
        match self {
            ImageFit::Fill => 0,
            ImageFit::Contain => 1,
            ImageFit::Cover => 2,
            ImageFit::None => 3,
        }
    }

    /// tag에서 복원합니다. 미지 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<ImageFit> {
        match tag {
            0 => Some(ImageFit::Fill),
            1 => Some(ImageFit::Contain),
            2 => Some(ImageFit::Cover),
            3 => Some(ImageFit::None),
            _ => None,
        }
    }
}

/// 이미지 확대·축소 sampling. document `SamplingMode`와 같은 의미·tag입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImageSampling {
    /// 선형 보간(기본).
    #[default]
    Linear,
    /// 최근접.
    Nearest,
}

impl ImageSampling {
    /// 안정 numeric tag(Linear=0, Nearest=1).
    pub const fn tag(self) -> u8 {
        match self {
            ImageSampling::Linear => 0,
            ImageSampling::Nearest => 1,
        }
    }

    /// tag에서 복원합니다. 미지 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<ImageSampling> {
        match tag {
            0 => Some(ImageSampling::Linear),
            1 => Some(ImageSampling::Nearest),
            _ => None,
        }
    }
}

/// 원본 이미지 기준 정규화(0..1) crop 영역. document `CropRect` 저장값과 같은 규칙입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageCrop {
    x: FiniteF32,
    y: FiniteF32,
    width: FiniteF32,
    height: FiniteF32,
}

impl ImageCrop {
    /// 원본 전체(crop 없음).
    pub const FULL: ImageCrop = ImageCrop {
        x: FiniteF32::ZERO,
        y: FiniteF32::ZERO,
        width: FiniteF32::ONE,
        height: FiniteF32::ONE,
    };

    /// `x, y ≥ 0`, `width, height > 0`, `x + width ≤ 1`, `y + height ≤ 1`을 만족하는 crop을
    /// 만듭니다. 범위 밖은 clamp하지 않고 거부합니다.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<ImageCrop, SceneValueError> {
        let crop = ImageCrop {
            x: finite(x, "image crop")?,
            y: finite(y, "image crop")?,
            width: finite(width, "image crop")?,
            height: finite(height, "image crop")?,
        };
        let ok = x >= 0.0
            && y >= 0.0
            && width > 0.0
            && height > 0.0
            && x + width <= 1.0
            && y + height <= 1.0;
        if ok {
            Ok(crop)
        } else {
            Err(SceneValueError::OutOfRange {
                field: "image crop",
            })
        }
    }

    /// `(x, y, width, height)`.
    pub const fn components(&self) -> (f32, f32, f32, f32) {
        (
            self.x.get(),
            self.y.get(),
            self.width.get(),
            self.height.get(),
        )
    }
}

/// 모서리 둥근 사각형 primitive(Frame 배경·테두리 geometry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RectPrimitive {
    /// node local 좌표계의 사각형.
    pub rect: Rect,
    /// 실효 모서리 radius.
    pub radii: CornerRadii,
}

/// path primitive(Shape 평가 결과와 Path node 공용).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPrimitive {
    /// node local 좌표계의 path geometry.
    pub geometry: PathGeometry,
    /// 채우기 규칙.
    pub fill_rule: FillRule,
}

/// text primitive. glyph 자체는 [`TextLayoutKey`]가 가리키는 layout 결과에 있습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextPrimitive {
    /// text layout 결과 key.
    pub layout: TextLayoutKey,
    /// node local 좌표계의 layout 상자.
    pub bounds: Rect,
}

/// image primitive. 픽셀은 [`AssetRef`]가 가리키는 asset을 renderer가 decode합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImagePrimitive {
    /// 이미지 asset.
    pub asset: AssetRef,
    /// node local 좌표계의 배치 상자(asset이 없어도 유지되는 placeholder bounds).
    pub bounds: Rect,
    /// 상자 배치 방식.
    pub fit: ImageFit,
    /// crop 영역.
    pub crop: ImageCrop,
    /// sampling 방식.
    pub sampling: ImageSampling,
    /// asset 고유 색공간 대신 쓸 색공간. 없으면 asset metadata를 따릅니다.
    pub color_space: Option<ColorSpace>,
}

/// scene node가 그리는 내용(primitive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneContent {
    /// 모서리 둥근 사각형.
    Rect(RectPrimitive),
    /// path.
    Path(PathPrimitive),
    /// text layout 결과.
    Text(TextPrimitive),
    /// 이미지.
    Image(ImagePrimitive),
}

impl SceneContent {
    /// 이 내용이 참조하는 외부 resource. 값만으로 그릴 수 있으면 `None`.
    pub fn resource(&self) -> Option<SceneResourceRef> {
        match self {
            SceneContent::Rect(_) | SceneContent::Path(_) => None,
            SceneContent::Text(text) => Some(SceneResourceRef::TextLayout(text.layout)),
            SceneContent::Image(image) => Some(SceneResourceRef::Image(image.asset)),
        }
    }
}

/// 전역 paint 순서 key. 클수록 위에 그립니다.
///
/// snapshot pre-order에서 엄격히 증가해야 합니다(모듈 문서의 z-order 불변식).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZOrder(
    /// 순서 값.
    pub u64,
);

/// 표시 여부(조상 상태를 반영한 실효 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Visibility {
    /// 그리고 hit test 대상이 됩니다.
    #[default]
    Visible,
    /// 그리지 않고 hit test 대상도 아닙니다. 구조는 유지해 표시 전환이 구조 변경이 되지 않습니다.
    Hidden,
}

impl Visibility {
    /// 안정 numeric tag(Visible=0, Hidden=1).
    pub const fn tag(self) -> u8 {
        match self {
            Visibility::Visible => 0,
            Visibility::Hidden => 1,
        }
    }

    /// tag에서 복원합니다. 미지 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<Visibility> {
        match tag {
            0 => Some(Visibility::Visible),
            1 => Some(Visibility::Hidden),
            _ => None,
        }
    }
}

/// pointer 입력 수용 여부(실효 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PointerEvents {
    /// hit test 대상이 됩니다.
    #[default]
    Auto,
    /// hit test에서 이 node를 건너뛰고 아래 node가 대상이 됩니다(자식은 자기 값을 따름).
    None,
}

impl PointerEvents {
    /// 안정 numeric tag(Auto=0, None=1).
    pub const fn tag(self) -> u8 {
        match self {
            PointerEvents::Auto => 0,
            PointerEvents::None => 1,
        }
    }

    /// tag에서 복원합니다. 미지 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<PointerEvents> {
        match tag {
            0 => Some(PointerEvents::Auto),
            1 => Some(PointerEvents::None),
            _ => None,
        }
    }
}

/// hit test metadata. 조상 상태(숨김·잠금 등)는 scene 생성 시 실효 값으로 반영합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HitTestMeta {
    /// 실효 표시 여부.
    pub visibility: Visibility,
    /// 실효 pointer-events.
    pub pointer_events: PointerEvents,
}

impl HitTestMeta {
    /// 보이고 pointer 입력을 받는 기본값.
    pub const INTERACTIVE: HitTestMeta = HitTestMeta {
        visibility: Visibility::Visible,
        pointer_events: PointerEvents::Auto,
    };

    /// 보이고 pointer 입력을 받으면 참입니다.
    pub fn is_hit_target(&self) -> bool {
        self.visibility == Visibility::Visible && self.pointer_events == PointerEvents::Auto
    }
}

/// scene tree 불변식 위반.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneTreeError {
    /// pre-order에서 z-order가 엄격히 증가하지 않습니다.
    ZOrderNotIncreasing {
        /// 앞 node(또는 앞 subtree의 마지막 node)의 z-order.
        previous: ZOrder,
        /// 뒤 node의 z-order.
        next: ZOrder,
    },
}

impl fmt::Display for SceneTreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneTreeError::ZOrderNotIncreasing { previous, next } => write!(
                f,
                "z-order는 pre-order에서 엄격히 증가해야 합니다(앞 {}, 뒤 {})",
                previous.0, next.0
            ),
        }
    }
}

impl std::error::Error for SceneTreeError {}

/// [`SceneNode`] 생성 입력(자식 제외).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneNodeSpec {
    /// 이 node를 만든 document node의 StableId(값 복사, document 객체 참조 아님).
    pub source: StableId,
    /// 그리는 내용.
    pub content: SceneContent,
    /// node local → world 변환.
    pub world_transform: Transform2D,
    /// node local 좌표계의 layout 상자.
    pub layout_rect: Rect,
    /// 이 node를 자르는 조상 clip.
    pub clip: ClipStackRef,
    /// paint stack.
    pub paint: PaintStackRef,
    /// 전역 paint 순서 key.
    pub z: ZOrder,
    /// hit test metadata.
    pub hit: HitTestMeta,
}

/// 구조 공유 갱신(XI.1.4) 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneEditError {
    /// 경로가 비어 있습니다.
    EmptyPath,
    /// 경로의 `level`번째 index가 범위 밖입니다.
    PathOutOfRange {
        /// 경로 단계(0 = 최상위).
        level: usize,
        /// 범위 밖 index.
        index: usize,
    },
    /// 갱신 결과가 z-order 불변식을 어깁니다.
    Tree(SceneTreeError),
}

impl fmt::Display for SceneEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneEditError::EmptyPath => f.write_str("scene 갱신 경로가 비어 있습니다"),
            SceneEditError::PathOutOfRange { level, index } => {
                write!(
                    f,
                    "scene 갱신 경로 {level}단계 index {index}가 범위 밖입니다"
                )
            }
            SceneEditError::Tree(error) => write!(f, "scene tree 오류: {error}"),
        }
    }
}

impl std::error::Error for SceneEditError {}

impl From<SceneTreeError> for SceneEditError {
    fn from(error: SceneTreeError) -> SceneEditError {
        SceneEditError::Tree(error)
    }
}

/// 불변 scene node.
///
/// 자식은 [`PersistentVec`]에 `Arc<SceneNode>`로 보관해 같은 subtree와 자식 목록 chunk를 여러
/// snapshot이 공유할 수 있습니다(XI.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneNode {
    source: StableId,
    content: SceneContent,
    world_transform: Transform2D,
    layout_rect: Rect,
    clip: ClipStackRef,
    paint: PaintStackRef,
    z: ZOrder,
    hit: HitTestMeta,
    children: PersistentVec<Arc<SceneNode>>,
    subtree_len: u64,
    last_z: ZOrder,
}

/// `previous` 다음에 `next` subtree가 와도 z-order 불변식이 유지되는지 검사합니다.
pub(crate) fn check_z_follows(previous: ZOrder, next: ZOrder) -> Result<(), SceneTreeError> {
    if next > previous {
        Ok(())
    } else {
        Err(SceneTreeError::ZOrderNotIncreasing { previous, next })
    }
}

impl SceneNode {
    /// spec과 자식(paint 순서)으로 node를 만듭니다.
    ///
    /// 자식 subtree의 z-order가 자신 다음부터 엄격히 증가하지 않으면 거부합니다.
    pub fn new(
        spec: SceneNodeSpec,
        children: Vec<Arc<SceneNode>>,
    ) -> Result<SceneNode, SceneTreeError> {
        let mut last_z = spec.z;
        let mut subtree_len = 1u64;
        for child in &children {
            check_z_follows(last_z, child.z)?;
            last_z = child.last_z;
            subtree_len = subtree_len.saturating_add(child.subtree_len);
        }
        Ok(SceneNode::assemble(
            spec,
            PersistentVec::from_vec(children),
            subtree_len,
            last_z,
        ))
    }

    /// 자식 없는 node.
    pub fn leaf(spec: SceneNodeSpec) -> SceneNode {
        let z = spec.z;
        SceneNode::assemble(spec, PersistentVec::new(), 1, z)
    }

    fn assemble(
        spec: SceneNodeSpec,
        children: PersistentVec<Arc<SceneNode>>,
        subtree_len: u64,
        last_z: ZOrder,
    ) -> SceneNode {
        SceneNode {
            source: spec.source,
            content: spec.content,
            world_transform: spec.world_transform,
            layout_rect: spec.layout_rect,
            clip: spec.clip,
            paint: spec.paint,
            z: spec.z,
            hit: spec.hit,
            children,
            subtree_len,
            last_z,
        }
    }

    /// 자기 field만 바꾸고 자식 목록은 그대로 공유하는 새 node(XI.1.4).
    ///
    /// 단일 property 편집처럼 subtree가 바뀌지 않는 갱신에 씁니다. 새 z-order가 첫 자식보다
    /// 앞서지 않으면 거부합니다.
    pub fn with_spec(&self, spec: SceneNodeSpec) -> Result<SceneNode, SceneTreeError> {
        let last_z = match (self.children.get(0), self.children.last()) {
            (Some(first), Some(last)) => {
                check_z_follows(spec.z, first.z)?;
                last.last_z
            }
            _ => spec.z,
        };
        Ok(SceneNode::assemble(
            spec,
            self.children.clone(),
            self.subtree_len,
            last_z,
        ))
    }

    /// `index`번째 자식을 바꾼 새 node(XI.1.4). 다른 자식과 자식 목록 chunk는 공유합니다.
    ///
    /// 새 자식 subtree가 앞 형제(또는 자신)와 뒤 형제 사이 z-order 범위에 맞지 않으면 거부합니다.
    pub fn with_child_replaced(
        &self,
        index: usize,
        child: Arc<SceneNode>,
    ) -> Result<SceneNode, SceneEditError> {
        let old = self
            .children
            .get(index)
            .ok_or(SceneEditError::PathOutOfRange { level: 0, index })?;
        let previous = match index.checked_sub(1) {
            Some(prev) => self
                .children
                .get(prev)
                .map_or(self.z, |sibling| sibling.last_z),
            None => self.z,
        };
        check_z_follows(previous, child.z)?;
        if let Some(next) = self.children.get(index + 1) {
            check_z_follows(child.last_z, next.z)?;
        }
        let last_z = if index + 1 == self.children.len() {
            child.last_z
        } else {
            self.last_z
        };
        let subtree_len = self
            .subtree_len
            .saturating_sub(old.subtree_len)
            .saturating_add(child.subtree_len);
        let children = self
            .children
            .set(index, child)
            .ok_or(SceneEditError::PathOutOfRange { level: 0, index })?;
        Ok(SceneNode {
            children,
            subtree_len,
            last_z,
            ..self.clone_fields()
        })
    }

    /// 자식을 제외한 field 복사본(자식 목록은 O(1) 공유 복제).
    fn clone_fields(&self) -> SceneNode {
        SceneNode {
            source: self.source,
            content: self.content.clone(),
            world_transform: self.world_transform,
            layout_rect: self.layout_rect,
            clip: self.clip.clone(),
            paint: self.paint.clone(),
            z: self.z,
            hit: self.hit,
            children: self.children.clone(),
            subtree_len: self.subtree_len,
            last_z: self.last_z,
        }
    }

    /// 자기 field를 [`SceneNodeSpec`]으로 돌려줍니다(자식 제외).
    pub fn spec(&self) -> SceneNodeSpec {
        SceneNodeSpec {
            source: self.source,
            content: self.content.clone(),
            world_transform: self.world_transform,
            layout_rect: self.layout_rect,
            clip: self.clip.clone(),
            paint: self.paint.clone(),
            z: self.z,
            hit: self.hit,
        }
    }

    /// 원본 document node의 StableId.
    pub const fn source(&self) -> StableId {
        self.source
    }

    /// 그리는 내용.
    pub const fn content(&self) -> &SceneContent {
        &self.content
    }

    /// node local → world 변환.
    pub const fn world_transform(&self) -> Transform2D {
        self.world_transform
    }

    /// node local 좌표계의 layout 상자.
    pub const fn layout_rect(&self) -> Rect {
        self.layout_rect
    }

    /// 이 node를 자르는 조상 clip.
    pub const fn clip(&self) -> &ClipStackRef {
        &self.clip
    }

    /// paint stack.
    pub const fn paint(&self) -> &PaintStackRef {
        &self.paint
    }

    /// 전역 paint 순서 key.
    pub const fn z(&self) -> ZOrder {
        self.z
    }

    /// subtree에서 가장 큰(마지막) z-order.
    pub const fn last_z(&self) -> ZOrder {
        self.last_z
    }

    /// hit test metadata.
    pub const fn hit(&self) -> HitTestMeta {
        self.hit
    }

    /// 자식 수.
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// `index`번째 자식(paint 순서).
    pub fn child(&self, index: usize) -> Option<&Arc<SceneNode>> {
        self.children.get(index)
    }

    /// 자식들(paint 순서).
    pub fn children(&self) -> crate::persistent::Iter<'_, Arc<SceneNode>> {
        self.children.iter()
    }

    /// 자식 목록(구조 공유 vector).
    pub const fn child_list(&self) -> &PersistentVec<Arc<SceneNode>> {
        &self.children
    }

    /// 자신을 포함한 subtree node 수.
    pub const fn subtree_len(&self) -> u64 {
        self.subtree_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::value::{Size2, Vec2};

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(Vec2::new(x, y).unwrap(), Size2::new(w, h).unwrap())
    }

    fn id(n: u8) -> StableId {
        StableId::from_bytes([n; 16])
    }

    #[test]
    fn value_types_reject_invalid_input() {
        assert_eq!(
            CornerRadii::new(1.0, -1.0, 0.0, 0.0),
            Err(SceneValueError::Negative {
                field: "corner radius"
            })
        );
        assert_eq!(
            CornerRadii::new(f32::NAN, 0.0, 0.0, 0.0),
            Err(SceneValueError::NonFinite {
                field: "corner radius"
            })
        );
        assert!(CornerRadii::new(0.0, 0.0, 0.0, 0.0).unwrap().is_zero());
        assert!(ImageCrop::new(0.25, 0.25, 0.5, 0.5).is_ok());
        for (x, y, w, h) in [
            (-0.1, 0.0, 0.5, 0.5),
            (0.0, 0.0, 0.0, 0.5),
            (0.6, 0.0, 0.5, 0.5),
            (0.0, 0.6, 0.5, 0.5),
        ] {
            assert_eq!(
                ImageCrop::new(x, y, w, h),
                Err(SceneValueError::OutOfRange {
                    field: "image crop"
                })
            );
        }
        assert_eq!(ImageCrop::FULL.components(), (0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn enum_tags_match_document_and_round_trip() {
        // document FitMode / SamplingMode 저장 tag와 같은 값.
        let fits = [
            (ImageFit::Fill, 0),
            (ImageFit::Contain, 1),
            (ImageFit::Cover, 2),
            (ImageFit::None, 3),
        ];
        for (fit, tag) in fits {
            assert_eq!(fit.tag(), tag);
            assert_eq!(ImageFit::from_tag(tag), Some(fit));
        }
        assert_eq!(ImageFit::from_tag(4), None);
        for (sampling, tag) in [(ImageSampling::Linear, 0), (ImageSampling::Nearest, 1)] {
            assert_eq!(sampling.tag(), tag);
            assert_eq!(ImageSampling::from_tag(tag), Some(sampling));
        }
        assert_eq!(ImageSampling::from_tag(2), None);
        assert_eq!(ImageFit::default(), ImageFit::Contain);
    }

    #[test]
    fn resources_are_value_identifiers() {
        let image = SceneContent::Image(ImagePrimitive {
            asset: AssetRef::new(id(9)),
            bounds: rect(0.0, 0.0, 10.0, 10.0),
            fit: ImageFit::Cover,
            crop: ImageCrop::FULL,
            sampling: ImageSampling::Nearest,
            color_space: Some(ColorSpace::DisplayP3),
        });
        assert_eq!(
            image.resource(),
            Some(SceneResourceRef::Image(AssetRef::new(id(9))))
        );
        let key = TextLayoutKey::from_bytes([7; 16]);
        let text = SceneContent::Text(TextPrimitive {
            layout: key,
            bounds: rect(0.0, 0.0, 5.0, 5.0),
        });
        assert_eq!(text.resource(), Some(SceneResourceRef::TextLayout(key)));
        assert_eq!(key.to_bytes(), [7; 16]);
        let shape = SceneContent::Rect(RectPrimitive {
            rect: rect(0.0, 0.0, 1.0, 1.0),
            radii: CornerRadii::ZERO,
        });
        assert_eq!(shape.resource(), None);
    }

    #[test]
    fn subtree_len_counts_shared_children() {
        let leaf = crate::fixtures::leaf(2, 2, 1.0, 1.0);
        let parent = SceneNode::new(
            crate::fixtures::spec(1, crate::fixtures::rect_content(4.0, 4.0), 1),
            vec![Arc::clone(&leaf)],
        )
        .unwrap();
        assert_eq!(parent.subtree_len(), 2);
        assert_eq!(parent.child_count(), 1);
        assert!(Arc::ptr_eq(parent.child(0).unwrap(), &leaf));
        assert!(parent.child(1).is_none());
        assert_eq!(parent.source(), crate::fixtures::id(1));
        assert_eq!(parent.last_z(), ZOrder(2));
    }

    #[test]
    fn children_must_follow_parent_in_z_order() {
        let child = crate::fixtures::leaf(2, 5, 1.0, 1.0);
        // 부모 z가 자식보다 크거나 같으면 거부.
        assert_eq!(
            SceneNode::new(
                crate::fixtures::spec(1, crate::fixtures::rect_content(1.0, 1.0), 5),
                vec![Arc::clone(&child)],
            )
            .unwrap_err(),
            SceneTreeError::ZOrderNotIncreasing {
                previous: ZOrder(5),
                next: ZOrder(5)
            }
        );
        // 형제는 앞 subtree의 마지막 z보다 커야 함: (1 → [3(4), 2]) 거부.
        let nested = Arc::new(
            SceneNode::new(
                crate::fixtures::spec(3, crate::fixtures::rect_content(1.0, 1.0), 3),
                vec![crate::fixtures::leaf(4, 4, 1.0, 1.0)],
            )
            .unwrap(),
        );
        assert_eq!(
            SceneNode::new(
                crate::fixtures::spec(1, crate::fixtures::rect_content(1.0, 1.0), 1),
                vec![nested, crate::fixtures::leaf(2, 2, 1.0, 1.0)],
            )
            .unwrap_err(),
            SceneTreeError::ZOrderNotIncreasing {
                previous: ZOrder(4),
                next: ZOrder(2)
            }
        );
    }

    #[test]
    fn hit_meta_and_tags() {
        assert!(HitTestMeta::INTERACTIVE.is_hit_target());
        assert!(!HitTestMeta {
            visibility: Visibility::Hidden,
            pointer_events: PointerEvents::Auto
        }
        .is_hit_target());
        assert!(!HitTestMeta {
            visibility: Visibility::Visible,
            pointer_events: PointerEvents::None
        }
        .is_hit_target());
        for v in [Visibility::Visible, Visibility::Hidden] {
            assert_eq!(Visibility::from_tag(v.tag()), Some(v));
        }
        for e in [PointerEvents::Auto, PointerEvents::None] {
            assert_eq!(PointerEvents::from_tag(e.tag()), Some(e));
        }
        assert_eq!(Visibility::from_tag(2), None);
        assert_eq!(PointerEvents::from_tag(2), None);
    }
}
