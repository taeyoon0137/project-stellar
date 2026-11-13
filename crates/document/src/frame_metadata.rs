//! Frame property metadata registry (VI.3.5).
//!
//! Frame payload의 각 속성에 대해 [`PropertyKey`], 값 타입, 기본값, 무효화 범위
//! (layout/paint)를 한 곳에 고정합니다. 편집 command·inspector·증분 무효화가
//! 이 표를 단일 근거로 사용합니다.
//!
//! **누락 검사**: [`frame_field_keys`]는 [`FramePayload`]를 `..` 없이 구조 분해하므로
//! payload에 필드가 추가되면 컴파일이 실패합니다. 테스트는 registry와 필드
//! 목록이 1:1인지, 기본값이 일치하는지 검사합니다.

use stellar_core_types::property::{PropertyKey, PropertyNameRegistry};

use crate::frame::{keys, FramePayload};

/// 속성 값 타입.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePropertyType {
    /// 비음수 크기(`Size2`).
    Size,
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
    /// 사람이 읽는 이름(식별에 사용 금지 — key가 식별자).
    pub name: &'static str,
    /// 값 타입.
    pub value_type: FramePropertyType,
    /// 기본값 설명(`FramePayload::default()`와 일치해야 함).
    pub default: &'static str,
    /// 무효화 범위.
    pub invalidation: Invalidation,
}

/// Frame 속성 registry. 순서는 key 오름차순입니다.
pub const FRAME_PROPERTIES: [FramePropertyMetadata; 7] = [
    FramePropertyMetadata {
        key: keys::SIZE,
        name: "size",
        value_type: FramePropertyType::Size,
        default: "0 × 0",
        invalidation: Invalidation::LAYOUT_AND_PAINT,
    },
    FramePropertyMetadata {
        key: keys::TRANSFORM,
        name: "transform",
        value_type: FramePropertyType::Transform,
        default: "identity",
        // 시각 변환: 배치(layout)는 바꾸지 않습니다.
        invalidation: Invalidation::PAINT,
    },
    FramePropertyMetadata {
        key: keys::CLIPPING,
        name: "clipping",
        value_type: FramePropertyType::Clipping,
        default: "frame-bounds",
        invalidation: Invalidation::PAINT,
    },
    FramePropertyMetadata {
        key: keys::BACKGROUND,
        name: "background",
        value_type: FramePropertyType::OptionalColor,
        default: "none",
        invalidation: Invalidation::PAINT,
    },
    FramePropertyMetadata {
        key: keys::BORDER,
        name: "border",
        value_type: FramePropertyType::OptionalBorder,
        default: "none",
        // 두께가 content box에 영향을 줄 수 있어 layout까지 보수적으로 무효화합니다.
        invalidation: Invalidation::LAYOUT_AND_PAINT,
    },
    FramePropertyMetadata {
        key: keys::CORNER_RADIUS,
        name: "corner-radius",
        value_type: FramePropertyType::CornerRadius,
        default: "0",
        invalidation: Invalidation::PAINT,
    },
    FramePropertyMetadata {
        key: keys::LAYOUT_MODE,
        name: "layout-mode",
        value_type: FramePropertyType::OptionalLayoutRef,
        default: "none",
        invalidation: Invalidation::LAYOUT_AND_PAINT,
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
pub fn frame_field_keys(payload: &FramePayload) -> [PropertyKey; 7] {
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
        keys::SIZE,
        keys::TRANSFORM,
        keys::CLIPPING,
        keys::BACKGROUND,
        keys::BORDER,
        keys::CORNER_RADIUS,
        keys::LAYOUT_MODE,
    ]
}

/// Frame 속성 이름을 이름 registry(III.2.1)에 등록합니다.
pub fn register_frame_property_names(registry: &mut PropertyNameRegistry) {
    for metadata in &FRAME_PROPERTIES {
        registry.register(metadata.key, metadata.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Clipping, CornerRadius};
    use stellar_core_types::value::{Size2, Transform2D};

    #[test]
    fn registry_and_payload_fields_match_one_to_one() {
        // 검증 기준(VI.3.5): registry와 실제 payload 필드 누락 검사 통과.
        let field_keys = frame_field_keys(&FramePayload::default());
        assert_eq!(field_keys.len(), FRAME_PROPERTIES.len());
        for key in field_keys {
            assert!(frame_property(key).is_some(), "registry 누락: {key:?}");
        }
        for metadata in &FRAME_PROPERTIES {
            assert!(
                field_keys.contains(&metadata.key),
                "payload에 없는 registry 항목: {}",
                metadata.name
            );
        }
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
            vec![keys::SIZE, keys::BORDER, keys::LAYOUT_MODE]
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
    }
}
