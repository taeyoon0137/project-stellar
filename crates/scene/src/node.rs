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

use std::fmt;
use std::sync::Arc;

use stellar_core_types::color::ColorSpace;
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::value::{FiniteF32, Rect};

use crate::geometry::{FillRule, PathGeometry};

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

/// [`SceneNode`] 생성 입력(자식 제외).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneNodeSpec {
    /// 이 node를 만든 document node의 StableId(값 복사, document 객체 참조 아님).
    pub source: StableId,
    /// 그리는 내용.
    pub content: SceneContent,
}

/// 불변 scene node.
///
/// 자식은 `Arc<SceneNode>`로 보관해 같은 subtree를 여러 snapshot이 공유할 수 있습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneNode {
    source: StableId,
    content: SceneContent,
    children: Arc<[Arc<SceneNode>]>,
    subtree_len: u64,
}

impl SceneNode {
    /// spec과 자식(paint 순서)으로 node를 만듭니다.
    pub fn new(spec: SceneNodeSpec, children: Vec<Arc<SceneNode>>) -> SceneNode {
        let subtree_len = children
            .iter()
            .fold(1u64, |sum, child| sum.saturating_add(child.subtree_len));
        SceneNode {
            source: spec.source,
            content: spec.content,
            children: children.into(),
            subtree_len,
        }
    }

    /// 자식 없는 node.
    pub fn leaf(spec: SceneNodeSpec) -> SceneNode {
        SceneNode::new(spec, Vec::new())
    }

    /// 원본 document node의 StableId.
    pub const fn source(&self) -> StableId {
        self.source
    }

    /// 그리는 내용.
    pub const fn content(&self) -> &SceneContent {
        &self.content
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
    pub fn children(&self) -> impl DoubleEndedIterator<Item = &Arc<SceneNode>> + '_ {
        self.children.iter()
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
        let leaf = Arc::new(SceneNode::leaf(SceneNodeSpec {
            source: id(2),
            content: SceneContent::Rect(RectPrimitive {
                rect: rect(0.0, 0.0, 1.0, 1.0),
                radii: CornerRadii::ZERO,
            }),
        }));
        let parent = SceneNode::new(
            SceneNodeSpec {
                source: id(1),
                content: SceneContent::Rect(RectPrimitive {
                    rect: rect(0.0, 0.0, 4.0, 4.0),
                    radii: CornerRadii::ZERO,
                }),
            },
            vec![Arc::clone(&leaf), Arc::clone(&leaf)],
        );
        assert_eq!(parent.subtree_len(), 3);
        assert_eq!(parent.child_count(), 2);
        assert!(Arc::ptr_eq(parent.child(0).unwrap(), &leaf));
        assert!(parent.child(2).is_none());
        assert_eq!(parent.source(), id(1));
    }
}
