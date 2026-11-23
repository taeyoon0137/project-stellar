//! semantic snapshot 속성 key (XIV.3.1).
//!
//! 속성은 `StableId + PropertyPath`로 식별합니다(아키텍처 §26.2). 문서 crate가 이미 정한
//! key는 그대로 재사용합니다.
//!
//! - Frame `0x01xx`([`frame`]), Image asset `0x0601`([`IMAGE_ASSET`]),
//!   ComponentInstance `0x0Axx`([`instance`]), Modifier `0x10xx`([`modifier`]).
//!
//! Shape·Path·Text·Image 필드 가운데 값 하나를 담는 경로는 문서 registry key(IX.11.3, `payload_metadata`)를
//! 씁니다. 값 여러 개를 한 경로에 담는 Rectangle size·corner radius, Line 끝점, Ellipse·Polygon bounds, Image
//! size와 도형 종류 tag는 snapshot 투영(`build.rs`)이 longhand 경로로 나뉠 때까지 전용 key를 유지합니다.
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

use stellar_document::payload_metadata::{image_keys, path_keys, shape_keys, text_keys};

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
/// Line cap tag(문서 key `stroke-linecap`, IX.11.3).
pub const LINE_CAP: PropertyKey = shape_keys::STROKE_LINECAP;
/// Polygon bounds.
pub const POLYGON_BOUNDS: PropertyKey = PropertyKey(0xE341);
/// Polygon 변 수(문서 key `polygon-sides`, IX.11.3).
pub const POLYGON_SIDES: PropertyKey = shape_keys::POLYGON_SIDES;
/// Polygon 회전(도, 문서 key `polygon-rotation`).
pub const POLYGON_ROTATION: PropertyKey = shape_keys::POLYGON_ROTATION;
/// Polygon corner radius(문서 key `polygon-corner-radius`).
pub const POLYGON_CORNER_RADIUS: PropertyKey = shape_keys::POLYGON_CORNER_RADIUS;

// --- Path·Text (문서 payload 단위 key, IX.11.3) ---

/// Path payload 전체(command·point id, 문서 canonical 인코딩, 문서 key `path-data`).
pub const PATH_DATA: PropertyKey = path_keys::PATH_DATA;
/// Text payload 전체(content·paragraph·style run, 문서 canonical 인코딩, 문서 key `text-content`).
///
/// style run은 content offset을 가리키므로 content와 분리하면 병합이 의미를 깨뜨립니다.
pub const TEXT_PAYLOAD: PropertyKey = text_keys::TEXT_CONTENT;

// --- Image (0xE6xx, asset 참조는 문서 key 0x0601) ---

/// Image 상자 크기.
pub const IMAGE_SIZE: PropertyKey = PropertyKey(0xE601);
/// Image fit tag(문서 key `object-fit`, IX.11.3).
pub const IMAGE_FIT: PropertyKey = image_keys::OBJECT_FIT;
/// Image crop `(x, y, width, height)`(문서 key `image-crop`).
pub const IMAGE_CROP: PropertyKey = image_keys::IMAGE_CROP;
/// Image sampling tag(문서 key `image-rendering`).
pub const IMAGE_SAMPLING: PropertyKey = image_keys::IMAGE_RENDERING;
/// Image 색공간 override tag(문서 key `image-color-space`).
pub const IMAGE_COLOR_SPACE: PropertyKey = image_keys::IMAGE_COLOR_SPACE;

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

/// semantic diff 표시 문자열(IX.11.2): 문서 registry의 CSS longhand 또는 Stellar 이름이고, 묶음 key는 대응
/// longhand 이름 목록입니다. 문서 registry에 없는 snapshot 전용 key는 `None`(호출자는 숫자 key를 씁니다).
pub fn display_name(key: PropertyKey) -> Option<String> {
    stellar_document::css::registry::display_label(key)
}

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
            ("modifier.shadow_offset", modifier::SHADOW_OFFSET),
            ("modifier.shadow_offset_x", modifier::SHADOW_OFFSET_X),
            ("modifier.shadow_offset_y", modifier::SHADOW_OFFSET_Y),
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
        // 전용 대역(0xE0xx~0xEFxx) 밖의 key는 문서 crate가 정한 key여야 합니다(registry 또는 instance key).
        for (name, key) in all_keys() {
            if (0xE000..=0xEFFF).contains(&key.0) {
                continue;
            }
            let documented = stellar_document::css::registry::display_label(key).is_some()
                || name.starts_with("instance.");
            assert!(
                documented,
                "{name}=0x{:04X}가 전용 대역 밖인데 문서 key가 아닙니다",
                key.0
            );
        }
    }

    #[test]
    fn shape_path_text_image_fields_with_a_document_key_do_not_use_snapshot_only_keys() {
        // 검증 기준(IX.11.3): 문서 key가 생긴 필드의 snapshot 전용 key 사용 0. 남은 0xE3xx·0xE6xx는 값 여러
        // 개를 한 경로에 담는 투영(build.rs 소유 범위가 longhand 경로로 나눌 때 교체)과 도형 종류 tag뿐입니다.
        let remaining: Vec<&str> = all_keys()
            .into_iter()
            .filter(|(_, key)| (0xE300..=0xE6FF).contains(&key.0))
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            remaining,
            [
                "shape.kind",
                "rect.size",
                "rect.corner_radius",
                "ellipse.bounds",
                "line.start",
                "line.end",
                "polygon.bounds",
                "image.size",
            ]
        );
        for key in [
            LINE_CAP,
            POLYGON_SIDES,
            POLYGON_ROTATION,
            POLYGON_CORNER_RADIUS,
            PATH_DATA,
            TEXT_PAYLOAD,
            IMAGE_FIT,
            IMAGE_CROP,
            IMAGE_SAMPLING,
            IMAGE_COLOR_SPACE,
        ] {
            assert!(display_name(key).is_some(), "0x{:04x}", key.0);
        }
    }

    #[test]
    fn display_names_follow_the_document_registry() {
        assert_eq!(
            display_name(frame::BACKGROUND).as_deref(),
            Some("background-color")
        );
        assert_eq!(display_name(frame::WIDTH).as_deref(), Some("width"));
        assert_eq!(display_name(frame::SIZE).as_deref(), Some("width, height"));
        assert_eq!(display_name(modifier::FILL_COLOR).as_deref(), Some("fill"));
        assert_eq!(
            display_name(modifier::OPACITY_VALUE).as_deref(),
            Some("opacity")
        );
        assert_eq!(display_name(VISIBLE), None);
    }

    #[test]
    fn asset_reference_classification() {
        assert!(is_asset_reference(IMAGE_ASSET));
        assert!(!is_asset_reference(IMAGE_SIZE));
        assert!(!is_asset_reference(frame::SIZE));
    }

    #[test]
    fn frame_snapshot_uses_longhand_keys_so_corner_edits_touch_disjoint_paths() {
        // 검증 기준(IX.11.22) 일부: 서로 다른 모서리를 바꾼 두 편집은 서로 다른 snapshot 경로만 바꿉니다.
        // 그래서 3-way merge가 같은 경로를 두 번 바꾸는 conflict를 만들 근거가 없습니다(merge 실행 test는
        // semantic-merge crate 범위).
        use stellar_core_types::ids::StableId;
        use stellar_core_types::order_key::OrderKey;
        use stellar_core_types::property::PropertyPath;
        use stellar_document::frame::{CornerRadius, FramePayload};
        use stellar_document::records::{NodePayload, NodeRecord};

        let id = StableId::new_v7(3, 0, 3);
        let record = |radius: CornerRadius| NodeRecord {
            id,
            parent: None,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: "card".into(),
            payload: NodePayload::Frame(FramePayload {
                corner_radius: radius,
                ..FramePayload::default()
            }),
        };
        let document = StableId::new_v7(1, 0, 1);
        let project =
            |radius| crate::build::project_document_node(document, &record(radius)).unwrap();
        let base = project(CornerRadius::ZERO);
        let ours = project(CornerRadius::new(4., 0., 0., 0.).unwrap());
        let theirs = project(CornerRadius::new(0., 0., 8., 0.).unwrap());
        let changed = |other: &crate::snapshot::SemanticObject| -> Vec<PropertyPath> {
            base.properties()
                .iter()
                .filter(|(path, value)| other.property(path) != Some(value))
                .map(|(path, _)| path.clone())
                .collect()
        };
        let path = |key| PropertyPath::with_keys(id, vec![key]);
        assert_eq!(changed(&ours), vec![path(frame::BORDER_TOP_LEFT_RADIUS)]);
        assert_eq!(
            changed(&theirs),
            vec![path(frame::BORDER_BOTTOM_RIGHT_RADIUS)]
        );
        assert!(
            base.property(&path(frame::CORNER_RADIUS)).is_none(),
            "묶음 key 경로 없음"
        );
        assert!(base.property(&path(frame::SIZE)).is_none());
        assert!(base.property(&path(frame::WIDTH)).is_some());
    }
}
