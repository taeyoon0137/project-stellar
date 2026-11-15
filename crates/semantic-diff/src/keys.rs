//! semantic snapshot 속성 key (XIV.3.1).
//!
//! 속성은 `StableId + PropertyPath`로 식별합니다(아키텍처 §26.2). 문서 crate가 이미 정한
//! key는 그대로 재사용합니다.
//!
//! - Frame `0x01xx`([`frame`]), Image asset `0x0601`([`IMAGE_ASSET`]),
//!   ComponentInstance `0x0Axx`([`instance`]), Modifier `0x10xx`([`modifier`]).
//!
//! 문서 crate에 아직 key가 없는 필드는 **semantic snapshot 전용 대역 `0xE0xx`~`0xEFxx`**에
//! 둡니다. 이 대역은 저장 포맷이 아니라 diff·merge 식별용이며(snapshot은 저장하지 않음),
//! 문서 crate가 같은 필드의 공식 key를 정하면 그 key로 교체합니다. 문서 crate의 node 종류
//! 대역(`kind << 8`)·modifier 대역(`0x10xx`)과 겹치지 않습니다.

use stellar_core_types::property::PropertyKey;

/// Frame 속성 key(문서 crate, VI.3.4).
pub use stellar_document::frame::keys as frame;

/// ComponentInstance 속성 key(문서 crate, IX.2.1).
pub use stellar_document::component::instance::keys as instance;

/// Modifier 속성 key(문서 crate, VI.9.7).
pub use stellar_document::modifier_metadata::keys as modifier;

/// Image asset 참조(문서 crate, VI.8.2).
pub const IMAGE_ASSET: PropertyKey = stellar_document::image::IMAGE_PROP_ASSET;

// --- 공통 node·정의 필드 (0xE0xx) ---

/// 표시 여부.
pub const VISIBLE: PropertyKey = PropertyKey(0xE001);
/// 잠금 여부.
pub const LOCKED: PropertyKey = PropertyKey(0xE002);
/// 표시 이름(식별에 사용하지 않음).
pub const DISPLAY_NAME: PropertyKey = PropertyKey(0xE003);

// --- 문서·페이지·View 정의 (0xE1xx) ---

/// 문서의 페이지 목록(표시 순).
pub const DOCUMENT_PAGES: PropertyKey = PropertyKey(0xE101);
/// 페이지의 View 목록(표시 순).
pub const PAGE_VIEWS: PropertyKey = PropertyKey(0xE102);
/// View의 root Frame.
pub const VIEW_ROOT_FRAME: PropertyKey = PropertyKey(0xE103);

/// 문서 기본 typography 참조.
pub const TEXT_DEFAULT_TYPOGRAPHY: PropertyKey = PropertyKey(0xE104);
/// 문서 기본 글자색.
pub const TEXT_DEFAULT_FILL: PropertyKey = PropertyKey(0xE105);
/// 문단 style entry, 원래 u32 index.
pub const TEXT_PARAGRAPH_STYLE: PropertyKey = PropertyKey(0xE106);
/// 문자 style entry, 원래 u32 index.
pub const TEXT_RUN_STYLE: PropertyKey = PropertyKey(0xE107);
/// 알 수 없는 버전의 Typography 원본.
pub const TEXT_OPAQUE_TYPOGRAPHY: PropertyKey = PropertyKey(0xE108);
/// hash로 등록한 프로젝트 글꼴 record.
pub const TEXT_FONT_ASSET: PropertyKey = PropertyKey(0xE109);

// 문단·문자 style entry 안의 필드. 경로는 `[TEXT_PARAGRAPH_STYLE | TEXT_RUN_STYLE, 필드]` + index이고,
// 같은 entry의 다른 필드는 따로 병합합니다.

/// style의 typography resource 참조.
pub const TEXT_STYLE_TYPOGRAPHY: PropertyKey = PropertyKey(0xE10A);
/// 문단 정렬.
pub const TEXT_STYLE_ALIGNMENT: PropertyKey = PropertyKey(0xE10B);
/// 문단 줄바꿈 방식.
pub const TEXT_STYLE_WRAPPING: PropertyKey = PropertyKey(0xE10C);
/// 문단 줄 높이.
pub const TEXT_STYLE_LINE_HEIGHT: PropertyKey = PropertyKey(0xE10D);
/// 문자 채우기 색.
pub const TEXT_STYLE_FILL: PropertyKey = PropertyKey(0xE10E);
/// 문자 letter-spacing.
pub const TEXT_STYLE_LETTER_SPACING: PropertyKey = PropertyKey(0xE10F);
/// 문자 OpenType feature 목록.
pub const TEXT_STYLE_FEATURES: PropertyKey = PropertyKey(0xE110);
/// 문자 variation axis 목록.
pub const TEXT_STYLE_VARIATIONS: PropertyKey = PropertyKey(0xE111);

// --- 프로젝트 metadata (0xE2xx) ---

/// 프로젝트 metadata 항목. 마지막 segment가 항목 이름(map key)입니다.
pub const PROJECT_METADATA: PropertyKey = PropertyKey(0xE201);

// --- Shape (0xE3xx) ---

/// 도형 종류 tag(`ShapeKind::tag`).
pub const SHAPE_KIND: PropertyKey = PropertyKey(0xE301);
/// Rectangle size.
pub const RECT_SIZE: PropertyKey = PropertyKey(0xE311);
/// Rectangle corner radius(원본 값).
pub const RECT_CORNER_RADIUS: PropertyKey = PropertyKey(0xE312);
/// Ellipse bounds.
pub const ELLIPSE_BOUNDS: PropertyKey = PropertyKey(0xE321);
/// Line 시작점.
pub const LINE_START: PropertyKey = PropertyKey(0xE331);
/// Line 끝점.
pub const LINE_END: PropertyKey = PropertyKey(0xE332);
/// Line cap tag.
pub const LINE_CAP: PropertyKey = PropertyKey(0xE333);
/// Polygon bounds.
pub const POLYGON_BOUNDS: PropertyKey = PropertyKey(0xE341);
/// Polygon 변 수.
pub const POLYGON_SIDES: PropertyKey = PropertyKey(0xE342);
/// Polygon 회전(도).
pub const POLYGON_ROTATION: PropertyKey = PropertyKey(0xE343);
/// Polygon corner radius.
pub const POLYGON_CORNER_RADIUS: PropertyKey = PropertyKey(0xE344);

// --- Path·Text (0xE4xx) ---

/// Path payload 전체(command·point id, 문서 canonical 인코딩).
pub const PATH_DATA: PropertyKey = PropertyKey(0xE401);
/// Text payload 전체(content·paragraph·style run, 문서 canonical 인코딩).
///
/// style run은 content offset을 가리키므로 content와 분리하면 병합이 의미를 깨뜨립니다.
pub const TEXT_PAYLOAD: PropertyKey = PropertyKey(0xE402);

// --- Image (0xE6xx, asset 참조는 문서 key 0x0601) ---

/// Image 상자 크기.
pub const IMAGE_SIZE: PropertyKey = PropertyKey(0xE601);
/// Image fit tag.
pub const IMAGE_FIT: PropertyKey = PropertyKey(0xE602);
/// Image crop `(x, y, width, height)`.
pub const IMAGE_CROP: PropertyKey = PropertyKey(0xE603);
/// Image sampling tag.
pub const IMAGE_SAMPLING: PropertyKey = PropertyKey(0xE604);
/// Image 색공간 override tag.
pub const IMAGE_COLOR_SPACE: PropertyKey = PropertyKey(0xE605);

// --- Component 정의·Slot (0xE7xx) ---

/// 컴포넌트 root Frame.
pub const COMPONENT_ROOT: PropertyKey = PropertyKey(0xE701);
/// 공개 Property 목록(interface).
pub const COMPONENT_PROPERTIES: PropertyKey = PropertyKey(0xE702);
/// Slot 목록(interface).
pub const COMPONENT_SLOTS: PropertyKey = PropertyKey(0xE703);
/// local token scope(오름차순 token 목록).
pub const COMPONENT_LOCAL_TOKENS: PropertyKey = PropertyKey(0xE704);
/// Slot host Frame.
pub const SLOT_HOST: PropertyKey = PropertyKey(0xE711);
/// Slot 허용 node 종류(`u16` LE 오름차순).
pub const SLOT_ALLOWED_KINDS: PropertyKey = PropertyKey(0xE712);
/// Slot 최소 content 수.
pub const SLOT_MIN: PropertyKey = PropertyKey(0xE713);
/// Slot 최대 content 수(없으면 무제한).
pub const SLOT_MAX: PropertyKey = PropertyKey(0xE714);
/// Slot default content.
pub const SLOT_DEFAULT_CONTENT: PropertyKey = PropertyKey(0xE715);

// --- Token (0xE8xx) ---

/// token 이름.
pub const TOKEN_NAME: PropertyKey = PropertyKey(0xE801);
/// token 타입 numeric id.
pub const TOKEN_TYPE: PropertyKey = PropertyKey(0xE802);
/// token literal 값.
pub const TOKEN_VALUE: PropertyKey = PropertyKey(0xE803);
/// token 값 expression root node.
pub const TOKEN_EXPRESSION: PropertyKey = PropertyKey(0xE804);

// --- Asset (0xE9xx) ---

/// asset content hash.
pub const ASSET_CONTENT_HASH: PropertyKey = PropertyKey(0xE901);
/// asset media type.
pub const ASSET_MEDIA_TYPE: PropertyKey = PropertyKey(0xE902);

// --- Expression AST node (0xEAxx) ---

/// AST node payload(종류·operator·literal·reference, expressions canonical 인코딩).
pub const EXPR_PAYLOAD: PropertyKey = PropertyKey(0xEA01);
/// AST node의 typed child ref 목록(`u32` 개수 + (`u32` slot code + 16바이트 id)*, slot 순서).
pub const EXPR_CHILDREN: PropertyKey = PropertyKey(0xEA02);

// --- View Variant override (0xEBxx) ---

/// Property patch. 마지막 segment는 대상 `PropertyPath` canonical bytes의 소문자 hex입니다.
pub const VARIANT_PROPERTY_PATCH: PropertyKey = PropertyKey(0xEB01);
/// node 구조 patch. 마지막 segment는 대상 node StableId입니다.
pub const VARIANT_STRUCTURE_PATCH: PropertyKey = PropertyKey(0xEB02);
/// Slot change. 마지막 segment는 `인스턴스 StableId/slot StableId`입니다.
pub const VARIANT_SLOT_CHANGE: PropertyKey = PropertyKey(0xEB03);

/// 값이 asset 참조인 속성 key. 이 key의 변경은 `AssetRefChange`로 분류합니다.
pub const ASSET_REFERENCE_KEYS: [PropertyKey; 1] = [IMAGE_ASSET];

/// key가 asset 참조 속성인지.
pub fn is_asset_reference(key: PropertyKey) -> bool {
    ASSET_REFERENCE_KEYS.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 이 module이 정의·재사용하는 모든 key(이름, 값).
    fn all_keys() -> Vec<(&'static str, PropertyKey)> {
        vec![
            ("frame.size", frame::SIZE),
            ("frame.width", frame::WIDTH),
            ("frame.height", frame::HEIGHT),
            ("frame.transform", frame::TRANSFORM),
            ("frame.clipping", frame::CLIPPING),
            ("frame.background", frame::BACKGROUND),
            ("frame.border", frame::BORDER),
            ("frame.corner_radius", frame::CORNER_RADIUS),
            (
                "frame.border-top-left-radius",
                frame::BORDER_TOP_LEFT_RADIUS,
            ),
            (
                "frame.border-top-right-radius",
                frame::BORDER_TOP_RIGHT_RADIUS,
            ),
            (
                "frame.border-bottom-right-radius",
                frame::BORDER_BOTTOM_RIGHT_RADIUS,
            ),
            (
                "frame.border-bottom-left-radius",
                frame::BORDER_BOTTOM_LEFT_RADIUS,
            ),
            ("frame.layout_mode", frame::LAYOUT_MODE),
            ("image.asset", IMAGE_ASSET),
            ("instance.component", instance::COMPONENT_REF),
            ("instance.property_overrides", instance::PROPERTY_OVERRIDES),
            ("instance.slot_overrides", instance::SLOT_OVERRIDES),
            ("modifier.enabled", modifier::ENABLED),
            ("visible", VISIBLE),
            ("locked", LOCKED),
            ("display_name", DISPLAY_NAME),
            ("document.pages", DOCUMENT_PAGES),
            ("page.views", PAGE_VIEWS),
            ("document.text_default_typography", TEXT_DEFAULT_TYPOGRAPHY),
            ("document.text_default_fill", TEXT_DEFAULT_FILL),
            ("document.text_paragraph_style", TEXT_PARAGRAPH_STYLE),
            ("document.text_run_style", TEXT_RUN_STYLE),
            ("document.text_opaque_typography", TEXT_OPAQUE_TYPOGRAPHY),
            ("document.text_font_asset", TEXT_FONT_ASSET),
            ("text_style.typography", TEXT_STYLE_TYPOGRAPHY),
            ("text_style.alignment", TEXT_STYLE_ALIGNMENT),
            ("text_style.wrapping", TEXT_STYLE_WRAPPING),
            ("text_style.line_height", TEXT_STYLE_LINE_HEIGHT),
            ("text_style.fill", TEXT_STYLE_FILL),
            ("text_style.letter_spacing", TEXT_STYLE_LETTER_SPACING),
            ("text_style.features", TEXT_STYLE_FEATURES),
            ("text_style.variations", TEXT_STYLE_VARIATIONS),
            ("view.root_frame", VIEW_ROOT_FRAME),
            ("project.metadata", PROJECT_METADATA),
            ("shape.kind", SHAPE_KIND),
            ("rect.size", RECT_SIZE),
            ("rect.corner_radius", RECT_CORNER_RADIUS),
            ("ellipse.bounds", ELLIPSE_BOUNDS),
            ("line.start", LINE_START),
            ("line.end", LINE_END),
            ("line.cap", LINE_CAP),
            ("polygon.bounds", POLYGON_BOUNDS),
            ("polygon.sides", POLYGON_SIDES),
            ("polygon.rotation", POLYGON_ROTATION),
            ("polygon.corner_radius", POLYGON_CORNER_RADIUS),
            ("path.data", PATH_DATA),
            ("text.payload", TEXT_PAYLOAD),
            ("image.size", IMAGE_SIZE),
            ("image.fit", IMAGE_FIT),
            ("image.crop", IMAGE_CROP),
            ("image.sampling", IMAGE_SAMPLING),
            ("image.color_space", IMAGE_COLOR_SPACE),
            ("component.root", COMPONENT_ROOT),
            ("component.properties", COMPONENT_PROPERTIES),
            ("component.slots", COMPONENT_SLOTS),
            ("component.local_tokens", COMPONENT_LOCAL_TOKENS),
            ("slot.host", SLOT_HOST),
            ("slot.allowed_kinds", SLOT_ALLOWED_KINDS),
            ("slot.min", SLOT_MIN),
            ("slot.max", SLOT_MAX),
            ("slot.default_content", SLOT_DEFAULT_CONTENT),
            ("token.name", TOKEN_NAME),
            ("token.type", TOKEN_TYPE),
            ("token.value", TOKEN_VALUE),
            ("token.expression", TOKEN_EXPRESSION),
            ("asset.content_hash", ASSET_CONTENT_HASH),
            ("asset.media_type", ASSET_MEDIA_TYPE),
            ("expr.payload", EXPR_PAYLOAD),
            ("expr.children", EXPR_CHILDREN),
            ("variant.property_patch", VARIANT_PROPERTY_PATCH),
            ("variant.structure_patch", VARIANT_STRUCTURE_PATCH),
            ("variant.slot_change", VARIANT_SLOT_CHANGE),
        ]
    }

    #[test]
    fn keys_are_unique() {
        let keys = all_keys();
        let mut values: Vec<u32> = keys.iter().map(|(_, k)| k.0).collect();
        values.sort_unstable();
        let before = values.len();
        values.dedup();
        assert_eq!(before, values.len(), "중복 key가 있습니다");
    }

    #[test]
    fn snapshot_only_keys_stay_in_reserved_band() {
        // 문서 crate key(0x01xx·0x06xx·0x0Axx·0x10xx)를 제외한 전용 key는 0xE0xx~0xEFxx.
        for (name, key) in all_keys() {
            let reused = name.starts_with("frame.")
                || name.starts_with("instance.")
                || name.starts_with("modifier.")
                || name == "image.asset";
            if !reused {
                assert!(
                    (0xE000..=0xEFFF).contains(&key.0),
                    "{name}=0x{:04X}가 전용 대역 밖입니다",
                    key.0
                );
            }
        }
    }

    #[test]
    fn asset_reference_classification() {
        assert!(is_asset_reference(IMAGE_ASSET));
        assert!(!is_asset_reference(IMAGE_SIZE));
        assert!(!is_asset_reference(frame::SIZE));
    }
}
