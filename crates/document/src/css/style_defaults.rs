//! 기본값 세트(style defaults): 명시하지 않은 longhand의 값 (ADR-0082, ADR-0086).
//!
//! 원본에는 사용자가 명시한 longhand만 있습니다([`crate::layout_record`]). 명시하지 않은 longhand는 활성 기본값
//! 세트가 정합니다. 레이아웃, CLI·MCP 조회, codegen이 모두 [`resolve`] 하나로 값을 풉니다. 엔진은 풀린 값만
//! 입력으로 받습니다(ADR-0082 결정 1).
//!
//! - 내장 세트는 [`CSS`](`css`, 기본)와 [`REACT_NATIVE`](`react-native`) 두 개입니다. 표 원본은 이 파일의
//!   [`builtin_entries`]이고, 문서 표(`docs/spec/style-defaults.md`)는 [`snapshot_text`]와 같아야 합니다.
//! - 사용자 세트([`UserSetSource`])는 `extends`로 다른 세트를 물려받고 longhand 몇 개만 바꿉니다. 사슬은 내장
//!   세트에서 끝나야 하고, 순환·모르는 세트는 거부합니다([`flatten`]).
//! - 세트 값은 노드 종류 구획([`NodeClass`])마다 다를 수 있습니다. 구획 값이 없으면 공통 값을 씁니다.
//! - 세트는 초기값만 정합니다. 명시 값은 세트 값과 같아도 명시로 남습니다(결정 5).
//!
//! 이 표가 다루는 longhand는 명시·미설정 구분이 있는 layout longhand([`LayoutLonghand`])입니다. payload 속성
//! (`background-color`, 테두리, payload 크기 …)은 모델에서 언제나 값이 있는 명시 속성이라 세트가 끼어들지
//! 않습니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::property::PropertyKey;

use super::mapping::{is_known_longhand, MappingStatus};
use super::registry::{self, PropertyScope, RegistryEntry};
use super::value;
use crate::layout_record::{
    format_value, payload_size, LayoutLonghand, LayoutRecord, LayoutValue, LayoutValueError,
};
use crate::node_kind::NodeKind;
use crate::records::NodePayload;

/// `style_defaults` 키가 없을 때의 세트.
pub const DEFAULT_SET: &str = CSS;
/// 브라우저 기본값 세트 이름.
pub const CSS: &str = "css";
/// React Native(Yoga) 기본값 세트 이름.
pub const REACT_NATIVE: &str = "react-native";
/// 내장 세트 이름(표시 순).
pub const BUILTIN_SETS: [&str; 2] = [CSS, REACT_NATIVE];
/// `extends` 사슬 최대 길이(사용자 세트 수).
pub const MAX_EXTENDS_DEPTH: usize = 16;

/// 세트 값의 노드 종류 구획.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeClass {
    /// Frame과 ComponentInstance(정의 root Frame을 그립니다).
    Frame,
    /// Text.
    Text,
    /// Image.
    Image,
    /// Shape와 Path.
    Shape,
}

impl NodeClass {
    /// 모든 구획(표시 순).
    pub const ALL: [NodeClass; 4] = [
        NodeClass::Frame,
        NodeClass::Text,
        NodeClass::Image,
        NodeClass::Shape,
    ];

    /// 파일·표의 구획 이름.
    pub fn name(self) -> &'static str {
        match self {
            NodeClass::Frame => "frame",
            NodeClass::Text => "text",
            NodeClass::Image => "image",
            NodeClass::Shape => "shape",
        }
    }

    /// 이름에서 찾습니다.
    pub fn from_name(name: &str) -> Option<NodeClass> {
        NodeClass::ALL.into_iter().find(|c| c.name() == name)
    }

    /// node 종류의 구획. Fragment는 상자가 없어 `None`입니다.
    pub fn of(kind: NodeKind) -> Option<NodeClass> {
        match kind {
            NodeKind::Frame | NodeKind::ComponentInstance => Some(NodeClass::Frame),
            NodeKind::Text => Some(NodeClass::Text),
            NodeKind::Image => Some(NodeClass::Image),
            NodeKind::Shape | NodeKind::Path => Some(NodeClass::Shape),
            NodeKind::Fragment => None,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// 내장 세트 값의 확인 근거.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// CSS initial value(명세).
    CssInitial,
    /// HTML UA stylesheet(`div` 등).
    UserAgent,
    /// Stellar 모델 대응(브라우저 기본과 같은 결과를 내는 값으로 정함).
    StellarMapping,
    /// React Native 공식 문서의 기본값.
    ReactNativeDocs,
    /// Yoga 동작(자동 최소 크기가 없음 등).
    YogaBehavior,
    /// 저자 스타일이 없을 때 브라우저의 계산값(initial value가 계산 단계에서 다른 값이 됨).
    CssComputed,
    /// React Native 구현의 기본값(공식 문서 표에는 없음).
    ReactNativeSource,
}

impl Basis {
    /// 모든 근거(표시 순).
    pub const ALL: [Basis; 7] = [
        Basis::CssInitial,
        Basis::CssComputed,
        Basis::UserAgent,
        Basis::StellarMapping,
        Basis::ReactNativeDocs,
        Basis::ReactNativeSource,
        Basis::YogaBehavior,
    ];

    /// 표기 이름.
    pub fn name(self) -> &'static str {
        match self {
            Basis::CssInitial => "css-initial",
            Basis::CssComputed => "css-computed",
            Basis::UserAgent => "user-agent",
            Basis::StellarMapping => "stellar-mapping",
            Basis::ReactNativeDocs => "react-native-docs",
            Basis::ReactNativeSource => "react-native-source",
            Basis::YogaBehavior => "yoga-behavior",
        }
    }

    /// 이름에서 찾습니다.
    pub fn from_name(name: &str) -> Option<Basis> {
        Basis::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// 내장 세트 표 한 줄.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinEntry {
    /// 세트 이름.
    pub set: &'static str,
    /// 구획(`None`이면 모든 구획).
    pub class: Option<NodeClass>,
    /// longhand.
    pub longhand: LayoutLonghand,
    /// 정규 CSS 값 텍스트.
    pub value: &'static str,
    /// 확인 근거.
    pub basis: Basis,
}

const fn entry(
    set: &'static str,
    class: Option<NodeClass>,
    longhand: LayoutLonghand,
    value: &'static str,
    basis: Basis,
) -> BuiltinEntry {
    BuiltinEntry {
        set,
        class,
        longhand,
        value,
        basis,
    }
}

/// 내장 세트 표. `css`는 모든 longhand의 공통 값을 갖고, `react-native`는 `css`를 물려받아 다른 값만 둡니다.
/// 표에 없는 `react-native` 값은 `css` 값을 그대로 물려받습니다(확인하지 못한 항목 포함, ADR-0082 결정 2).
pub fn builtin_entries() -> &'static [BuiltinEntry] {
    use Basis::*;
    use LayoutLonghand as L;
    const C: &str = CSS;
    const R: &str = REACT_NATIVE;
    const FRAME: Option<NodeClass> = Some(NodeClass::Frame);
    const TABLE: &[BuiltinEntry] = &[
        entry(C, None, L::Position, "static", CssInitial),
        entry(C, None, L::BoxSizing, "content-box", CssInitial),
        entry(C, None, L::Top, "auto", CssInitial),
        entry(C, None, L::Right, "auto", CssInitial),
        entry(C, None, L::Bottom, "auto", CssInitial),
        entry(C, None, L::Left, "auto", CssInitial),
        entry(C, None, L::Width, "auto", CssInitial),
        entry(C, None, L::Height, "auto", CssInitial),
        entry(C, None, L::MinWidth, "auto", CssInitial),
        entry(C, None, L::MinHeight, "auto", CssInitial),
        entry(C, None, L::MaxWidth, "none", CssInitial),
        entry(C, None, L::MaxHeight, "none", CssInitial),
        entry(C, None, L::AspectRatio, "auto", CssInitial),
        entry(C, None, L::FlexGrow, "0", CssInitial),
        entry(C, None, L::FlexShrink, "1", CssInitial),
        entry(C, None, L::FlexBasis, "auto", CssInitial),
        entry(C, None, L::AlignSelf, "auto", CssInitial),
        entry(C, None, L::JustifySelf, "auto", CssInitial),
        entry(C, None, L::GridColumnStart, "auto", CssInitial),
        entry(C, None, L::GridColumnEnd, "auto", CssInitial),
        entry(C, None, L::GridRowStart, "auto", CssInitial),
        entry(C, None, L::GridRowEnd, "auto", CssInitial),
        // CSS initial은 `inline`이지만 Stellar에는 inline formatting context가 없고 Frame은 `div`입니다(ADR-0076
        // 결정 3). Text·Image·Shape도 상자 하나로 배치하므로 같은 값을 씁니다.
        entry(C, None, L::Display, "block", UserAgent),
        entry(C, None, L::PaddingTop, "0px", CssInitial),
        entry(C, None, L::PaddingRight, "0px", CssInitial),
        entry(C, None, L::PaddingBottom, "0px", CssInitial),
        entry(C, None, L::PaddingLeft, "0px", CssInitial),
        entry(C, None, L::FlexDirection, "row", CssInitial),
        entry(C, None, L::FlexWrap, "nowrap", CssInitial),
        entry(C, None, L::JustifyContent, "normal", CssInitial),
        entry(C, None, L::AlignContent, "normal", CssInitial),
        entry(C, None, L::AlignItems, "normal", CssInitial),
        // CSS initial은 `legacy`입니다. `legacy`는 정렬 keyword 없이 쓰면 `normal`과 같습니다.
        entry(C, None, L::JustifyItems, "normal", StellarMapping),
        entry(C, None, L::RowGap, "normal", CssInitial),
        entry(C, None, L::ColumnGap, "normal", CssInitial),
        entry(C, None, L::GridTemplateColumns, "none", CssInitial),
        entry(C, None, L::GridTemplateRows, "none", CssInitial),
        entry(C, None, L::GridAutoColumns, "auto", CssInitial),
        entry(C, None, L::GridAutoRows, "auto", CssInitial),
        entry(C, None, L::GridAutoFlow, "row", CssInitial),
        // React Native(Yoga): View·Text·Image 모두.
        entry(R, None, L::Position, "relative", ReactNativeDocs),
        entry(R, None, L::BoxSizing, "border-box", ReactNativeDocs),
        entry(R, None, L::FlexShrink, "0", ReactNativeDocs),
        entry(R, None, L::MinWidth, "0px", YogaBehavior),
        entry(R, None, L::MinHeight, "0px", YogaBehavior),
        // View(Frame): flex container, 세로 main 축.
        entry(R, FRAME, L::Display, "flex", ReactNativeDocs),
        entry(R, FRAME, L::FlexDirection, "column", ReactNativeDocs),
        entry(R, FRAME, L::AlignContent, "flex-start", ReactNativeDocs),
    ];
    TABLE
}

// --- payload longhand 기본값 (IX.11.24, ADR-0115 결정 1) ---

/// 기본값 세트가 값을 두는 payload longhand(registry 항목) 목록(registry 순서).
///
/// registry([`registry::entries`])에서 다음 조건을 모두 만족하는 항목입니다. [`LayoutLonghand::ALL`]과 합치면
/// Stellar가 node에 저장하는 CSS longhand 전체입니다. CSS longhand 하나가 아닌 Stellar 단일 값(`visible`, Frame
/// `clipping`, Path `path-data`)은 빠집니다(ADR-0115 결정 1).
/// - 범위가 node 종류 payload(Frame·Shape·Path·Text·Image)입니다. 공통 필드(`0x00xx`)와 modifier(`0x10xx`)는
///   longhand가 아닙니다. modifier는 stack 구조이고, 없으면 효과가 없습니다.
/// - 묶음 key가 아니고 매핑 상태가 `stellar-only`가 아니며, 이름이 그 항목의 CSS longhand 이름입니다.
/// - 이름이 layout longhand가 아닙니다. payload 크기 `width`·`height`는 layout 표의 값을 씁니다.
pub fn payload_longhands() -> Vec<RegistryEntry> {
    registry::entries()
        .into_iter()
        .filter(|e| scope_class(e.scope).is_some())
        .filter(|e| e.bundle.is_empty() && e.css.status != MappingStatus::StellarOnly)
        .filter(|e| e.css.longhands.contains(&e.name) && is_known_longhand(e.name))
        .filter(|e| LayoutLonghand::from_name(e.name).is_none())
        .collect()
}

/// registry 범위의 구획. payload 범위가 아니면 `None`입니다.
pub fn scope_class(scope: PropertyScope) -> Option<NodeClass> {
    match scope {
        PropertyScope::Frame => Some(NodeClass::Frame),
        PropertyScope::Shape | PropertyScope::Path => Some(NodeClass::Shape),
        PropertyScope::Text => Some(NodeClass::Text),
        PropertyScope::Image => Some(NodeClass::Image),
        PropertyScope::Node | PropertyScope::Modifier => None,
    }
}

/// 내장 세트의 payload longhand 표 한 줄. 구획은 registry 범위가 정합니다([`scope_class`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadEntry {
    /// 세트 이름.
    pub set: &'static str,
    /// registry 범위.
    pub scope: PropertyScope,
    /// registry key.
    pub key: PropertyKey,
    /// 정규 CSS 값 텍스트(registry 값 문법 formatter의 출력과 같음).
    pub value: &'static str,
    /// 확인 근거.
    pub basis: Basis,
}

const fn payload(
    set: &'static str,
    scope: PropertyScope,
    key: PropertyKey,
    value: &'static str,
    basis: Basis,
) -> PayloadEntry {
    PayloadEntry {
        set,
        scope,
        key,
        value,
        basis,
    }
}

/// 내장 세트의 payload longhand 표. `css`는 [`payload_longhands`] 전부의 값을 갖고, `react-native`는 다른 값만
/// 둡니다. 표에 없는 `react-native` 값은 `css` 값을 물려받습니다(ADR-0082 결정 2).
///
/// payload 값은 지금 모델에서 언제나 명시 값이라 [`resolve`]는 이 표를 읽지 않습니다. 이 표는 payload를 희소
/// 저장으로 바꾸는 schema step, codegen 차이 출력, 조회가 같은 값을 쓰도록 registry 항목마다 한 곳에 둡니다.
pub fn payload_entries() -> &'static [PayloadEntry] {
    use crate::css::border::keys as border;
    use crate::css::typography::keys as typo;
    use crate::frame::keys as frame;
    use crate::payload_metadata::{image_keys as image, path_keys as path, shape_keys as shape};
    use Basis::*;
    use PropertyScope::{Frame, Image, Path, Shape, Text};
    const C: &str = CSS;
    const R: &str = REACT_NATIVE;
    const TABLE: &[PayloadEntry] = &[
        payload(C, Frame, frame::TRANSFORM, "none", CssInitial),
        // `transparent`의 정규 표기(속성 모델 8.3절)입니다.
        payload(C, Frame, frame::BACKGROUND, "#00000000", CssInitial),
        payload(C, Frame, frame::BORDER_TOP_LEFT_RADIUS, "0px", CssInitial),
        payload(C, Frame, frame::BORDER_TOP_RIGHT_RADIUS, "0px", CssInitial),
        payload(
            C,
            Frame,
            frame::BORDER_BOTTOM_RIGHT_RADIUS,
            "0px",
            CssInitial,
        ),
        payload(
            C,
            Frame,
            frame::BORDER_BOTTOM_LEFT_RADIUS,
            "0px",
            CssInitial,
        ),
        // `border-*-width` initial은 `medium`이지만 style이 `none`이면 계산값이 0입니다.
        payload(C, Frame, border::BORDER_TOP_WIDTH, "0px", CssComputed),
        payload(C, Frame, border::BORDER_TOP_STYLE, "none", CssInitial),
        // `border-*-color` initial은 `currentcolor`이고, 저자 스타일이 없으면 `color` 계산값은 검정입니다.
        payload(C, Frame, border::BORDER_TOP_COLOR, "#000000", CssComputed),
        payload(C, Frame, border::BORDER_RIGHT_WIDTH, "0px", CssComputed),
        payload(C, Frame, border::BORDER_RIGHT_STYLE, "none", CssInitial),
        payload(C, Frame, border::BORDER_RIGHT_COLOR, "#000000", CssComputed),
        payload(C, Frame, border::BORDER_BOTTOM_WIDTH, "0px", CssComputed),
        payload(C, Frame, border::BORDER_BOTTOM_STYLE, "none", CssInitial),
        payload(
            C,
            Frame,
            border::BORDER_BOTTOM_COLOR,
            "#000000",
            CssComputed,
        ),
        payload(C, Frame, border::BORDER_LEFT_WIDTH, "0px", CssComputed),
        payload(C, Frame, border::BORDER_LEFT_STYLE, "none", CssInitial),
        payload(C, Frame, border::BORDER_LEFT_COLOR, "#000000", CssComputed),
        payload(C, Shape, shape::BORDER_TOP_LEFT_RADIUS, "0px", CssInitial),
        payload(C, Shape, shape::BORDER_TOP_RIGHT_RADIUS, "0px", CssInitial),
        payload(
            C,
            Shape,
            shape::BORDER_BOTTOM_RIGHT_RADIUS,
            "0px",
            CssInitial,
        ),
        payload(
            C,
            Shape,
            shape::BORDER_BOTTOM_LEFT_RADIUS,
            "0px",
            CssInitial,
        ),
        payload(C, Shape, shape::STROKE_LINECAP, "butt", CssInitial),
        payload(C, Path, path::FILL_RULE, "nonzero", CssInitial),
        // `font-family` initial은 UA가 정합니다. 기본 표준 글꼴은 serif 계열입니다.
        payload(C, Text, typo::FONT_FAMILY, "serif", UserAgent),
        // `font-size` initial `medium`은 UA 기본 16px입니다.
        payload(C, Text, typo::FONT_SIZE, "16px", UserAgent),
        payload(C, Text, typo::FONT_WEIGHT, "400", CssInitial),
        payload(C, Text, typo::FONT_STYLE, "normal", CssInitial),
        payload(C, Text, typo::FONT_STRETCH, "100%", CssInitial),
        payload(C, Text, typo::LINE_HEIGHT, "normal", CssInitial),
        // `letter-spacing: normal`은 0으로 계산합니다(CSS Text 3).
        payload(C, Text, typo::LETTER_SPACING, "0px", CssComputed),
        payload(C, Image, image::OBJECT_FIT, "fill", CssInitial),
        payload(C, Image, image::IMAGE_RENDERING, "auto", CssInitial),
        // React Native View: 테두리 style 기본이 실선이라 `borderWidth`만 주어도 그립니다.
        payload(
            R,
            Frame,
            border::BORDER_TOP_STYLE,
            "solid",
            ReactNativeSource,
        ),
        payload(
            R,
            Frame,
            border::BORDER_RIGHT_STYLE,
            "solid",
            ReactNativeSource,
        ),
        payload(
            R,
            Frame,
            border::BORDER_BOTTOM_STYLE,
            "solid",
            ReactNativeSource,
        ),
        payload(
            R,
            Frame,
            border::BORDER_LEFT_STYLE,
            "solid",
            ReactNativeSource,
        ),
        // React Native Text: 플랫폼 시스템 글꼴, 기본 글자 크기 14.
        payload(R, Text, typo::FONT_FAMILY, "system-ui", ReactNativeSource),
        payload(R, Text, typo::FONT_SIZE, "14px", ReactNativeSource),
        // React Native Image: `resizeMode` 기본값 `cover`.
        payload(R, Image, image::OBJECT_FIT, "cover", ReactNativeDocs),
    ];
    TABLE
}

/// 세트 사슬(자식 먼저)의 내장 세트에서 payload longhand 값을 찾습니다. 사용자 세트는 payload longhand를 두지
/// 않습니다(ADR-0115 결정 1).
fn chain_payload_value(
    chain: &[String],
    scope: PropertyScope,
    key: PropertyKey,
) -> Option<&'static str> {
    chain
        .iter()
        .filter(|link| is_builtin(link))
        .find_map(|link| {
            payload_entries()
                .iter()
                .find(|e| e.set == link.as_str() && e.scope == scope && e.key == key)
                .map(|e| e.value)
        })
}

/// payload longhand 값이 registry 값 문법으로 읽히는지와 정규 텍스트인지 봅니다. 문법 parser가 받지 않는 값은
/// registry 매핑의 값 변형 규칙(`line-height: normal` 등)에 같은 선언이 있을 때만 받습니다.
pub fn check_payload_value(entry: &RegistryEntry, value: &str) -> Result<(), String> {
    let declaration = format!("{}: {value}", entry.name);
    if entry.css.rules.iter().any(|r| r.css == declaration) {
        return Ok(());
    }
    let grammar = entry
        .css
        .grammar
        .ok_or_else(|| format!("{}에 값 문법이 없습니다", entry.name))?;
    let parsed = value::parse(grammar, value).map_err(|e| format!("{}: {e}", entry.name))?;
    let canonical = value::format(&parsed, grammar);
    if canonical == value {
        Ok(())
    } else {
        Err(format!(
            "{}: {value}의 정규 표기는 {canonical}입니다",
            entry.name
        ))
    }
}

/// 내장 세트의 부모(`react-native` → `css`). `css`는 뿌리입니다.
fn builtin_parent(name: &str) -> Option<&'static str> {
    match name {
        REACT_NATIVE => Some(CSS),
        _ => None,
    }
}

/// 내장 세트인지.
pub fn is_builtin(name: &str) -> bool {
    BUILTIN_SETS.contains(&name)
}

/// 사용자 세트 원본(파일 `design/style-defaults/<name>.stellar.yaml`의 의미 모델).
#[derive(Debug, Clone, PartialEq)]
pub struct UserSetSource {
    /// 세트 이름(파일 이름 stem).
    pub name: String,
    /// 물려받는 세트.
    pub extends: String,
    /// 모든 구획 공통 값.
    pub common: Vec<(LayoutLonghand, LayoutValue)>,
    /// 구획별 값.
    pub sections: Vec<(NodeClass, Vec<(LayoutLonghand, LayoutValue)>)>,
}

/// 기본값 세트 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleDefaultsError {
    /// 세트가 없습니다.
    UnknownSet {
        /// 이름.
        name: String,
    },
    /// `extends` 사슬이 순환합니다.
    Cycle {
        /// 사슬(첫 이름으로 다시 돌아옴).
        chain: Vec<String>,
    },
    /// `extends` 사슬이 너무 깁니다.
    TooDeep {
        /// 시작 세트.
        name: String,
    },
    /// 사용자 세트가 내장 세트 이름을 씁니다.
    ReservedName {
        /// 이름.
        name: String,
    },
    /// 세트 이름 형식이 아닙니다.
    InvalidName {
        /// 이름.
        name: String,
    },
    /// 같은 longhand를 한 구획에 두 번 적었습니다.
    DuplicateLonghand {
        /// 세트.
        set: String,
        /// longhand.
        longhand: LayoutLonghand,
    },
    /// 모르는 longhand 이름.
    UnknownLonghand {
        /// 세트.
        set: String,
        /// 이름.
        name: String,
    },
    /// 값이 longhand 문법에 맞지 않습니다(`var()` 포함).
    InvalidValue {
        /// 세트.
        set: String,
        /// longhand.
        longhand: LayoutLonghand,
        /// 원인.
        error: LayoutValueError,
    },
}

impl fmt::Display for StyleDefaultsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StyleDefaultsError::UnknownSet { name } => write!(f, "기본값 세트 {name}이 없습니다"),
            StyleDefaultsError::Cycle { chain } => {
                write!(f, "extends 사슬이 순환합니다: {}", chain.join(" → "))
            }
            StyleDefaultsError::TooDeep { name } => write!(
                f,
                "{name}의 extends 사슬이 {MAX_EXTENDS_DEPTH}단계를 넘습니다"
            ),
            StyleDefaultsError::ReservedName { name } => {
                write!(
                    f,
                    "{name}은 내장 세트 이름이라 사용자 세트에 쓸 수 없습니다"
                )
            }
            StyleDefaultsError::InvalidName { name } => write!(
                f,
                "세트 이름 {name}은 소문자·숫자·하이픈(첫 글자 소문자, 64자 이하)이어야 합니다"
            ),
            StyleDefaultsError::DuplicateLonghand { set, longhand } => {
                write!(f, "세트 {set}에 {longhand}가 두 번 있습니다")
            }
            StyleDefaultsError::UnknownLonghand { set, name } => {
                write!(
                    f,
                    "세트 {set}: {name}은 기본값을 둘 수 있는 longhand가 아닙니다"
                )
            }
            StyleDefaultsError::InvalidValue {
                set,
                longhand,
                error,
            } => write!(f, "세트 {set}의 {longhand}: {error}"),
        }
    }
}

impl std::error::Error for StyleDefaultsError {}

impl StyleDefaultsError {
    /// 안정 진단 code.
    pub fn code(&self) -> &'static str {
        match self {
            StyleDefaultsError::UnknownSet { .. } => "unknown-style-defaults",
            StyleDefaultsError::Cycle { .. } => "style-defaults-cycle",
            StyleDefaultsError::TooDeep { .. } => "style-defaults-too-deep",
            StyleDefaultsError::ReservedName { .. } => "style-defaults-reserved-name",
            StyleDefaultsError::InvalidName { .. } => "style-defaults-invalid-name",
            StyleDefaultsError::DuplicateLonghand { .. } => "style-defaults-duplicate-longhand",
            StyleDefaultsError::UnknownLonghand { .. } => "style-defaults-unknown-longhand",
            StyleDefaultsError::InvalidValue { .. } => "style-defaults-invalid-value",
        }
    }
}

/// 세트 이름 형식: `[a-z][a-z0-9-]{0,63}`.
pub fn is_valid_set_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// `extends` 사슬을 풀어 고정 배열로 평탄화한 세트.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatSet {
    name: String,
    chain: Vec<String>,
    /// [구획][longhand 순번] 값.
    values: Vec<Vec<LayoutValue>>,
}

fn longhand_index(longhand: LayoutLonghand) -> usize {
    longhand.index()
}

impl FlatSet {
    /// 세트 이름.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 자기부터 뿌리(`css`)까지의 사슬.
    pub fn chain(&self) -> &[String] {
        &self.chain
    }

    /// `class` 구획의 `longhand` 기본값.
    pub fn value(&self, class: NodeClass, longhand: LayoutLonghand) -> &LayoutValue {
        &self.values[class.index()][longhand_index(longhand)]
    }

    /// payload longhand(registry `scope`·`key`)의 기본값 텍스트. [`payload_longhands`]에 없는 항목은 `None`입니다.
    /// 사용자 세트는 payload longhand를 두지 않으므로 사슬의 내장 세트 값입니다.
    pub fn payload_value(&self, scope: PropertyScope, key: PropertyKey) -> Option<&'static str> {
        chain_payload_value(&self.chain, scope, key)
    }

    fn apply(&mut self, class: Option<NodeClass>, longhand: LayoutLonghand, value: LayoutValue) {
        let index = longhand_index(longhand);
        match class {
            Some(class) => self.values[class.index()][index] = value,
            None => {
                for row in &mut self.values {
                    row[index] = value.clone();
                }
            }
        }
    }
}

fn builtin_value(entry: &BuiltinEntry) -> LayoutValue {
    entry
        .longhand
        .parse(entry.value)
        .unwrap_or_else(|e| panic!("내장 세트 {} {}: {e}", entry.set, entry.longhand))
}

fn flatten_builtin(name: &str) -> FlatSet {
    let mut chain = vec![name.to_string()];
    let mut parent = builtin_parent(name);
    while let Some(p) = parent {
        chain.push(p.to_string());
        parent = builtin_parent(p);
    }
    let placeholder = LayoutValue::Keyword("auto");
    let mut set = FlatSet {
        name: name.to_string(),
        chain: chain.clone(),
        values: vec![vec![placeholder; LayoutLonghand::ALL.len()]; NodeClass::ALL.len()],
    };
    // 뿌리부터 적용합니다.
    for link in chain.iter().rev() {
        for entry in builtin_entries().iter().filter(|e| e.set == link) {
            set.apply(entry.class, entry.longhand, builtin_value(entry));
        }
    }
    set
}

/// 내장 세트를 평탄화합니다.
pub fn builtin(name: &str) -> Option<FlatSet> {
    is_builtin(name).then(|| flatten_builtin(name))
}

/// 사용자 세트 하나를 검사합니다(이름, longhand 중복). 값 문법은 원본을 읽을 때 이미 검사됩니다.
pub fn check_source(source: &UserSetSource) -> Result<(), StyleDefaultsError> {
    if is_builtin(&source.name) {
        return Err(StyleDefaultsError::ReservedName {
            name: source.name.clone(),
        });
    }
    if !is_valid_set_name(&source.name) {
        return Err(StyleDefaultsError::InvalidName {
            name: source.name.clone(),
        });
    }
    let groups = std::iter::once(&source.common).chain(source.sections.iter().map(|(_, v)| v));
    for group in groups {
        let mut seen = std::collections::BTreeSet::new();
        for (longhand, value) in group {
            if !seen.insert(*longhand) {
                return Err(StyleDefaultsError::DuplicateLonghand {
                    set: source.name.clone(),
                    longhand: *longhand,
                });
            }
            if !longhand.accepts(value) {
                return Err(StyleDefaultsError::InvalidValue {
                    set: source.name.clone(),
                    longhand: *longhand,
                    error: LayoutValueError {
                        code: "invalid-value",
                        message: format!("{value}은 정규 값이 아닙니다"),
                    },
                });
            }
        }
    }
    Ok(())
}

/// `name` 세트를 평탄화합니다. `user`는 사용자 세트 원본을 이름으로 찾습니다.
pub fn flatten<'a>(
    name: &str,
    user: impl Fn(&str) -> Option<&'a UserSetSource>,
) -> Result<FlatSet, StyleDefaultsError> {
    let mut sources: Vec<&UserSetSource> = Vec::new();
    let mut current = name.to_string();
    let mut visited: Vec<String> = Vec::new();
    loop {
        if is_builtin(&current) {
            break;
        }
        if visited.contains(&current) {
            visited.push(current);
            return Err(StyleDefaultsError::Cycle { chain: visited });
        }
        if sources.len() >= MAX_EXTENDS_DEPTH {
            return Err(StyleDefaultsError::TooDeep {
                name: name.to_string(),
            });
        }
        let source = user(&current).ok_or_else(|| StyleDefaultsError::UnknownSet {
            name: current.clone(),
        })?;
        check_source(source)?;
        visited.push(current.clone());
        sources.push(source);
        current = source.extends.clone();
    }
    let mut set = flatten_builtin(&current);
    let mut chain: Vec<String> = sources.iter().map(|s| s.name.clone()).collect();
    chain.append(&mut set.chain);
    for source in sources.iter().rev() {
        for (longhand, value) in &source.common {
            set.apply(None, *longhand, value.clone());
        }
        for (class, values) in &source.sections {
            for (longhand, value) in values {
                set.apply(Some(*class), *longhand, value.clone());
            }
        }
    }
    set.name = name.to_string();
    set.chain = chain;
    Ok(set)
}

/// 값의 출처(ADR-0082 결정 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueSource {
    /// 사용자가 명시한 값(layout record 또는 payload).
    Explicit,
    /// 활성 기본값 세트의 값.
    Default {
        /// 세트 이름.
        set: String,
    },
}

impl fmt::Display for ValueSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueSource::Explicit => f.write_str("explicit"),
            ValueSource::Default { set } => write!(f, "default({set})"),
        }
    }
}

/// 풀린 longhand 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedValue {
    /// longhand.
    pub longhand: LayoutLonghand,
    /// 값.
    pub value: LayoutValue,
    /// 출처.
    pub source: ValueSource,
}

/// node 하나의 풀린 layout longhand 전체([`LayoutLonghand::ALL`] 순서).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLayout {
    /// 구획.
    pub class: NodeClass,
    /// 값(모든 longhand).
    pub values: Vec<ResolvedValue>,
}

impl ResolvedLayout {
    /// longhand 값.
    pub fn get(&self, longhand: LayoutLonghand) -> &ResolvedValue {
        &self.values[longhand_index(longhand)]
    }

    /// 엔진 입력용 `(longhand 이름, 정규 CSS 값)` 목록. 모든 longhand가 한 번씩 나옵니다.
    pub fn declarations(&self) -> Vec<(&'static str, String)> {
        self.values
            .iter()
            .map(|v| (v.longhand.name(), format_value(&v.value)))
            .collect()
    }
}

/// node의 layout longhand를 모두 풉니다(ADR-0082 결정 1). 명시 값(layout record, payload 크기)이 있으면 그
/// 값이고 출처는 `explicit`, 없으면 `set`의 구획 값이고 출처는 `default(<세트>)`입니다. node 종류에 붙지 않는
/// longhand(Frame이 아닌 node의 container longhand)는 언제나 세트 값입니다. Fragment는 상자가 없어 `None`입니다.
pub fn resolve(
    set: &FlatSet,
    payload: &NodePayload,
    record: Option<&LayoutRecord>,
) -> Option<ResolvedLayout> {
    resolve_kind(set, payload.kind(), record, payload_size(payload))
}

/// [`resolve`]의 node 종류·payload 크기 판. `payload_size`는 payload 크기가 `width`·`height` 저장 위치인 종류의
/// 크기(px)입니다. payload 크기를 희소하게 바꾸는 schema step 전에도 같은 해석을 쓰도록 나눠 둡니다.
pub fn resolve_kind(
    set: &FlatSet,
    kind: NodeKind,
    record: Option<&LayoutRecord>,
    payload_size: Option<(f32, f32)>,
) -> Option<ResolvedLayout> {
    let class = NodeClass::of(kind)?;
    let size = payload_size;
    let values = LayoutLonghand::ALL
        .into_iter()
        .map(|longhand| {
            let payload_value = match (longhand, size) {
                (LayoutLonghand::Width, Some((w, _))) => Some(LayoutValue::Length(w + 0.0)),
                (LayoutLonghand::Height, Some((_, h))) => Some(LayoutValue::Length(h + 0.0)),
                _ => None,
            };
            let explicit = payload_value.or_else(|| {
                record
                    .filter(|_| longhand.applies_to(kind))
                    .and_then(|r| r.get(longhand).cloned())
            });
            match explicit {
                Some(value) => ResolvedValue {
                    longhand,
                    value,
                    source: ValueSource::Explicit,
                },
                None => ResolvedValue {
                    longhand,
                    value: set.value(class, longhand).clone(),
                    source: ValueSource::Default {
                        set: set.name.clone(),
                    },
                },
            }
        })
        .collect();
    Some(ResolvedLayout { class, values })
}

/// `active` 세트가 `target` 세트(대상 프레임워크 고유 기본값)와 다른 longhand와 `active` 값(구획별, longhand
/// 순서). codegen이 차이만 명시 선언으로 내는 데 씁니다(ADR-0082 결정 6). 두 세트가 같으면 빈 목록입니다.
pub fn differences(
    active: &FlatSet,
    target: &FlatSet,
    class: NodeClass,
) -> Vec<(LayoutLonghand, LayoutValue)> {
    LayoutLonghand::ALL
        .into_iter()
        .filter(|l| active.value(class, *l) != target.value(class, *l))
        .map(|l| (l, active.value(class, l).clone()))
        .collect()
}

/// 내장 세트 표 전체의 결정론적 텍스트(`docs/spec/style-defaults.md` 표와 비교).
///
/// 한 줄은 `| longhand | css | react-native |`이고 값은 `frame/text/image/shape` 구획 값이 같으면 하나, 다르면
/// `구획=값` 목록입니다. `react-native` 값이 `css`와 같으면 `=`(물려받음)입니다.
pub fn snapshot_text() -> String {
    let css = flatten_builtin(CSS);
    let rn = flatten_builtin(REACT_NATIVE);
    let cell = |set: &FlatSet, longhand: LayoutLonghand| -> String {
        let values: Vec<String> = NodeClass::ALL
            .iter()
            .map(|c| format_value(set.value(*c, longhand)))
            .collect();
        if values.iter().all(|v| *v == values[0]) {
            format!("`{}`", values[0])
        } else {
            NodeClass::ALL
                .iter()
                .zip(&values)
                .map(|(c, v)| format!("{}=`{v}`", c.name()))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    let mut out = String::from("| longhand | css | react-native |\n|---|---|---|\n");
    for longhand in LayoutLonghand::ALL {
        let css_cell = cell(&css, longhand);
        let rn_cell = cell(&rn, longhand);
        let rn_cell = if rn_cell == css_cell {
            "=".to_string()
        } else {
            rn_cell
        };
        out.push_str(&format!(
            "| `{}` | {css_cell} | {rn_cell} |\n",
            longhand.name()
        ));
    }
    out
}

/// 내장 세트 payload longhand 표의 결정론적 텍스트(`docs/spec/style-defaults.md` 2.4절 표와 비교).
///
/// 한 줄은 `| 구획 | registry key | longhand | css | react-native |`이고 registry 순서입니다. `react-native` 값이
/// `css`와 같으면 `=`(물려받음)입니다.
pub fn payload_snapshot_text() -> String {
    let css = flatten_builtin(CSS);
    let rn = flatten_builtin(REACT_NATIVE);
    let mut out =
        String::from("| 구획 | key | longhand | css | react-native |\n|---|---|---|---|---|\n");
    for entry in payload_longhands() {
        let class = scope_class(entry.scope).expect("payload 범위");
        let css_value = css.payload_value(entry.scope, entry.key).unwrap_or("?");
        let rn_value = rn.payload_value(entry.scope, entry.key).unwrap_or("?");
        let rn_cell = if rn_value == css_value {
            "=".to_string()
        } else {
            format!("`{rn_value}`")
        };
        out.push_str(&format!(
            "| {} | `0x{:04x}` | `{}` | `{css_value}` | {rn_cell} |\n",
            class.name(),
            entry.key.0,
            entry.name
        ));
    }
    out
}

/// 내장 세트 conformance fixture(IX.11.31, ADR-0115 결정 2). 형식은 파일 머리 주석에 있습니다.
pub const CONFORMANCE_FIXTURE: &str = include_str!("style_defaults_conformance.txt");

/// css 줄 출처 접두사. `chrome:`만 관측값과 기대값이 같고, 나머지는 다른 이유입니다.
const CSS_SOURCES: [&str; 4] = [
    "chrome:",
    "stellar-mapping:",
    "css-computed:",
    "user-agent:",
];
/// react-native 줄 출처 접두사.
const RN_SOURCES: [&str; 4] = ["rn-docs:", "rn-source:", "yoga:", "inherits:"];

/// 브라우저 계산값 표기(`rgb(0, 0, 0)`, `rgba(0, 0, 0, 0)`)를 registry 색 정규 표기로 바꿉니다. 색 해석·표기는
/// registry 값 문법과 같은 [`value::parse_color`]·[`value::format_color`]입니다. 색이 아니면 그대로입니다.
fn normalize_observed(observed: &str) -> String {
    if !(observed.starts_with("rgb(") || observed.starts_with("rgba(")) {
        return observed.to_string();
    }
    value::parse_color(observed)
        .map(value::format_color)
        .unwrap_or_else(|_| observed.to_string())
}

/// `fixture`를 `css`·`react-native` 평탄화 세트와 비교해 어긋난 점을 돌려줍니다. 빈 목록이면 표와 fixture가
/// 같습니다. 모든 (세트, 구획, longhand)가 정확히 한 번 나와야 하고, 줄마다 출처가 있어야 합니다.
pub fn conformance_mismatches(fixture: &str, css: &FlatSet, rn: &FlatSet) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let payload = payload_longhands();
    for (number, line) in fixture.lines().enumerate() {
        let line_no = number + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [set_name, class_name, longhand, expected, observed, source] = fields[..] else {
            problems.push(format!("{line_no}: 칸이 여섯 개가 아닙니다"));
            continue;
        };
        let (set, prefixes) = match set_name {
            CSS => (css, &CSS_SOURCES),
            REACT_NATIVE => (rn, &RN_SOURCES),
            other => {
                problems.push(format!("{line_no}: 모르는 세트 {other}"));
                continue;
            }
        };
        let Some(class) = NodeClass::from_name(class_name) else {
            problems.push(format!("{line_no}: 모르는 구획 {class_name}"));
            continue;
        };
        let table = |set: &FlatSet| -> Option<String> {
            match LayoutLonghand::from_name(longhand) {
                Some(l) => Some(format_value(set.value(class, l))),
                None => payload
                    .iter()
                    .find(|e| e.name == longhand && scope_class(e.scope) == Some(class))
                    .and_then(|e| set.payload_value(e.scope, e.key))
                    .map(str::to_string),
            }
        };
        let Some(value) = table(set) else {
            problems.push(format!(
                "{line_no}: {class_name} 구획에 {longhand} longhand가 없습니다"
            ));
            continue;
        };
        if !seen.insert((set_name, class, longhand.to_string())) {
            problems.push(format!(
                "{line_no}: {set_name} {class_name} {longhand}가 두 번 있습니다"
            ));
        }
        if value != expected {
            problems.push(format!(
                "{line_no}: {set_name} {class_name} {longhand} 표 값 {value}, fixture 기대값 {expected}"
            ));
        }
        let Some(prefix) = prefixes.iter().find(|p| source.starts_with(**p)) else {
            problems.push(format!(
                "{line_no}: 출처가 없거나 접두사가 맞지 않습니다: {source}"
            ));
            continue;
        };
        if source.len() <= prefix.len() + 1 {
            problems.push(format!("{line_no}: 출처 설명이 비어 있습니다"));
        }
        if set_name == CSS {
            if observed == "-" || observed.is_empty() {
                problems.push(format!("{line_no}: css 줄에 관측값이 없습니다"));
            }
            let same = normalize_observed(observed) == expected;
            if same != (*prefix == "chrome:") {
                problems.push(format!(
                    "{line_no}: 관측값 {observed}과 기대값 {expected}의 차이와 출처 {prefix}가 맞지 않습니다"
                ));
            }
        } else {
            if observed != "-" {
                problems.push(format!("{line_no}: react-native 줄의 관측값은 -입니다"));
            }
            if *prefix == "inherits:" && table(css).as_deref() != Some(expected) {
                problems.push(format!(
                    "{line_no}: inherits 줄의 기대값이 css 값과 다릅니다"
                ));
            }
        }
    }
    for set_name in BUILTIN_SETS {
        for class in NodeClass::ALL {
            for longhand in LayoutLonghand::ALL {
                if !seen.contains(&(set_name, class, longhand.name().to_string())) {
                    problems.push(format!(
                        "{set_name} {} {longhand} 줄이 없습니다",
                        class.name()
                    ));
                }
            }
        }
        for entry in &payload {
            let class = scope_class(entry.scope).expect("payload 범위");
            if !seen.contains(&(set_name, class, entry.name.to_string())) {
                problems.push(format!(
                    "{set_name} {} {} 줄이 없습니다",
                    class.name(),
                    entry.name
                ));
            }
        }
    }
    problems
}

/// 사용자 세트 원본을 이름으로 모은 표.
pub type UserSets = BTreeMap<String, UserSetSource>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment::FragmentPayload;
    use crate::frame::FramePayload;
    use crate::text::TextPayload;
    use stellar_core_types::ids::StableId;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn user(name: &str, extends: &str, common: &[(LayoutLonghand, &str)]) -> UserSetSource {
        UserSetSource {
            name: name.into(),
            extends: extends.into(),
            common: common
                .iter()
                .map(|(l, v)| (*l, l.parse(v).unwrap()))
                .collect(),
            sections: Vec::new(),
        }
    }

    #[test]
    fn builtin_tables_cover_every_longhand_and_parse() {
        for longhand in LayoutLonghand::ALL {
            let rows: Vec<_> = builtin_entries()
                .iter()
                .filter(|e| e.set == CSS && e.longhand == longhand)
                .collect();
            assert_eq!(rows.len(), 1, "css 세트에 {longhand} 공통 값 하나");
            assert!(rows[0].class.is_none());
        }
        for entry in builtin_entries() {
            assert!(is_builtin(entry.set));
            builtin_value(entry);
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in builtin_entries() {
            assert!(
                seen.insert((entry.set, entry.class, entry.longhand)),
                "중복 {entry:?}"
            );
        }
    }

    #[test]
    fn css_set_is_css_initial_with_div_display_and_content_box() {
        let css = builtin(CSS).unwrap();
        for class in NodeClass::ALL {
            assert_eq!(
                css.value(class, LayoutLonghand::Display),
                &LayoutValue::Keyword("block")
            );
            assert_eq!(
                css.value(class, LayoutLonghand::BoxSizing),
                &LayoutValue::Keyword("content-box")
            );
            assert_eq!(
                css.value(class, LayoutLonghand::FlexShrink),
                &LayoutValue::Number(1.0)
            );
            assert_eq!(
                css.value(class, LayoutLonghand::MinWidth),
                &LayoutValue::Keyword("auto")
            );
        }
        assert_eq!(css.chain(), ["css"]);
    }

    #[test]
    fn react_native_set_inherits_css_and_overrides_view_defaults() {
        let rn = builtin(REACT_NATIVE).unwrap();
        assert_eq!(rn.chain(), ["react-native", "css"]);
        let frame = NodeClass::Frame;
        assert_eq!(
            rn.value(frame, LayoutLonghand::Display),
            &LayoutValue::Keyword("flex")
        );
        assert_eq!(
            rn.value(frame, LayoutLonghand::FlexDirection),
            &LayoutValue::Keyword("column")
        );
        assert_eq!(
            rn.value(frame, LayoutLonghand::AlignContent),
            &LayoutValue::Keyword("flex-start")
        );
        assert_eq!(
            rn.value(NodeClass::Text, LayoutLonghand::Display),
            &LayoutValue::Keyword("block"),
            "확인하지 못한 항목은 css 값을 물려받음"
        );
        for class in NodeClass::ALL {
            assert_eq!(
                rn.value(class, LayoutLonghand::FlexShrink),
                &LayoutValue::Number(0.0)
            );
            assert_eq!(
                rn.value(class, LayoutLonghand::MinWidth),
                &LayoutValue::Length(0.0)
            );
            assert_eq!(
                rn.value(class, LayoutLonghand::BoxSizing),
                &LayoutValue::Keyword("border-box")
            );
            assert_eq!(
                rn.value(class, LayoutLonghand::Position),
                &LayoutValue::Keyword("relative")
            );
        }
    }

    #[test]
    fn user_sets_extend_and_override_by_section() {
        let mut compact = user(
            "compact",
            REACT_NATIVE,
            &[(LayoutLonghand::FlexShrink, "1")],
        );
        compact.sections.push((
            NodeClass::Frame,
            vec![(LayoutLonghand::RowGap, LayoutValue::Length(8.0))],
        ));
        let dense = user("dense", "compact", &[(LayoutLonghand::RowGap, "4px")]);
        let sets: UserSets = [compact, dense]
            .into_iter()
            .map(|s| (s.name.clone(), s))
            .collect();
        let flat = flatten("dense", |n| sets.get(n)).unwrap();
        assert_eq!(flat.chain(), ["dense", "compact", "react-native", "css"]);
        assert_eq!(
            flat.value(NodeClass::Frame, LayoutLonghand::FlexShrink),
            &LayoutValue::Number(1.0)
        );
        // dense 공통 값이 compact frame 구획 값보다 나중(자식)이라 이깁니다.
        assert_eq!(
            flat.value(NodeClass::Frame, LayoutLonghand::RowGap),
            &LayoutValue::Length(4.0)
        );
        assert_eq!(
            flat.value(NodeClass::Text, LayoutLonghand::RowGap),
            &LayoutValue::Length(4.0)
        );
        let compact = flatten("compact", |n| sets.get(n)).unwrap();
        assert_eq!(
            compact.value(NodeClass::Frame, LayoutLonghand::RowGap),
            &LayoutValue::Length(8.0)
        );
        assert_eq!(
            compact.value(NodeClass::Text, LayoutLonghand::RowGap),
            &LayoutValue::Keyword("normal")
        );
    }

    #[test]
    fn cycles_unknown_sets_and_reserved_names_are_rejected() {
        let a = user("a", "b", &[]);
        let b = user("b", "a", &[]);
        let sets: UserSets = [a, b].into_iter().map(|s| (s.name.clone(), s)).collect();
        assert_eq!(
            flatten("a", |n| sets.get(n)).unwrap_err(),
            StyleDefaultsError::Cycle {
                chain: vec!["a".into(), "b".into(), "a".into()]
            }
        );
        assert_eq!(
            flatten("missing", |n| sets.get(n)).unwrap_err().code(),
            "unknown-style-defaults"
        );
        let orphan = user("orphan", "nowhere", &[]);
        let sets: UserSets = [(orphan.name.clone(), orphan)].into_iter().collect();
        assert_eq!(
            flatten("orphan", |n| sets.get(n)).unwrap_err(),
            StyleDefaultsError::UnknownSet {
                name: "nowhere".into()
            }
        );
        let reserved = user("css", "react-native", &[]);
        assert_eq!(
            check_source(&reserved).unwrap_err().code(),
            "style-defaults-reserved-name"
        );
        let bad = user("Bad_Name", "css", &[]);
        assert_eq!(
            check_source(&bad).unwrap_err().code(),
            "style-defaults-invalid-name"
        );
        let mut duplicate = user("dup", "css", &[(LayoutLonghand::Left, "1px")]);
        duplicate
            .common
            .push((LayoutLonghand::Left, LayoutValue::Length(2.0)));
        assert_eq!(
            check_source(&duplicate).unwrap_err().code(),
            "style-defaults-duplicate-longhand"
        );
        // 사슬 길이 상한.
        let chain: UserSets = (0..=MAX_EXTENDS_DEPTH)
            .map(|i| {
                let extends = if i == MAX_EXTENDS_DEPTH {
                    CSS.to_string()
                } else {
                    format!("s{}", i + 1)
                };
                let s = user(&format!("s{i}"), &extends, &[]);
                (s.name.clone(), s)
            })
            .collect();
        assert_eq!(
            flatten("s0", |n| chain.get(n)).unwrap_err().code(),
            "style-defaults-too-deep"
        );
    }

    #[test]
    fn resolve_marks_explicit_and_default_sources() {
        let css = builtin(CSS).unwrap();
        let rn = builtin(REACT_NATIVE).unwrap();
        let frame = NodePayload::Frame(FramePayload::default());
        let mut record = LayoutRecord::new(sid(1));
        record
            .set(
                LayoutLonghand::FlexDirection,
                LayoutValue::Keyword("column"),
            )
            .unwrap();
        let resolved = resolve(&rn, &frame, Some(&record)).unwrap();
        assert_eq!(resolved.declarations().len(), LayoutLonghand::ALL.len());
        let direction = resolved.get(LayoutLonghand::FlexDirection);
        assert_eq!(
            direction.source,
            ValueSource::Explicit,
            "세트 값과 같아도 명시"
        );
        assert_eq!(direction.source.to_string(), "explicit");
        let display = resolved.get(LayoutLonghand::Display);
        assert_eq!(display.value, LayoutValue::Keyword("flex"));
        assert_eq!(display.source.to_string(), "default(react-native)");
        // payload 크기는 명시 width·height.
        let width = resolved.get(LayoutLonghand::Width);
        assert_eq!(width.value, LayoutValue::Length(0.0));
        assert_eq!(width.source, ValueSource::Explicit);
        // Text의 container longhand는 언제나 세트 값.
        let text = NodePayload::Text(TextPayload::empty(0));
        let resolved = resolve(&css, &text, Some(&record)).unwrap();
        assert_eq!(resolved.class, NodeClass::Text);
        let direction = resolved.get(LayoutLonghand::FlexDirection);
        assert_eq!(direction.value, LayoutValue::Keyword("row"));
        assert_eq!(direction.source.to_string(), "default(css)");
        assert_eq!(
            resolved.get(LayoutLonghand::Width).value,
            LayoutValue::Keyword("auto")
        );
        assert!(resolve(&css, &NodePayload::Fragment(FragmentPayload), None).is_none());
    }

    #[test]
    fn differences_list_only_differing_values() {
        let css = builtin(CSS).unwrap();
        let rn = builtin(REACT_NATIVE).unwrap();
        assert!(differences(&css, &css, NodeClass::Frame).is_empty());
        let rn_over_css: Vec<String> = differences(&rn, &css, NodeClass::Frame)
            .iter()
            .map(|(l, v)| format!("{l}: {v}"))
            .collect();
        assert_eq!(
            rn_over_css,
            [
                "position: relative",
                "box-sizing: border-box",
                "min-width: 0px",
                "min-height: 0px",
                "flex-shrink: 0",
                "display: flex",
                "flex-direction: column",
                "align-content: flex-start",
            ]
        );
        let css_over_rn: Vec<String> = differences(&css, &rn, NodeClass::Text)
            .iter()
            .map(|(l, v)| format!("{l}: {v}"))
            .collect();
        assert_eq!(
            css_over_rn,
            [
                "position: static",
                "box-sizing: content-box",
                "min-width: auto",
                "min-height: auto",
                "flex-shrink: 1",
            ]
        );
    }

    #[test]
    fn spec_document_table_matches_snapshot() {
        let spec = include_str!("../../../../docs/spec/style-defaults.md");
        let table = snapshot_text();
        assert!(
            spec.contains(&table),
            "docs/spec/style-defaults.md 표가 snapshot_text()와 다릅니다:\n{table}"
        );
        let payload = payload_snapshot_text();
        assert!(
            spec.contains(&payload),
            "docs/spec/style-defaults.md payload 표가 payload_snapshot_text()와 다릅니다:\n{payload}"
        );
    }

    #[test]
    fn every_registry_longhand_has_values_in_both_builtin_sets() {
        // IX.11.24 검증 기준: 모든 registry longhand에 두 세트의 값이 있음(물려받음 포함).
        let longhands = payload_longhands();
        let names: Vec<&str> = longhands.iter().map(|e| e.name).collect();
        for expected in [
            "background-color",
            "border-top-style",
            "border-left-color",
            "border-top-left-radius",
            "transform",
            "stroke-linecap",
            "fill-rule",
            "font-family",
            "line-height",
            "letter-spacing",
            "object-fit",
            "image-rendering",
        ] {
            assert!(
                names.contains(&expected),
                "{expected}가 payload longhand에 없음"
            );
        }
        // payload 크기와 stellar-only 항목은 payload longhand가 아닙니다(layout 표·세트 밖).
        for excluded in [
            "width",
            "height",
            "x1",
            "polygon-sides",
            "text-content",
            "path-data",
            "clipping",
            "visible",
        ] {
            assert!(!names.contains(&excluded), "{excluded}");
        }
        for set in BUILTIN_SETS {
            let flat = builtin(set).unwrap();
            for entry in &longhands {
                let value = flat
                    .payload_value(entry.scope, entry.key)
                    .unwrap_or_else(|| panic!("{set} 세트에 {} 값이 없음", entry.name));
                check_payload_value(entry, value).unwrap_or_else(|e| panic!("{set}: {e}"));
            }
        }
        // 표 행: css는 longhand마다 하나, 모든 행은 payload longhand이고 (세트, 범위, key)가 겹치지 않음.
        let mut seen = std::collections::BTreeSet::new();
        for row in payload_entries() {
            assert!(is_builtin(row.set));
            assert!(
                longhands
                    .iter()
                    .any(|e| e.scope == row.scope && e.key == row.key),
                "{row:?}는 payload longhand가 아님"
            );
            assert!(seen.insert((row.set, row.scope, row.key.0)), "중복 {row:?}");
        }
        assert_eq!(
            payload_entries().iter().filter(|r| r.set == CSS).count(),
            longhands.len()
        );
    }

    #[test]
    fn conformance_fixture_matches_tables() {
        // IX.11.31 검증 기준: 세트별 fixture가 registry 표 값과 일치, 기대값 출처 기록 누락 0.
        let css = builtin(CSS).unwrap();
        let rn = builtin(REACT_NATIVE).unwrap();
        let problems = conformance_mismatches(CONFORMANCE_FIXTURE, &css, &rn);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn conformance_fixture_fails_when_a_table_value_changes() {
        // IX.11.31 검증 기준: 표 값을 바꾸면 fixture가 실패.
        let css = builtin(CSS).unwrap();
        let rn = builtin(REACT_NATIVE).unwrap();
        let mut changed = rn.clone();
        changed.apply(
            Some(NodeClass::Frame),
            LayoutLonghand::FlexDirection,
            LayoutValue::Keyword("row"),
        );
        let problems = conformance_mismatches(CONFORMANCE_FIXTURE, &css, &changed);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("flex-direction"));
        // payload 값이 바뀐 세트(다른 내장 세트의 사슬)도 잡습니다.
        let swapped = conformance_mismatches(CONFORMANCE_FIXTURE, &rn, &css);
        assert!(swapped.iter().any(|p| p.contains("object-fit")));
        // 출처가 없는 줄, 빠진 줄, 관측값과 출처가 맞지 않는 줄.
        let fixture = CONFORMANCE_FIXTURE.replacen(
            "css | frame | position | static | static | chrome: div 계산값",
            "css | frame | position | static | static | 메모",
            1,
        );
        assert!(conformance_mismatches(&fixture, &css, &rn)
            .iter()
            .any(|p| p.contains("출처")));
        let fixture = CONFORMANCE_FIXTURE.replacen(
            "css | frame | position | static | static | chrome: div 계산값\n",
            "",
            1,
        );
        assert!(conformance_mismatches(&fixture, &css, &rn)
            .iter()
            .any(|p| p.contains("줄이 없습니다")));
        assert_eq!(normalize_observed("rgba(0, 0, 0, 0)"), "#00000000");
        assert_eq!(normalize_observed("rgb(0, 0, 0)"), "#000000");
        assert_eq!(normalize_observed("auto"), "auto");
    }

    #[test]
    fn payload_values_reject_non_canonical_or_unparsable_text() {
        let background = registry::entry_by_name(PropertyScope::Frame, "background-color").unwrap();
        assert!(check_payload_value(&background, "#00000000").is_ok());
        assert!(
            check_payload_value(&background, "transparent").is_err(),
            "정규 표기가 아님"
        );
        assert!(check_payload_value(&background, "TRANSPARENT").is_err());
        assert!(check_payload_value(&background, "currentcolor").is_err());
        let line_height = registry::entry_by_name(PropertyScope::Text, "line-height").unwrap();
        assert!(
            check_payload_value(&line_height, "normal").is_ok(),
            "값 변형 규칙"
        );
        assert!(check_payload_value(&line_height, "1.5").is_ok());
        let letter = registry::entry_by_name(PropertyScope::Text, "letter-spacing").unwrap();
        assert!(check_payload_value(&letter, "normal").is_err());
    }

    #[test]
    fn react_native_payload_overrides_and_inherited_values() {
        use crate::css::typography::keys as typo;
        use crate::payload_metadata::image_keys;
        let css = builtin(CSS).unwrap();
        let rn = builtin(REACT_NATIVE).unwrap();
        assert_eq!(
            rn.payload_value(PropertyScope::Image, image_keys::OBJECT_FIT),
            Some("cover")
        );
        assert_eq!(
            css.payload_value(PropertyScope::Image, image_keys::OBJECT_FIT),
            Some("fill")
        );
        assert_eq!(
            rn.payload_value(PropertyScope::Text, typo::FONT_SIZE),
            Some("14px")
        );
        assert_eq!(
            rn.payload_value(PropertyScope::Text, typo::LINE_HEIGHT),
            Some("normal"),
            "물려받음"
        );
        // 사용자 세트는 사슬의 내장 세트 값을 씁니다.
        let sets: UserSets = [user("compact", REACT_NATIVE, &[])]
            .into_iter()
            .map(|s| (s.name.clone(), s))
            .collect();
        let compact = flatten("compact", |n| sets.get(n)).unwrap();
        assert_eq!(
            compact.payload_value(PropertyScope::Text, typo::FONT_SIZE),
            Some("14px")
        );
        // payload longhand가 아닌 항목.
        assert_eq!(
            css.payload_value(
                PropertyScope::Node,
                crate::payload_metadata::node_keys::VISIBLE
            ),
            None
        );
        for basis in Basis::ALL {
            assert_eq!(Basis::from_name(basis.name()), Some(basis));
        }
    }
}
