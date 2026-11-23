//! Frame property metadata registry (VI.3.5).
//!
//! Frame payload의 각 속성에 대해 [`PropertyKey`], 값 타입, 기본값, 무효화 범위
//! (layout/paint)를 한 곳에 고정합니다. 편집 command·inspector·증분 무효화가
//! 이 표를 단일 근거로 사용합니다.
//!
//! **누락 검사**: [`frame_field_keys`]는 [`FramePayload`]를 `..` 없이 구조 분해하므로
//! payload에 필드가 추가되면 컴파일이 실패합니다. 테스트는 registry와 필드
//! 목록이 일치하는지, 이전 크기·모서리 반경 묶음 key가 보존되는지 검사합니다.

use stellar_core_types::property::{PropertyKey, PropertyNameRegistry};

use crate::css::grammar::{CssGrammar, LENGTH_NON_NEGATIVE, LENGTH_SIZING};
use crate::css::mapping::{ConversionRule, CssMapping, MappingStatus};
use crate::frame::{keys, FramePayload};

/// 속성 값 타입.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePropertyType {
    /// 비음수 크기(`Size2`).
    Size,
    /// 고정 길이(폭, 높이 또는 모서리 반경, 비음수).
    Dimension,
    /// 2D 아핀 변환.
    Transform,
    /// clipping 방식.
    Clipping,
    /// 선택적 색.
    OptionalColor,
    /// 선택적 테두리.
    OptionalBorder,
    /// 4귀퉁이 radius.
    CornerRadius,
    /// 선택적 layout mode 참조.
    OptionalLayoutRef,
}

/// 속성 변경이 무효화하는 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalidation {
    /// layout 재계산이 필요한지(자신·자손 배치에 영향).
    pub layout: bool,
    /// paint(scene·GPU 리소스) 갱신이 필요한지.
    pub paint: bool,
}

impl Invalidation {
    /// layout + paint.
    pub const LAYOUT_AND_PAINT: Invalidation = Invalidation {
        layout: true,
        paint: true,
    };
    /// paint만.
    pub const PAINT: Invalidation = Invalidation {
        layout: false,
        paint: true,
    };
}

/// 한 속성의 metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramePropertyMetadata {
    /// 고정 numeric key.
    pub key: PropertyKey,
    /// CSS longhand 이름 또는 CSS 대응이 없을 때의 Stellar 이름(IX.11.2). 식별에 쓰지 않습니다(key가
    /// 식별자). 묶음 key의 표시 문자열은 [`FramePropertyMetadata::display_label`]입니다.
    pub name: &'static str,
    /// 값 타입.
    pub value_type: FramePropertyType,
    /// 기본값 설명(`FramePayload::default()`와 일치해야 함).
    pub default: &'static str,
    /// 무효화 범위.
    pub invalidation: Invalidation,
    /// 묶음 key(IX.11.22, ADR-0024 결정 14)가 나뉜 longhand key. 비어 있으면 longhand 하나인 보통 항목입니다.
    /// 묶음 key는 이전 `SetProperty` bytes·journal의 decode·실행에만 쓰고 새 편집은 longhand를 씁니다.
    pub longhands: &'static [PropertyKey],
    /// CSS 매핑(속성 모델 5장, IX.11.2).
    pub css: CssMapping,
}

impl FramePropertyMetadata {
    /// 묶음 key인지.
    pub const fn is_bundle(&self) -> bool {
        !self.longhands.is_empty()
    }

    /// 표시 문자열: 보통 항목은 `name`, 묶음 key는 대응 longhand 이름 목록(`width, height`)입니다.
    pub fn display_label(&self) -> String {
        if self.is_bundle() {
            self.css.longhands.join(", ")
        } else {
            self.name.to_string()
        }
    }
}

const BORDER_LONGHANDS: &[&str] = &[
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
];

const RADIUS_LONGHANDS: &[&str] = &[
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-right-radius",
    "border-bottom-left-radius",
];

/// `clipping` 변형별 CSS 대응(속성 모델 5장, ADR-0075 결정 3).
pub const CLIPPING_RULES: &[ConversionRule] = &[
    ConversionRule {
        variant: "disabled",
        css: "overflow: visible",
        status: MappingStatus::Exact,
    },
    ConversionRule {
        variant: "frame-bounds",
        css: "overflow: clip; overflow-clip-margin: border-box",
        status: MappingStatus::Converted,
    },
    ConversionRule {
        variant: "path",
        css: "clip-path: url(#<path-id>)",
        status: MappingStatus::Converted,
    },
];

const RADIUS_NOTE: &str = "px 원형 반경만 받습니다(타원 x/y·%는 subset). bounds를 넘으면 같은 비율로 줄이는 규칙은 CSS와 같습니다";

/// `width`·`height` 순서의 크기 longhand.
const SIZE_LONGHANDS: [PropertyKey; 2] = [keys::WIDTH, keys::HEIGHT];

/// Frame 속성 registry. 순서는 key 오름차순입니다.
pub const FRAME_PROPERTIES: [FramePropertyMetadata; 13] = [
    FramePropertyMetadata {
        key: keys::SIZE,
        name: "size",
        value_type: FramePropertyType::Size,
        default: "0 × 0",
        invalidation: Invalidation::LAYOUT_AND_PAINT,
        longhands: &SIZE_LONGHANDS,
        css: CssMapping::converted(&["width", "height"], None)
            .note("이전 command·journal의 폭·높이 묶음 key. 새 편집은 width·height longhand를 씁니다"),
    },
    FramePropertyMetadata {
        key: keys::TRANSFORM,
        name: "transform",
        value_type: FramePropertyType::Transform,
        default: "identity",
        // 시각 변환: 배치(layout)는 바꾸지 않습니다.
        invalidation: Invalidation::PAINT,
        longhands: &[],
        css: CssMapping::converted(&["transform", "transform-origin"], Some(CssGrammar::Transform))
            .initial_differs()
            .note("선형 부분은 transform: matrix()로, 원점은 border box 좌상단(transform-origin: 0 0, CSS initial 50% 50%)입니다. 이동 성분은 위치이며 IX.11.9가 위치 longhand로 옮깁니다"),
    },
    FramePropertyMetadata {
        key: keys::CLIPPING,
        name: "clipping",
        value_type: FramePropertyType::Clipping,
        default: "frame-bounds",
        invalidation: Invalidation::PAINT,
        longhands: &[],
        css: CssMapping::converted(
            &["overflow-x", "overflow-y", "overflow-clip-margin", "clip-path"],
            None,
        )
        .initial_differs()
        .rules(CLIPPING_RULES)
        .note("CSS longhand로 나누지 않는 Stellar 고유 단일 속성입니다. 자식을 border box와 바깥 corner radius로 자릅니다(scene FrameClipInput::Bounds). 기본값 frame-bounds, CSS initial overflow: visible"),
    },
    FramePropertyMetadata {
        key: keys::BACKGROUND,
        name: "background-color",
        value_type: FramePropertyType::OptionalColor,
        default: "none",
        invalidation: Invalidation::PAINT,
        longhands: &[],
        css: CssMapping::exact(&["background-color"], CssGrammar::Color { allow_none: true })
            .note("한 겹 단색만 있습니다. 없음 = transparent"),
    },
    FramePropertyMetadata {
        key: keys::BORDER,
        name: "border",
        value_type: FramePropertyType::OptionalBorder,
        default: "none",
        // 두께가 content box에 영향을 줄 수 있어 layout까지 보수적으로 무효화합니다.
        invalidation: Invalidation::LAYOUT_AND_PAINT,
        longhands: &[],
        css: CssMapping::converted(BORDER_LONGHANDS, Some(CssGrammar::Border))
            .subset()
            .note("4변 같은 두께·색, style solid 하나, 없음 = none입니다. 12 longhand 분할은 IX.11.10"),
    },
    FramePropertyMetadata {
        key: keys::CORNER_RADIUS,
        name: "border-radius",
        value_type: FramePropertyType::CornerRadius,
        default: "0",
        invalidation: Invalidation::PAINT,
        longhands: &keys::BORDER_RADIUS_LONGHANDS,
        css: CssMapping::exact(RADIUS_LONGHANDS, CssGrammar::Corners)
            .subset()
            .note("이전 command·journal의 네 모서리 묶음 key. 새 편집은 border-*-radius longhand를 씁니다"),
    },
    FramePropertyMetadata {
        key: keys::LAYOUT_MODE,
        name: "layout-mode",
        value_type: FramePropertyType::OptionalLayoutRef,
        default: "none",
        invalidation: Invalidation::LAYOUT_AND_PAINT,
        longhands: &[],
        css: CssMapping::stellar_only(None)
            .note("layout 정의 resource 참조입니다(CSS class와 비슷). 저장 위치는 IX.11.6이 정합니다"),
    },
    FramePropertyMetadata {
        key: keys::WIDTH,
        name: "width",
        value_type: FramePropertyType::Dimension,
        default: "0",
        invalidation: Invalidation::LAYOUT_AND_PAINT,
        longhands: &[],
        css: CssMapping::exact(&["width"], LENGTH_SIZING)
            .subset()
            .note("지금은 고정 px만 저장합니다(%·auto·fit-content는 IX.11.6)"),
    },
    FramePropertyMetadata {
        key: keys::HEIGHT,
        name: "height",
        value_type: FramePropertyType::Dimension,
        default: "0",
        invalidation: Invalidation::LAYOUT_AND_PAINT,
        longhands: &[],
        css: CssMapping::exact(&["height"], LENGTH_SIZING)
            .subset()
            .note("지금은 고정 px만 저장합니다(%·auto·fit-content는 IX.11.6)"),
    },
    FramePropertyMetadata {
        key: keys::BORDER_TOP_LEFT_RADIUS,
        name: "border-top-left-radius",
        value_type: FramePropertyType::Dimension,
        default: "0",
        invalidation: Invalidation::PAINT,
        longhands: &[],
        css: CssMapping::exact(&["border-top-left-radius"], LENGTH_NON_NEGATIVE)
            .subset()
            .note(RADIUS_NOTE),
    },
    FramePropertyMetadata {
        key: keys::BORDER_TOP_RIGHT_RADIUS,
        name: "border-top-right-radius",
        value_type: FramePropertyType::Dimension,
        default: "0",
        invalidation: Invalidation::PAINT,
        longhands: &[],
        css: CssMapping::exact(&["border-top-right-radius"], LENGTH_NON_NEGATIVE)
            .subset()
            .note(RADIUS_NOTE),
    },
    FramePropertyMetadata {
        key: keys::BORDER_BOTTOM_RIGHT_RADIUS,
        name: "border-bottom-right-radius",
        value_type: FramePropertyType::Dimension,
        default: "0",
        invalidation: Invalidation::PAINT,
        longhands: &[],
        css: CssMapping::exact(&["border-bottom-right-radius"], LENGTH_NON_NEGATIVE)
            .subset()
            .note(RADIUS_NOTE),
    },
    FramePropertyMetadata {
        key: keys::BORDER_BOTTOM_LEFT_RADIUS,
        name: "border-bottom-left-radius",
        value_type: FramePropertyType::Dimension,
        default: "0",
        invalidation: Invalidation::PAINT,
        longhands: &[],
        css: CssMapping::exact(&["border-bottom-left-radius"], LENGTH_NON_NEGATIVE)
            .subset()
            .note(RADIUS_NOTE),
    },
];

/// key로 metadata를 찾습니다.
pub fn frame_property(key: PropertyKey) -> Option<&'static FramePropertyMetadata> {
    FRAME_PROPERTIES.iter().find(|m| m.key == key)
}

/// payload의 모든 필드에 대응하는 key 목록.
///
/// `..` 없는 구조 분해라 [`FramePayload`]에 필드가 추가·제거되면 컴파일이
/// 실패합니다 — registry 누락이 빌드 단계에서 드러납니다.
pub fn frame_field_keys(payload: &FramePayload) -> [PropertyKey; 11] {
    let FramePayload {
        size: _,
        transform: _,
        clipping: _,
        background: _,
        border: _,
        corner_radius: _,
        layout_mode: _,
    } = payload;
    [
        keys::TRANSFORM,
        keys::CLIPPING,
        keys::BACKGROUND,
        keys::BORDER,
        keys::LAYOUT_MODE,
        keys::WIDTH,
        keys::HEIGHT,
        keys::BORDER_TOP_LEFT_RADIUS,
        keys::BORDER_TOP_RIGHT_RADIUS,
        keys::BORDER_BOTTOM_RIGHT_RADIUS,
        keys::BORDER_BOTTOM_LEFT_RADIUS,
    ]
}

/// Frame 속성 이름을 이름 registry(III.2.1)에 등록합니다. 묶음 key는 등록하지 않습니다. 이름으로 key를 찾는
/// 편집 경로(`key_of("border-radius")`)가 새 편집에 쓰지 않는 묶음 key를 고르지 않게 합니다(ADR-0075 결정 1).
pub fn register_frame_property_names(registry: &mut PropertyNameRegistry) {
    for metadata in FRAME_PROPERTIES.iter().filter(|m| !m.is_bundle()) {
        registry.register(metadata.key, metadata.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Clipping, CornerRadius};
    use stellar_core_types::value::{Size2, Transform2D};

    #[test]
    fn registry_covers_payload_fields_and_legacy_groups() {
        // 검증 기준(VI.3.5): registry와 실제 payload 필드 누락 검사 통과.
        let field_keys = frame_field_keys(&FramePayload::default());
        assert_eq!(field_keys.len() + 2, FRAME_PROPERTIES.len());
        for key in field_keys {
            assert!(frame_property(key).is_some(), "registry 누락: {key:?}");
        }
        for metadata in &FRAME_PROPERTIES {
            assert!(
                metadata.is_bundle() || field_keys.contains(&metadata.key),
                "payload에 없는 registry 항목: {}",
                metadata.name
            );
        }
    }

    #[test]
    fn bundle_keys_map_to_registered_longhands() {
        // 검증 기준(IX.11.22): 묶음 key → longhand 대응. 묶음은 SIZE·CORNER_RADIUS 둘뿐이고 longhand는
        // 모두 보통 항목이며 같은 무효화 범위를 가집니다.
        let bundles: Vec<PropertyKey> = FRAME_PROPERTIES
            .iter()
            .filter(|m| m.is_bundle())
            .map(|m| m.key)
            .collect();
        assert_eq!(bundles, vec![keys::SIZE, keys::CORNER_RADIUS]);
        for bundle in FRAME_PROPERTIES.iter().filter(|m| m.is_bundle()) {
            for longhand in bundle.longhands {
                let metadata = frame_property(*longhand).unwrap();
                assert!(!metadata.is_bundle());
                assert_eq!(
                    metadata.invalidation, bundle.invalidation,
                    "{}",
                    metadata.name
                );
                assert_eq!(longhand.0 >> 8, bundle.key.0 >> 8, "같은 대역");
            }
        }
        assert_eq!(
            frame_property(keys::SIZE).unwrap().longhands,
            &[keys::WIDTH, keys::HEIGHT]
        );
        assert_eq!(
            frame_property(keys::CORNER_RADIUS).unwrap().longhands,
            &keys::BORDER_RADIUS_LONGHANDS
        );
    }

    #[test]
    fn keys_and_names_are_unique_and_sorted() {
        let mut keys: Vec<u32> = FRAME_PROPERTIES.iter().map(|m| m.key.0).collect();
        let sorted = {
            let mut k = keys.clone();
            k.sort_unstable();
            k
        };
        assert_eq!(keys, sorted, "key 오름차순");
        keys.dedup();
        assert_eq!(keys.len(), FRAME_PROPERTIES.len(), "key 중복 금지");
        let mut names: Vec<&str> = FRAME_PROPERTIES.iter().map(|m| m.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), FRAME_PROPERTIES.len(), "이름 중복 금지");
    }

    #[test]
    fn defaults_match_payload_default() {
        let d = FramePayload::default();
        assert_eq!(d.size, Size2::ZERO);
        assert_eq!(frame_property(keys::SIZE).unwrap().default, "0 × 0");
        assert_eq!(frame_property(keys::WIDTH).unwrap().default, "0");
        assert_eq!(frame_property(keys::HEIGHT).unwrap().default, "0");
        assert_eq!(d.transform, Transform2D::identity());
        assert_eq!(frame_property(keys::TRANSFORM).unwrap().default, "identity");
        assert_eq!(d.clipping, Clipping::FrameBounds);
        assert_eq!(
            frame_property(keys::CLIPPING).unwrap().default,
            "frame-bounds"
        );
        assert!(d.background.is_none());
        assert!(d.border.is_none());
        assert_eq!(d.corner_radius, CornerRadius::ZERO);
        assert!(d.layout_mode.is_none());
    }

    #[test]
    fn invalidation_classes_are_fixed() {
        let layout_keys: Vec<PropertyKey> = FRAME_PROPERTIES
            .iter()
            .filter(|m| m.invalidation.layout)
            .map(|m| m.key)
            .collect();
        assert_eq!(
            layout_keys,
            vec![
                keys::SIZE,
                keys::BORDER,
                keys::LAYOUT_MODE,
                keys::WIDTH,
                keys::HEIGHT
            ]
        );
        // 모든 속성은 paint를 무효화합니다.
        assert!(FRAME_PROPERTIES.iter().all(|m| m.invalidation.paint));
    }

    #[test]
    fn names_register_into_property_name_registry() {
        let mut registry = PropertyNameRegistry::new();
        register_frame_property_names(&mut registry);
        assert_eq!(registry.name_of(keys::CLIPPING), Some("clipping"));
        assert_eq!(registry.key_of("layout-mode"), Some(keys::LAYOUT_MODE));
        assert_eq!(registry.key_of("background-color"), Some(keys::BACKGROUND));
        // 묶음 key는 이름으로 찾을 수 없습니다(새 편집은 longhand).
        assert_eq!(registry.key_of("border-radius"), None);
        assert_eq!(registry.key_of("size"), None);
        assert_eq!(registry.name_of(keys::CORNER_RADIUS), None);
    }
}
