//! Shape·Image property metadata registry (UX.2.1, IX.11.3 일부).
//!
//! Frame registry(`frame_metadata.rs`)와 같은 방식으로 Shape·Image payload의 longhand마다 [`PropertyKey`]
//! 하나를 고정합니다(ADR-0024 결정 14, ADR-0070). key 상위 byte는 node 종류 번호입니다(Shape `0x03xx`,
//! Image `0x06xx`). 이름은 CSS longhand 이름이고, CSS에 대응이 없는 항목은 `stellar_only`입니다
//! (속성 모델 6장). Text style longhand는 Typography record(IX.11.11)가 생긴 뒤 같은 규칙으로 더합니다.
//!
//! **누락 검사**: [`shape_field_keys`]·[`image_field_keys`]는 파라미터를 `..` 없이 구조 분해하므로 필드가
//! 추가·제거되면 컴파일이 실패합니다. 테스트는 registry와 필드 목록이 일치하는지 검사합니다.

use stellar_core_types::property::PropertyKey;

use crate::frame_metadata::Invalidation;
use crate::image::ImagePayload;
use crate::shape::{EllipseParams, LineParams, RectangleParams, ShapeKind};

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
    /// CSS에 대응이 없는 Stellar 전용 속성인지.
    pub stellar_only: bool,
    /// 무효화 범위.
    pub invalidation: Invalidation,
}

const fn entry(
    key: PropertyKey,
    name: &'static str,
    value_type: PayloadPropertyType,
    stellar_only: bool,
    invalidation: Invalidation,
) -> PayloadPropertyMetadata {
    PayloadPropertyMetadata {
        key,
        name,
        value_type,
        stellar_only,
        invalidation,
    }
}

use PayloadPropertyType::{Angle, Coordinate, Count, Dimension, Keyword};

/// Shape 속성 registry. 순서는 key 오름차순입니다.
pub const SHAPE_PROPERTIES: [PayloadPropertyMetadata; 14] = [
    entry(
        shape_keys::WIDTH,
        "width",
        Dimension,
        false,
        Invalidation::LAYOUT_AND_PAINT,
    ),
    entry(
        shape_keys::HEIGHT,
        "height",
        Dimension,
        false,
        Invalidation::LAYOUT_AND_PAINT,
    ),
    entry(
        shape_keys::BORDER_TOP_LEFT_RADIUS,
        "border-top-left-radius",
        Dimension,
        false,
        Invalidation::PAINT,
    ),
    entry(
        shape_keys::BORDER_TOP_RIGHT_RADIUS,
        "border-top-right-radius",
        Dimension,
        false,
        Invalidation::PAINT,
    ),
    entry(
        shape_keys::BORDER_BOTTOM_RIGHT_RADIUS,
        "border-bottom-right-radius",
        Dimension,
        false,
        Invalidation::PAINT,
    ),
    entry(
        shape_keys::BORDER_BOTTOM_LEFT_RADIUS,
        "border-bottom-left-radius",
        Dimension,
        false,
        Invalidation::PAINT,
    ),
    entry(
        shape_keys::X1,
        "x1",
        Coordinate,
        true,
        Invalidation::LAYOUT_AND_PAINT,
    ),
    entry(
        shape_keys::Y1,
        "y1",
        Coordinate,
        true,
        Invalidation::LAYOUT_AND_PAINT,
    ),
    entry(
        shape_keys::X2,
        "x2",
        Coordinate,
        true,
        Invalidation::LAYOUT_AND_PAINT,
    ),
    entry(
        shape_keys::Y2,
        "y2",
        Coordinate,
        true,
        Invalidation::LAYOUT_AND_PAINT,
    ),
    entry(
        shape_keys::STROKE_LINECAP,
        "stroke-linecap",
        Keyword,
        false,
        Invalidation::PAINT,
    ),
    entry(
        shape_keys::POLYGON_SIDES,
        "polygon-sides",
        Count,
        true,
        Invalidation::PAINT,
    ),
    entry(
        shape_keys::POLYGON_ROTATION,
        "polygon-rotation",
        Angle,
        true,
        Invalidation::PAINT,
    ),
    entry(
        shape_keys::POLYGON_CORNER_RADIUS,
        "polygon-corner-radius",
        Dimension,
        true,
        Invalidation::PAINT,
    ),
];

/// Image 속성 registry. 순서는 key 오름차순입니다.
pub const IMAGE_PROPERTIES: [PayloadPropertyMetadata; 4] = [
    entry(
        image_keys::WIDTH,
        "width",
        Dimension,
        false,
        Invalidation::LAYOUT_AND_PAINT,
    ),
    entry(
        image_keys::HEIGHT,
        "height",
        Dimension,
        false,
        Invalidation::LAYOUT_AND_PAINT,
    ),
    entry(
        image_keys::OBJECT_FIT,
        "object-fit",
        Keyword,
        false,
        Invalidation::PAINT,
    ),
    entry(
        image_keys::IMAGE_RENDERING,
        "image-rendering",
        Keyword,
        false,
        Invalidation::PAINT,
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

/// Image payload 필드 중 편집 key가 있는 것. asset·crop·color space는 Stellar 고유 구조라 key를 두지 않습니다.
pub fn image_field_keys(payload: &ImagePayload) -> [PropertyKey; 4] {
    let ImagePayload {
        asset: _,
        size: _,
        fit: _,
        crop: _,
        sampling: _,
        color_space_override: _,
    } = payload;
    [
        image_keys::WIDTH,
        image_keys::HEIGHT,
        image_keys::OBJECT_FIT,
        image_keys::IMAGE_RENDERING,
    ]
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
            assert_ne!(m.key, IMAGE_PROP_ASSET);
        }
        let mut all: Vec<u32> = SHAPE_PROPERTIES
            .iter()
            .chain(&IMAGE_PROPERTIES)
            .map(|m| m.key.0)
            .chain(FRAME_PROPERTIES.iter().map(|m| m.key.0))
            .collect();
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count, "key 중복 금지");
        for table in [&SHAPE_PROPERTIES[..], &IMAGE_PROPERTIES[..]] {
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
    }
}
