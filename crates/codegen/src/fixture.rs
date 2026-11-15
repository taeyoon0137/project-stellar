//! 테스트 전용 resolved View fixture(XXII.1.1~XXII.4.2 공용).
//!
//! ```text
//! 1 Frame(Stack 세로) "화면"                  fill = token surface(P3)
//! ├ 2 Text "설정"                              text color: 공통 → variant 100 → android×variant 100
//! ├ 3 Fragment
//! │ ├ 4 Image(label = 속성 4.3, 읽기 순서 1)   SF Symbol 속성(Apple native)
//! │ └ 5 Shape(장식)
//! ├ 6 Frame SingleSelectionGroup("크기")        backdrop filter(Android unsupported)
//! │ └ 7 Frame List(single, collection = 7)     (radio group에 흡수)
//! │   ├ 8 Frame Button ─ 9 Text "작게"
//! │   └ 10 Frame Button("크게")
//! ├ 11 Frame TextInput(placeholder "메모")
//! ├ 12 Frame Button("저장", disabled = state 52)  radius 8(ios 12), fill = token accent
//! ├ 13 Frame(Grid 2열) — ios에서 제외         fill = 확장 범위 sRGB
//! │ ├ 15 Text "새 기능"
//! │ └ 18 Shape(장식)
//! └ 16 Frame Modal("알림", dismiss 63)
//!   └ 17 Text "저장되었습니다"
//! ```

use std::collections::BTreeMap;

use stellar_accessibility::control::{ControlSemantics, SelectionMode, SourceControl};
use stellar_accessibility::generate::generate;
use stellar_accessibility::node::{
    AccessibilityBinding, AccessibilityNode, AccessibilityRole, ActionKind, ActionTarget, StateKind,
};
use stellar_accessibility::snapshot::AccessibilitySnapshot;
use stellar_accessibility::source::{SourceKind, SourceNode, SourceTree};
use stellar_accessibility::tree::AccessibilityTree;
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{FiniteF32, Insets, Length, Scalar};
use stellar_layout::flex::FlexItem;
use stellar_layout::geometry::Axis;
use stellar_layout::grid::{GridItem, GridLayout, TrackSize};
use stellar_layout::input::{
    ContainerLayout, IntrinsicContent, LayoutContent, LayoutInput, LayoutNode, SizingMode,
};
use stellar_layout::overlay::{OverlayAlignment, OverlayLayout};
use stellar_layout::sizing::SizeLimits;
use stellar_layout::stack::{CrossAlignment, MainAlignment, StackLayout};
use stellar_target_profile::builtin::{self, properties as p};
use stellar_target_profile::capability::{CapabilityPropertyId, ContractCode, ElementKind};
use stellar_target_profile::overrides::{OverrideScope, VariantLayer};
use stellar_target_profile::profile::TargetProfileId;
use stellar_target_profile::registry::{ProfileRegistryVersion, RegistryCatalog};
use stellar_target_profile::spec::{
    AndroidApiLevel, AndroidProfile, ApplePlatform, AppleProfile, CapabilitySetVersion,
    DottedVersion, ProfileSpec, TargetProfile, WebBaseline, WebProfile,
};
use stellar_target_profile::validate::{PropertyCapabilityMap, ViewModel, ViewNode};

use crate::ir::{build_ir, CodegenIr, CodegenSources, StateDecl, TokenDecl};

/// 마지막 바이트가 `n`인 결정론 StableId.
pub fn sid(n: u8) -> StableId {
    let mut bytes = [0u8; 16];
    bytes[15] = n;
    StableId::from_bytes(bytes)
}

/// `n` 순서의 OrderKey.
pub fn key(n: u32) -> OrderKey {
    let [_, hi, mid, lo] = n.to_be_bytes();
    OrderKey::from_bytes(vec![hi, mid, lo, 1]).expect("후행 1이라 정규형")
}

/// 길이.
pub fn len(px: f32) -> Length {
    Length::new(px).expect("유한")
}

/// 사방 같은 inset.
pub fn inset(t: f32, r: f32, b: f32, l: f32) -> Insets {
    Insets::new(t, r, b, l).expect("유한")
}

/// sRGB 색.
pub fn srgb(r: f32, g: f32, b: f32, a: f32) -> TypedValue {
    TypedValue::Color(Color::new(ColorSpace::Srgb, r, g, b, a).expect("색"))
}

/// Display P3 색.
pub fn p3(r: f32, g: f32, b: f32, a: f32) -> TypedValue {
    TypedValue::Color(Color::new(ColorSpace::DisplayP3, r, g, b, a).expect("색"))
}

/// 텍스트 값.
pub fn text(value: &str) -> TypedValue {
    TypedValue::String(value.to_string())
}

/// 속성 경로.
pub fn path(node: u8, key: u32) -> PropertyPath {
    PropertyPath::with_keys(sid(node), vec![PropertyKey(key)])
}

/// fill 색 속성 key(capability 대상 아님).
pub const KEY_FILL: u32 = 4;
/// 텍스트 색 속성 key.
pub const KEY_TEXT_COLOR: u32 = 5;
/// image label 문자열 key.
pub const KEY_LABEL: u32 = 3;
/// corner radius key.
pub const KEY_RADIUS: u32 = 1;
/// drop shadow key.
pub const KEY_SHADOW: u32 = 2;
/// backdrop filter key.
pub const KEY_BACKDROP: u32 = 101;
/// SF Symbol key.
pub const KEY_SF_SYMBOL: u32 = 201;

/// 테스트 PropertyPath → capability 대응.
pub struct KeyMap;

impl PropertyCapabilityMap for KeyMap {
    fn capability_of(&self, path: &PropertyPath) -> Option<CapabilityPropertyId> {
        Some(match path.keys().first()?.0 {
            KEY_RADIUS => p::CORNER_RADIUS,
            KEY_SHADOW => p::DROP_SHADOW,
            KEY_BACKDROP => p::CSS_BACKDROP_FILTER,
            KEY_SF_SYMBOL => p::APPLE_SF_SYMBOL,
            _ => return None,
        })
    }
}

fn profile_id(text: &str) -> TargetProfileId {
    TargetProfileId::new(text).expect("id")
}

fn pin() -> ProfileRegistryVersion {
    ProfileRegistryVersion::new(1).expect("pin")
}

/// Web profile `site`(Baseline 2024).
pub fn web_profile() -> TargetProfile {
    TargetProfile::new(
        profile_id("site"),
        pin(),
        ProfileSpec::Web(WebProfile {
            baseline: WebBaseline::new(2024).expect("baseline"),
            capability_set_version: CapabilitySetVersion::new(1).expect("set"),
        }),
    )
}

/// Apple profile `ios`(iOS 26).
pub fn ios_profile() -> TargetProfile {
    apple_profile("ios", ApplePlatform::Ios)
}

/// Apple profile `mac`(macOS 26).
pub fn mac_profile() -> TargetProfile {
    apple_profile("mac", ApplePlatform::MacOs)
}

fn apple_profile(name: &str, platform: ApplePlatform) -> TargetProfile {
    TargetProfile::new(
        profile_id(name),
        pin(),
        ProfileSpec::Apple(AppleProfile {
            platform,
            minimum_os: DottedVersion::new(26, 0, 0),
        }),
    )
}

/// Android profile `android`(API 35, Compose UI 1.7.0).
pub fn android_profile() -> TargetProfile {
    TargetProfile::new(
        profile_id("android"),
        pin(),
        ProfileSpec::Android(AndroidProfile {
            api_level: AndroidApiLevel::new(35).expect("api"),
            ui_system_version: DottedVersion::new(1, 7, 0),
        }),
    )
}

/// 상태 변수 id.
pub const STATE_SIZE: u8 = 50;
/// 메모 상태 변수 id.
pub const STATE_MEMO: u8 = 51;
/// 저장 중 상태 변수 id.
pub const STATE_SAVING: u8 = 52;
/// variant id.
pub const VARIANT_ERROR: u8 = 100;
/// surface token id.
pub const TOKEN_SURFACE: u8 = 70;
/// accent token id.
pub const TOKEN_ACCENT: u8 = 71;
/// spacing token id.
pub const TOKEN_SPACING: u8 = 72;

/// fixture 입력 전체.
pub struct Fixture {
    /// View.
    pub view: ViewModel,
    /// variant 층.
    pub variants: VariantLayer,
    /// layout.
    pub layout: LayoutInput,
    /// 접근성 snapshot.
    pub accessibility: AccessibilitySnapshot,
    /// 텍스트 내용.
    pub texts: BTreeMap<StableId, String>,
    /// 상태 변수.
    pub states: Vec<StateDecl>,
    /// token.
    pub tokens: Vec<TokenDecl>,
    /// token binding.
    pub token_bindings: BTreeMap<PropertyPath, StableId>,
    /// registry catalog.
    pub catalog: RegistryCatalog,
}

fn view_node(
    id: u8,
    parent: Option<u8>,
    element: ElementKind,
    contracts: &[ContractCode],
) -> ViewNode {
    ViewNode {
        id: sid(id),
        parent: parent.map(sid),
        element,
        contracts: contracts.to_vec(),
    }
}

fn view() -> ViewModel {
    use ContractCode as C;
    use ElementKind as E;
    let nodes = vec![
        view_node(1, None, E::FRAME, &[]),
        view_node(2, Some(1), E::TEXT, &[]),
        view_node(3, Some(1), E::FRAGMENT, &[]),
        view_node(4, Some(3), E::IMAGE, &[]),
        view_node(5, Some(3), E::SHAPE, &[]),
        view_node(6, Some(1), E::FRAME, &[C::SINGLE_SELECTION_GROUP]),
        view_node(7, Some(6), E::FRAME, &[C::LIST]),
        view_node(8, Some(7), E::FRAME, &[C::BUTTON]),
        view_node(9, Some(8), E::TEXT, &[]),
        view_node(10, Some(7), E::FRAME, &[C::BUTTON]),
        view_node(11, Some(1), E::FRAME, &[C::TEXT_INPUT]),
        view_node(12, Some(1), E::FRAME, &[C::BUTTON]),
        view_node(13, Some(1), E::FRAME, &[]),
        view_node(15, Some(13), E::TEXT, &[]),
        view_node(18, Some(13), E::SHAPE, &[]),
        view_node(16, Some(1), E::FRAME, &[C::MODAL]),
        view_node(17, Some(16), E::TEXT, &[]),
    ];
    let mut view = ViewModel {
        nodes,
        ..ViewModel::default()
    };
    let c = &mut view.common;
    c.set(path(1, KEY_FILL), p3(0.96, 0.96, 0.98, 1.0));
    c.set(path(2, KEY_TEXT_COLOR), srgb(0.1, 0.1, 0.12, 1.0));
    c.set(path(4, KEY_LABEL), text("프로필 사진"));
    c.set(path(4, KEY_SF_SYMBOL), text("person.crop.circle"));
    c.set(path(6, KEY_BACKDROP), TypedValue::Length(len(12.0)));
    c.set(path(12, KEY_RADIUS), TypedValue::Length(len(8.0)));
    c.set(path(12, KEY_FILL), srgb(0.0, 0.48, 1.0, 1.0));
    c.set(path(12, KEY_SHADOW), TypedValue::Length(len(4.0)));
    c.set(path(13, KEY_FILL), srgb(1.1, 0.5, -0.05, 1.0));
    let ios = view.overrides.profile_mut(&profile_id("ios"));
    ios.set(
        OverrideScope::Always,
        path(12, KEY_RADIUS),
        TypedValue::Length(len(12.0)),
    );
    ios.exclude_node(sid(13));
    let android = view.overrides.profile_mut(&profile_id("android"));
    android.set(
        OverrideScope::Variant(sid(VARIANT_ERROR)),
        path(2, KEY_TEXT_COLOR),
        srgb(0.7, 0.0, 0.0, 1.0),
    );
    view
}

fn variants() -> VariantLayer {
    let mut values = BTreeMap::new();
    values.insert(
        path(2, KEY_TEXT_COLOR),
        (sid(VARIANT_ERROR), srgb(0.8, 0.1, 0.1, 1.0)),
    );
    VariantLayer::new(vec![sid(VARIANT_ERROR)], values)
}

fn intrinsic(width: f32, height: f32) -> LayoutContent {
    LayoutContent::Intrinsic(IntrinsicContent {
        width: len(width),
        height: len(height),
        baseline: Some(len(height * 0.75)),
    })
}

fn hug(node: LayoutNode) -> LayoutNode {
    node.with_sizing(SizingMode::HugContent, SizingMode::HugContent)
}

fn grow(node: LayoutNode) -> LayoutNode {
    node.with_flex(FlexItem {
        grow: Scalar::new(1.0).expect("유한"),
        shrink: Scalar::new(1.0).expect("유한"),
        ..FlexItem::default()
    })
}

fn layout() -> LayoutInput {
    let mut root_stack = StackLayout::new(Axis::Vertical);
    root_stack.gap = len(16.0);
    root_stack.cross_alignment = CrossAlignment::Stretch;
    let mut row = StackLayout::new(Axis::Horizontal);
    row.gap = len(8.0);
    row.cross_alignment = CrossAlignment::Center;
    let mut chip = StackLayout::new(Axis::Horizontal);
    chip.main_alignment = MainAlignment::Center;
    let mut grid = GridLayout::new(
        vec![
            TrackSize::Fraction(Scalar::new(1.0).expect("유한")),
            TrackSize::Fraction(Scalar::new(1.0).expect("유한")),
        ],
        Vec::new(),
    );
    grid.column_gap = len(8.0);

    let root = LayoutNode::new(sid(1))
        .with_container(ContainerLayout::Stack(root_stack))
        .with_padding(inset(24.0, 24.0, 24.0, 24.0))
        .with_fixed_size(len(390.0), len(844.0));
    let root = [2u8, 4, 5, 6, 11, 12, 13, 16]
        .into_iter()
        .enumerate()
        .fold(root, |node, (order, child)| {
            node.with_child(sid(child), key(order as u32 + 1))
        });
    let nodes = vec![
        root,
        hug(LayoutNode::new(sid(2))).with_content(intrinsic(48.0, 28.0)),
        LayoutNode::new(sid(4)).with_fixed_size(len(64.0), len(64.0)),
        LayoutNode::new(sid(5)).with_sizing(SizingMode::Fill, SizingMode::Fixed(len(1.0))),
        LayoutNode::new(sid(6))
            .with_container(ContainerLayout::Stack(row))
            .with_sizing(SizingMode::Fill, SizingMode::HugContent)
            .with_child(sid(7), key(1)),
        LayoutNode::new(sid(7))
            .with_container(ContainerLayout::Stack(row))
            .with_sizing(SizingMode::Fill, SizingMode::HugContent)
            .with_child(sid(8), key(1))
            .with_child(sid(10), key(2)),
        grow(hug(LayoutNode::new(sid(8))))
            .with_container(ContainerLayout::Stack(chip))
            .with_padding(inset(12.0, 16.0, 12.0, 16.0))
            .with_child(sid(9), key(1)),
        hug(LayoutNode::new(sid(9))).with_content(intrinsic(32.0, 20.0)),
        grow(hug(LayoutNode::new(sid(10)))).with_padding(inset(12.0, 16.0, 12.0, 16.0)),
        LayoutNode::new(sid(11))
            .with_sizing(SizingMode::Fill, SizingMode::Fixed(len(44.0)))
            .with_padding(inset(8.0, 12.0, 8.0, 12.0)),
        LayoutNode::new(sid(12))
            .with_container(ContainerLayout::Overlay(OverlayLayout {
                horizontal: OverlayAlignment::Center,
                vertical: OverlayAlignment::Center,
            }))
            .with_sizing(SizingMode::Fill, SizingMode::Fixed(len(48.0)))
            .with_limits(SizeLimits {
                min_width: Some(len(120.0)),
                ..SizeLimits::default()
            }),
        LayoutNode::new(sid(13))
            .with_container(ContainerLayout::Grid(grid))
            .with_sizing(SizingMode::Fill, SizingMode::HugContent)
            .with_child(sid(15), key(1))
            .with_child(sid(18), key(2)),
        hug(LayoutNode::new(sid(15)))
            .with_content(intrinsic(56.0, 20.0))
            .with_grid(GridItem::at(1, 1, 1, 1)),
        LayoutNode::new(sid(18))
            .with_fixed_size(len(24.0), len(24.0))
            .with_grid(GridItem::at(2, 1, 1, 1)),
        LayoutNode::new(sid(16))
            .with_sizing(SizingMode::Fixed(len(320.0)), SizingMode::Fixed(len(120.0)))
            .with_child(sid(17), key(1)),
        hug(LayoutNode::new(sid(17)))
            .with_content(intrinsic(120.0, 20.0))
            .with_position(len(16.0), len(16.0)),
    ];
    LayoutInput::new(sid(1), nodes)
}

fn accessibility() -> AccessibilitySnapshot {
    let mut source = SourceTree::new();
    let frame = |n: u8| SourceNode::new(sid(n), SourceKind::Frame);
    for node in [
        frame(1).with_children([2, 3, 6, 11, 12, 13, 16].map(sid)),
        SourceNode::new(sid(2), SourceKind::Text),
        SourceNode::new(sid(3), SourceKind::Fragment).with_children([sid(4), sid(5)]),
        SourceNode::new(sid(4), SourceKind::Image)
            .with_label(AccessibilityBinding::Property(path(4, KEY_LABEL)))
            .with_reading_order(1),
        SourceNode::new(sid(5), SourceKind::Graphic).decorative(),
        frame(6)
            .with_label(AccessibilityBinding::text("크기"))
            .with_control(SourceControl::single_selection_group(
                sid(7),
                Some(sid(STATE_SIZE)),
            ))
            .with_children([sid(7)]),
        frame(7)
            .with_control(SourceControl::list(sid(7), SelectionMode::Single, None))
            .with_children([sid(8), sid(10)]),
        frame(8)
            .with_control(SourceControl::button(sid(61)))
            .with_children([sid(9)]),
        SourceNode::new(sid(9), SourceKind::Text),
        frame(10)
            .with_label(AccessibilityBinding::text("크게"))
            .with_control(SourceControl::button(sid(62))),
        frame(11).with_control(SourceControl {
            semantics: ControlSemantics::TextInput {
                value: Some(sid(STATE_MEMO)),
                placeholder: Some(AccessibilityBinding::text("메모")),
                focused: None,
            },
            focusable: true,
        }),
        frame(12)
            .with_label(AccessibilityBinding::text("저장"))
            .with_control(SourceControl {
                semantics: ControlSemantics::Button {
                    activate: Some(sid(60)),
                    focused: None,
                    disabled: Some(AccessibilityBinding::State(sid(STATE_SAVING))),
                    pressed: None,
                },
                focusable: true,
            }),
        frame(13).with_children([sid(15), sid(18)]),
        SourceNode::new(sid(15), SourceKind::Text),
        SourceNode::new(sid(18), SourceKind::Graphic).decorative(),
        frame(16)
            .with_label(AccessibilityBinding::text("알림"))
            .with_control(SourceControl::modal(Some(sid(63))))
            .with_children([sid(17)]),
        SourceNode::new(sid(17), SourceKind::Text),
    ] {
        source.insert(node).expect("source");
    }
    source.push_root(sid(1));
    let tree = generate(&source).expect("접근성 생성");
    AccessibilitySnapshot::from_tree(&tree).expect("snapshot")
}

impl Fixture {
    /// fixture를 만듭니다.
    pub fn new() -> Fixture {
        let texts = [
            (2, "설정"),
            (9, "작게"),
            (15, "새 기능"),
            (17, "저장되었습니다"),
        ]
        .into_iter()
        .map(|(id, value)| (sid(id), value.to_string()))
        .collect();
        let states = vec![
            StateDecl {
                id: sid(STATE_MEMO),
                name: "memo".to_string(),
                initial: text(""),
            },
            StateDecl {
                id: sid(STATE_SIZE),
                name: "selectedSize".to_string(),
                initial: TypedValue::String(sid(8).to_string()),
            },
            StateDecl {
                id: sid(STATE_SAVING),
                name: "isSaving".to_string(),
                initial: TypedValue::Boolean(false),
            },
        ];
        let tokens = vec![
            TokenDecl {
                id: sid(TOKEN_SPACING),
                name: "spacing.md".to_string(),
                value: TypedValue::Length(len(16.0)),
            },
            TokenDecl {
                id: sid(TOKEN_SURFACE),
                name: "surface".to_string(),
                value: p3(0.96, 0.96, 0.98, 1.0),
            },
            TokenDecl {
                id: sid(TOKEN_ACCENT),
                name: "accent".to_string(),
                value: srgb(0.0, 0.48, 1.0, 1.0),
            },
        ];
        let mut token_bindings = BTreeMap::new();
        token_bindings.insert(path(1, KEY_FILL), sid(TOKEN_SURFACE));
        token_bindings.insert(path(12, KEY_FILL), sid(TOKEN_ACCENT));
        Fixture {
            view: view(),
            variants: variants(),
            layout: layout(),
            accessibility: accessibility(),
            texts,
            states,
            tokens,
            token_bindings,
            catalog: builtin::catalog().expect("catalog"),
        }
    }

    /// 변환 입력.
    pub fn sources(&self) -> CodegenSources<'_> {
        CodegenSources {
            view: &self.view,
            capability_map: &KeyMap,
            variants: &self.variants,
            layout: &self.layout,
            accessibility: &self.accessibility,
            texts: &self.texts,
            states: &self.states,
            tokens: &self.tokens,
            token_bindings: &self.token_bindings,
        }
    }

    /// profile의 IR.
    pub fn ir(&self, profile: &TargetProfile) -> CodegenIr {
        build_ir(&self.sources(), profile, &self.catalog).expect("IR")
    }
}

/// root Frame(generic) 아래 Fragment(2) ─ Text 3 "숨길 장식"(접근성 entry 없음)과 Text 4 "보임"(staticText)을 둔
/// fixture입니다. Fragment 아래에서 시작하는 장식 subtree의 숨김을 검사합니다.
pub fn fragment_fixture() -> Fixture {
    let view = ViewModel {
        nodes: vec![
            view_node(1, None, ElementKind::FRAME, &[]),
            view_node(2, Some(1), ElementKind::FRAGMENT, &[]),
            view_node(3, Some(2), ElementKind::TEXT, &[]),
            view_node(4, Some(1), ElementKind::TEXT, &[]),
        ],
        ..ViewModel::default()
    };
    let layout = LayoutInput::new(
        sid(1),
        vec![
            LayoutNode::new(sid(1))
                .with_child(sid(3), key(1))
                .with_child(sid(4), key(2)),
            LayoutNode::new(sid(3)),
            LayoutNode::new(sid(4)),
        ],
    );
    let mut tree = AccessibilityTree::new();
    tree.insert(AccessibilityNode::new(sid(1), AccessibilityRole::Generic))
        .expect("root");
    tree.insert(
        AccessibilityNode::new(sid(4), AccessibilityRole::StaticText)
            .with_label(AccessibilityBinding::NodeText(sid(4))),
    )
    .expect("text");
    tree.push_root(sid(1));
    tree.push_child(sid(1), sid(4));
    let texts = [(3, "숨길 장식"), (4, "보임")]
        .into_iter()
        .map(|(id, value)| (sid(id), value.to_string()))
        .collect();
    Fixture {
        view,
        variants: VariantLayer::none(),
        layout,
        accessibility: AccessibilitySnapshot::from_tree(&tree).expect("snapshot"),
        texts,
        states: Vec::new(),
        tokens: Vec::new(),
        token_bindings: BTreeMap::new(),
        catalog: builtin::catalog().expect("catalog"),
    }
}

/// 마지막 4바이트가 `n`인 결정론 StableId(큰 fixture용).
pub fn sid32(n: u32) -> StableId {
    let mut bytes = [0u8; 16];
    bytes[12..].copy_from_slice(&n.to_be_bytes());
    StableId::from_bytes(bytes)
}

/// 큰 목록 fixture 항목 StableId의 시작값(상태 id와 겹치지 않음).
const LIST_ITEM_BASE: u32 = 0x0100_0000;

/// root Frame(세로 Stack, 이름 "목록"인 단일 선택 List) 아래 선택 가능한 ListItem Frame `items`개를 둔 fixture입니다.
/// 항목 selected는 상태 `STATE_SIZE`가 항목 StableId와 같은지이고 select 동작이 그 상태를 씁니다(adapter 시간 상한
/// 검사용).
pub fn list_fixture(items: u32) -> Fixture {
    let root = sid(1);
    let selection = sid(STATE_SIZE);
    let ids: Vec<StableId> = (0..items).map(|i| sid32(LIST_ITEM_BASE + i)).collect();
    let mut nodes = vec![view_node(1, None, ElementKind::FRAME, &[])];
    nodes.extend(ids.iter().map(|id| ViewNode {
        id: *id,
        parent: Some(root),
        element: ElementKind::FRAME,
        contracts: Vec::new(),
    }));
    let view = ViewModel {
        nodes,
        ..ViewModel::default()
    };
    let mut root_layout = LayoutNode::new(root)
        .with_container(ContainerLayout::Stack(StackLayout::new(Axis::Vertical)));
    for (order, id) in ids.iter().enumerate() {
        root_layout = root_layout.with_child(*id, key(order as u32 + 1));
    }
    let mut layout_nodes = vec![root_layout];
    layout_nodes.extend(ids.iter().map(|id| LayoutNode::new(*id)));
    let mut tree = AccessibilityTree::new();
    tree.insert(
        AccessibilityNode::new(root, AccessibilityRole::List)
            .with_label(AccessibilityBinding::text("목록")),
    )
    .expect("list");
    for id in &ids {
        tree.insert(
            AccessibilityNode::new(*id, AccessibilityRole::ListItem)
                .with_label(AccessibilityBinding::text("항목"))
                .with_state(
                    StateKind::Selected,
                    AccessibilityBinding::SelectionMatches {
                        selection,
                        item: *id,
                    },
                )
                .with_action(ActionKind::Select, ActionTarget::State(selection)),
        )
        .expect("item");
    }
    tree.push_root(root);
    tree.set_children(root, ids.clone());
    let initial = ids.first().map(ToString::to_string).unwrap_or_default();
    Fixture {
        view,
        variants: VariantLayer::none(),
        layout: LayoutInput::new(root, layout_nodes),
        accessibility: AccessibilitySnapshot::from_tree(&tree).expect("snapshot"),
        texts: BTreeMap::new(),
        states: vec![StateDecl {
            id: selection,
            name: "selection".to_string(),
            initial: TypedValue::String(initial),
        }],
        tokens: Vec::new(),
        token_bindings: BTreeMap::new(),
        catalog: builtin::catalog().expect("catalog"),
    }
}

/// `tests/fixtures/<name>` golden과 비교합니다. `STELLAR_UPDATE_SNAPSHOT_FIXTURES`가 있으면 먼저 갱신합니다.
pub fn check_golden(name: &str, actual: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    if std::env::var_os("STELLAR_UPDATE_SNAPSHOT_FIXTURES").is_some() {
        std::fs::write(&path, actual).expect("golden 갱신");
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "{name}이 golden과 다릅니다. 의도한 변경이면 STELLAR_UPDATE_SNAPSHOT_FIXTURES=1로 갱신하세요.\n{actual}"
    );
}

/// 값이 `FiniteF32`인 수.
pub fn number(value: f32) -> TypedValue {
    TypedValue::Number(FiniteF32::new(value).expect("유한"))
}
