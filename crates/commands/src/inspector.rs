//! 속성 패널 표면 (UX.2.2, ADR-0070).
//!
//! 선택한 node들의 CSS 속성 행 snapshot([`inspector_rows`])과, CSS 값 텍스트 하나로 선택 전체를 바꾸는 command
//! 목록([`edit_commands`])을 만듭니다. 셸은 행 이름·값 텍스트를 표시하고 입력 텍스트(또는 수치 조절 delta)만
//! 넘깁니다. 값 해석·정규화·node 종류별 저장 위치(payload longhand, Fill·Stroke modifier)는 여기서 정합니다.
//!
//! - 행은 UX 명세 6.2절 표에 있고 저장 위치(PropertyKey)가 있는 속성만 냅니다. 묶음 행(`border-radius`)은
//!   longhand 4개를 펼친 shorthand이며 편집은 longhand 변경 집합 하나입니다(속성 모델 11장).
//! - 다중 선택은 모든 node에 있는 속성만 내고, 값이 다르면 `MIXED`입니다. 편집은 선택 전체를 한 transaction으로
//!   바꿉니다(Undo 한 번).
//! - 잠긴 node(자신 또는 조상)는 `locked` 행 외에는 읽기 전용이고 편집을 거부합니다(D8, [`NodeAccess`]).
//!   잠금·흐름 부모는 node마다 한 번 계산해 행 판정에 다시 씁니다.
//! - 이름·표시·잠금 편집은 Layers와 같은 command(`rename_node`·`set_visibility`·`set_locked`)입니다.
//! - `fill`·`stroke`는 늘 있는 행입니다. 칠하기가 없으면 `none`이고, 값을 넣으면 modifier를 선언 블록 붙여넣기와
//!   같은 삽입 순위(`modifier_ops::css_insert_order`, Transform → Fill → Stroke)에 맞춰 넣고, `none`이면
//!   뺍니다. 선이 없는 node에 `stroke` 색을 넣으면 `stroke-width`는 CSS 초기값 1px로 시작합니다. 색 기본값은 코어에 두지 않으므로(D7), 선이 없는 node의
//!   `stroke-width`만 바꾸거나 색 없는 새 `border`를 넣는 입력은 거부합니다.
//! - node마다 modifier stack을 한 번만 훑어 요약하고 속성 행은 그 요약에서 읽습니다(속성 수만큼 stack을 다시
//!   훑거나 복제하지 않음).

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::value::{FiniteF32, Transform2D};
use stellar_document::frame::{keys as frame_keys, Border, Clipping};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload};
use stellar_document::modifier_metadata::keys as modifier_keys;
use stellar_document::payload_metadata::{image_keys, shape_keys};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::ShapeGeometry;

use crate::change::node_keys;
use crate::command::Command;
use crate::css_value::{format, parse, CssGrammar, CssValue, CssValueError};
use crate::lock::{Blocked, EditIntent, NodeAccess};
use crate::modifier_ops::{
    css_insert_order, read_payload, InsertModifier, RemoveModifier, SetModifierProperty,
};
use crate::property::{read_property, PropertyValue, SetProperty};

/// 행 flag.
pub mod flags {
    /// 선택한 node들의 값이 다름.
    pub const MIXED: u32 = 1;
    /// 편집할 수 없음(잠김).
    pub const READ_ONLY: u32 = 1 << 1;
    /// 값이 색(견본 표시).
    pub const COLOR: u32 = 1 << 2;
    /// longhand를 묶은 shorthand 행. 뒤에 `LONGHAND` 행이 이어집니다.
    pub const GROUP: u32 = 1 << 3;
    /// 앞 `GROUP` 행에 속한 longhand.
    pub const LONGHAND: u32 = 1 << 4;
    /// CSS에 대응이 없는 Stellar 전용 속성.
    pub const STELLAR_ONLY: u32 = 1 << 5;
    /// 수치 조절(화살표·scrub)을 받음.
    pub const NUMERIC: u32 = 1 << 6;
    /// keyword 하나를 고름(선택지 있음).
    pub const KEYWORD: u32 = 1 << 7;
    /// true·false.
    pub const BOOL: u32 = 1 << 8;
}

/// 속성 패널 섹션(속성 모델 12장 CSS 범주). 숫자 순서가 표시 순서입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum InspectorSection {
    /// 이름·표시·잠금.
    Layer = 1,
    /// `left`·`top`.
    Position = 2,
    /// `width`·`height`.
    Size = 3,
    /// `overflow`.
    Overflow = 4,
    /// Text `color`.
    Typography = 5,
    /// `background-color`.
    Background = 6,
    /// `fill`·`stroke`.
    FillStroke = 7,
    /// `border`·`border-radius`.
    Border = 8,
    /// 종류별(Line 끝점, Polygon 변 수, Image `object-fit`).
    Kind = 9,
}

/// 속성 패널이 다루는 속성(UX 명세 6.2절 표에 있는 것만).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectorProperty {
    /// 레이어 이름.
    Name,
    /// 표시 여부.
    Visible,
    /// 잠금.
    Locked,
    /// `left`(Frame transform 이동 성분, 그 밖은 Transform modifier).
    Left,
    /// `top`.
    Top,
    /// `width`.
    Width,
    /// `height`.
    Height,
    /// `overflow`(Frame clipping).
    Overflow,
    /// Text `color`(Fill modifier).
    Color,
    /// `background-color`.
    BackgroundColor,
    /// `fill`(Fill modifier).
    Fill,
    /// `stroke`(Stroke modifier 색).
    Stroke,
    /// `stroke-width`.
    StrokeWidth,
    /// `border`.
    Border,
    /// `border-radius` shorthand.
    BorderRadius,
    /// `border-*-radius`(TL, TR, BR, BL index).
    Corner(u8),
    /// Line `x1`.
    X1,
    /// Line `y1`.
    Y1,
    /// Line `x2`.
    X2,
    /// Line `y2`.
    Y2,
    /// Polygon 변 수.
    PolygonSides,
    /// Polygon 회전.
    PolygonRotation,
    /// Polygon 모서리 반경.
    PolygonCornerRadius,
    /// Image `object-fit`.
    ObjectFit,
}

use InspectorProperty as P;

/// 표시 순서.
const ORDER: [InspectorProperty; 27] = [
    P::Name,
    P::Visible,
    P::Locked,
    P::Left,
    P::Top,
    P::Width,
    P::Height,
    P::Overflow,
    P::Color,
    P::BackgroundColor,
    P::Fill,
    P::Stroke,
    P::StrokeWidth,
    P::Border,
    P::BorderRadius,
    P::Corner(0),
    P::Corner(1),
    P::Corner(2),
    P::Corner(3),
    P::X1,
    P::Y1,
    P::X2,
    P::Y2,
    P::PolygonSides,
    P::PolygonRotation,
    P::PolygonCornerRadius,
    P::ObjectFit,
];

const FIT: &[&str] = &["fill", "contain", "cover", "none"];
const OVERFLOW: &[&str] = &["visible", "clip"];
const CORNER_NAMES: [&str; 4] = [
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-right-radius",
    "border-bottom-left-radius",
];

/// 선이 없는 node에 `stroke` 색을 넣을 때의 두께: CSS `stroke-width` 초기값(1px)입니다. 사용자 기본 스타일이
/// 아니라 CSS가 정한 값입니다(색 기본값은 셸 설정, D7).
const STROKE_WIDTH_INITIAL: FiniteF32 = FiniteF32::ONE;

const LENGTH: CssGrammar = CssGrammar::Length {
    non_negative: false,
    sizing: false,
};
const SIZE: CssGrammar = CssGrammar::Length {
    non_negative: true,
    sizing: true,
};
const DIMENSION: CssGrammar = CssGrammar::Length {
    non_negative: true,
    sizing: false,
};

/// 선택 node의 분류(행 적용 범위).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeClass {
    /// Frame.
    Frame = 0,
    /// Rectangle.
    Rectangle = 1,
    /// Ellipse.
    Ellipse = 2,
    /// Line.
    Line = 3,
    /// Polygon.
    Polygon = 4,
    /// Text.
    Text = 5,
    /// Image.
    Image = 6,
    /// Fragment·Path·컴포넌트 인스턴스(레이어 행만). Path는 앱 캔버스가 아직 그리지 않고 만드는 도구도 없어
    /// 칠하기·선 행을 두지 않습니다(바꿔도 보이지 않는 값이 됨).
    Other = 7,
}

impl NodeClass {
    /// payload 분류.
    pub fn of(payload: &NodePayload) -> NodeClass {
        match payload {
            NodePayload::Frame(_) => NodeClass::Frame,
            NodePayload::Shape(ShapeGeometry::Rectangle(_)) => NodeClass::Rectangle,
            NodePayload::Shape(ShapeGeometry::Ellipse(_)) => NodeClass::Ellipse,
            NodePayload::Shape(ShapeGeometry::Line(_)) => NodeClass::Line,
            NodePayload::Shape(ShapeGeometry::Polygon(_)) => NodeClass::Polygon,
            NodePayload::Text(_) => NodeClass::Text,
            NodePayload::Image(_) => NodeClass::Image,
            _ => NodeClass::Other,
        }
    }

    fn is_shape(self) -> bool {
        matches!(
            self,
            NodeClass::Rectangle | NodeClass::Ellipse | NodeClass::Line | NodeClass::Polygon
        )
    }
}

impl InspectorProperty {
    /// CSS longhand 이름(Stellar 전용이면 Stellar 이름). 셸과 주고받는 식별 이름입니다.
    pub fn name(self) -> &'static str {
        match self {
            P::Name => "name",
            P::Visible => "visible",
            P::Locked => "locked",
            P::Left => "left",
            P::Top => "top",
            P::Width => "width",
            P::Height => "height",
            P::Overflow => "overflow",
            P::Color => "color",
            P::BackgroundColor => "background-color",
            P::Fill => "fill",
            P::Stroke => "stroke",
            P::StrokeWidth => "stroke-width",
            P::Border => "border",
            P::BorderRadius => "border-radius",
            P::Corner(index) => CORNER_NAMES[usize::from(index) & 3],
            P::X1 => "x1",
            P::Y1 => "y1",
            P::X2 => "x2",
            P::Y2 => "y2",
            P::PolygonSides => "polygon-sides",
            P::PolygonRotation => "polygon-rotation",
            P::PolygonCornerRadius => "polygon-corner-radius",
            P::ObjectFit => "object-fit",
        }
    }

    /// 모든 속성(표시 순서).
    pub fn all() -> &'static [InspectorProperty] {
        &ORDER
    }

    /// 이름으로 찾습니다.
    pub fn from_name(name: &str) -> Option<InspectorProperty> {
        ORDER.into_iter().find(|p| p.name() == name)
    }

    /// 섹션.
    pub fn section(self) -> InspectorSection {
        use InspectorSection as S;
        match self {
            P::Name | P::Visible | P::Locked => S::Layer,
            P::Left | P::Top => S::Position,
            P::Width | P::Height => S::Size,
            P::Overflow => S::Overflow,
            P::Color => S::Typography,
            P::BackgroundColor => S::Background,
            P::Fill | P::Stroke | P::StrokeWidth => S::FillStroke,
            P::Border | P::BorderRadius | P::Corner(_) => S::Border,
            _ => S::Kind,
        }
    }

    /// 값 문법.
    pub fn grammar(self) -> CssGrammar {
        match self {
            P::Name => CssGrammar::Text,
            P::Visible | P::Locked => CssGrammar::Bool,
            P::Left | P::Top | P::X1 | P::Y1 | P::X2 | P::Y2 => LENGTH,
            P::Width | P::Height => SIZE,
            P::StrokeWidth | P::Corner(_) | P::PolygonCornerRadius => DIMENSION,
            P::Overflow => CssGrammar::Keyword(OVERFLOW),
            P::Color => CssGrammar::Color { allow_none: false },
            P::BackgroundColor | P::Fill | P::Stroke => CssGrammar::Color { allow_none: true },
            P::Border => CssGrammar::Border,
            P::BorderRadius => CssGrammar::Corners,
            P::PolygonSides => CssGrammar::Count { min: 3, max: 1000 },
            P::PolygonRotation => CssGrammar::Angle,
            P::ObjectFit => CssGrammar::Keyword(FIT),
        }
    }

    /// keyword 선택지(없으면 빈 목록).
    pub fn choices(self) -> &'static [&'static str] {
        match self.grammar() {
            CssGrammar::Keyword(choices) => choices,
            CssGrammar::Bool => &["true", "false"],
            _ => &[],
        }
    }

    fn base_flags(self) -> u32 {
        let grammar = self.grammar();
        let mut out = 0;
        if grammar.is_numeric() {
            out |= flags::NUMERIC;
        }
        match grammar {
            CssGrammar::Color { .. } => out |= flags::COLOR,
            CssGrammar::Keyword(_) => out |= flags::KEYWORD,
            CssGrammar::Bool => out |= flags::BOOL,
            _ => {}
        }
        match self {
            P::BorderRadius => out |= flags::GROUP,
            P::Corner(_) => out |= flags::LONGHAND,
            P::Name
            | P::Visible
            | P::Locked
            | P::X1
            | P::Y1
            | P::X2
            | P::Y2
            | P::PolygonSides
            | P::PolygonRotation
            | P::PolygonCornerRadius => out |= flags::STELLAR_ONLY,
            _ => {}
        }
        out
    }

    /// `class` node에 이 속성이 있는지.
    pub fn applies(self, class: NodeClass) -> bool {
        use NodeClass as C;
        match self {
            P::Name | P::Visible | P::Locked => true,
            P::Left | P::Top => class != C::Other,
            P::Width | P::Height => matches!(
                class,
                C::Frame | C::Rectangle | C::Ellipse | C::Polygon | C::Image
            ),
            P::Overflow | P::BackgroundColor | P::Border => class == C::Frame,
            P::BorderRadius | P::Corner(_) => matches!(class, C::Frame | C::Rectangle),
            P::Color => class == C::Text,
            P::Fill => matches!(class, C::Rectangle | C::Ellipse | C::Polygon),
            P::Stroke | P::StrokeWidth => class.is_shape(),
            P::X1 | P::Y1 | P::X2 | P::Y2 => class == C::Line,
            P::PolygonSides | P::PolygonRotation | P::PolygonCornerRadius => class == C::Polygon,
            P::ObjectFit => class == C::Image,
        }
    }

    /// 저장 위치 key. 묶음 행은 longhand가 저장하므로 `PropertyKey(0)`입니다.
    pub fn key(self, class: NodeClass) -> PropertyKey {
        let frame = class == NodeClass::Frame;
        let image = class == NodeClass::Image;
        match self {
            P::Name => node_keys::DISPLAY_NAME,
            P::Visible => node_keys::VISIBLE,
            P::Locked => node_keys::LOCKED,
            P::Left | P::Top if frame => frame_keys::TRANSFORM,
            P::Left | P::Top => modifier_keys::TRANSFORM_MATRIX,
            P::Width if frame => frame_keys::WIDTH,
            P::Width if image => image_keys::WIDTH,
            P::Width => shape_keys::WIDTH,
            P::Height if frame => frame_keys::HEIGHT,
            P::Height if image => image_keys::HEIGHT,
            P::Height => shape_keys::HEIGHT,
            P::Overflow => frame_keys::CLIPPING,
            P::Color | P::Fill => modifier_keys::FILL_COLOR,
            P::BackgroundColor => frame_keys::BACKGROUND,
            P::Stroke => modifier_keys::STROKE_COLOR,
            P::StrokeWidth => modifier_keys::STROKE_WIDTH,
            P::Border => frame_keys::BORDER,
            P::BorderRadius => PropertyKey(0),
            P::Corner(i) if frame => frame_keys::BORDER_RADIUS_LONGHANDS[usize::from(i) & 3],
            P::Corner(i) => shape_keys::BORDER_RADIUS_LONGHANDS[usize::from(i) & 3],
            P::X1 => shape_keys::X1,
            P::Y1 => shape_keys::Y1,
            P::X2 => shape_keys::X2,
            P::Y2 => shape_keys::Y2,
            P::PolygonSides => shape_keys::POLYGON_SIDES,
            P::PolygonRotation => shape_keys::POLYGON_ROTATION,
            P::PolygonCornerRadius => shape_keys::POLYGON_CORNER_RADIUS,
            P::ObjectFit => image_keys::OBJECT_FIT,
        }
    }
}

/// 속성 패널 한 행.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectorRow {
    /// 속성.
    pub property: InspectorProperty,
    /// 첫 선택 node 기준 저장 key.
    pub key: PropertyKey,
    /// [`flags`].
    pub flags: u32,
    /// 정규 CSS 값 텍스트(`MIXED`면 빈 문자열).
    pub value: String,
}

/// 선택 전체의 속성 패널 snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InspectorSnapshot {
    /// 문서에 있는 선택 node 수.
    pub count: u32,
    /// 선택에 있는 [`NodeClass`] bit 집합(`1 << class`).
    pub classes: u32,
    /// 단일 선택이면 레이어 이름, 선택이 없으면 현재 Page 요약(`Page · View`, runtime이 채움).
    pub display_name: String,
    /// 행(섹션·표시 순서).
    pub rows: Vec<InspectorRow>,
}

/// node의 활성 modifier 요약. stack을 한 번 훑어 만들고 모든 속성 행이 나눠 씁니다.
struct Paint {
    /// 앞쪽 연속 Transform modifier를 합성한 변환.
    transform: Transform2D,
    /// 앞쪽 연속 Transform modifier 중 마지막(이동 편집 대상).
    last_transform: Option<ModifierEntry>,
    /// 첫 활성 Fill.
    fill: Option<ModifierEntry>,
    /// 첫 활성 Stroke.
    stroke: Option<ModifierEntry>,
}

fn paint(model: &DocumentModel, id: StableId) -> Paint {
    let mut out = Paint {
        transform: Transform2D::identity(),
        last_transform: None,
        fill: None,
        stroke: None,
    };
    let Some(stack) = model.modifier_stack(id) else {
        return out;
    };
    let mut leading = true;
    for entry in stack.entries().iter().filter(|e| e.enabled) {
        match (&entry.payload, leading) {
            (ModifierPayload::Transform(next), true) => {
                out.transform = out.transform.then(next).unwrap_or(out.transform);
                out.last_transform = Some(entry.clone());
            }
            _ => leading = false,
        }
        match entry.kind {
            ModifierKind::Fill if out.fill.is_none() => out.fill = Some(entry.clone()),
            ModifierKind::Stroke if out.stroke.is_none() => out.stroke = Some(entry.clone()),
            _ => {}
        }
    }
    out
}

/// node가 활성 Fill modifier를 가지는지. Text `color`는 Fill이 없으면 문서 기본 글자색을 물려받아 표시할 뿐
/// 명시한 값이 아닙니다(선언 블록 복사가 명시 값만 내는 데 씁니다, ADR-0082 결정 5).
pub(crate) fn has_fill(model: &DocumentModel, id: StableId) -> bool {
    paint(model, id).fill.is_some()
}

struct NodeView {
    record: NodeRecord,
    class: NodeClass,
    access: NodeAccess,
    paint: Paint,
}

impl NodeView {
    /// 이 속성 편집의 D8·흐름 부모 판정. 조상 정보는 [`node_view`]에서 계산한 값을 다시 씁니다.
    fn permission(&self, property: InspectorProperty) -> Result<(), Blocked> {
        let intent = if property == P::Locked {
            EditIntent::ToggleLock
        } else {
            EditIntent::Modify(property.key(self.class))
        };
        self.access.permits(intent)
    }
}

fn node_view(model: &DocumentModel, id: StableId) -> Option<NodeView> {
    let record = model.record(id)?;
    Some(NodeView {
        class: NodeClass::of(&record.payload),
        access: NodeAccess::of(model, id)?,
        paint: paint(model, id),
        record,
    })
}

/// 위치(`left`·`top`, 캔버스 이동·크기 원점)를 고칠 Transform modifier: 켜진 modifier 앞부분 Transform 가운데
/// 마지막 것입니다. 없으면 맨 앞에 새로 넣습니다. 속성 탭과 캔버스(ADR-0071)가 이 규칙 하나를 씁니다.
pub fn position_modifier(model: &DocumentModel, id: StableId) -> Option<ModifierEntry> {
    paint(model, id).last_transform
}

fn length_of(value: f32) -> CssValue {
    CssValue::Length(value)
}

fn read_scalar(record: &NodeRecord, key: PropertyKey) -> Option<f32> {
    match read_property(record, key)? {
        PropertyValue::NonNegative(v) | PropertyValue::Number(v) => Some(v.get()),
        _ => None,
    }
}

fn read_keyword(record: &NodeRecord, key: PropertyKey) -> Option<usize> {
    match read_property(record, key)? {
        PropertyValue::Keyword(tag) => Some(usize::from(tag)),
        _ => None,
    }
}

fn text_default_color(model: &DocumentModel) -> Color {
    model
        .definition()
        .text_styles
        .default_fill
        .unwrap_or_else(|| Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).expect("black"))
}

fn read(model: &DocumentModel, node: &NodeView, property: InspectorProperty) -> Option<CssValue> {
    let record = &node.record;
    let key = property.key(node.class);
    let paint = &node.paint;
    Some(match property {
        P::Name => CssValue::Text(record.display_name.clone()),
        P::Visible => CssValue::Bool(record.visible),
        P::Locked => CssValue::Bool(record.locked),
        P::Left | P::Top => {
            let transform = match &record.payload {
                NodePayload::Frame(frame) => frame.transform,
                _ => paint.transform,
            };
            let (_, _, _, _, x, y) = transform.components();
            length_of(if property == P::Left { x } else { y })
        }
        P::Overflow => match &record.payload {
            NodePayload::Frame(frame) => CssValue::Keyword(match frame.clipping {
                Clipping::Disabled => 0,
                _ => 1,
            }),
            _ => return None,
        },
        P::BackgroundColor => match &record.payload {
            NodePayload::Frame(frame) => CssValue::Color(frame.background),
            _ => return None,
        },
        P::Border => match &record.payload {
            NodePayload::Frame(frame) => {
                CssValue::Border(frame.border.map(|b| (b.thickness(), b.color)))
            }
            _ => return None,
        },
        P::Color => CssValue::Color(Some(match paint.fill.as_ref().map(|e| &e.payload) {
            Some(ModifierPayload::Fill { color }) => *color,
            _ => text_default_color(model),
        })),
        P::Fill => CssValue::Color(match paint.fill.as_ref().map(|e| &e.payload) {
            Some(ModifierPayload::Fill { color }) => Some(*color),
            _ => None,
        }),
        P::Stroke | P::StrokeWidth => {
            let stroke = paint.stroke.as_ref().map(|e| &e.payload);
            match (property, stroke) {
                (P::Stroke, Some(ModifierPayload::Stroke { color, .. })) => {
                    CssValue::Color(Some(*color))
                }
                (P::Stroke, _) => CssValue::Color(None),
                (_, Some(ModifierPayload::Stroke { width, .. })) => length_of(width.get()),
                _ => length_of(0.),
            }
        }
        P::BorderRadius => {
            let mut corners = [0.; 4];
            for (index, corner) in corners.iter_mut().enumerate() {
                *corner = read_scalar(record, P::Corner(index as u8).key(node.class))?;
            }
            CssValue::Corners(corners)
        }
        P::PolygonSides => CssValue::Count(read_scalar(record, key)? as u32),
        P::PolygonRotation => CssValue::Angle(read_scalar(record, key)?),
        P::ObjectFit => CssValue::Keyword(read_keyword(record, key)?),
        _ => length_of(read_scalar(record, key)?),
    })
}

/// 선택의 속성 행을 만듭니다. 문서에 없는 id는 건너뜁니다. 비용은 선택 node 수 × (modifier 수 + 조상 수 +
/// 속성 수)입니다.
pub fn inspector_rows(model: &DocumentModel, selection: &[StableId]) -> InspectorSnapshot {
    let nodes: Vec<NodeView> = selection
        .iter()
        .filter_map(|id| node_view(model, *id))
        .collect();
    let mut snapshot = InspectorSnapshot {
        count: nodes.len() as u32,
        classes: nodes.iter().fold(0, |bits, n| bits | 1 << (n.class as u8)),
        display_name: match nodes.as_slice() {
            [node] => node.record.display_name.clone(),
            _ => String::new(),
        },
        rows: Vec::new(),
    };
    let Some(first) = nodes.first() else {
        return snapshot;
    };
    let mut order: Vec<InspectorProperty> = ORDER
        .into_iter()
        .filter(|p| nodes.iter().all(|n| p.applies(n.class)))
        .collect();
    order.sort_by_key(|p| p.section());
    for property in order {
        let grammar = property.grammar();
        // 값은 해석한 값끼리 비교하고 텍스트는 한 번만 만듭니다(정규 텍스트는 값과 1:1).
        let mut value: Option<CssValue> = None;
        let mut mixed = false;
        let mut read_only = false;
        let mut readable = true;
        for node in &nodes {
            let Some(current) = read(model, node, property) else {
                readable = false;
                break;
            };
            match &value {
                None => value = Some(current),
                Some(previous) if !mixed && *previous != current => mixed = true,
                Some(_) => {}
            }
            read_only |= node.permission(property).is_err();
        }
        if !readable {
            continue;
        }
        let mut flags = property.base_flags();
        if mixed {
            flags |= flags::MIXED;
        }
        if read_only {
            flags |= flags::READ_ONLY;
        }
        if matches!(property, P::Left | P::Top) && nodes.iter().any(|n| n.record.parent.is_none()) {
            // 부모 없는 Frame의 위치는 캔버스 배치(Stellar 전용, 속성 모델 4.6절)입니다.
            flags |= flags::STELLAR_ONLY;
        }
        snapshot.rows.push(InspectorRow {
            property,
            key: property.key(first.class),
            flags,
            value: match (&value, mixed) {
                (Some(value), false) => format(value, grammar),
                _ => String::new(),
            },
        });
    }
    snapshot
}

/// 편집 입력.
#[derive(Debug, Clone, PartialEq)]
pub enum InspectorInput {
    /// 해석한 값으로 설정.
    Set(CssValue),
    /// 각 node의 현재 값에 더함(px·deg·개수). 혼합 값의 화살표·scrub입니다.
    Adjust(f64),
}

/// 셸 입력 텍스트를 속성 문법으로 해석합니다. 문서와 무관한 순수 함수라 셸 호출에서 바로 진단할 수 있습니다.
pub fn parse_input(property: InspectorProperty, text: &str) -> Result<CssValue, CssValueError> {
    parse(property.grammar(), text)
}

/// 편집 거부 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectorEditError(pub String);

fn reject(reason: impl Into<String>) -> InspectorEditError {
    InspectorEditError(reason.into())
}

fn finite(value: f32) -> Result<FiniteF32, InspectorEditError> {
    FiniteF32::new(value).map_err(|_| reject("값이 유한하지 않습니다"))
}

fn adjusted(
    current: &CssValue,
    delta: f64,
    grammar: CssGrammar,
) -> Result<CssValue, InspectorEditError> {
    let shift = |v: f32| v + delta as f32;
    Ok(match (current, grammar) {
        (CssValue::Length(v), CssGrammar::Length { non_negative, .. }) => {
            let next = shift(*v);
            CssValue::Length(if non_negative { next.max(0.) } else { next })
        }
        (CssValue::Angle(v), _) => CssValue::Angle(shift(*v)),
        (CssValue::Count(v), CssGrammar::Count { min, max }) => {
            let next = (f64::from(*v) + delta.round()).clamp(f64::from(min), f64::from(max));
            CssValue::Count(next as u32)
        }
        (CssValue::Corners(corners), _) => CssValue::Corners(corners.map(|c| shift(c).max(0.))),
        _ => return Err(reject("이 속성은 수치 조절을 받지 않습니다")),
    })
}

/// 새 modifier를 넣는 command. ID는 호출자가 준 공급자(`new_id`)에서 받습니다.
fn insert(
    model: &DocumentModel,
    node: StableId,
    payload: ModifierPayload,
    new_id: &mut dyn FnMut() -> StableId,
) -> Command {
    let kind = payload.kind();
    Command::InsertModifier(InsertModifier {
        node,
        entry: ModifierEntry {
            id: ModifierId::new(new_id()),
            order: css_insert_order(model, node, kind),
            kind,
            enabled: true,
            payload,
        },
    })
}

fn set_property(node: StableId, key: PropertyKey, value: PropertyValue) -> Command {
    Command::SetProperty(SetProperty { node, key, value })
}

fn set_modifier(
    node: StableId,
    entry: &ModifierEntry,
    key: PropertyKey,
    value: PropertyValue,
) -> Command {
    Command::SetModifierProperty(SetModifierProperty {
        node,
        modifier: entry.id,
        key,
        value,
    })
}

/// 한 node에 `value`를 쓰는 command(값이 같으면 빈 목록).
fn node_commands(
    model: &DocumentModel,
    node: &NodeView,
    property: InspectorProperty,
    value: &CssValue,
    new_id: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, InspectorEditError> {
    let id = node.record.id;
    let class = node.class;
    let key = property.key(class);
    let mismatch = || reject("값이 이 속성의 문법과 맞지 않습니다");
    let mut out = Vec::new();
    match (property, value) {
        // 이름·표시·잠금은 Layers와 같은 command입니다(VII.2.5 specialization).
        (P::Name, CssValue::Text(name)) => out.push(Command::rename_node(id, name.clone())),
        (P::Visible, CssValue::Bool(v)) => out.push(Command::set_visibility(id, *v)),
        (P::Locked, CssValue::Bool(v)) => out.push(Command::set_locked(id, *v)),
        (P::Left | P::Top, CssValue::Length(v)) => {
            let set = |t: Transform2D| {
                let (a, b, c, d, x, y) = t.components();
                let (x, y) = if property == P::Left {
                    (*v, y)
                } else {
                    (x, *v)
                };
                Transform2D::new(a, b, c, d, x, y).map_err(|_| reject("위치가 유한하지 않습니다"))
            };
            if let NodePayload::Frame(frame) = &node.record.payload {
                out.push(set_property(
                    id,
                    key,
                    PropertyValue::Transform(set(frame.transform)?),
                ));
            } else {
                let (_, _, _, _, x, y) = node.paint.transform.components();
                let (dx, dy) = if property == P::Left {
                    (*v - x, 0.)
                } else {
                    (0., *v - y)
                };
                match &node.paint.last_transform {
                    Some(entry) => {
                        let ModifierPayload::Transform(t) = &entry.payload else {
                            unreachable!("Transform만 골랐습니다");
                        };
                        let (a, b, c, d, tx, ty) = t.components();
                        let next = Transform2D::new(a, b, c, d, tx + dx, ty + dy)
                            .map_err(|_| reject("위치가 유한하지 않습니다"))?;
                        out.push(set_modifier(id, entry, key, PropertyValue::Transform(next)));
                    }
                    // 값이 같으면 빈 이동 modifier를 넣지 않습니다(빈 변경을 history에 남기지 않음).
                    None if dx == 0. && dy == 0. => {}
                    None => {
                        let t = Transform2D::new(1., 0., 0., 1., dx, dy)
                            .map_err(|_| reject("위치가 유한하지 않습니다"))?;
                        out.push(insert(model, id, ModifierPayload::Transform(t), new_id));
                    }
                }
            }
        }
        (P::Overflow, CssValue::Keyword(index)) => {
            let clipping = if *index == 0 {
                Clipping::Disabled
            } else {
                Clipping::FrameBounds
            };
            if let NodePayload::Frame(frame) = &node.record.payload {
                if matches!(frame.clipping, Clipping::Path(_)) {
                    return Err(reject(
                        "clip-path가 있는 Frame은 overflow를 바꿀 수 없습니다",
                    ));
                }
            }
            out.push(set_property(id, key, PropertyValue::Clipping(clipping)));
        }
        (P::BackgroundColor, CssValue::Color(color)) => {
            out.push(set_property(id, key, PropertyValue::OptionalColor(*color)))
        }
        (P::Border, CssValue::Border(border)) => {
            let border = match border {
                None => None,
                Some((width, color)) => Some(
                    Border::new(*color, *width)
                        .map_err(|_| reject("테두리 두께가 잘못되었습니다"))?,
                ),
            };
            out.push(set_property(id, key, PropertyValue::OptionalBorder(border)));
        }
        (P::Border, CssValue::BorderParts(width, color)) => {
            // 빠진 쪽(두께 또는 색)은 지금 테두리 값을 이어 씁니다. 테두리가 없으면 색 기본값을 코어가 정하지
            // 않으므로 거부합니다(D7).
            let existing = match &node.record.payload {
                NodePayload::Frame(frame) => frame.border,
                _ => None,
            };
            let Some(existing) = existing else {
                return Err(reject(
                    "테두리가 없는 Frame에는 두께와 색을 함께 넣으세요(예: 1px solid #000000)",
                ));
            };
            let next = Border::new(
                color.unwrap_or(existing.color),
                width.unwrap_or(existing.thickness()),
            )
            .map_err(|_| reject("테두리 두께가 잘못되었습니다"))?;
            out.push(set_property(
                id,
                key,
                PropertyValue::OptionalBorder(Some(next)),
            ));
        }
        (P::Color | P::Fill, CssValue::Color(color)) => match (color, &node.paint.fill) {
            (Some(color), Some(entry)) => {
                out.push(set_modifier(id, entry, key, PropertyValue::Color(*color)))
            }
            (Some(color), None) => out.push(insert(
                model,
                id,
                ModifierPayload::Fill { color: *color },
                new_id,
            )),
            (None, Some(entry)) => out.push(Command::RemoveModifier(RemoveModifier {
                node: id,
                modifier: entry.id,
            })),
            (None, None) => {}
        },
        (P::Stroke, CssValue::Color(color)) => match (color, &node.paint.stroke) {
            (Some(color), Some(entry)) => {
                out.push(set_modifier(id, entry, key, PropertyValue::Color(*color)))
            }
            (Some(color), None) => out.push(insert(
                model,
                id,
                ModifierPayload::Stroke {
                    color: *color,
                    width: STROKE_WIDTH_INITIAL,
                },
                new_id,
            )),
            (None, Some(entry)) => out.push(Command::RemoveModifier(RemoveModifier {
                node: id,
                modifier: entry.id,
            })),
            (None, None) => {}
        },
        (P::StrokeWidth, CssValue::Length(width)) => match &node.paint.stroke {
            Some(entry) => out.push(set_modifier(
                id,
                entry,
                key,
                PropertyValue::NonNegative(finite(*width)?),
            )),
            None if *width > 0. => {
                return Err(reject("선이 없는 레이어입니다: 먼저 stroke 색을 넣으세요"))
            }
            None => {}
        },
        (P::BorderRadius, CssValue::Corners(corners)) => {
            for (index, radius) in corners.iter().enumerate() {
                let corner = P::Corner(index as u8);
                out.extend(node_commands(
                    model,
                    node,
                    corner,
                    &CssValue::Length(*radius),
                    new_id,
                )?);
            }
        }
        (P::Width | P::Height | P::Corner(_) | P::PolygonCornerRadius, CssValue::Length(v)) => out
            .push(set_property(
                id,
                key,
                PropertyValue::NonNegative(finite(*v)?),
            )),
        (P::X1 | P::Y1 | P::X2 | P::Y2, CssValue::Length(v)) => {
            out.push(set_property(id, key, PropertyValue::Number(finite(*v)?)))
        }
        (P::PolygonRotation, CssValue::Angle(v)) => {
            out.push(set_property(id, key, PropertyValue::Number(finite(*v)?)))
        }
        (P::PolygonSides, CssValue::Count(v)) => out.push(set_property(
            id,
            key,
            PropertyValue::Number(finite(*v as f32)?),
        )),
        (P::ObjectFit, CssValue::Keyword(index)) => {
            let tag = u8::try_from(*index).map_err(|_| mismatch())?;
            out.push(set_property(id, key, PropertyValue::Keyword(tag)))
        }
        _ => return Err(mismatch()),
    }
    // 값이 같은 SetProperty·SetModifierProperty는 뺍니다(history에 빈 변경을 남기지 않음).
    out.retain(|command| match command {
        Command::SetProperty(set) => {
            read_property(&node.record, set.key).as_ref() != Some(&set.value)
        }
        Command::SetModifierProperty(set) => {
            let current = model
                .modifier_stack(set.node)
                .and_then(|stack| stack.entries().iter().find(|e| e.id == set.modifier))
                .and_then(|entry| read_payload(&entry.payload, set.key));
            current.as_ref() != Some(&set.value)
        }
        _ => true,
    });
    Ok(out)
}

/// 선택 전체에 `input`을 적용하는 command 목록. 값이 모두 같으면 빈 목록입니다.
///
/// `new_id`는 새 modifier(Fill·Stroke·위치 Transform)를 넣을 때만 부르며, 부를 때마다 다른 영속 ID를 돌려줘야
/// 합니다. 제품 경로는 UUIDv7(OS 난수, `stellar_project::new_ids`)을 넘깁니다. command ID에서 파생하지 않습니다.
pub fn edit_commands(
    model: &DocumentModel,
    selection: &[StableId],
    property: InspectorProperty,
    input: &InspectorInput,
    new_id: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, InspectorEditError> {
    let mut commands = Vec::new();
    if selection.is_empty() {
        return Err(reject("선택한 레이어가 없습니다"));
    }
    for id in selection {
        let node =
            node_view(model, *id).ok_or_else(|| reject("선택한 레이어가 문서에 없습니다"))?;
        if !property.applies(node.class) {
            return Err(reject(format!(
                "{} 속성이 없는 레이어가 선택에 있습니다",
                property.name()
            )));
        }
        match node.permission(property) {
            Ok(()) => {}
            Err(Blocked::FlowParent) => {
                return Err(reject(
                    "흐름 레이아웃 안의 레이어는 위치를 바꿀 수 없습니다",
                ))
            }
            Err(_) => return Err(reject("잠긴 레이어는 바꿀 수 없습니다")),
        }
        let value = match input {
            InspectorInput::Set(value) => value.clone(),
            InspectorInput::Adjust(delta) => {
                let current = read(model, &node, property)
                    .ok_or_else(|| reject("현재 값을 읽을 수 없습니다"))?;
                adjusted(&current, *delta, property.grammar())?
            }
        };
        commands.extend(node_commands(model, &node, property, &value, new_id)?);
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_fixtures::sid;
    use crate::lock::NodeLock;
    use crate::property::tests::seven_kind_model;
    use crate::state::DocumentState;
    use crate::transaction::CommandTransaction;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;

    /// 테스트용 새 modifier ID 공급자(`base`마다 다른 UUIDv7 열).
    fn new_ids(base: u64) -> impl FnMut() -> StableId {
        let mut n = 0;
        move || {
            n += 1;
            StableId::new_v7(base, 0, n)
        }
    }

    fn names(snapshot: &InspectorSnapshot) -> Vec<&'static str> {
        snapshot.rows.iter().map(|r| r.property.name()).collect()
    }

    fn value<'a>(snapshot: &'a InspectorSnapshot, name: &str) -> &'a InspectorRow {
        snapshot
            .rows
            .iter()
            .find(|r| r.property.name() == name)
            .unwrap_or_else(|| panic!("{name} 행 없음: {:?}", names(snapshot)))
    }

    fn apply(state: &mut DocumentState, commands: Vec<Command>) -> CommandTransaction {
        let mut transaction = CommandTransaction::new(
            crate::id::CommandId::from_bytes([9; 16]),
            crate::id::CommandOrigin::User,
        );
        for command in commands {
            transaction = transaction.with_command(state.revision(), command);
        }
        state.execute_transaction(&transaction).unwrap().inverse
    }

    #[test]
    #[ignore = "release에서 Fill·Stroke·Transform이 있는 Rectangle 1만 개 선택의 snapshot 시간 측정"]
    fn measure_snapshot_10k_rectangles() {
        use stellar_core_types::order_key::OrderKey;
        use stellar_core_types::value::Size2;
        use stellar_document::shape::RectangleParams;
        let mut model = DocumentModel::new(crate::command::test_fixtures::definition());
        model
            .insert_node(crate::command::test_fixtures::frame(10, None, 0x10))
            .unwrap();
        model
            .insert_node(crate::command::test_fixtures::frame(11, Some(10), 0x10))
            .unwrap();
        let mut ids = Vec::with_capacity(10_000);
        let mut key: Option<OrderKey> = None;
        for index in 0..10_000u64 {
            let mut record = crate::command::test_fixtures::frame(1_000 + index, Some(11), 0x10);
            let next = OrderKey::between(key.as_ref(), None);
            record.order_key = next.clone();
            key = Some(next);
            record.payload = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(10., 10.).unwrap(),
                corner_radius: Default::default(),
            }));
            ids.push(record.id);
            model.insert_node(record).unwrap();
        }
        let mut state = DocumentState::new(model);
        let mut fresh = new_ids(77);
        for (property, text) in [
            (P::Fill, "#ff0000"),
            (P::Stroke, "#0000ff"),
            (P::Left, "5px"),
        ] {
            let input = InspectorInput::Set(parse_input(property, text).unwrap());
            let commands =
                edit_commands(state.model(), &ids, property, &input, &mut fresh).unwrap();
            apply(&mut state, commands);
        }
        let mut times = Vec::new();
        for _ in 0..7 {
            let started = std::time::Instant::now();
            let snapshot = inspector_rows(state.model(), &ids);
            times.push(started.elapsed());
            assert_eq!(snapshot.count, 10_000);
        }
        times.sort();
        eprintln!(
            "inspector snapshot 1만 개: 중앙값 {:?} (최소 {:?}, 최대 {:?})",
            times[3], times[0], times[6]
        );
    }

    #[test]
    fn every_node_kind_has_rows_with_keys_and_sections() {
        let (model, ids) = seven_kind_model();
        let expected: [&[&str]; 7] = [
            &[
                "width",
                "background-color",
                "border",
                "border-radius",
                "border-top-left-radius",
                "overflow",
                "left",
            ],
            &["width", "fill", "stroke", "stroke-width", "border-radius"],
            &["width", "height", "fill"],
            &["x1", "y2", "stroke"],
            &["polygon-sides", "polygon-rotation", "fill"],
            &["color", "left", "top"],
            &["width", "object-fit"],
        ];
        for (id, wanted) in ids.iter().zip(expected) {
            let snapshot = inspector_rows(&model, &[*id]);
            assert_eq!(snapshot.count, 1);
            for name in wanted {
                value(&snapshot, name);
            }
            // 묶음 행 외에는 모두 저장 key가 있습니다.
            for row in &snapshot.rows {
                assert!(
                    row.key.0 != 0 || row.flags & flags::GROUP != 0,
                    "{:?}",
                    row.property
                );
            }
            // 섹션 순서대로 정렬됩니다.
            assert!(snapshot
                .rows
                .windows(2)
                .all(|w| w[0].property.section() <= w[1].property.section()));
        }
        let frame = inspector_rows(&model, &[ids[0]]);
        assert!(names(&frame).iter().all(|n| *n != "fill"));
        // 명세 6.2절 표에 없는 행(`stroke-linecap`, `image-rendering`)은 내지 않습니다.
        for id in ids {
            let rows = inspector_rows(&model, &[id]);
            assert!(names(&rows)
                .iter()
                .all(|n| *n != "stroke-linecap" && *n != "image-rendering"));
        }
        let rect = inspector_rows(&model, &[ids[1]]);
        assert_eq!(value(&rect, "border-radius").value, "1px 2px 3px 4px");
        assert_eq!(value(&rect, "fill").value, "none");
        assert_eq!(
            value(&rect, "border-radius").flags & flags::GROUP,
            flags::GROUP
        );
        let polygon = inspector_rows(&model, &[ids[4]]);
        assert_eq!(value(&polygon, "polygon-sides").value, "5");
        assert_eq!(value(&polygon, "polygon-rotation").value, "0deg");
    }

    #[test]
    fn multi_selection_shows_shared_rows_and_mixed_values_then_edits_once() {
        let (model, ids) = seven_kind_model();
        let mut state = DocumentState::new(model);
        // Rectangle(80×40)와 Ellipse(30×20).
        let selection = [ids[1], ids[2]];
        let snapshot = inspector_rows(state.model(), &selection);
        assert_eq!(snapshot.count, 2);
        let width = value(&snapshot, "width");
        assert_eq!(width.flags & flags::MIXED, flags::MIXED);
        assert_eq!(width.value, "");
        assert!(names(&snapshot).iter().all(|n| *n != "border-radius"));
        let before = model_snapshot_json(state.model()).unwrap();
        let input = InspectorInput::Set(parse_input(P::Width, "320").unwrap());
        let commands =
            edit_commands(state.model(), &selection, P::Width, &input, &mut new_ids(3)).unwrap();
        assert_eq!(commands.len(), 2);
        let undo_width = apply(&mut state, commands);
        let after = inspector_rows(state.model(), &selection);
        assert_eq!(value(&after, "width").value, "320px");
        // 혼합 값 화살표: node마다 +10.
        let commands = edit_commands(
            state.model(),
            &selection,
            P::Height,
            &InspectorInput::Adjust(10.),
            &mut new_ids(4),
        )
        .unwrap();
        let undo_height = apply(&mut state, commands);
        let heights: Vec<String> = selection
            .iter()
            .map(|id| {
                value(&inspector_rows(state.model(), &[*id]), "height")
                    .value
                    .clone()
            })
            .collect();
        assert_eq!(heights, ["50px", "30px"]);
        // 같은 값은 command를 만들지 않습니다.
        let same =
            edit_commands(state.model(), &selection, P::Width, &input, &mut new_ids(5)).unwrap();
        assert!(same.is_empty());
        // 역 transaction 두 번이면 처음 상태(편집 하나 = Undo 하나).
        for inverse in [undo_height, undo_width] {
            state
                .execute_transaction(&inverse.rebased(state.revision()))
                .unwrap();
        }
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
    }

    #[test]
    fn fill_and_stroke_rows_insert_update_and_remove_modifiers_in_render_order() {
        let (model, ids) = seven_kind_model();
        let mut state = DocumentState::new(model);
        let rect = [ids[1]];
        let before = model_snapshot_json(state.model()).unwrap();
        let mut fresh = new_ids(7);
        for (property, text) in [
            (P::Stroke, "#ff0000"),
            (P::Fill, "#3366ff"),
            (P::Left, "24"),
        ] {
            let input = InspectorInput::Set(parse_input(property, text).unwrap());
            let commands =
                edit_commands(state.model(), &rect, property, &input, &mut fresh).unwrap();
            apply(&mut state, commands);
        }
        let stack = state.model().modifier_stack(ids[1]).unwrap();
        let kinds: Vec<ModifierKind> = stack.entries().iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                ModifierKind::Transform,
                ModifierKind::Fill,
                ModifierKind::Stroke
            ]
        );
        let snapshot = inspector_rows(state.model(), &rect);
        assert_eq!(value(&snapshot, "fill").value, "#3366ff");
        assert_eq!(value(&snapshot, "stroke").value, "#ff0000");
        assert_eq!(value(&snapshot, "stroke-width").value, "1px");
        assert_eq!(value(&snapshot, "left").value, "24px");
        let input = InspectorInput::Set(parse_input(P::Fill, "none").unwrap());
        let commands =
            edit_commands(state.model(), &rect, P::Fill, &input, &mut new_ids(8)).unwrap();
        apply(&mut state, commands);
        assert_eq!(
            value(&inspector_rows(state.model(), &rect), "fill").value,
            "none"
        );
        // Text color는 Fill modifier입니다.
        let text = [ids[5]];
        assert_eq!(
            value(&inspector_rows(state.model(), &text), "color").value,
            "#000000"
        );
        let input = InspectorInput::Set(parse_input(P::Color, "rgb(51 102 255)").unwrap());
        let commands =
            edit_commands(state.model(), &text, P::Color, &input, &mut new_ids(9)).unwrap();
        apply(&mut state, commands);
        assert_eq!(
            value(&inspector_rows(state.model(), &text), "color").value,
            "#3366ff"
        );
        assert_ne!(model_snapshot_json(state.model()).unwrap(), before);
    }

    #[test]
    fn shorthand_and_locks_follow_css_and_d8() {
        let (model, ids) = seven_kind_model();
        let mut state = DocumentState::new(model);
        let frame = [ids[0]];
        let input = InspectorInput::Set(parse_input(P::BorderRadius, "4px 8px").unwrap());
        let commands = edit_commands(
            state.model(),
            &frame,
            P::BorderRadius,
            &input,
            &mut new_ids(1),
        )
        .unwrap();
        assert_eq!(commands.len(), 4);
        apply(&mut state, commands);
        let snapshot = inspector_rows(state.model(), &frame);
        assert_eq!(value(&snapshot, "border-radius").value, "4px 8px");
        assert_eq!(value(&snapshot, "border-top-right-radius").value, "8px");
        let input = InspectorInput::Set(parse_input(P::Border, "2px solid #ff0000").unwrap());
        let commands =
            edit_commands(state.model(), &frame, P::Border, &input, &mut new_ids(1)).unwrap();
        apply(&mut state, commands);
        assert_eq!(
            value(&inspector_rows(state.model(), &frame), "border").value,
            "2px solid #ff0000"
        );
        // 잠그면 locked 외 행은 읽기 전용이고 편집을 거부합니다.
        let lock = InspectorInput::Set(CssValue::Bool(true));
        let commands =
            edit_commands(state.model(), &frame, P::Locked, &lock, &mut new_ids(1)).unwrap();
        apply(&mut state, commands);
        let snapshot = inspector_rows(state.model(), &frame);
        assert_eq!(
            value(&snapshot, "width").flags & flags::READ_ONLY,
            flags::READ_ONLY
        );
        assert_eq!(value(&snapshot, "locked").flags & flags::READ_ONLY, 0);
        let width = InspectorInput::Set(CssValue::Length(10.));
        assert!(edit_commands(state.model(), &frame, P::Width, &width, &mut new_ids(1)).is_err());
        // 자식(13)은 조상이 잠겨 locked 행도 읽기 전용입니다.
        let child = inspector_rows(state.model(), &[sid(13)]);
        assert_eq!(
            value(&child, "locked").flags & flags::READ_ONLY,
            flags::READ_ONLY
        );
        // 종류에 없는 속성은 거부합니다.
        let fill = InspectorInput::Set(CssValue::Color(None));
        assert!(edit_commands(state.model(), &[ids[6]], P::Fill, &fill, &mut new_ids(1)).is_err());
    }

    #[test]
    fn missing_stroke_and_border_parts_never_get_a_core_default_color() {
        let (model, ids) = seven_kind_model();
        let mut state = DocumentState::new(model);
        let rect = [ids[1]];
        // 선이 없으면 stroke-width만으로 선을 만들지 않습니다(색 기본값은 셸 설정, D7).
        let width = InspectorInput::Set(parse_input(P::StrokeWidth, "3").unwrap());
        assert!(edit_commands(
            state.model(),
            &rect,
            P::StrokeWidth,
            &width,
            &mut new_ids(1)
        )
        .is_err());
        assert!(edit_commands(
            state.model(),
            &rect,
            P::StrokeWidth,
            &InspectorInput::Adjust(1.),
            &mut new_ids(1)
        )
        .is_err());
        // 색을 넣으면 CSS 초기값 1px로 선이 생기고, 그 뒤 두께를 바꿀 수 있습니다.
        for (property, text) in [(P::Stroke, "#3366ff"), (P::StrokeWidth, "3")] {
            let input = InspectorInput::Set(parse_input(property, text).unwrap());
            let commands =
                edit_commands(state.model(), &rect, property, &input, &mut new_ids(2)).unwrap();
            apply(&mut state, commands);
        }
        let snapshot = inspector_rows(state.model(), &rect);
        assert_eq!(value(&snapshot, "stroke").value, "#3366ff");
        assert_eq!(value(&snapshot, "stroke-width").value, "3px");
        // 테두리 없는 Frame에 두께만 넣으면 거부하고, 테두리가 있으면 빠진 쪽을 이어 씁니다.
        let frame = [ids[0]];
        let partial = InspectorInput::Set(parse_input(P::Border, "4px").unwrap());
        assert!(
            edit_commands(state.model(), &frame, P::Border, &partial, &mut new_ids(3)).is_err()
        );
        let full = InspectorInput::Set(parse_input(P::Border, "2px solid #ff0000").unwrap());
        let commands =
            edit_commands(state.model(), &frame, P::Border, &full, &mut new_ids(3)).unwrap();
        apply(&mut state, commands);
        let commands =
            edit_commands(state.model(), &frame, P::Border, &partial, &mut new_ids(4)).unwrap();
        apply(&mut state, commands);
        let recolor = InspectorInput::Set(parse_input(P::Border, "blue").unwrap());
        let commands =
            edit_commands(state.model(), &frame, P::Border, &recolor, &mut new_ids(5)).unwrap();
        apply(&mut state, commands);
        assert_eq!(
            value(&inspector_rows(state.model(), &frame), "border").value,
            "4px solid #0000ff"
        );
    }

    #[test]
    fn node_lock_blocks_edits_but_lets_the_node_itself_be_unlocked() {
        let (model, ids) = seven_kind_model();
        let mut state = DocumentState::new(model);
        let lock = InspectorInput::Set(CssValue::Bool(true));
        let commands =
            edit_commands(state.model(), &[ids[0]], P::Locked, &lock, &mut new_ids(1)).unwrap();
        apply(&mut state, commands);
        let own = NodeLock::of(state.model(), ids[0]).unwrap();
        assert_eq!(
            own,
            NodeLock {
                own: true,
                ancestor: false
            }
        );
        assert!(own.blocks(node_keys::DISPLAY_NAME) && !own.blocks(node_keys::LOCKED));
        let child = NodeLock::of(state.model(), sid(13)).unwrap();
        assert!(child.ancestor && child.blocks(node_keys::LOCKED));
        assert_eq!(NodeLock::of(state.model(), sid(999)), None);
    }
}
