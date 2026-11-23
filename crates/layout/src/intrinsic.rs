//! content intrinsic 측정 hook (X.6.4).
//!
//! # 종류와 기본 intrinsic
//!
//! 측정이 필요한 콘텐츠는 [`ContentKind`] 네 종류입니다. 각 종류는 레이아웃 입력에 담기는 기본 값
//! ([`MeasuredContent`])과 그 값으로 측정하는 기본 구현([`IntrinsicMeasure`])을 가집니다.
//!
//! | 종류 | 기본 intrinsic |
//! |---|---|
//! | `Shape` | geometry bounds(가로·세로) 그대로. 가용 크기·mode와 무관하고 baseline이 없습니다. |
//! | `Text` | 단어 폭 목록을 가용 너비에서 greedy로 줄바꿈합니다. 가용 너비가 없으면 max-content는 한 줄(단어 + 단어 사이 공백), min-content는 가장 넓은 단어 폭에서 줄바꿈합니다. 높이 = 줄 수 × line height(빈 텍스트도 한 줄), baseline = ascent(첫 줄). 줄은 최소 한 단어라 단어가 가용 너비보다 넓으면 넘칩니다. |
//! | `Image` | natural 크기. 한 축만 definite면 natural 비율로 다른 축을 정합니다(한 번 반올림, ties away from zero). 두 축이 모두 definite·indefinite이거나 natural 크기에 0이 있으면 natural 그대로입니다. baseline이 없습니다. |
//! | `ComponentInstance` | 자식을 가질 수 있는 유일한 종류입니다. 엔진은 측정기를 부르지 않고 virtual subtree를 node의 `display`로 배치합니다(ADR-0076). |
//!
//! 실제 텍스트 shaping·이미지 decode 결과가 아니라 입력에 담긴 결정적 값입니다. 실제 엔진은 아래
//! hook으로 연결합니다(텍스트는 X.10.4).
//!
//! # 측정 요청
//!
//! [`MeasureRequest`]의 `available`은 축별 content box 크기(padding 제외)이며 definite 축만 `Some`
//! 입니다. `mode`는 indefinite 축을 최소 콘텐츠(가능한 한 좁게, 예: 가장 넓은 단어)로 볼지 최대
//! 콘텐츠(줄바꿈 없음)로 볼지입니다. 엔진은 크기가 콘텐츠에 의존하는 축(`auto`·`fit-content` 등)에서만 측정
//! 결과를 쓰며, definite 축은 엔진이 정한 크기를 씁니다.
//!
//! # 외부 hook
//!
//! [`ContentMeasurer`]는 계산 때 넘기는 측정기입니다([`crate::engine::compute_layout_with`]). 텍스트
//! 엔진 등은 이를 구현해 기본 구현을 대체하고, 처리하지 않는 종류는 [`MeasuredContent::measure`]
//! (기본 구현)에 위임합니다. [`DefaultMeasurer`]는 기본 구현만 씁니다. 측정은 결정적이어야 합니다(같은
//! 요청 → 같은 결과). 깊은 tree는 전용 stack thread에서 계산하므로 측정기는 `Sync`입니다.
//!
//! # Text 엔진 연결 (X.10.4)
//!
//! [`TextLayoutHook`]은 텍스트 엔진이 내는 값(min-content·max-content 너비, 너비 제약에서의 줄 배치 높이와 첫 baseline)을
//! 그대로 받는 hook이고, [`TextHookMeasurer`]가 그것을 [`ContentMeasurer`]로 바꿉니다. 엔진은 Text leaf의 `auto` 너비를
//! `fit-content`(min-content·max-content 사이)로, 높이를 정한 너비에서의 줄 배치 높이로, 첫 baseline을 그 줄 배치의 첫
//! baseline + 위쪽 padding으로 씁니다. baseline은 container 첫 baseline(block·flex·grid)과 flex baseline 정렬(X.3.6)로
//! 이어집니다. 텍스트 엔진 crate(`stellar-text`)와 이 crate는 서로 의존하지 않으므로 hook 구현은 두 crate에 함께 의존할 수
//! 있는 상위 crate가 둡니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::Length;

use crate::codec::{Reader, Writer};
use crate::error::LayoutError;
use crate::geometry::Axis;
use crate::input::length_unit;
use crate::property::LayoutProperty;
use crate::rounding::{LayoutUnit, LayoutUnitError};

/// intrinsic 측정 대상 콘텐츠 종류 (X.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContentKind {
    /// 벡터 shape.
    Shape,
    /// 텍스트.
    Text,
    /// 이미지.
    Image,
    /// component instance(virtual subtree를 자식으로 가짐).
    ComponentInstance,
}

impl ContentKind {
    /// 모든 종류(canonical tag 순).
    pub const ALL: [ContentKind; 4] = [
        ContentKind::Shape,
        ContentKind::Text,
        ContentKind::Image,
        ContentKind::ComponentInstance,
    ];

    /// 사람이 읽는 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ContentKind::Shape => "shape",
            ContentKind::Text => "text",
            ContentKind::Image => "image",
            ContentKind::ComponentInstance => "component-instance",
        }
    }

    /// canonical tag.
    pub const fn tag(self) -> u8 {
        match self {
            ContentKind::Shape => 0,
            ContentKind::Text => 1,
            ContentKind::Image => 2,
            ContentKind::ComponentInstance => 3,
        }
    }

    /// 자식을 가질 수 있는지 여부(`ComponentInstance`만).
    pub const fn allows_children(self) -> bool {
        matches!(self, ContentKind::ComponentInstance)
    }
}

impl fmt::Display for ContentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// indefinite 축의 측정 기준 (X.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum IntrinsicMode {
    /// 최소 콘텐츠(가능한 한 좁게).
    MinContent,
    /// 최대 콘텐츠(줄바꿈 없음).
    #[default]
    MaxContent,
}

/// intrinsic 측정 요청 (X.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeasureRequest {
    /// 측정 node.
    pub node: StableId,
    /// 콘텐츠 종류.
    pub kind: ContentKind,
    /// 축별 가용 content 크기(padding 제외). definite 축만 `Some`.
    pub available: [Option<LayoutUnit>; 2],
    /// indefinite 축의 측정 기준.
    pub mode: IntrinsicMode,
    /// 자식 subtree를 container 배치로 측정한 extent(`ComponentInstance`만 `Some`).
    pub subtree: Option<[LayoutUnit; 2]>,
}

impl MeasureRequest {
    /// 축의 가용 크기.
    pub const fn available(&self, axis: Axis) -> Option<LayoutUnit> {
        self.available[axis.index()]
    }
}

/// intrinsic 측정 결과 (X.6.4). 크기는 content 크기(padding 제외, 비음수)입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntrinsicSize {
    /// 너비.
    pub width: LayoutUnit,
    /// 높이.
    pub height: LayoutUnit,
    /// 콘텐츠 top 기준 첫 baseline. 없으면 `None`.
    pub baseline: Option<LayoutUnit>,
}

impl IntrinsicSize {
    /// baseline 없는 크기.
    pub const fn new(width: LayoutUnit, height: LayoutUnit) -> IntrinsicSize {
        IntrinsicSize {
            width,
            height,
            baseline: None,
        }
    }
}

/// 측정 실패 (X.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeasureError {
    /// 사유(사람이 읽는 설명).
    pub reason: &'static str,
}

impl From<LayoutUnitError> for MeasureError {
    fn from(_: LayoutUnitError) -> MeasureError {
        MeasureError {
            reason: "측정 수치가 레이아웃 범위를 벗어났습니다",
        }
    }
}

/// 콘텐츠 종류별 intrinsic 측정 (X.6.4).
pub trait IntrinsicMeasure {
    /// 콘텐츠 종류.
    fn kind(&self) -> ContentKind;

    /// 요청에 대한 content 크기와 baseline을 측정합니다.
    fn measure(&self, request: &MeasureRequest) -> Result<IntrinsicSize, MeasureError>;
}

/// 계산 때 넘기는 외부 측정 hook (X.6.4).
///
/// 처리하지 않는 콘텐츠는 `content.measure(request)`(기본 구현)에 위임합니다.
pub trait ContentMeasurer: Sync {
    /// 요청을 측정합니다.
    fn measure(
        &self,
        request: &MeasureRequest,
        content: &MeasuredContent,
    ) -> Result<IntrinsicSize, MeasureError>;

    /// 측정 규칙의 revision. 같은 입력이라도 이 값이 바뀌면(예: 폰트 로드) 이전 측정 cache를 쓰지
    /// 않습니다(X.7.2). 기본값 0.
    fn revision(&self) -> u64 {
        0
    }
}

/// 기본 구현만 쓰는 측정기 (X.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DefaultMeasurer;

impl ContentMeasurer for DefaultMeasurer {
    fn measure(
        &self,
        request: &MeasureRequest,
        content: &MeasuredContent,
    ) -> Result<IntrinsicSize, MeasureError> {
        content.measure(request)
    }
}

/// 텍스트 엔진의 줄 배치 결과 (X.10.4). 크기는 content 크기(padding 제외)입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextLineLayout {
    /// 가장 넓은 줄 너비.
    pub width: LayoutUnit,
    /// 줄 배치 높이(content height).
    pub height: LayoutUnit,
    /// 콘텐츠 top 기준 첫 줄 baseline. 줄이 없으면 `None`.
    pub first_baseline: Option<LayoutUnit>,
}

/// 텍스트 엔진 hook (X.10.4).
///
/// `node`는 Text node, `text`는 레이아웃 입력에 담긴 결정적 기본 값입니다(hook은 무시하고 자신의 문서 값으로 계산해도
/// 됩니다). 결과는 같은 입력에서 결정적이어야 합니다.
pub trait TextLayoutHook: Sync {
    /// min-content 너비(가장 넓은 줄바꿈 불가 단위).
    fn min_content_width(
        &self,
        node: StableId,
        text: &TextIntrinsic,
    ) -> Result<LayoutUnit, MeasureError>;

    /// 너비 `width`(없으면 줄바꿈 없음)에서의 줄 배치.
    fn layout(
        &self,
        node: StableId,
        text: &TextIntrinsic,
        width: Option<LayoutUnit>,
    ) -> Result<TextLineLayout, MeasureError>;

    /// 결과 revision(폰트 로드 등). 바뀌면 계산 간 cache를 쓰지 않습니다.
    fn revision(&self) -> u64 {
        0
    }
}

/// [`TextLayoutHook`]으로 Text를 측정하고 나머지 종류는 기본 구현에 위임하는 측정기 (X.10.4).
#[derive(Debug, Clone, Copy)]
pub struct TextHookMeasurer<'a, H: ?Sized>(pub &'a H);

impl<H: TextLayoutHook + ?Sized> ContentMeasurer for TextHookMeasurer<'_, H> {
    fn measure(
        &self,
        request: &MeasureRequest,
        content: &MeasuredContent,
    ) -> Result<IntrinsicSize, MeasureError> {
        let MeasuredContent::Text(text) = content else {
            return content.measure(request);
        };
        let width = match (request.available(Axis::Horizontal), request.mode) {
            (Some(width), _) => Some(width),
            (None, IntrinsicMode::MaxContent) => None,
            (None, IntrinsicMode::MinContent) => {
                Some(self.0.min_content_width(request.node, text)?)
            }
        };
        let lines = self.0.layout(request.node, text, width)?;
        Ok(IntrinsicSize {
            width: lines.width,
            height: lines.height,
            baseline: lines.first_baseline,
        })
    }

    fn revision(&self) -> u64 {
        self.0.revision()
    }
}

/// 길이를 측정용 subpixel로 변환합니다(입력 검증을 통과한 값은 실패하지 않음).
fn unit(length: Length) -> Result<LayoutUnit, MeasureError> {
    LayoutUnit::from_length(length).map_err(MeasureError::from)
}

/// Shape 기본 intrinsic: geometry bounds (X.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShapeIntrinsic {
    /// bounds 너비(비음수).
    pub width: Length,
    /// bounds 높이(비음수).
    pub height: Length,
}

impl IntrinsicMeasure for ShapeIntrinsic {
    fn kind(&self) -> ContentKind {
        ContentKind::Shape
    }

    fn measure(&self, _: &MeasureRequest) -> Result<IntrinsicSize, MeasureError> {
        Ok(IntrinsicSize::new(unit(self.width)?, unit(self.height)?))
    }
}

/// Text 기본 intrinsic: 단어 폭 기반 greedy 줄바꿈 (X.6.4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextIntrinsic {
    /// 단어(줄바꿈 불가 단위) 폭, 논리 순서(비음수).
    pub words: Vec<Length>,
    /// 같은 줄 단어 사이 공백 폭(비음수).
    pub space: Length,
    /// 줄 높이(비음수).
    pub line_height: Length,
    /// 첫 줄 top에서 baseline까지 거리.
    pub ascent: Length,
}

impl TextIntrinsic {
    /// 폭 `limit` 안에서 greedy로 줄을 나눈 (줄 수, 가장 넓은 줄 폭). `limit`이 `None`이면 한 줄.
    fn break_lines(
        words: &[LayoutUnit],
        space: LayoutUnit,
        limit: Option<LayoutUnit>,
    ) -> Result<(usize, LayoutUnit), LayoutUnitError> {
        let mut lines = 1usize;
        let mut widest = LayoutUnit::ZERO;
        let mut current: Option<LayoutUnit> = None;
        for &word in words {
            let next = match current {
                None => word,
                Some(width) => width.checked_add(space)?.checked_add(word)?,
            };
            current = match (current, limit) {
                (Some(width), Some(limit)) if next > limit => {
                    widest = widest.max(width);
                    lines += 1;
                    Some(word)
                }
                _ => Some(next),
            };
        }
        Ok((lines, widest.max(current.unwrap_or(LayoutUnit::ZERO))))
    }
}

impl IntrinsicMeasure for TextIntrinsic {
    fn kind(&self) -> ContentKind {
        ContentKind::Text
    }

    fn measure(&self, request: &MeasureRequest) -> Result<IntrinsicSize, MeasureError> {
        let words = self
            .words
            .iter()
            .map(|w| unit(*w))
            .collect::<Result<Vec<_>, _>>()?;
        let space = unit(self.space)?;
        let limit = match (request.available(Axis::Horizontal), request.mode) {
            (Some(width), _) => Some(width),
            (None, IntrinsicMode::MaxContent) => None,
            (None, IntrinsicMode::MinContent) => {
                Some(words.iter().copied().max().unwrap_or(LayoutUnit::ZERO))
            }
        };
        let (lines, widest) = TextIntrinsic::break_lines(&words, space, limit)?;
        let lines = i32::try_from(lines).map_err(|_| MeasureError {
            reason: "텍스트 줄 수가 레이아웃 범위를 벗어났습니다",
        })?;
        let height = unit(self.line_height)?.checked_mul_int(lines)?;
        Ok(IntrinsicSize {
            width: widest,
            height,
            baseline: Some(unit(self.ascent)?),
        })
    }
}

/// Image 기본 intrinsic: natural 크기와 비율 (X.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageIntrinsic {
    /// natural 너비(비음수).
    pub width: Length,
    /// natural 높이(비음수).
    pub height: Length,
}

impl IntrinsicMeasure for ImageIntrinsic {
    fn kind(&self) -> ContentKind {
        ContentKind::Image
    }

    fn measure(&self, request: &MeasureRequest) -> Result<IntrinsicSize, MeasureError> {
        let width = unit(self.width)?;
        let height = unit(self.height)?;
        let ratio_ok = width > LayoutUnit::ZERO && height > LayoutUnit::ZERO;
        Ok(
            match (
                request.available(Axis::Horizontal),
                request.available(Axis::Vertical),
            ) {
                (Some(w), None) if ratio_ok => IntrinsicSize::new(
                    w,
                    w.checked_mul_ratio(i64::from(height.raw()), i64::from(width.raw()))?,
                ),
                (None, Some(h)) if ratio_ok => IntrinsicSize::new(
                    h.checked_mul_ratio(i64::from(width.raw()), i64::from(height.raw()))?,
                    h,
                ),
                _ => IntrinsicSize::new(width, height),
            },
        )
    }
}

/// ComponentInstance 기본 intrinsic: virtual subtree extent (X.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ComponentInstanceIntrinsic;

impl IntrinsicMeasure for ComponentInstanceIntrinsic {
    fn kind(&self) -> ContentKind {
        ContentKind::ComponentInstance
    }

    fn measure(&self, request: &MeasureRequest) -> Result<IntrinsicSize, MeasureError> {
        let [width, height] = request.subtree.unwrap_or([LayoutUnit::ZERO; 2]);
        Ok(IntrinsicSize::new(width, height))
    }
}

/// 측정이 필요한 콘텐츠와 기본 intrinsic 값 (X.6.4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MeasuredContent {
    /// 벡터 shape.
    Shape(ShapeIntrinsic),
    /// 텍스트.
    Text(TextIntrinsic),
    /// 이미지.
    Image(ImageIntrinsic),
    /// component instance.
    ComponentInstance(ComponentInstanceIntrinsic),
}

impl IntrinsicMeasure for MeasuredContent {
    fn kind(&self) -> ContentKind {
        match self {
            MeasuredContent::Shape(c) => c.kind(),
            MeasuredContent::Text(c) => c.kind(),
            MeasuredContent::Image(c) => c.kind(),
            MeasuredContent::ComponentInstance(c) => c.kind(),
        }
    }

    fn measure(&self, request: &MeasureRequest) -> Result<IntrinsicSize, MeasureError> {
        match self {
            MeasuredContent::Shape(c) => c.measure(request),
            MeasuredContent::Text(c) => c.measure(request),
            MeasuredContent::Image(c) => c.measure(request),
            MeasuredContent::ComponentInstance(c) => c.measure(request),
        }
    }
}

/// 기본 intrinsic 값의 범위를 검증합니다.
pub(crate) fn validate_measured(
    node: StableId,
    content: &MeasuredContent,
) -> Result<(), LayoutError> {
    let property = LayoutProperty::Content;
    let check = |length: Length, allow_negative: bool| {
        length_unit(node, property, length, allow_negative).map(|_| ())
    };
    match content {
        MeasuredContent::Shape(ShapeIntrinsic { width, height })
        | MeasuredContent::Image(ImageIntrinsic { width, height }) => {
            check(*width, false)?;
            check(*height, false)
        }
        MeasuredContent::Text(text) => {
            for word in &text.words {
                check(*word, false)?;
            }
            check(text.space, false)?;
            check(text.line_height, false)?;
            check(text.ascent, true)
        }
        MeasuredContent::ComponentInstance(_) => Ok(()),
    }
}

/// 측정 콘텐츠 canonical bytes(kind tag + 값).
pub(crate) fn encode_measured(w: &mut Writer, content: &MeasuredContent) {
    w.u8(content.kind().tag());
    match content {
        MeasuredContent::Shape(ShapeIntrinsic { width, height })
        | MeasuredContent::Image(ImageIntrinsic { width, height }) => {
            w.length(*width);
            w.length(*height);
        }
        MeasuredContent::Text(text) => {
            w.len(text.words.len());
            for word in &text.words {
                w.length(*word);
            }
            w.length(text.space);
            w.length(text.line_height);
            w.length(text.ascent);
        }
        MeasuredContent::ComponentInstance(_) => {}
    }
}

/// 측정 콘텐츠 canonical bytes를 읽습니다.
pub(crate) fn decode_measured(r: &mut Reader<'_>) -> Result<MeasuredContent, LayoutError> {
    Ok(match r.u8()? {
        0 => MeasuredContent::Shape(ShapeIntrinsic {
            width: r.length("shape width")?,
            height: r.length("shape height")?,
        }),
        1 => {
            let count = r.len(4)?;
            let mut words = Vec::with_capacity(count);
            for _ in 0..count {
                words.push(r.length("text word")?);
            }
            MeasuredContent::Text(TextIntrinsic {
                words,
                space: r.length("text space")?,
                line_height: r.length("text line height")?,
                ascent: r.length("text ascent")?,
            })
        }
        2 => MeasuredContent::Image(ImageIntrinsic {
            width: r.length("image width")?,
            height: r.length("image height")?,
        }),
        3 => MeasuredContent::ComponentInstance(ComponentInstanceIntrinsic),
        _ => {
            return Err(LayoutError::Malformed {
                reason: "알 수 없는 content kind tag",
            })
        }
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engine::{compute_layout, compute_layout_with};
    use crate::input::{LayoutContent, LayoutInput, LayoutNode};
    use crate::output::LayoutResult;
    use crate::style::{Display, ItemAlignment, LayoutStyle, Position, Size};
    use crate::test_support::{id, key, len, px, size_px, style};
    use stellar_core_types::value::Insets;

    /// 텍스트 콘텐츠(단어 폭 px, 공백 5, 줄 높이 12, ascent 9).
    pub(crate) fn text(words: &[f32]) -> LayoutContent {
        LayoutContent::Measured(MeasuredContent::Text(TextIntrinsic {
            words: words.iter().map(|w| len(*w)).collect(),
            space: len(5.0),
            line_height: len(12.0),
            ascent: len(9.0),
        }))
    }

    /// 이미지 콘텐츠(natural px).
    pub(crate) fn image(width: f32, height: f32) -> LayoutContent {
        LayoutContent::Measured(MeasuredContent::Image(ImageIntrinsic {
            width: len(width),
            height: len(height),
        }))
    }

    fn shape(width: f32, height: f32) -> LayoutContent {
        LayoutContent::Measured(MeasuredContent::Shape(ShapeIntrinsic {
            width: len(width),
            height: len(height),
        }))
    }

    fn instance() -> LayoutContent {
        LayoutContent::Measured(MeasuredContent::ComponentInstance(
            ComponentInstanceIntrinsic,
        ))
    }

    /// root(500×500 block) 아래 absolute node 하나를 (0, 0)에 배치합니다(`auto` 크기 = `fit-content`).
    fn single(node: LayoutNode, extra: Vec<LayoutNode>) -> LayoutResult {
        let root = LayoutNode::new(id(1))
            .with_style(fixed_style(500.0, 500.0))
            .with_child(node.id, key(1));
        let mut node = node;
        node.style.position = Position::Absolute;
        let mut nodes = vec![root, node];
        nodes.extend(extra);
        compute_layout(&LayoutInput::new(id(1), nodes)).unwrap()
    }

    fn dims(result: &LayoutResult, n: u64) -> (f64, f64) {
        let r = result.get(id(n)).unwrap().local_rect();
        (r.width().to_px_f64(), r.height().to_px_f64())
    }

    fn sized(width: Size, height: Size) -> LayoutStyle {
        style(|s| {
            s.width = width;
            s.height = height;
        })
    }

    fn fixed_style(width: f32, height: f32) -> LayoutStyle {
        sized(size_px(width), size_px(height))
    }

    #[test]
    fn shape_default_intrinsic_is_geometry_bounds() {
        let node = LayoutNode::new(id(2))
            .with_style(style(|s| {
                s.padding = Insets::new(2.0, 2.0, 2.0, 2.0).unwrap()
            }))
            .with_content(shape(30.0, 20.0));
        let result = single(node, vec![]);
        assert_eq!(dims(&result, 2), (34.0, 24.0));
        assert_eq!(result.get(id(2)).unwrap().baseline(), None);
        let node = LayoutNode::new(id(2))
            .with_style(sized(size_px(10.0), Size::Auto))
            .with_content(shape(30.0, 20.0));
        assert_eq!(dims(&single(node, vec![]), 2), (10.0, 20.0));
    }

    #[test]
    fn text_default_intrinsic_wraps_words_greedily() {
        let words = [10.0, 20.0, 15.0];
        let run = |width: Size| {
            let node = LayoutNode::new(id(2))
                .with_style(style(|s| {
                    s.width = width;
                    s.padding = Insets::new(3.0, 0.0, 0.0, 0.0).unwrap();
                }))
                .with_content(text(&words));
            single(node, vec![])
        };
        // max-content 한 줄: 10 + 5 + 20 + 5 + 15 = 55, 높이 12 + padding top 3.
        let result = run(Size::Auto);
        assert_eq!(dims(&result, 2), (55.0, 15.0));
        assert_eq!(result.get(id(2)).unwrap().baseline(), Some(px(12)));
        assert_eq!(dims(&run(size_px(30.0)), 2), (30.0, 39.0));
        assert_eq!(dims(&run(size_px(40.0)), 2), (40.0, 27.0));
        assert_eq!(dims(&run(size_px(5.0)), 2), (5.0, 39.0));
        // min-content keyword: 가장 넓은 단어 20에서 줄바꿈.
        assert_eq!(dims(&run(Size::MinContent), 2), (20.0, 39.0));
        let empty = LayoutNode::new(id(2)).with_content(text(&[]));
        assert_eq!(dims(&single(empty, vec![]), 2), (0.0, 12.0));
        let content = TextIntrinsic {
            words: words.iter().map(|w| len(*w)).collect(),
            space: len(5.0),
            line_height: len(12.0),
            ascent: len(9.0),
        };
        let request = |mode| MeasureRequest {
            node: id(2),
            kind: ContentKind::Text,
            available: [None, None],
            mode,
            subtree: None,
        };
        assert_eq!(
            content.measure(&request(IntrinsicMode::MinContent)),
            Ok(IntrinsicSize {
                width: px(20),
                height: px(36),
                baseline: Some(px(9))
            })
        );
        assert_eq!(
            content
                .measure(&request(IntrinsicMode::MaxContent))
                .unwrap()
                .width,
            px(55)
        );
    }

    #[test]
    fn image_default_intrinsic_keeps_natural_ratio() {
        let run = |width: Size, height: Size, natural: (f32, f32)| {
            let node = LayoutNode::new(id(2))
                .with_style(sized(width, height))
                .with_content(image(natural.0, natural.1));
            dims(&single(node, vec![]), 2)
        };
        let auto = Size::Auto;
        assert_eq!(run(auto, auto, (64.0, 48.0)), (64.0, 48.0));
        assert_eq!(run(size_px(32.0), auto, (64.0, 48.0)), (32.0, 24.0));
        assert_eq!(run(auto, size_px(24.0), (64.0, 48.0)), (32.0, 24.0));
        let node = LayoutNode::new(id(2))
            .with_style(sized(size_px(100.0), auto))
            .with_content(image(3.0, 1.0));
        let result = single(node, vec![]);
        assert_eq!(result.get(id(2)).unwrap().local_rect().height().raw(), 2133);
        assert_eq!(run(size_px(5.0), auto, (0.0, 10.0)), (5.0, 10.0));
    }

    #[test]
    fn component_instance_lays_out_its_children_with_its_display() {
        let child = LayoutNode::new(id(3)).with_style(fixed_style(40.0, 10.0));
        let node = LayoutNode::new(id(2))
            .with_content(instance())
            .with_child(id(3), key(1));
        let result = single(node, vec![child]);
        assert_eq!(dims(&result, 2), (40.0, 10.0));
        let a = LayoutNode::new(id(3)).with_style(fixed_style(10.0, 4.0));
        let b = LayoutNode::new(id(4)).with_style(fixed_style(20.0, 6.0));
        let node = LayoutNode::new(id(2))
            .with_style(style(|s| s.display = Display::Flex))
            .with_content(instance())
            .with_child(id(3), key(1))
            .with_child(id(4), key(2));
        assert_eq!(dims(&single(node, vec![a, b]), 2), (30.0, 6.0));
        let empty = LayoutNode::new(id(2)).with_content(instance());
        assert_eq!(dims(&single(empty, vec![]), 2), (0.0, 0.0));
        for leaf in [text(&[1.0]), image(1.0, 1.0), shape(1.0, 1.0)] {
            let input = LayoutInput::new(
                id(1),
                vec![
                    LayoutNode::new(id(1))
                        .with_content(leaf)
                        .with_child(id(2), key(1)),
                    LayoutNode::new(id(2)),
                ],
            );
            assert_eq!(
                input.validate(),
                Err(LayoutError::ChildrenOnContentLeaf { node: id(1) })
            );
        }
    }

    /// Text만 고정 결과(100×7, baseline 5)로 대체하고 나머지는 기본 구현에 위임하는 측정기.
    struct FixedText;

    impl ContentMeasurer for FixedText {
        fn measure(
            &self,
            request: &MeasureRequest,
            content: &MeasuredContent,
        ) -> Result<IntrinsicSize, MeasureError> {
            match request.kind {
                ContentKind::Text => Ok(IntrinsicSize {
                    width: px(100),
                    height: px(7),
                    baseline: Some(px(5)),
                }),
                _ => content.measure(request),
            }
        }
    }

    /// 항상 실패하거나 음수 크기를 내는 측정기.
    struct Broken {
        negative: bool,
    }

    impl ContentMeasurer for Broken {
        fn measure(
            &self,
            _: &MeasureRequest,
            _: &MeasuredContent,
        ) -> Result<IntrinsicSize, MeasureError> {
            if self.negative {
                Ok(IntrinsicSize::new(px(-1), px(1)))
            } else {
                Err(MeasureError {
                    reason: "테스트 실패",
                })
            }
        }
    }

    #[test]
    fn external_measurer_replaces_defaults_and_errors_are_typed() {
        let input = LayoutInput::new(
            id(1),
            vec![
                LayoutNode::new(id(1))
                    .with_style(style(|s| {
                        s.display = Display::Flex;
                        s.align_items = ItemAlignment::FlexStart;
                    }))
                    .with_child(id(2), key(1))
                    .with_child(id(3), key(2)),
                LayoutNode::new(id(2)).with_content(text(&[10.0])),
                LayoutNode::new(id(3)).with_content(image(8.0, 30.0)),
            ],
        );
        let result = compute_layout_with(&input, &FixedText).unwrap();
        assert_eq!(dims(&result, 2), (100.0, 7.0));
        assert_eq!(result.get(id(2)).unwrap().baseline(), Some(px(5)));
        // 위임한 Image는 기본 구현 natural 크기.
        assert_eq!(dims(&result, 3), (8.0, 30.0));
        assert_eq!(dims(&result, 1), (108.0, 30.0));
        // 기본 측정기와는 Text 결과가 다릅니다.
        assert_eq!(dims(&compute_layout(&input).unwrap(), 2), (10.0, 12.0));
        assert_eq!(
            compute_layout_with(&input, &Broken { negative: false }),
            Err(LayoutError::ContentMeasure {
                node: id(2),
                kind: ContentKind::Text,
                reason: "테스트 실패"
            })
        );
        assert!(matches!(
            compute_layout_with(&input, &Broken { negative: true }),
            Err(LayoutError::ContentMeasure { node, .. }) if node == id(2)
        ));
    }

    #[test]
    fn measured_content_round_trips_and_validates() {
        let contents = [
            shape(3.5, 4.0),
            text(&[1.0, 2.5]),
            image(640.0, 480.0),
            instance(),
        ];
        let mut nodes = vec![LayoutNode::new(id(1))];
        for (i, content) in contents.iter().enumerate() {
            let n = i as u64 + 2;
            nodes[0] = nodes[0].clone().with_child(id(n), key(i as u32 + 1));
            nodes.push(LayoutNode::new(id(n)).with_content(content.clone()));
        }
        let input = LayoutInput::new(id(1), nodes);
        assert_eq!(input.validate(), Ok(()));
        let bytes = input.encode();
        let decoded = LayoutInput::decode(&bytes).unwrap();
        assert_eq!(decoded, input);
        assert_eq!(decoded.encode(), bytes);
        // kind tag golden.
        let tags: Vec<u8> = ContentKind::ALL.iter().map(|k| k.tag()).collect();
        assert_eq!(tags, vec![0, 1, 2, 3]);
        assert!(decode_measured(&mut Reader::new(&[9])).is_err());
        // 음수 단어 폭·line height 거부, ascent는 음수 허용.
        let bad = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_content(text(&[-1.0]))],
        );
        assert_eq!(
            bad.validate(),
            Err(LayoutError::NegativeValue {
                node: id(1),
                property: LayoutProperty::Content
            })
        );
        let negative_ascent = LayoutContent::Measured(MeasuredContent::Text(TextIntrinsic {
            words: vec![],
            space: len(0.0),
            line_height: len(1.0),
            ascent: len(-2.0),
        }));
        let ok = LayoutInput::new(
            id(1),
            vec![LayoutNode::new(id(1)).with_content(negative_ascent)],
        );
        assert_eq!(ok.validate(), Ok(()));
        assert_eq!(DefaultMeasurer.revision(), 0);
        assert_eq!(ContentKind::Text.to_string(), "text");
    }
}

#[cfg(test)]
mod text_hook_tests;
