//! Modifier property metadata와 무효화 class registry (VI.9.7).
//!
//! modifier stack(VI.9)의 각 속성에 대해 [`PropertyKey`], 소유 modifier 종류, 값 타입,
//! 기본값, 무효화 class(geometry/layout/paint)를 한 곳에 고정합니다. 편집
//! command(VII.1.4)·inspector·scene dirty 변환(XI.2.1)이 이 표를 단일 근거로 사용합니다.
//!
//! **key 규칙**: `0x1000 | (kind << 4) | field`. kind nibble 0은 모든 modifier에 공통인
//! 항목(enabled·order)이고, 1..=12는 [`ModifierKind`] numeric 값입니다. Frame(`0x01xx`)·
//! Image(`0x06xx`) 등 node 종류별 key 대역과 겹치지 않습니다.
//!
//! **무효화 정책**: modifier는 layout 이후 평가되는 비파괴 stack(기술 아키텍처 §12.6)이라
//! 원본 geometry와 layout 입력을 바꾸지 않습니다. 따라서 어떤 modifier 속성도
//! [`InvalidationClass::Layout`]을 무효화하지 않습니다. 평가된 geometry나 visual bounds
//! (overflow 포함)가 바뀌면 [`InvalidationClass::Geometry`], 색·합성만 바뀌면
//! [`InvalidationClass::Paint`]입니다.
//!
//! **누락 검사**: [`modifier_payload_keys`]는 [`ModifierPayload`]를 wildcard arm과 `..` 없이
//! 구조 분해하므로 variant·필드가 추가·제거되면 컴파일이 실패합니다. 테스트는 registry와
//! 종류별 payload 필드가 1:1인지 검사합니다.

use stellar_core_types::property::{PropertyKey, PropertyNameRegistry};

use crate::css::grammar::{CssGrammar, LENGTH_NON_NEGATIVE, LENGTH_SIGNED};
use crate::css::mapping::CssMapping;
use crate::frame_metadata::Invalidation;
use crate::modifier::{ModifierKind, ModifierPayload};

/// 무효화 class. 숫자 값은 bitset 위치입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum InvalidationClass {
    /// 평가된 geometry 또는 visual bounds 재계산(tessellation·hit test·culling·dirty rect).
    Geometry = 0,
    /// layout 재계산(자신·자손·형제 배치).
    Layout = 1,
    /// paint(scene·GPU 리소스) 갱신.
    Paint = 2,
}

impl InvalidationClass {
    /// 정의된 모든 class.
    pub const ALL: [InvalidationClass; 3] = [
        InvalidationClass::Geometry,
        InvalidationClass::Layout,
        InvalidationClass::Paint,
    ];

    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            InvalidationClass::Geometry => "geometry",
            InvalidationClass::Layout => "layout",
            InvalidationClass::Paint => "paint",
        }
    }

    const fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// 무효화 class 집합(bitset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InvalidationSet(u8);

impl InvalidationSet {
    /// 빈 집합.
    pub const EMPTY: InvalidationSet = InvalidationSet(0);
    /// paint만.
    pub const PAINT: InvalidationSet = InvalidationSet::EMPTY.with(InvalidationClass::Paint);
    /// geometry + paint.
    pub const GEOMETRY_AND_PAINT: InvalidationSet =
        InvalidationSet::PAINT.with(InvalidationClass::Geometry);

    /// `class`를 더한 집합.
    pub const fn with(self, class: InvalidationClass) -> InvalidationSet {
        InvalidationSet(self.0 | class.bit())
    }

    /// 합집합.
    pub const fn union(self, other: InvalidationSet) -> InvalidationSet {
        InvalidationSet(self.0 | other.0)
    }

    /// `class`를 포함하는지.
    pub const fn contains(self, class: InvalidationClass) -> bool {
        self.0 & class.bit() != 0
    }

    /// 비어 있는지.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// 포함된 class(정의 순).
    pub fn classes(self) -> Vec<InvalidationClass> {
        InvalidationClass::ALL
            .into_iter()
            .filter(|c| self.contains(*c))
            .collect()
    }
}

impl From<Invalidation> for InvalidationSet {
    /// Frame registry(VI.3.5)의 layout/paint 범위를 같은 class 집합으로 옮깁니다.
    /// command 결과(VII.1.4)가 node·modifier 무효화를 한 집합으로 합칠 때 씁니다.
    fn from(value: Invalidation) -> InvalidationSet {
        let mut set = InvalidationSet::EMPTY;
        if value.layout {
            set = set.with(InvalidationClass::Layout);
        }
        if value.paint {
            set = set.with(InvalidationClass::Paint);
        }
        set
    }
}

/// modifier 속성 key. 숫자 값은 저장·진단 경로의 일부입니다.
pub mod keys {
    use stellar_core_types::property::PropertyKey;

    /// 공통: 활성 여부(VI.9.3).
    pub const ENABLED: PropertyKey = PropertyKey(0x1001);
    /// 공통: stack 안 순서 key(VI.9.1).
    pub const ORDER: PropertyKey = PropertyKey(0x1002);
    /// Transform: 추가 변환 행렬.
    pub const TRANSFORM_MATRIX: PropertyKey = PropertyKey(0x1011);
    /// Boolean: 연산.
    pub const BOOLEAN_OP: PropertyKey = PropertyKey(0x1021);
    /// Boolean: 피연산 node 참조 목록.
    pub const BOOLEAN_OPERANDS: PropertyKey = PropertyKey(0x1022);
    /// StrokeExpand: 확장 폭.
    pub const STROKE_EXPAND_WIDTH: PropertyKey = PropertyKey(0x1031);
    /// Corner: radius.
    pub const CORNER_RADIUS: PropertyKey = PropertyKey(0x1041);
    /// Mask: 원본 node 참조.
    pub const MASK_SOURCE: PropertyKey = PropertyKey(0x1051);
    /// Clip: 원본 node 참조.
    pub const CLIP_SOURCE: PropertyKey = PropertyKey(0x1061);
    /// Fill: 색.
    pub const FILL_COLOR: PropertyKey = PropertyKey(0x1071);
    /// Stroke: 색.
    pub const STROKE_COLOR: PropertyKey = PropertyKey(0x1081);
    /// Stroke: 두께.
    pub const STROKE_WIDTH: PropertyKey = PropertyKey(0x1082);
    /// Shadow: 색.
    pub const SHADOW_COLOR: PropertyKey = PropertyKey(0x1091);
    /// Shadow: 이전 SetModifierProperty와 journal의 오프셋 x·y 묶음(IX.11.22). 새 편집은
    /// [`SHADOW_OFFSET_X`]·[`SHADOW_OFFSET_Y`]를 씁니다. 번호는 재사용하지 않습니다.
    pub const SHADOW_OFFSET: PropertyKey = PropertyKey(0x1092);
    /// Shadow: blur 반경.
    pub const SHADOW_BLUR: PropertyKey = PropertyKey(0x1093);
    /// Shadow: 오프셋 x(px, 음수 허용, IX.11.22).
    pub const SHADOW_OFFSET_X: PropertyKey = PropertyKey(0x1094);
    /// Shadow: 오프셋 y(px, 음수 허용, IX.11.22).
    pub const SHADOW_OFFSET_Y: PropertyKey = PropertyKey(0x1095);
    /// [`SHADOW_OFFSET`] 묶음이 나뉜 longhand(x, y 순).
    pub const SHADOW_OFFSET_LONGHANDS: [PropertyKey; 2] = [SHADOW_OFFSET_X, SHADOW_OFFSET_Y];
    /// Blur: 반경.
    pub const BLUR_RADIUS: PropertyKey = PropertyKey(0x10A1);
    /// Blend: mode.
    pub const BLEND_MODE: PropertyKey = PropertyKey(0x10B1);
    /// Opacity: 값.
    pub const OPACITY_VALUE: PropertyKey = PropertyKey(0x10C1);
}

/// modifier 속성 값 타입.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierPropertyType {
    /// bool.
    Bool,
    /// stack 순서 key(`OrderKey`).
    OrderKey,
    /// 2D 아핀 변환.
    Transform,
    /// boolean 연산 종류.
    BooleanOp,
    /// node 참조 목록.
    NodeRefList,
    /// node 참조 하나(NIL = 미지정).
    NodeRef,
    /// 비음수 finite 길이.
    NonNegative,
    /// 색.
    Color,
    /// 2D 오프셋(묶음 key 전용).
    Offset,
    /// 부호 있는 finite 길이(px).
    Coordinate,
    /// 0..=1 범위 값.
    UnitInterval,
    /// blend mode.
    BlendMode,
}

/// 속성 변경이 무효화하는 범위의 결정 규칙.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidationRule {
    /// 고정 집합.
    Fixed(InvalidationSet),
    /// 대상 modifier 종류의 payload 속성 무효화 합집합([`kind_invalidation`]).
    /// enabled 전환·순서 이동처럼 modifier 전체의 기여가 바뀌는 공통 항목에 씁니다.
    KindUnion,
}

/// 한 modifier 속성의 metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifierPropertyMetadata {
    /// 고정 numeric key.
    pub key: PropertyKey,
    /// 소유 modifier 종류. `None`이면 모든 종류에 공통인 항목입니다.
    pub owner: Option<ModifierKind>,
    /// CSS longhand 이름 또는 CSS 대응이 없을 때의 Stellar 이름(IX.11.2). 식별에 쓰지 않습니다(key가
    /// 식별자).
    pub name: &'static str,
    /// 값 타입.
    pub value_type: ModifierPropertyType,
    /// 기본값 설명(`ModifierPayload::default_for`·`push_back`과 일치해야 함).
    pub default: &'static str,
    /// 무효화 규칙.
    pub invalidation: InvalidationRule,
    /// 묶음 key(IX.11.22)가 나뉜 longhand key. 비어 있으면 longhand 하나인 보통 항목입니다. 묶음 key는
    /// 이전 command bytes·journal의 decode·실행에만 쓰고 새 편집은 longhand를 씁니다.
    pub longhands: &'static [PropertyKey],
    /// CSS 매핑(속성 모델 9장, IX.11.2).
    pub css: CssMapping,
}

impl ModifierPropertyMetadata {
    /// 묶음 key인지.
    pub const fn is_bundle(&self) -> bool {
        !self.longhands.is_empty()
    }

    /// 표시 문자열: 보통 항목은 `name`, 묶음 key는 대응 longhand 이름 목록입니다.
    pub fn display_label(&self) -> String {
        if self.is_bundle() {
            self.longhands
                .iter()
                .filter_map(|key| modifier_property(*key))
                .map(|m| m.name)
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            self.name.to_string()
        }
    }
}

const fn common(
    key: PropertyKey,
    name: &'static str,
    value_type: ModifierPropertyType,
    default: &'static str,
    css: CssMapping,
) -> ModifierPropertyMetadata {
    ModifierPropertyMetadata {
        key,
        owner: None,
        name,
        value_type,
        default,
        invalidation: InvalidationRule::KindUnion,
        longhands: &[],
        css,
    }
}

const fn field(
    key: PropertyKey,
    owner: ModifierKind,
    name: &'static str,
    value_type: ModifierPropertyType,
    default: &'static str,
    invalidation: InvalidationSet,
    css: CssMapping,
) -> ModifierPropertyMetadata {
    ModifierPropertyMetadata {
        key,
        owner: Some(owner),
        name,
        value_type,
        default,
        invalidation: InvalidationRule::Fixed(invalidation),
        longhands: &[],
        css,
    }
}

const fn bundle(
    field: ModifierPropertyMetadata,
    longhands: &'static [PropertyKey],
) -> ModifierPropertyMetadata {
    ModifierPropertyMetadata { longhands, ..field }
}

const GP: InvalidationSet = InvalidationSet::GEOMETRY_AND_PAINT;
const P: InvalidationSet = InvalidationSet::PAINT;

const BOOLEAN_OPS: &[&str] = &["union", "subtract", "intersect", "exclude"];
const BLEND_MODES: &[&str] = &[
    "normal", "multiply", "screen", "overlay", "darken", "lighten",
];
const SHADOW_TARGETS: &[&str] = &["box-shadow", "filter"];
const SHADOW_NOTE: &str = "Frame은 box-shadow, 그 밖은 filter: drop-shadow()입니다. spread·inset이 없고 blur 반경과 CSS 표준편차의 관계는 XI.10.4에서 고정합니다";

/// modifier 속성 registry. 순서는 key 오름차순입니다. 이름은 CSS longhand이거나 CSS 대응이 없을 때의
/// Stellar 이름입니다(IX.11.2, 속성 모델 9장).
pub const MODIFIER_PROPERTIES: [ModifierPropertyMetadata; 20] = [
    common(
        keys::ENABLED,
        "enabled",
        ModifierPropertyType::Bool,
        "true",
        CssMapping::stellar_only(Some(CssGrammar::Bool)).note("비활성 항목은 출력하지 않습니다"),
    ),
    common(
        keys::ORDER,
        "modifier-order",
        ModifierPropertyType::OrderKey,
        "stack 끝(push_back)",
        CssMapping::stellar_only(None)
            .note("stack 순서입니다. Web export는 CSS 고정 순서로 선형화합니다(속성 모델 9.1절)"),
    ),
    field(
        keys::TRANSFORM_MATRIX,
        ModifierKind::Transform,
        "transform",
        ModifierPropertyType::Transform,
        "identity",
        GP,
        CssMapping::converted(&["transform"], Some(CssGrammar::Transform)).note(
            "행렬 원점은 border box 좌상단입니다. export는 transform-origin o에 대해 translate(-o) matrix() translate(o)로 감쌉니다",
        ),
    ),
    field(
        keys::BOOLEAN_OP,
        ModifierKind::Boolean,
        "boolean-operation",
        ModifierPropertyType::BooleanOp,
        "union",
        GP,
        CssMapping::stellar_only(Some(CssGrammar::Keyword(BOOLEAN_OPS)))
            .note("export에서 path로 굽습니다"),
    ),
    field(
        keys::BOOLEAN_OPERANDS,
        ModifierKind::Boolean,
        "boolean-operands",
        ModifierPropertyType::NodeRefList,
        "[]",
        GP,
        CssMapping::stellar_only(None).note("node 참조 목록입니다"),
    ),
    field(
        keys::STROKE_EXPAND_WIDTH,
        ModifierKind::StrokeExpand,
        "stroke-expand-width",
        ModifierPropertyType::NonNegative,
        "1",
        GP,
        CssMapping::stellar_only(Some(LENGTH_NON_NEGATIVE)).note("export에서 path로 굽습니다"),
    ),
    field(
        keys::CORNER_RADIUS,
        ModifierKind::Corner,
        "corner-radius",
        ModifierPropertyType::NonNegative,
        "0",
        GP,
        CssMapping::stellar_only(Some(LENGTH_NON_NEGATIVE)).note(
            "path 모서리 처리입니다. Frame·Rectangle 반경은 border-*-radius가 맡습니다",
        ),
    ),
    // mask·clip 원본은 보이는 영역(visual bounds)을 바꿉니다.
    field(
        keys::MASK_SOURCE,
        ModifierKind::Mask,
        "mask-image",
        ModifierPropertyType::NodeRef,
        "nil",
        GP,
        CssMapping::approximated(&["mask-image", "mask-type"], None).note(
            "inline SVG mask를 url(#id)로 참조합니다. alpha·luminance mode가 정해지기 전(속성 모델 15장 O14)이라 approximated입니다",
        ),
    ),
    field(
        keys::CLIP_SOURCE,
        ModifierKind::Clip,
        "clip-path",
        ModifierPropertyType::NodeRef,
        "nil",
        GP,
        CssMapping::converted(&["clip-path"], None)
            .note("inline SVG clipPath를 url(#id)로 참조합니다"),
    ),
    field(
        keys::FILL_COLOR,
        ModifierKind::Fill,
        "fill",
        ModifierPropertyType::Color,
        "srgb(0, 0, 0, 1)",
        P,
        CssMapping::converted(&["fill", "background-image"], Some(CssGrammar::Color { allow_none: false }))
            .note("Shape·Path는 fill, Frame은 배경 layer입니다. Frame border가 있으면 overlay 요소로 냅니다(approximated, 속성 모델 9.2절)"),
    ),
    field(
        keys::STROKE_COLOR,
        ModifierKind::Stroke,
        "stroke",
        ModifierPropertyType::Color,
        "srgb(0, 0, 0, 1)",
        P,
        CssMapping::converted(&["stroke", "box-shadow"], Some(CssGrammar::Color { allow_none: false }))
            .note("Shape·Path는 stroke(가운데 정렬), Frame은 box-shadow: inset(안쪽 정렬)입니다"),
    ),
    // 두께는 stroke 외곽 geometry와 visual bounds를 바꿉니다.
    field(
        keys::STROKE_WIDTH,
        ModifierKind::Stroke,
        "stroke-width",
        ModifierPropertyType::NonNegative,
        "1",
        GP,
        CssMapping::converted(&["stroke-width", "box-shadow"], Some(LENGTH_NON_NEGATIVE)),
    ),
    field(
        keys::SHADOW_COLOR,
        ModifierKind::Shadow,
        "shadow-color",
        ModifierPropertyType::Color,
        "srgb(0, 0, 0, 1)",
        P,
        CssMapping::approximated(SHADOW_TARGETS, Some(CssGrammar::Color { allow_none: false }))
            .note(SHADOW_NOTE),
    ),
    // 오프셋·blur는 그림자 overflow(visual bounds)를 바꿉니다.
    bundle(
        field(
            keys::SHADOW_OFFSET,
            ModifierKind::Shadow,
            "shadow-offset",
            ModifierPropertyType::Offset,
            "(0, 0)",
            GP,
            CssMapping::approximated(SHADOW_TARGETS, None).note(
                "이전 command·journal의 offset x·y 묶음 key. 새 편집은 shadow-offset-x·y를 씁니다",
            ),
        ),
        &keys::SHADOW_OFFSET_LONGHANDS,
    ),
    field(
        keys::SHADOW_BLUR,
        ModifierKind::Shadow,
        "shadow-blur",
        ModifierPropertyType::NonNegative,
        "0",
        GP,
        CssMapping::approximated(SHADOW_TARGETS, Some(LENGTH_NON_NEGATIVE)).note(SHADOW_NOTE),
    ),
    field(
        keys::SHADOW_OFFSET_X,
        ModifierKind::Shadow,
        "shadow-offset-x",
        ModifierPropertyType::Coordinate,
        "0",
        GP,
        CssMapping::approximated(SHADOW_TARGETS, Some(LENGTH_SIGNED)).note(SHADOW_NOTE),
    ),
    field(
        keys::SHADOW_OFFSET_Y,
        ModifierKind::Shadow,
        "shadow-offset-y",
        ModifierPropertyType::Coordinate,
        "0",
        GP,
        CssMapping::approximated(SHADOW_TARGETS, Some(LENGTH_SIGNED)).note(SHADOW_NOTE),
    ),
    field(
        keys::BLUR_RADIUS,
        ModifierKind::Blur,
        "blur-radius",
        ModifierPropertyType::NonNegative,
        "0",
        GP,
        CssMapping::approximated(&["filter"], Some(LENGTH_NON_NEGATIVE)).note(
            "filter: blur()입니다. 합성 색공간과 반경 정의가 다릅니다(속성 모델 8.2절, XI.10.4)",
        ),
    ),
    field(
        keys::BLEND_MODE,
        ModifierKind::Blend,
        "mix-blend-mode",
        ModifierPropertyType::BlendMode,
        "normal",
        P,
        CssMapping::approximated(&["mix-blend-mode", "isolation"], Some(CssGrammar::Keyword(BLEND_MODES)))
            .subset()
            .note("keyword는 같고 합성 색공간이 다릅니다(선형 공간 합성, 14장 E1)"),
    ),
    field(
        keys::OPACITY_VALUE,
        ModifierKind::Opacity,
        "opacity",
        ModifierPropertyType::UnitInterval,
        "1",
        P,
        CssMapping::exact(&["opacity"], CssGrammar::UnitInterval)
            .note("그룹 opacity입니다. 겹침 합성 색공간은 속성 모델 8.2절"),
    ),
];

/// key로 metadata를 찾습니다.
pub fn modifier_property(key: PropertyKey) -> Option<&'static ModifierPropertyMetadata> {
    MODIFIER_PROPERTIES.iter().find(|m| m.key == key)
}

/// `kind` modifier가 새 편집에서 받는 속성(공통 항목 포함, key 순). 묶음 key는 빠집니다.
pub fn properties_of(kind: ModifierKind) -> Vec<&'static ModifierPropertyMetadata> {
    MODIFIER_PROPERTIES
        .iter()
        .filter(|m| m.owner.is_none_or(|owner| owner == kind) && !m.is_bundle())
        .collect()
}

/// `kind` modifier 전체의 무효화 범위: payload 속성 무효화의 합집합.
///
/// modifier 삽입·삭제·enabled 전환·순서 이동처럼 그 modifier의 기여 전체가 바뀌는
/// 변경에 사용합니다.
pub fn kind_invalidation(kind: ModifierKind) -> InvalidationSet {
    MODIFIER_PROPERTIES
        .iter()
        .filter(|m| m.owner == Some(kind))
        .fold(InvalidationSet::EMPTY, |acc, m| match m.invalidation {
            InvalidationRule::Fixed(set) => acc.union(set),
            InvalidationRule::KindUnion => acc,
        })
}

/// `kind` modifier의 `key` 속성 변경이 무효화하는 범위. 그 종류가 가지지 않는
/// 속성이면 `None`.
pub fn invalidation_of(kind: ModifierKind, key: PropertyKey) -> Option<InvalidationSet> {
    let metadata = modifier_property(key)?;
    match metadata.owner {
        Some(owner) if owner != kind => None,
        _ => Some(match metadata.invalidation {
            InvalidationRule::Fixed(set) => set,
            InvalidationRule::KindUnion => kind_invalidation(kind),
        }),
    }
}

/// payload의 모든 필드에 대응하는 key 목록(필드 순).
///
/// wildcard arm과 `..` 없는 구조 분해라 [`ModifierPayload`]에 variant나 필드가
/// 추가·제거되면 컴파일이 실패합니다 — registry 누락이 빌드 단계에서 드러납니다.
pub fn modifier_payload_keys(payload: &ModifierPayload) -> Vec<PropertyKey> {
    match payload {
        ModifierPayload::Transform(_matrix) => vec![keys::TRANSFORM_MATRIX],
        ModifierPayload::Boolean { op: _, operands: _ } => {
            vec![keys::BOOLEAN_OP, keys::BOOLEAN_OPERANDS]
        }
        ModifierPayload::StrokeExpand { width: _ } => vec![keys::STROKE_EXPAND_WIDTH],
        ModifierPayload::Corner { radius: _ } => vec![keys::CORNER_RADIUS],
        ModifierPayload::Mask { source: _ } => vec![keys::MASK_SOURCE],
        ModifierPayload::Clip { source: _ } => vec![keys::CLIP_SOURCE],
        ModifierPayload::Fill { color: _ } => vec![keys::FILL_COLOR],
        ModifierPayload::Stroke { color: _, width: _ } => {
            vec![keys::STROKE_COLOR, keys::STROKE_WIDTH]
        }
        ModifierPayload::Shadow {
            color: _,
            offset: _,
            blur: _,
        } => vec![
            keys::SHADOW_COLOR,
            keys::SHADOW_OFFSET_X,
            keys::SHADOW_OFFSET_Y,
            keys::SHADOW_BLUR,
        ],
        ModifierPayload::Blur { radius: _ } => vec![keys::BLUR_RADIUS],
        ModifierPayload::Blend { mode: _ } => vec![keys::BLEND_MODE],
        ModifierPayload::Opacity { value: _ } => vec![keys::OPACITY_VALUE],
    }
}

/// modifier 속성 이름을 이름 registry(III.2.1)에 등록합니다. 묶음 key(Shadow offset 0x1092)는 등록하지 않습니다
/// (ADR-0075 결정 1).
pub fn register_modifier_property_names(registry: &mut PropertyNameRegistry) {
    for metadata in MODIFIER_PROPERTIES.iter().filter(|m| !m.is_bundle()) {
        registry.register(metadata.key, metadata.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifier::{BlendMode, BooleanOp, ModifierId, ModifierStack};
    use std::collections::BTreeSet;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::StableId;
    use stellar_core_types::value::{FiniteF32, Transform2D, Vec2};

    #[test]
    fn registry_covers_every_payload_field_without_omission() {
        // 검증 기준(VI.9.7): registry 누락 검사 통과.
        for kind in ModifierKind::ALL {
            let field_keys = modifier_payload_keys(&ModifierPayload::default_for(kind));
            assert!(!field_keys.is_empty(), "{}: payload 속성 없음", kind.name());
            // payload 필드 → registry(소유 종류 일치).
            for key in &field_keys {
                let metadata = modifier_property(*key)
                    .unwrap_or_else(|| panic!("registry 누락: {} {key:?}", kind.name()));
                assert_eq!(metadata.owner, Some(kind), "{}", metadata.name);
            }
            // registry(이 종류 소유) → payload 필드.
            let mut registered: Vec<PropertyKey> = MODIFIER_PROPERTIES
                .iter()
                .filter(|m| m.owner == Some(kind) && !m.is_bundle())
                .map(|m| m.key)
                .collect();
            let mut sorted_fields = field_keys.clone();
            registered.sort_unstable();
            sorted_fields.sort_unstable();
            assert_eq!(
                registered,
                sorted_fields,
                "{}: registry↔필드 1:1",
                kind.name()
            );
        }
        // 모든 kind-소유 항목은 어떤 kind의 payload 필드입니다(고아 항목 없음).
        let all_field_keys: BTreeSet<u32> = ModifierKind::ALL
            .into_iter()
            .flat_map(|k| modifier_payload_keys(&ModifierPayload::default_for(k)))
            .map(|k| k.0)
            .collect();
        for metadata in MODIFIER_PROPERTIES
            .iter()
            .filter(|m| m.owner.is_some() && !m.is_bundle())
        {
            assert!(
                all_field_keys.contains(&metadata.key.0),
                "payload에 없는 registry 항목: {}",
                metadata.name
            );
        }
    }

    #[test]
    fn common_properties_apply_to_every_kind() {
        for key in [keys::ENABLED, keys::ORDER] {
            let metadata = modifier_property(key).unwrap();
            assert_eq!(metadata.owner, None);
            assert_eq!(metadata.invalidation, InvalidationRule::KindUnion);
        }
        for kind in ModifierKind::ALL {
            let props = properties_of(kind);
            assert!(props.iter().any(|m| m.key == keys::ENABLED));
            assert!(props.iter().any(|m| m.key == keys::ORDER));
            // 공통 2개 + 자신의 payload 필드만.
            let fields = modifier_payload_keys(&ModifierPayload::default_for(kind));
            assert_eq!(props.len(), 2 + fields.len(), "{}", kind.name());
        }
    }

    #[test]
    fn bundle_keys_split_into_registered_longhands_in_the_same_band() {
        // 검증 기준(IX.11.22): 묶음 key → longhand 대응, 묶음 key는 새 편집 목록에서 빠짐.
        let bundles: Vec<&ModifierPropertyMetadata> = MODIFIER_PROPERTIES
            .iter()
            .filter(|m| m.is_bundle())
            .collect();
        assert_eq!(bundles.len(), 1);
        let offset = bundles[0];
        assert_eq!(offset.key, keys::SHADOW_OFFSET);
        assert_eq!(offset.longhands, &keys::SHADOW_OFFSET_LONGHANDS);
        for longhand in offset.longhands {
            let metadata = modifier_property(*longhand).unwrap();
            assert_eq!(metadata.owner, offset.owner);
            assert!(!metadata.is_bundle());
            assert_eq!(longhand.0 >> 4, offset.key.0 >> 4, "같은 대역");
            assert_eq!(metadata.invalidation, offset.invalidation);
        }
        assert!(!properties_of(ModifierKind::Shadow)
            .iter()
            .any(|m| m.key == keys::SHADOW_OFFSET));
    }

    #[test]
    fn keys_follow_numbering_rule_and_are_unique_sorted() {
        let keys: Vec<u32> = MODIFIER_PROPERTIES.iter().map(|m| m.key.0).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "key 오름차순");
        sorted.dedup();
        assert_eq!(sorted.len(), MODIFIER_PROPERTIES.len(), "key 중복 금지");
        for metadata in &MODIFIER_PROPERTIES {
            let key = metadata.key.0;
            assert_eq!(key >> 8, 0x10, "{}: modifier 대역", metadata.name);
            let kind_nibble = ((key >> 4) & 0xF) as u16;
            let expected = metadata.owner.map_or(0, ModifierKind::as_u16);
            assert_eq!(kind_nibble, expected, "{}: kind nibble", metadata.name);
            assert_ne!(key & 0xF, 0, "{}: field 0은 예약", metadata.name);
        }
        let mut names: Vec<&str> = MODIFIER_PROPERTIES.iter().map(|m| m.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MODIFIER_PROPERTIES.len(), "이름 중복 금지");
    }

    #[test]
    fn value_types_and_defaults_match_payload_defaults() {
        let ty = |key| modifier_property(key).unwrap().value_type;
        let default = |key| modifier_property(key).unwrap().default;
        let black = Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap();

        assert_eq!(
            ModifierPayload::default_for(ModifierKind::Transform),
            ModifierPayload::Transform(Transform2D::identity())
        );
        assert_eq!(ty(keys::TRANSFORM_MATRIX), ModifierPropertyType::Transform);
        assert_eq!(default(keys::TRANSFORM_MATRIX), "identity");

        assert_eq!(
            ModifierPayload::default_for(ModifierKind::Boolean),
            ModifierPayload::Boolean {
                op: BooleanOp::Union,
                operands: vec![]
            }
        );
        assert_eq!(default(keys::BOOLEAN_OP), "union");
        assert_eq!(
            ty(keys::BOOLEAN_OPERANDS),
            ModifierPropertyType::NodeRefList
        );
        assert_eq!(default(keys::BOOLEAN_OPERANDS), "[]");

        assert_eq!(
            ModifierPayload::default_for(ModifierKind::StrokeExpand),
            ModifierPayload::StrokeExpand {
                width: FiniteF32::ONE
            }
        );
        assert_eq!(default(keys::STROKE_EXPAND_WIDTH), "1");
        assert_eq!(
            ModifierPayload::default_for(ModifierKind::Corner),
            ModifierPayload::Corner {
                radius: FiniteF32::ZERO
            }
        );
        assert_eq!(default(keys::CORNER_RADIUS), "0");
        assert_eq!(ty(keys::MASK_SOURCE), ModifierPropertyType::NodeRef);
        assert_eq!(default(keys::MASK_SOURCE), "nil");
        assert_eq!(default(keys::CLIP_SOURCE), "nil");
        assert_eq!(
            ModifierPayload::default_for(ModifierKind::Fill),
            ModifierPayload::Fill { color: black }
        );
        assert_eq!(default(keys::FILL_COLOR), "srgb(0, 0, 0, 1)");
        assert_eq!(
            ModifierPayload::default_for(ModifierKind::Shadow),
            ModifierPayload::Shadow {
                color: black,
                offset: Vec2::ZERO,
                blur: FiniteF32::ZERO
            }
        );
        assert_eq!(default(keys::SHADOW_OFFSET), "(0, 0)");
        assert_eq!(ty(keys::SHADOW_OFFSET), ModifierPropertyType::Offset);
        assert_eq!(default(keys::SHADOW_OFFSET_X), "0");
        assert_eq!(ty(keys::SHADOW_OFFSET_Y), ModifierPropertyType::Coordinate);
        assert_eq!(
            ModifierPayload::default_for(ModifierKind::Blend),
            ModifierPayload::Blend {
                mode: BlendMode::Normal
            }
        );
        assert_eq!(default(keys::BLEND_MODE), "normal");
        assert_eq!(
            ModifierPayload::default_for(ModifierKind::Opacity),
            ModifierPayload::Opacity {
                value: FiniteF32::ONE
            }
        );
        assert_eq!(ty(keys::OPACITY_VALUE), ModifierPropertyType::UnitInterval);
        assert_eq!(default(keys::OPACITY_VALUE), "1");

        // 공통 항목 기본값: push_back은 enabled=true로 stack 끝에 추가합니다.
        let mut stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        stack
            .push_back(
                ModifierId::new(StableId::new_v7(2, 0, 2)),
                ModifierKind::Blur,
            )
            .unwrap();
        assert!(stack.entries()[0].enabled);
        assert_eq!(default(keys::ENABLED), "true");
    }

    #[test]
    fn invalidation_classes_are_fixed() {
        // paint만 무효화하는 속성(색·합성).
        let paint_only: Vec<PropertyKey> = MODIFIER_PROPERTIES
            .iter()
            .filter(|m| m.invalidation == InvalidationRule::Fixed(InvalidationSet::PAINT))
            .map(|m| m.key)
            .collect();
        assert_eq!(
            paint_only,
            vec![
                keys::FILL_COLOR,
                keys::STROKE_COLOR,
                keys::SHADOW_COLOR,
                keys::BLEND_MODE,
                keys::OPACITY_VALUE,
            ]
        );
        // 나머지 kind 속성은 geometry + paint.
        for metadata in MODIFIER_PROPERTIES.iter().filter(|m| m.owner.is_some()) {
            if !paint_only.contains(&metadata.key) {
                assert_eq!(
                    metadata.invalidation,
                    InvalidationRule::Fixed(InvalidationSet::GEOMETRY_AND_PAINT),
                    "{}",
                    metadata.name
                );
            }
        }
        // modifier는 layout을 무효화하지 않습니다(post-layout 비파괴 stack).
        for kind in ModifierKind::ALL {
            assert!(!kind_invalidation(kind).contains(InvalidationClass::Layout));
            assert!(kind_invalidation(kind).contains(InvalidationClass::Paint));
        }
        // kind 전체 범위 golden.
        let golden: [(ModifierKind, InvalidationSet); 12] = [
            (ModifierKind::Transform, GP),
            (ModifierKind::Boolean, GP),
            (ModifierKind::StrokeExpand, GP),
            (ModifierKind::Corner, GP),
            (ModifierKind::Mask, GP),
            (ModifierKind::Clip, GP),
            (ModifierKind::Fill, P),
            (ModifierKind::Stroke, GP),
            (ModifierKind::Shadow, GP),
            (ModifierKind::Blur, GP),
            (ModifierKind::Blend, P),
            (ModifierKind::Opacity, P),
        ];
        for (kind, expected) in golden {
            assert_eq!(kind_invalidation(kind), expected, "{}", kind.name());
        }
    }

    #[test]
    fn invalidation_lookup_resolves_kind_union_and_owner() {
        assert_eq!(
            invalidation_of(ModifierKind::Opacity, keys::ENABLED),
            Some(InvalidationSet::PAINT)
        );
        assert_eq!(
            invalidation_of(ModifierKind::Transform, keys::ORDER),
            Some(InvalidationSet::GEOMETRY_AND_PAINT)
        );
        assert_eq!(
            invalidation_of(ModifierKind::Stroke, keys::STROKE_COLOR),
            Some(InvalidationSet::PAINT)
        );
        // 다른 종류의 속성·미등록 key.
        assert_eq!(
            invalidation_of(ModifierKind::Fill, keys::TRANSFORM_MATRIX),
            None
        );
        assert_eq!(
            invalidation_of(ModifierKind::Fill, PropertyKey(0x1FFF)),
            None
        );
    }

    #[test]
    fn invalidation_set_operations() {
        let set = InvalidationSet::EMPTY.with(InvalidationClass::Layout);
        assert!(set.contains(InvalidationClass::Layout));
        assert!(!set.contains(InvalidationClass::Paint));
        let merged = set.union(InvalidationSet::GEOMETRY_AND_PAINT);
        assert_eq!(merged.classes(), InvalidationClass::ALL.to_vec());
        assert!(InvalidationSet::EMPTY.is_empty());
        assert_eq!(InvalidationClass::Geometry.name(), "geometry");
        // Frame registry 범위와 같은 집합으로 합칠 수 있습니다.
        assert_eq!(
            InvalidationSet::from(Invalidation::PAINT),
            InvalidationSet::PAINT
        );
        assert_eq!(
            InvalidationSet::from(Invalidation::LAYOUT_AND_PAINT).classes(),
            vec![InvalidationClass::Layout, InvalidationClass::Paint]
        );
    }

    #[test]
    fn names_register_into_property_name_registry() {
        let mut registry = PropertyNameRegistry::new();
        register_modifier_property_names(&mut registry);
        assert_eq!(registry.name_of(keys::MASK_SOURCE), Some("mask-image"));
        assert_eq!(registry.key_of("opacity"), Some(keys::OPACITY_VALUE));
        assert_eq!(registry.key_of("shadow-offset"), None, "묶음 key");
        assert_eq!(
            registry.key_of("shadow-offset-x"),
            Some(keys::SHADOW_OFFSET_X)
        );
        // Frame 속성 key와 충돌하지 않습니다.
        crate::frame_metadata::register_frame_property_names(&mut registry);
        assert_eq!(registry.name_of(keys::ENABLED), Some("enabled"));
    }
}
