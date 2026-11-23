//! 저장 View 내보내기의 CSS layout 부분 (IX.11.30, XXII.2.2, ADR-0089).
//!
//! Web HTML+CSS 경로는 [`crate::web_frame::export_web_frame_node`] 하나입니다. 이 모듈은 그 경로가 node마다
//! 부르는 layout 계획([`plan_element`])과, 같은 계획을 React Native 스타일 객체로 낮추는 출력
//! ([`export_react_native`])을 둡니다. 새 tree walk·값 parser·formatter를 두지 않습니다.
//!
//! - 값: 명시하지 않은 longhand는 활성 기본값 세트로 푸는 문서 해석 함수 하나(`style_defaults::resolve_kind`)로
//!   얻고, CSS 텍스트는 문서 formatter(`layout_record::format_value`, `css::value::format_length`)로 씁니다.
//! - 대상 기본값: Web은 `css` 세트, RN은 `react-native` 세트입니다. 출력은 명시 값과 "활성 세트 ≠ 대상 기본값"
//!   차이만 냅니다(ADR-0082 결정 6). Web 차이는 특이도 0 기반 규칙이 맡고 요소 규칙에는 명시 값만 남습니다.
//! - 배치: IX.11.9(위치·transform 분리) 전이므로 저장 transform 이동을 위치로 쓰는 Free 배치와 CSS 흐름 배치를
//!   [`Placement`]로 가릅니다(ADR-0089 결정 4).

use std::collections::BTreeMap;

use stellar_codegen::react_native::{
    lower, property_name, style_key, style_sheet, RnInput, RnStyle, RnValue,
};
use stellar_codegen::web::frame::FrameStyle;
use stellar_codegen::web::layout::{self as web_layout, BaseRules, ClassDefaults, Declaration};
use stellar_document::css::style_defaults::{
    self as sd, differences, resolve_kind, FlatSet, NodeClass, ResolvedLayout, ValueSource,
};
use stellar_document::css::value::{format_length, format_percentage};
use stellar_document::layout_record::{
    format_value, payload_size, LayoutLonghand as L, LayoutRecord, LayoutRole, LayoutValue,
};
use stellar_document::records::NodePayload;
use stellar_project::manifest_style_defaults::{active_set, StyleDefaultsFileError};
use stellar_target_profile::capability::CapabilityStatus;

use crate::core_types::ids::StableId;
use crate::snapshot::ProjectSnapshot;
use crate::web_frame::{export_web_frame_node, HtmlDiagnostic, HtmlError, WebFrameHtml};

/// 고정 Free Frame 출력 형식 이름(layout record가 없고 활성 세트가 Web 기본값과 같음).
pub const FORMAT_FIXED: &str = "fixed-free-frame-html";
/// CSS layout 출력 형식 이름.
pub const FORMAT_LAYOUT: &str = "css-layout-html";

/// 내보내기 하나가 쓰는 기본값 세트들.
#[derive(Debug, Clone)]
pub struct StyleContext {
    /// 프로젝트 활성 세트(`stellar.yaml`의 `style_defaults`).
    pub active: FlatSet,
    /// Web 대상 고유 기본값(`css` 세트).
    pub web: FlatSet,
    /// React Native 대상 고유 기본값(`react-native` 세트).
    pub react_native: FlatSet,
}

impl StyleContext {
    /// 스냅샷의 매니페스트와 사용자 세트 파일에서 활성 세트를 평탄화합니다.
    pub fn load(snapshot: &ProjectSnapshot) -> Result<StyleContext, StyleDefaultsFileError> {
        Ok(StyleContext {
            active: active_set(snapshot.root(), snapshot.manifest())?,
            web: sd::builtin(sd::CSS).expect("내장 css 세트"),
            react_native: sd::builtin(sd::REACT_NATIVE).expect("내장 react-native 세트"),
        })
    }

    /// 출력에 나오는 구획의 Web 기반 규칙(활성 세트 ≠ `css` 세트인 longhand).
    pub fn web_base_rules(&self, root_element: &str, classes: &[NodeClass]) -> BaseRules {
        let classes: Vec<ClassDefaults> = classes
            .iter()
            .map(|class| ClassDefaults {
                class: class.name(),
                declarations: differences(&self.active, &self.web, *class)
                    .into_iter()
                    .map(|(longhand, value)| {
                        Declaration::new(longhand.name(), format_value(&value))
                    })
                    .collect(),
            })
            .collect();
        web_layout::base_rules(root_element, &classes)
    }

    /// 활성 세트가 이 구획에서 Web 기본값과 다른지.
    pub fn differs_from_web(&self, class: NodeClass) -> bool {
        !differences(&self.active, &self.web, class).is_empty()
    }
}

/// 요소의 배치 방식(IX.11.9 전 연결 규칙, ADR-0089 결정 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// 내보내는 root. 부모가 없어 item longhand를 내지 않습니다.
    Root,
    /// layout record의 위치 지정이 없는 Free 자식. 저장 transform 이동을 `position: absolute` + `left`·`top`으로
    /// 냅니다.
    Free,
    /// 명시 inset이 있는 absolute 자식. record의 inset을 쓰고 transform 이동은 시각 이동입니다.
    Inset,
    /// CSS 흐름 item(flex·grid·block 흐름). transform 이동은 시각 이동입니다.
    Flow,
}

impl Placement {
    /// 출력 이름.
    pub fn name(self) -> &'static str {
        match self {
            Placement::Root => "root",
            Placement::Free => "free",
            Placement::Inset => "inset",
            Placement::Flow => "flow",
        }
    }
}

/// 선언의 출처.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationOrigin {
    /// 사용자가 명시한 값(layout record 또는 payload 크기) 그대로.
    Explicit,
    /// 배치 규칙이 정한 값(`position`, Free `left`·`top`).
    Placement,
    /// Stellar border(레이아웃 공간 없음)를 CSS border로 바꾸며 보정한 값.
    Converted,
    /// 표시 여부(`display: none`).
    Visibility,
}

impl DeclarationOrigin {
    /// 출력 이름.
    pub fn name(self) -> &'static str {
        match self {
            DeclarationOrigin::Explicit => "explicit",
            DeclarationOrigin::Placement => "placement",
            DeclarationOrigin::Converted => "converted",
            DeclarationOrigin::Visibility => "visibility",
        }
    }
}

/// 요소 규칙의 layout 선언 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleDeclaration {
    /// longhand 이름.
    pub name: &'static str,
    /// 정규 CSS 값.
    pub value: String,
    /// 출처.
    pub origin: DeclarationOrigin,
}

/// 계획 진단(상위가 node 진단으로 옮깁니다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanNote {
    /// 실패 진단인지.
    pub error: bool,
    /// 안정 code(`approximated-*`, `unsupported-*`, `ignored-*`).
    pub code: &'static str,
    /// 설명.
    pub message: String,
}

/// 자식에게 넘기는 부모 상자 정보. Fragment는 상자가 없어 그대로 통과시킵니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParentLayout {
    /// 자식이 CSS 흐름 item인지 정하는 부모 상태: `display`가 `flex`·`grid`이거나 `display`를 명시했습니다.
    pub flow: bool,
    /// 부모 `display`가 `block`입니다(flex item 속성이 효과가 없음).
    pub block: bool,
    /// 부모 Frame의 Stellar border 두께(px).
    pub border: f32,
    /// 부모 main 축이 세로(`flex-direction: column`·`column-reverse`)입니다(`flex-basis` 축).
    pub column: bool,
}

/// 요소 하나의 layout 계획.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementStyle {
    /// 출력 tree의 node(문서 node 또는 펼친 컴포넌트 node의 내부 key).
    pub node: StableId,
    /// DOM id. 요소를 배치한 뒤 상위가 채웁니다.
    pub element: String,
    /// 기본값 구획.
    pub class: NodeClass,
    /// 배치.
    pub placement: Placement,
    /// 활성 세트로 푼 40개 longhand.
    pub resolved: ResolvedLayout,
    /// Web 요소 규칙 layout 선언(longhand 순서, Free `left`·`top` 포함).
    pub declarations: Vec<StyleDeclaration>,
    /// 저장 transform 이동(px).
    pub translation: (f32, f32),
    /// paint 선언의 matrix 이동(Free는 소수부, Root는 0, 그 밖은 전체).
    pub paint_translation: (f32, f32),
    /// 저장 transform 선형 부분이 단위 행렬인지.
    pub linear_identity: bool,
    /// 부모 `display`가 `block`인지.
    pub parent_block: bool,
    /// 표시 여부(조상 반영).
    pub visible: bool,
    /// 자식에게 넘길 부모 정보.
    pub children: ParentLayout,
    /// layout record가 있는지.
    pub has_record: bool,
}

fn keyword(value: &LayoutValue) -> Option<&'static str> {
    match value {
        LayoutValue::Keyword(k) => Some(k),
        _ => None,
    }
}

const ROOT_IGNORED: [L; 15] = [
    L::Top,
    L::Right,
    L::Bottom,
    L::Left,
    L::FlexGrow,
    L::FlexShrink,
    L::FlexBasis,
    L::AlignSelf,
    L::JustifySelf,
    L::GridColumnStart,
    L::GridColumnEnd,
    L::GridRowStart,
    L::GridRowEnd,
    L::Position,
    L::BoxSizing,
];
const INSETS: [L; 4] = [L::Top, L::Right, L::Bottom, L::Left];
const SIZES: [(L, bool); 7] = [
    (L::Width, true),
    (L::Height, false),
    (L::MinWidth, true),
    (L::MinHeight, false),
    (L::MaxWidth, true),
    (L::MaxHeight, false),
    (L::FlexBasis, true),
];

fn px(value: &LayoutValue) -> f32 {
    match value {
        LayoutValue::Length(v) => *v,
        _ => 0.0,
    }
}

/// node 하나의 layout을 계획합니다. `payload`는 저장 payload(Rectangle은 Shape, 인스턴스 root는 펼친 Frame)이고
/// `style`은 같은 node의 paint 값입니다. `parent`가 `None`이면 내보내는 root입니다.
pub fn plan_element(
    context: &StyleContext,
    node: StableId,
    payload: &NodePayload,
    record: Option<&LayoutRecord>,
    parent: Option<ParentLayout>,
    style: &FrameStyle,
) -> (ElementStyle, Vec<PlanNote>) {
    let kind = payload.kind();
    let resolved = resolve_kind(&context.active, kind, record, payload_size(payload))
        .expect("상자를 가진 node");
    let class = resolved.class;
    let has = |l: L| record.is_some_and(|r| r.get(l).is_some());
    let value = |l: L| &resolved.get(l).value;
    let explicit = |l: L| resolved.get(l).source == ValueSource::Explicit;
    let mut notes = Vec::new();
    let mut note = |error: bool, code: &'static str, message: String| {
        notes.push(PlanNote {
            error,
            code,
            message,
        })
    };
    let position = keyword(value(L::Position)).unwrap_or("static");
    let placement = match parent {
        None => Placement::Root,
        Some(p) => {
            if matches!(position, "static" | "relative") && (p.flow || has(L::Position)) {
                Placement::Flow
            } else if INSETS.iter().any(|l| has(*l)) {
                Placement::Inset
            } else {
                Placement::Free
            }
        }
    };
    let parent_border = parent.map_or(0.0, |p| p.border);
    let border = style.border.map_or(0.0, |(_, w)| w.get());
    let content_box = keyword(value(L::BoxSizing)) == Some("content-box");
    let display = keyword(value(L::Display)).unwrap_or("block");
    let children = ParentLayout {
        flow: matches!(display, "flex" | "grid") || has(L::Display),
        block: display == "block",
        border,
        column: matches!(
            keyword(value(L::FlexDirection)),
            Some("column" | "column-reverse")
        ),
    };
    let mut out: Vec<StyleDeclaration> = Vec::new();
    let mut push = |name: &'static str, value: String, origin: DeclarationOrigin| {
        out.push(StyleDeclaration {
            name,
            value,
            origin,
        })
    };
    let free_position = style.free_position();
    for longhand in L::ALL {
        let name = longhand.name();
        let text = || format_value(value(longhand));
        if placement == Placement::Root && ROOT_IGNORED.contains(&longhand) {
            if longhand == L::Position {
                push(name, "relative".into(), DeclarationOrigin::Placement);
            } else if longhand == L::BoxSizing {
                // root도 border 보정 규칙은 같습니다(아래 BoxSizing 처리와 같은 분기).
                if border > 0.0 {
                    push(
                        name,
                        "border-box".into(),
                        box_origin(explicit(longhand), content_box),
                    );
                } else if explicit(longhand) {
                    push(name, text(), DeclarationOrigin::Explicit);
                }
                continue;
            } else if has(longhand) {
                note(
                    false,
                    "ignored-root-item-longhand",
                    format!("내보내는 root에는 부모가 없어 명시한 {name}을 내지 않습니다"),
                );
            }
            if longhand == L::Position && has(longhand) && position == "absolute" {
                note(
                    false,
                    "ignored-root-item-longhand",
                    "내보내는 root에는 부모가 없어 명시한 position: absolute를 relative로 냅니다"
                        .into(),
                );
            }
            continue;
        }
        match longhand {
            L::Position => match placement {
                Placement::Free | Placement::Inset => {
                    push(name, "absolute".into(), DeclarationOrigin::Placement)
                }
                // 모든 요소를 positioned로 내보내는 stacking 규칙(속성 모델 3장): static은 relative로 냅니다.
                _ if position == "static" => {
                    push(name, "relative".into(), DeclarationOrigin::Placement)
                }
                _ if explicit(longhand) => push(name, text(), DeclarationOrigin::Explicit),
                _ => {}
            },
            L::BoxSizing => {
                if border > 0.0 {
                    push(
                        name,
                        "border-box".into(),
                        box_origin(explicit(longhand), content_box),
                    );
                } else if explicit(longhand) {
                    push(name, text(), DeclarationOrigin::Explicit);
                }
            }
            L::Top | L::Right | L::Bottom | L::Left => match placement {
                Placement::Free => {
                    if let Some((_, v)) = free_position.iter().find(|(n, _)| *n == name) {
                        push(name, v.clone(), DeclarationOrigin::Placement);
                    }
                }
                Placement::Inset if explicit(longhand) => match value(longhand) {
                    LayoutValue::Length(v) if parent_border > 0.0 => push(
                        name,
                        format!(
                            "calc({} - {})",
                            format_length(*v),
                            format_length(parent_border)
                        ),
                        DeclarationOrigin::Converted,
                    ),
                    LayoutValue::Percentage(_) if parent_border > 0.0 => {
                        note(false, "approximated-border-inset", format!(
                                "부모 border가 있으면 {name} 백분율의 기준(padding box)이 Stellar(border 0)와 border 두께만큼 다릅니다"));
                        push(name, text(), DeclarationOrigin::Explicit);
                    }
                    _ => push(name, text(), DeclarationOrigin::Explicit),
                },
                Placement::Flow if explicit(longhand) => {
                    if position == "static" {
                        note(false, "ignored-static-inset", format!(
                            "position: static에서 {name}은 효과가 없어 내지 않습니다(stacking 규칙으로 relative를 냅니다)"));
                    } else {
                        push(name, text(), DeclarationOrigin::Explicit);
                    }
                }
                _ => {}
            },
            L::Display => {
                if !style.visible {
                    push(name, "none".into(), DeclarationOrigin::Visibility);
                } else if explicit(longhand) {
                    push(name, text(), DeclarationOrigin::Explicit);
                }
            }
            L::PaddingTop | L::PaddingRight | L::PaddingBottom | L::PaddingLeft if border > 0.0 => {
                // Stellar border는 레이아웃 공간을 차지하지 않습니다. CSS border 안쪽 content box 위치를 맞추려고
                // padding에서 border 두께를 뺍니다(속성 모델 5장 border 행).
                let p = px(value(longhand));
                let converted = (p - border).max(0.0);
                if p < border && children.flow {
                    note(false, "approximated-border-padding", format!(
                        "{name} {}가 border {}보다 작아 흐름 자식의 content box가 {}만큼 안쪽으로 밀립니다",
                        format_length(p), format_length(border), format_length(border - p)));
                }
                let default = px(context.active.value(class, longhand));
                if explicit(longhand) || converted != default {
                    push(
                        name,
                        format_length(converted),
                        if converted == p {
                            DeclarationOrigin::Explicit
                        } else {
                            DeclarationOrigin::Converted
                        },
                    );
                }
            }
            _ if SIZES.iter().any(|(l, _)| *l == longhand) && explicit(longhand) => {
                let horizontal = SIZES
                    .iter()
                    .find(|(l, _)| *l == longhand)
                    .is_some_and(|(_, h)| *h);
                let horizontal = if longhand == L::FlexBasis {
                    !parent.is_some_and(|p| p.column)
                } else {
                    horizontal
                };
                let padding = if horizontal {
                    px(value(L::PaddingLeft)) + px(value(L::PaddingRight))
                } else {
                    px(value(L::PaddingTop)) + px(value(L::PaddingBottom))
                };
                if border > 0.0 && content_box && padding > 0.0 {
                    // content-box 크기를 border-box로 내며 padding 합을 더합니다(같은 border box).
                    match value(longhand) {
                        LayoutValue::Length(v) => push(
                            name,
                            format_length(v + padding),
                            DeclarationOrigin::Converted,
                        ),
                        LayoutValue::Percentage(v) => push(
                            name,
                            format!(
                                "calc({} + {})",
                                format_percentage(*v),
                                format_length(padding)
                            ),
                            DeclarationOrigin::Converted,
                        ),
                        _ => push(name, text(), DeclarationOrigin::Explicit),
                    }
                } else {
                    push(name, text(), DeclarationOrigin::Explicit);
                }
            }
            _ if explicit(longhand) => push(name, text(), DeclarationOrigin::Explicit),
            _ => {}
        }
    }
    let translation = style.translation();
    let paint_translation = match placement {
        Placement::Root => (0.0, 0.0),
        Placement::Free => style.free_translation(),
        Placement::Inset | Placement::Flow => translation,
    };
    let element = ElementStyle {
        node,
        element: String::new(),
        class,
        placement,
        resolved,
        declarations: out,
        translation,
        paint_translation,
        linear_identity: style.linear_is_identity(),
        parent_block: parent.is_some_and(|p| p.block),
        visible: style.visible,
        children,
        has_record: record.is_some_and(|r| !r.is_empty()),
    };
    (element, notes)
}

fn box_origin(explicit: bool, content_box: bool) -> DeclarationOrigin {
    if explicit && !content_box {
        DeclarationOrigin::Explicit
    } else {
        DeclarationOrigin::Converted
    }
}

/// 요소 규칙 본문의 layout 부분.
pub fn layout_block(element: &ElementStyle) -> String {
    let declarations: Vec<Declaration> = element
        .declarations
        .iter()
        .map(|d| Declaration::new(d.name, d.value.clone()))
        .collect();
    web_layout::declaration_block(&declarations)
}

/// React Native 스타일 객체 출력.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactNativeStyles {
    /// 같은 내보내기의 Web 투영(검증·진단·요소 계획).
    pub web: WebFrameHtml,
    /// `StyleSheet.create` 모듈. Web 투영에 오류가 있으면 `None`입니다.
    pub source: Option<String>,
    /// RN 낮춤 진단(요소 순서).
    pub diagnostics: Vec<HtmlDiagnostic>,
}

fn rn_input(value: &LayoutValue) -> RnInput {
    match value {
        LayoutValue::Keyword(k) => RnInput::Keyword(k),
        LayoutValue::Length(v) => RnInput::Px(*v),
        LayoutValue::Percentage(v) => RnInput::Percent(*v),
        LayoutValue::Number(v) => RnInput::Number(*v),
        other => RnInput::Text(format_value(other)),
    }
}

const FLEX_CONTAINER: [L; 7] = [
    L::FlexDirection,
    L::FlexWrap,
    L::JustifyContent,
    L::AlignContent,
    L::AlignItems,
    L::RowGap,
    L::ColumnGap,
];
const FLEX_ITEM: [L; 4] = [L::FlexGrow, L::FlexShrink, L::FlexBasis, L::AlignSelf];

/// 저장 View(또는 Frame 하나)의 layout을 RN 스타일 객체로 냅니다. 같은 Web 투영의 계획을 쓰고, 명시 값과
/// "활성 세트 ≠ `react-native` 세트" 차이만 냅니다(ADR-0082 결정 6). paint 속성은 내지 않습니다.
pub fn export_react_native(
    snapshot: &ProjectSnapshot,
    document: StableId,
    view: StableId,
    frame: Option<StableId>,
) -> Result<ReactNativeStyles, HtmlError> {
    let web = export_web_frame_node(snapshot, document, view, frame)?;
    if web.diagnostics.iter().any(|d| d.error) {
        return Ok(ReactNativeStyles {
            web,
            source: None,
            diagnostics: Vec::new(),
        });
    }
    let target = sd::builtin(sd::REACT_NATIVE).expect("내장 react-native 세트");
    let mut diagnostics = Vec::new();
    let mut styles = Vec::new();
    for element in &web.styles {
        let (style, notes) = react_native_style(element, &target);
        for (error, code, message) in notes {
            diagnostics.push(HtmlDiagnostic {
                error,
                node: element.node,
                code,
                message: format!("node {}: {message}", element.node),
            });
        }
        styles.push(style);
    }
    diagnostics.insert(
        0,
        HtmlDiagnostic {
            error: false,
            node: web.root_frame,
            code: "react-native-layout-only",
            message: format!(
                "node {}: layout longhand만 냅니다. 배경·테두리·반경·clip 같은 paint 속성과 요소 구조는 내지 않습니다",
                web.root_frame
            ),
        },
    );
    Ok(ReactNativeStyles {
        source: Some(style_sheet(&styles)),
        web,
        diagnostics,
    })
}

type RnNotes = Vec<(bool, &'static str, String)>;

fn emit_rn(
    entries: &mut Vec<(String, RnValue)>,
    notes: &mut RnNotes,
    longhand: L,
    value: &LayoutValue,
) {
    let lowering = lower(longhand.name(), &rn_input(value));
    let reason = lowering.reason.unwrap_or("");
    match lowering.status {
        CapabilityStatus::Native => {}
        CapabilityStatus::Approximated => notes.push((
            false,
            "approximated-react-native",
            format!("{}: {}: {reason}", longhand.name(), format_value(value)),
        )),
        _ => notes.push((
            false,
            "unsupported-react-native",
            format!("{}: {}: {reason}", longhand.name(), format_value(value)),
        )),
    }
    if let Some(v) = lowering.value {
        entries.push((property_name(longhand.name()), v));
    }
}

fn react_native_style(element: &ElementStyle, target: &FlatSet) -> (RnStyle, RnNotes) {
    let mut entries: Vec<(String, RnValue)> = Vec::new();
    let mut notes = Vec::new();
    let resolved = &element.resolved;
    let display = keyword(&resolved.get(L::Display).value).unwrap_or("block");
    let position = keyword(&resolved.get(L::Position).value).unwrap_or("static");
    let emit = emit_rn;
    for longhand in L::ALL {
        let rv = resolved.get(longhand);
        let explicit = rv.source == ValueSource::Explicit;
        let differs = rv.value != *target.value(element.class, longhand);
        let root_item = longhand.role() == LayoutRole::Item
            && !matches!(
                longhand,
                L::Width
                    | L::Height
                    | L::MinWidth
                    | L::MinHeight
                    | L::MaxWidth
                    | L::MaxHeight
                    | L::AspectRatio
                    | L::BoxSizing
            );
        if element.placement == Placement::Root && root_item {
            continue;
        }
        match longhand {
            L::Position => match element.placement {
                Placement::Free | Placement::Inset => {
                    entries.push(("position".into(), RnValue::Text("absolute".into())))
                }
                // Web 투영은 stacking 규칙(속성 모델 3장)으로 `static`을 `relative`로 냅니다. 그래서 absolute
                // 자식의 containing block이 언제나 부모 요소입니다. RN에 `static`을 쓰면 absolute 자식이 더 위
                // 조상을 기준으로 놓이므로, Web 결과와 같은 RN 기본값 `relative`로 둡니다(항목을 쓰지 않음).
                _ if position == "static" => {}
                _ if explicit || differs => emit(&mut entries, &mut notes, longhand, &rv.value),
                _ => {}
            },
            L::Top | L::Right | L::Bottom | L::Left => match element.placement {
                Placement::Free => {
                    let (x, y) = element.translation;
                    if longhand == L::Left {
                        entries.push(("left".into(), RnValue::Number(x)));
                    } else if longhand == L::Top {
                        entries.push(("top".into(), RnValue::Number(y)));
                    }
                }
                Placement::Inset if explicit => emit(&mut entries, &mut notes, longhand, &rv.value),
                Placement::Flow if explicit && position != "static" => {
                    emit(&mut entries, &mut notes, longhand, &rv.value)
                }
                _ => {}
            },
            L::Display => {
                if !element.visible {
                    entries.push(("display".into(), RnValue::Text("none".into())));
                } else if explicit || differs {
                    emit(&mut entries, &mut notes, longhand, &rv.value);
                }
            }
            // block container에서 flex container 속성은 효과가 없습니다. RN 기본(column, stretch)이 block 흐름을
            // 근사하므로 내지 않습니다.
            _ if display == "block" && FLEX_CONTAINER.contains(&longhand) => {}
            // block 부모의 자식에게 flex item 속성은 효과가 없고, 자동 최소 크기도 없습니다.
            _ if element.parent_block
                && (FLEX_ITEM.contains(&longhand)
                    || (matches!(longhand, L::MinWidth | L::MinHeight)
                        && rv.value == LayoutValue::Keyword("auto"))) => {}
            _ if explicit || differs => emit(&mut entries, &mut notes, longhand, &rv.value),
            _ => {}
        }
    }
    if !element.linear_identity {
        notes.push((
            false,
            "unsupported-react-native",
            "transform 선형 부분(회전·배율·기울임)은 낼 수 없습니다".into(),
        ));
    }
    let (x, y) = element.translation;
    if matches!(element.placement, Placement::Flow | Placement::Inset) && (x != 0.0 || y != 0.0) {
        entries.push(("transform".into(), RnValue::Translate(x, y)));
    }
    (
        RnStyle {
            key: style_key(&element.element),
            entries,
        },
        notes,
    )
}

/// 기반 규칙 선언 수(진단·요약용).
pub fn base_rule_count(rules: &BaseRules) -> usize {
    rules.common.len()
        + rules
            .by_class
            .iter()
            .map(|c| c.declarations.len())
            .sum::<usize>()
}

/// 구획 목록(출력 순서, 중복 없음).
pub fn present_classes(styles: &[ElementStyle]) -> Vec<NodeClass> {
    let mut seen = BTreeMap::new();
    for s in styles {
        seen.insert(s.class, ());
    }
    NodeClass::ALL
        .into_iter()
        .filter(|c| seen.contains_key(c))
        .collect()
}

/// 테스트 전용 고정 문서(`test-support`).
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod fixture;

#[cfg(test)]
mod tests;
