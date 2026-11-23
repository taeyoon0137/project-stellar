//! 공통 node 필드·Shape·Path·Text·Image property metadata registry (UX.2.1, IX.11.3).
//!
//! Frame registry(`frame_metadata.rs`)와 같은 방식으로 payload의 longhand마다 [`PropertyKey`] 하나를
//! 고정합니다(ADR-0024 결정 14, ADR-0070). key 상위 byte는 node 종류 번호입니다(Shape `0x03xx`, Path
//! `0x04xx`, Text `0x05xx`, Image `0x06xx`). 이름은 CSS longhand 이름이고, CSS에 대응이 없는 항목은 Stellar
//! 이름이며 매핑 상태가 `stellar-only`입니다(속성 모델 3장·6장·7장). 각 항목은 CSS 매핑([`CssMapping`])을
//! 가집니다(IX.11.3).
//!
//! Path command·point 목록과 Text content·paragraph·style run은 payload 단위 key 하나를 유지합니다(속성 모델
//! 15장 O12). Text style longhand key는 IX.11.11이 Typography record와 함께 같은 규칙으로 할당합니다.
//!
//! **누락 검사**: [`shape_field_keys`]·[`image_field_keys`]·[`path_field_keys`]·[`text_field_keys`]는 파라미터를
//! `..` 없이 구조 분해하므로 필드가 추가·제거되면 컴파일이 실패합니다. 테스트는 registry와 필드 목록이
//! 일치하는지 검사합니다.

use stellar_core_types::property::PropertyKey;

use crate::css::grammar::{CssGrammar, LENGTH_NON_NEGATIVE, LENGTH_SIGNED, LENGTH_SIZING};
use crate::css::mapping::CssMapping;
use crate::frame_metadata::Invalidation;
use crate::image::ImagePayload;
use crate::path::PathData;
use crate::shape::{EllipseParams, LineParams, RectangleParams, ShapeKind};
use crate::text::TextPayload;

/// 공통 node 필드 key(모든 node 종류, 속성 모델 3장). 숫자는 command `node_keys`와 같습니다.
pub mod node_keys {
    use stellar_core_types::property::PropertyKey;

    /// 표시 여부.
    pub const VISIBLE: PropertyKey = PropertyKey(0x0001);
    /// 잠금.
    pub const LOCKED: PropertyKey = PropertyKey(0x0002);
    /// 레이어 이름.
    pub const DISPLAY_NAME: PropertyKey = PropertyKey(0x0003);
}

/// Path payload key(`0x04xx`).
pub mod path_keys {
    use stellar_core_types::property::PropertyKey;

    /// command·point 목록과 winding 전체(payload 단위, SVG `d`).
    pub const PATH_DATA: PropertyKey = PropertyKey(0x0401);
    /// `fill-rule`(winding).
    pub const FILL_RULE: PropertyKey = PropertyKey(0x0402);
}

/// Text payload key(`0x05xx`).
pub mod text_keys {
    use stellar_core_types::property::PropertyKey;

    /// content·paragraph·style run 전체(payload 단위). style run이 content offset을 가리켜 나누지 않습니다.
    pub const TEXT_CONTENT: PropertyKey = PropertyKey(0x0501);
}

/// Shape payload key(`0x03xx`).
pub mod shape_keys {
    use stellar_core_types::property::PropertyKey;

    /// `width`(Rectangle 크기, Ellipse·Polygon bounds 폭).
    pub const WIDTH: PropertyKey = PropertyKey(0x0301);
    /// `height`.
    pub const HEIGHT: PropertyKey = PropertyKey(0x0302);
    /// Rectangle `border-top-left-radius`.
    pub const BORDER_TOP_LEFT_RADIUS: PropertyKey = PropertyKey(0x0303);
    /// Rectangle `border-top-right-radius`.
    pub const BORDER_TOP_RIGHT_RADIUS: PropertyKey = PropertyKey(0x0304);
    /// Rectangle `border-bottom-right-radius`.
    pub const BORDER_BOTTOM_RIGHT_RADIUS: PropertyKey = PropertyKey(0x0305);
    /// Rectangle `border-bottom-left-radius`.
    pub const BORDER_BOTTOM_LEFT_RADIUS: PropertyKey = PropertyKey(0x0306);
    /// TL, TR, BR, BL 순서의 Rectangle 모서리 longhand.
    pub const BORDER_RADIUS_LONGHANDS: [PropertyKey; 4] = [
        BORDER_TOP_LEFT_RADIUS,
        BORDER_TOP_RIGHT_RADIUS,
        BORDER_BOTTOM_RIGHT_RADIUS,
        BORDER_BOTTOM_LEFT_RADIUS,
    ];
    /// Line 시작 x(SVG `x1`).
    pub const X1: PropertyKey = PropertyKey(0x0307);
    /// Line 시작 y(SVG `y1`).
    pub const Y1: PropertyKey = PropertyKey(0x0308);
    /// Line 끝 x(SVG `x2`).
    pub const X2: PropertyKey = PropertyKey(0x0309);
    /// Line 끝 y(SVG `y2`).
    pub const Y2: PropertyKey = PropertyKey(0x030a);
    /// Line `stroke-linecap`.
    pub const STROKE_LINECAP: PropertyKey = PropertyKey(0x030b);
    /// Polygon 변 수(Stellar 전용).
    pub const POLYGON_SIDES: PropertyKey = PropertyKey(0x030c);
    /// Polygon 꼭짓점 회전(Stellar 전용, deg).
    pub const POLYGON_ROTATION: PropertyKey = PropertyKey(0x030d);
    /// Polygon 모서리 반경(Stellar 전용).
    pub const POLYGON_CORNER_RADIUS: PropertyKey = PropertyKey(0x030e);
}

/// Image payload key(`0x06xx`). `0x0601`은 asset 참조(`image::IMAGE_PROP_ASSET`)입니다.
pub mod image_keys {
    use stellar_core_types::property::PropertyKey;

    /// `width`(배치 상자 폭).
    pub const WIDTH: PropertyKey = PropertyKey(0x0602);
    /// `height`.
    pub const HEIGHT: PropertyKey = PropertyKey(0x0603);
    /// `object-fit`.
    pub const OBJECT_FIT: PropertyKey = PropertyKey(0x0604);
    /// `image-rendering`(sampling).
    pub const IMAGE_RENDERING: PropertyKey = PropertyKey(0x0605);
    /// crop 영역 `(x, y, width, height)`(Stellar 전용, 원본 기준 0..1).
    pub const IMAGE_CROP: PropertyKey = PropertyKey(0x0606);
    /// asset 색공간 override(Stellar 전용).
    pub const IMAGE_COLOR_SPACE: PropertyKey = PropertyKey(0x0607);
}

/// payload 속성 값 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadPropertyType {
    /// 비음수 길이(px).
    Dimension,
    /// 부호 있는 길이(px).
    Coordinate,
    /// 각도(deg).
    Angle,
    /// 정수 개수.
    Count,
    /// keyword(저장 tag).
    Keyword,
    /// bool(공통 node 필드).
    Bool,
    /// 문자열(공통 node 필드).
    Text,
    /// 구조 값(참조·영역·payload 단위). `SetProperty`로 바꾸지 않습니다.
    Structured,
}

/// 한 속성의 metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadPropertyMetadata {
    /// 고정 numeric key.
    pub key: PropertyKey,
    /// CSS longhand 이름(Stellar 전용이면 Stellar 이름). 식별자는 key입니다.
    pub name: &'static str,
    /// 값 종류.
    pub value_type: PayloadPropertyType,
    /// 무효화 범위.
    pub invalidation: Invalidation,
    /// CSS 매핑(속성 모델 3장·6장·7장, IX.11.3).
    pub css: CssMapping,
}

impl PayloadPropertyMetadata {
    /// CSS에 대응이 없는 Stellar 전용 속성인지(매핑 상태 `stellar-only`).
    pub fn is_stellar_only(&self) -> bool {
        self.css.status == crate::css::mapping::MappingStatus::StellarOnly
    }

    /// `SetProperty`로 바꿀 수 있는 값인지(구조 값은 아님).
    pub fn is_settable(&self) -> bool {
        self.value_type != PayloadPropertyType::Structured
    }
}

const fn entry(
    key: PropertyKey,
    name: &'static str,
    value_type: PayloadPropertyType,
    invalidation: Invalidation,
    css: CssMapping,
) -> PayloadPropertyMetadata {
    PayloadPropertyMetadata {
        key,
        name,
        value_type,
        invalidation,
        css,
    }
}

use PayloadPropertyType::{Angle, Bool, Coordinate, Count, Dimension, Keyword, Structured, Text};

const LINECAPS: &[&str] = &["butt", "round", "square"];
const FITS: &[&str] = &["fill", "contain", "cover", "none"];
const RENDERINGS: &[&str] = &["auto", "pixelated"];
const FILL_RULES: &[&str] = &["nonzero", "evenodd"];
const POLYGON_SIDES: CssGrammar = CssGrammar::Count { min: 3, max: 1000 };
const RADIUS_NOTE: &str =
    "px 원형 반경만 받습니다(타원·%는 subset). SVG rect는 반경 하나라 SVG에서는 path로 냅니다";
const POLYGON_NOTE: &str = "export에서 path로 굽습니다";

/// 공통 node 필드 registry(속성 모델 3장). 순서는 key 오름차순입니다.
pub const NODE_PROPERTIES: [PayloadPropertyMetadata; 3] = [
    entry(
        node_keys::VISIBLE,
        "visible",
        Bool,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::converted(&["display"], Some(CssGrammar::Bool)).note(
            "visible = false는 display: none입니다(flow에서 빠지므로 visibility: hidden이 아님)",
        ),
    ),
    entry(
        node_keys::LOCKED,
        "locked",
        Bool,
        Invalidation::PAINT,
        CssMapping::stellar_only(Some(CssGrammar::Bool)).note("편집 메타데이터입니다"),
    ),
    entry(
        node_keys::DISPLAY_NAME,
        "name",
        Text,
        Invalidation::PAINT,
        CssMapping::stellar_only(Some(CssGrammar::Text))
            .note("레이어 이름입니다. export는 주석·식별자 힌트로만 씁니다"),
    ),
];

/// Path 속성 registry. 순서는 key 오름차순입니다.
pub const PATH_PROPERTIES: [PayloadPropertyMetadata; 2] = [
    entry(
        path_keys::PATH_DATA,
        "path-data",
        Structured,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::converted(&["d"], None).note(
            "MoveTo·LineTo·CubicTo·Close와 point StableId 목록입니다. SVG d·CSS path()로 냅니다. merge 단위는 payload 전체입니다(속성 모델 15장 O12)",
        ),
    ),
    entry(
        path_keys::FILL_RULE,
        "fill-rule",
        Structured,
        Invalidation::PAINT,
        CssMapping::exact(&["fill-rule"], CssGrammar::Keyword(FILL_RULES))
            .note("winding은 path-data payload 단위 안에 함께 저장됩니다"),
    ),
];

/// Text 속성 registry. 순서는 key 오름차순입니다. style longhand는 IX.11.11이 더합니다.
pub const TEXT_PROPERTIES: [PayloadPropertyMetadata; 1] = [entry(
    text_keys::TEXT_CONTENT,
    "text-content",
    Structured,
    Invalidation::LAYOUT_AND_PAINT,
    CssMapping::stellar_only(None).note(
        "UserText 내용·paragraph·style run 전체입니다. SVG export는 outline path입니다. style longhand key는 IX.11.11",
    ),
)];

/// Shape 속성 registry. 순서는 key 오름차순입니다.
pub const SHAPE_PROPERTIES: [PayloadPropertyMetadata; 14] = [
    entry(
        shape_keys::WIDTH,
        "width",
        Dimension,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::converted(&["width"], Some(LENGTH_SIZING)).note("Rectangle 크기, Ellipse·Polygon bounds 폭입니다. Ellipse는 border-radius: 50% 또는 SVG ellipse로 냅니다"),
    ),
    entry(
        shape_keys::HEIGHT,
        "height",
        Dimension,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::converted(&["height"], Some(LENGTH_SIZING)).note("Rectangle 크기, Ellipse·Polygon bounds 높이입니다"),
    ),
    entry(
        shape_keys::BORDER_TOP_LEFT_RADIUS,
        "border-top-left-radius",
        Dimension,
        Invalidation::PAINT,
        CssMapping::exact(&["border-top-left-radius"], LENGTH_NON_NEGATIVE).subset().note(RADIUS_NOTE),
    ),
    entry(
        shape_keys::BORDER_TOP_RIGHT_RADIUS,
        "border-top-right-radius",
        Dimension,
        Invalidation::PAINT,
        CssMapping::exact(&["border-top-right-radius"], LENGTH_NON_NEGATIVE).subset().note(RADIUS_NOTE),
    ),
    entry(
        shape_keys::BORDER_BOTTOM_RIGHT_RADIUS,
        "border-bottom-right-radius",
        Dimension,
        Invalidation::PAINT,
        CssMapping::exact(&["border-bottom-right-radius"], LENGTH_NON_NEGATIVE).subset().note(RADIUS_NOTE),
    ),
    entry(
        shape_keys::BORDER_BOTTOM_LEFT_RADIUS,
        "border-bottom-left-radius",
        Dimension,
        Invalidation::PAINT,
        CssMapping::exact(&["border-bottom-left-radius"], LENGTH_NON_NEGATIVE).subset().note(RADIUS_NOTE),
    ),
    entry(
        shape_keys::X1,
        "x1",
        Coordinate,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::stellar_only(Some(LENGTH_SIGNED)).note("SVG line x1입니다. CSS 상자 속성이 아닙니다"),
    ),
    entry(
        shape_keys::Y1,
        "y1",
        Coordinate,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::stellar_only(Some(LENGTH_SIGNED)).note("SVG line y1입니다"),
    ),
    entry(
        shape_keys::X2,
        "x2",
        Coordinate,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::stellar_only(Some(LENGTH_SIGNED)).note("SVG line x2입니다"),
    ),
    entry(
        shape_keys::Y2,
        "y2",
        Coordinate,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::stellar_only(Some(LENGTH_SIGNED)).note("SVG line y2입니다"),
    ),
    entry(
        shape_keys::STROKE_LINECAP,
        "stroke-linecap",
        Keyword,
        Invalidation::PAINT,
        CssMapping::exact(&["stroke-linecap"], CssGrammar::Keyword(LINECAPS)),
    ),
    entry(
        shape_keys::POLYGON_SIDES,
        "polygon-sides",
        Count,
        Invalidation::PAINT,
        CssMapping::stellar_only(Some(POLYGON_SIDES)).note(POLYGON_NOTE),
    ),
    entry(
        shape_keys::POLYGON_ROTATION,
        "polygon-rotation",
        Angle,
        Invalidation::PAINT,
        CssMapping::stellar_only(Some(CssGrammar::Angle)).note(POLYGON_NOTE),
    ),
    entry(
        shape_keys::POLYGON_CORNER_RADIUS,
        "polygon-corner-radius",
        Dimension,
        Invalidation::PAINT,
        CssMapping::stellar_only(Some(LENGTH_NON_NEGATIVE)).note(POLYGON_NOTE),
    ),
];

/// Image 속성 registry. 순서는 key 오름차순입니다. asset 참조(0x0601)는 `image::IMAGE_PROP_ASSET`입니다.
pub const IMAGE_PROPERTIES: [PayloadPropertyMetadata; 7] = [
    entry(
        crate::image::IMAGE_PROP_ASSET,
        "image-asset",
        Structured,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::stellar_only(None).note("AssetRef 참조입니다. export에서 <img src>로 냅니다"),
    ),
    entry(
        image_keys::WIDTH,
        "width",
        Dimension,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::converted(&["width"], Some(LENGTH_SIZING)).note("배치 상자 폭입니다"),
    ),
    entry(
        image_keys::HEIGHT,
        "height",
        Dimension,
        Invalidation::LAYOUT_AND_PAINT,
        CssMapping::converted(&["height"], Some(LENGTH_SIZING)).note("배치 상자 높이입니다"),
    ),
    entry(
        image_keys::OBJECT_FIT,
        "object-fit",
        Keyword,
        Invalidation::PAINT,
        CssMapping::exact(&["object-fit"], CssGrammar::Keyword(FITS))
            .initial_differs()
            .subset()
            .note("기본값 contain, CSS initial fill. scale-down·object-position은 없습니다"),
    ),
    entry(
        image_keys::IMAGE_RENDERING,
        "image-rendering",
        Keyword,
        Invalidation::PAINT,
        CssMapping::approximated(&["image-rendering"], Some(CssGrammar::Keyword(RENDERINGS)))
            .note("auto가 선형 보간을 보장하지 않습니다"),
    ),
    entry(
        image_keys::IMAGE_CROP,
        "image-crop",
        Structured,
        Invalidation::PAINT,
        CssMapping::stellar_only(None).note(
            "원본 기준 0..1 영역입니다. export는 wrapper와 object-position·크기로 근사합니다",
        ),
    ),
    entry(
        image_keys::IMAGE_COLOR_SPACE,
        "image-color-space",
        Structured,
        Invalidation::PAINT,
        CssMapping::stellar_only(None)
            .note("asset 색공간 override입니다. export에서 래스터화합니다"),
    ),
];

/// 도형 종류가 가진 key 목록(registry 순서).
pub fn shape_kind_keys(kind: ShapeKind) -> &'static [PropertyKey] {
    use shape_keys::*;
    match kind {
        ShapeKind::Rectangle => &[
            WIDTH,
            HEIGHT,
            BORDER_TOP_LEFT_RADIUS,
            BORDER_TOP_RIGHT_RADIUS,
            BORDER_BOTTOM_RIGHT_RADIUS,
            BORDER_BOTTOM_LEFT_RADIUS,
        ],
        ShapeKind::Ellipse => &[WIDTH, HEIGHT],
        ShapeKind::Line => &[X1, Y1, X2, Y2, STROKE_LINECAP],
        ShapeKind::Polygon => &[
            WIDTH,
            HEIGHT,
            POLYGON_SIDES,
            POLYGON_ROTATION,
            POLYGON_CORNER_RADIUS,
        ],
    }
}

/// `kind` 도형에서 `key`의 metadata.
pub fn shape_property(
    kind: ShapeKind,
    key: PropertyKey,
) -> Option<&'static PayloadPropertyMetadata> {
    if !shape_kind_keys(kind).contains(&key) {
        return None;
    }
    SHAPE_PROPERTIES.iter().find(|m| m.key == key)
}

/// Image `key`의 metadata.
pub fn image_property(key: PropertyKey) -> Option<&'static PayloadPropertyMetadata> {
    IMAGE_PROPERTIES.iter().find(|m| m.key == key)
}

/// 도형 파라미터 필드에 대응하는 key. `..` 없는 구조 분해라 필드가 바뀌면 컴파일이 실패합니다.
pub fn shape_field_keys(
    rectangle: &RectangleParams,
    ellipse: &EllipseParams,
    line: &LineParams,
) -> [&'static [PropertyKey]; 3] {
    let RectangleParams {
        size: _,
        corner_radius: _,
    } = rectangle;
    let EllipseParams { bounds: _ } = ellipse;
    let LineParams {
        start: _,
        end: _,
        cap: _,
    } = line;
    // Polygon 필드는 private이라 접근자(bounds·sides·rotation_degrees·corner_radius)로 대응합니다.
    [
        shape_kind_keys(ShapeKind::Rectangle),
        shape_kind_keys(ShapeKind::Ellipse),
        shape_kind_keys(ShapeKind::Line),
    ]
}

/// Image payload 필드에 대응하는 key(필드 순). asset·crop·color space는 Stellar 고유 구조 값이라 `SetProperty`로
/// 바꾸지 않지만 key와 CSS 매핑은 가집니다.
pub fn image_field_keys(payload: &ImagePayload) -> [PropertyKey; 7] {
    let ImagePayload {
        asset: _,
        size: _,
        fit: _,
        crop: _,
        sampling: _,
        color_space_override: _,
    } = payload;
    [
        crate::image::IMAGE_PROP_ASSET,
        image_keys::WIDTH,
        image_keys::HEIGHT,
        image_keys::OBJECT_FIT,
        image_keys::IMAGE_CROP,
        image_keys::IMAGE_RENDERING,
        image_keys::IMAGE_COLOR_SPACE,
    ]
}

/// Path payload 필드(접근자 `winding`·`commands`, 필드가 private라 구조 분해 대신 접근자로 대응)의 key.
/// command·point 목록은 payload 단위 key 하나입니다.
pub fn path_field_keys(path: &PathData) -> [PropertyKey; 2] {
    let _ = (path.winding(), path.commands());
    [path_keys::FILL_RULE, path_keys::PATH_DATA]
}

/// Text payload 필드(접근자 `content`·`paragraphs`·`style_runs`)의 key. content·paragraph·style run은 payload
/// 단위 key 하나입니다.
pub fn text_field_keys(text: &TextPayload) -> [PropertyKey; 1] {
    let _ = text;
    [text_keys::TEXT_CONTENT]
}

/// Path `key`의 metadata.
pub fn path_property(key: PropertyKey) -> Option<&'static PayloadPropertyMetadata> {
    PATH_PROPERTIES.iter().find(|m| m.key == key)
}

/// Text `key`의 metadata.
pub fn text_property(key: PropertyKey) -> Option<&'static PayloadPropertyMetadata> {
    TEXT_PROPERTIES.iter().find(|m| m.key == key)
}

/// 공통 node 필드 `key`의 metadata.
pub fn node_property(key: PropertyKey) -> Option<&'static PayloadPropertyMetadata> {
    NODE_PROPERTIES.iter().find(|m| m.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::keys as frame_keys;
    use crate::frame_metadata::FRAME_PROPERTIES;
    use crate::image::IMAGE_PROP_ASSET;
    use crate::shape::ShapeKind;

    #[test]
    fn keys_follow_node_kind_upper_byte_and_are_unique() {
        // 검증 기준(UX.2.1): key 상위 byte 규칙(Shape 0x03xx, Image 0x06xx) 준수, 중복 0.
        for m in &SHAPE_PROPERTIES {
            assert_eq!(m.key.0 >> 8, 0x03, "{}", m.name);
        }
        for m in &IMAGE_PROPERTIES {
            assert_eq!(m.key.0 >> 8, 0x06, "{}", m.name);
        }
        for m in &PATH_PROPERTIES {
            assert_eq!(m.key.0 >> 8, 0x04, "{}", m.name);
        }
        for m in &TEXT_PROPERTIES {
            assert_eq!(m.key.0 >> 8, 0x05, "{}", m.name);
        }
        for m in &NODE_PROPERTIES {
            assert_eq!(m.key.0 >> 8, 0x00, "{}", m.name);
        }
        let mut all: Vec<u32> = SHAPE_PROPERTIES
            .iter()
            .chain(&IMAGE_PROPERTIES)
            .chain(&PATH_PROPERTIES)
            .chain(&TEXT_PROPERTIES)
            .chain(&NODE_PROPERTIES)
            .map(|m| m.key.0)
            .chain(FRAME_PROPERTIES.iter().map(|m| m.key.0))
            .collect();
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count, "key 중복 금지");
        for table in [
            &SHAPE_PROPERTIES[..],
            &IMAGE_PROPERTIES[..],
            &PATH_PROPERTIES[..],
            &NODE_PROPERTIES[..],
        ] {
            assert!(
                table.windows(2).all(|w| w[0].key < w[1].key),
                "key 오름차순"
            );
        }
    }

    #[test]
    fn every_shape_kind_key_is_registered_and_names_match_frame_longhands() {
        for kind in ShapeKind::ALL {
            assert!(!shape_kind_keys(kind).is_empty());
            for key in shape_kind_keys(kind) {
                assert!(shape_property(kind, *key).is_some(), "{kind:?} {key:?}");
            }
        }
        // 모든 registry 항목은 적어도 한 도형 종류에 속합니다.
        for m in &SHAPE_PROPERTIES {
            assert!(ShapeKind::ALL
                .iter()
                .any(|k| shape_kind_keys(*k).contains(&m.key)));
        }
        // 같은 CSS longhand는 Frame과 같은 이름을 씁니다.
        let frame_name = |key| FRAME_PROPERTIES.iter().find(|m| m.key == key).unwrap().name;
        assert_eq!(
            shape_property(ShapeKind::Rectangle, shape_keys::WIDTH)
                .unwrap()
                .name,
            frame_name(frame_keys::WIDTH)
        );
        for (shape, frame) in shape_keys::BORDER_RADIUS_LONGHANDS
            .iter()
            .zip(frame_keys::BORDER_RADIUS_LONGHANDS)
        {
            assert_eq!(
                shape_property(ShapeKind::Rectangle, *shape).unwrap().name,
                frame_name(frame)
            );
        }
        assert!(shape_property(ShapeKind::Ellipse, shape_keys::X1).is_none());
        assert!(shape_property(ShapeKind::Line, shape_keys::WIDTH).is_none());
    }

    #[test]
    fn image_registry_matches_payload_keys() {
        use stellar_core_types::ids::StableId;
        use stellar_core_types::reference::AssetRef;
        use stellar_core_types::value::Size2;
        let payload = ImagePayload::new(
            AssetRef::new(StableId::from_bytes([7; 16])),
            Size2::new(4.0, 3.0).unwrap(),
        );
        let keys = image_field_keys(&payload);
        assert_eq!(keys.len(), IMAGE_PROPERTIES.len());
        for key in keys {
            assert!(image_property(key).is_some());
        }
        assert_eq!(
            image_property(IMAGE_PROP_ASSET).unwrap().name,
            "image-asset"
        );
        // 구조 값은 SetProperty 대상이 아닙니다.
        let settable: Vec<&str> = IMAGE_PROPERTIES
            .iter()
            .filter(|m| m.is_settable())
            .map(|m| m.name)
            .collect();
        assert_eq!(
            settable,
            ["width", "height", "object-fit", "image-rendering"]
        );
    }

    #[test]
    fn path_and_text_registries_match_payload_fields() {
        // 검증 기준(IX.11.3): payload 필드와 registry 1:1 누락 검사.
        use crate::path::{PathData, WindingRule};
        use crate::text::TextPayload;
        let path = PathData::new(WindingRule::NonZero, Vec::new()).unwrap();
        let keys = path_field_keys(&path);
        assert_eq!(keys.len(), PATH_PROPERTIES.len());
        for key in keys {
            assert!(path_property(key).is_some());
        }
        let text = TextPayload::new(
            stellar_core_types::text::UserText::new("카드"),
            vec![crate::text::Paragraph { style: 0 }],
            Vec::new(),
        )
        .unwrap();
        let keys = text_field_keys(&text);
        assert_eq!(keys.len(), TEXT_PROPERTIES.len());
        for key in keys {
            assert!(text_property(key).is_some());
        }
        assert!(PATH_PROPERTIES.iter().all(|m| !m.is_settable()));
        assert!(TEXT_PROPERTIES.iter().all(|m| !m.is_settable()));
        assert_eq!(
            node_property(node_keys::VISIBLE).unwrap().css.longhands,
            &["display"]
        );
        assert!(node_property(node_keys::LOCKED).unwrap().is_stellar_only());
    }
}
