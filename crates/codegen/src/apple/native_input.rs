//! 저장 View 투영 → CodegenIR 입력 (ADR-0107 결정 8).
//!
//! 문서 계층(ops)이 저장 View를 Web 투영과 같은 규칙으로 읽어 node마다 [`NativeNodeInput`]을 넘기면, 이 함수가
//! [`CodegenSources`]를 채워 [`build_ir`]를 부릅니다. layout 값은 문서 정규 formatter가 쓴 40개 longhand 선언 목록이고
//! [`stellar_layout::declared::style_from_declarations`]로만 엔진 입력이 됩니다(값 parser를 새로 두지 않음). 접근성은
//! [`stellar_accessibility::generate`]로 만듭니다.
//!
//! IR 입력은 다음을 채웁니다(ADR-0121 결정 3).
//!
//! - CSS 기반 속성 사용(`css_properties`): node마다 값이 엔진 생략값(`LayoutStyle::default()`)과 다른 layout
//!   longhand, 0이 아닌 시각 이동(`frame/transform`). container longhand는 Frame·ComponentInstance만 씁니다. IR이 이
//!   사용마다 XXII.1.4 matrix 판정(IR mapping)을 붙입니다.
//! - 상태·상태 머신·`MotionSpec`·Context(`states`·`machines`·`motions`·`contexts`): 호출자가 [`NativeBehaviors`]로
//!   넘긴 값을 그대로 씁니다. 문서 값 변환은 문서 계층(ops)이 하고(`ops::codegen::motion` 등), 이 함수는 다시 해석하지
//!   않습니다. [`build_native_ir`]는 빈 [`NativeBehaviors`]입니다.
//!
//! 문서가 아직 복원하지 못하는 control contract·token·Variant·target capability 속성은 채우지 않습니다(ADR-0049,
//! XXII.1.1).

use std::collections::BTreeMap;
use std::fmt;

use stellar_accessibility::generate::generate;
use stellar_accessibility::snapshot::AccessibilitySnapshot;
use stellar_accessibility::source::{SourceKind, SourceNode, SourceTree};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::value::Transform2D;
use stellar_layout::declared::style_from_declarations;
use stellar_layout::input::{LayoutInput, LayoutNode};
use stellar_target_profile::builtin;
use stellar_target_profile::capability::{CapabilityPropertyId, ElementKind};
use stellar_target_profile::overrides::VariantLayer;
use stellar_target_profile::spec::TargetProfile;
use stellar_target_profile::validate::{PropertyCapabilityMap, ViewModel, ViewNode};

use crate::ir::context::IrContexts;
use crate::ir::{
    build_ir, CodegenIr, CodegenSources, CssPropertyUse, IrError, IrMotion, IrStateMachine,
    StateDecl,
};
use stellar_layout::style::LayoutStyle;
use stellar_target_profile::css_matrix::MappingInput;

/// node 하나의 입력(문서 preorder, 형제는 문서 순서).
#[derive(Debug, Clone, PartialEq)]
pub struct NativeNodeInput {
    /// StableId.
    pub id: StableId,
    /// 부모(투영 root면 `None`).
    pub parent: Option<StableId>,
    /// element 종류.
    pub element: ElementKind,
    /// 40개 layout longhand의 (이름, 정규 CSS 값). Fragment는 비어 있습니다.
    pub declarations: Vec<(&'static str, String)>,
    /// layout 뒤 시각 이동(px).
    pub translation: (f32, f32),
    /// Text 내용.
    pub text: Option<String>,
    /// 접근성에서 숨김(조상 반영).
    pub hidden: bool,
}

/// 입력 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeInputError {
    /// 입력이 비었습니다.
    Empty,
    /// layout 선언을 엔진 입력으로 바꾸지 못했습니다.
    Declarations {
        /// node.
        node: StableId,
        /// 이유.
        reason: String,
    },
    /// 접근성 tree를 만들지 못했습니다.
    Accessibility(String),
    /// 내장 capability registry를 읽지 못했습니다.
    Registry(String),
    /// IR 변환 실패.
    Ir(IrError),
}

impl fmt::Display for NativeInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NativeInputError::Empty => f.write_str("내보낼 node가 없습니다"),
            NativeInputError::Declarations { node, reason } => {
                write!(f, "node {node}의 layout 선언을 읽지 못했습니다: {reason}")
            }
            NativeInputError::Accessibility(reason) => {
                write!(f, "접근성 tree를 만들지 못했습니다: {reason}")
            }
            NativeInputError::Registry(reason) => {
                write!(f, "내장 capability registry를 읽지 못했습니다: {reason}")
            }
            NativeInputError::Ir(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for NativeInputError {}

struct NoCapabilities;

impl PropertyCapabilityMap for NoCapabilities {
    fn capability_of(&self, _path: &PropertyPath) -> Option<CapabilityPropertyId> {
        None
    }
}

fn order_key(position: usize) -> OrderKey {
    let [_, hi, mid, lo] = (position as u32 + 1).to_be_bytes();
    OrderKey::from_bytes(vec![hi, mid, lo, 1]).expect("후행 1이라 정규형")
}

fn source_kind(element: ElementKind) -> SourceKind {
    match element {
        ElementKind::FRAME | ElementKind::COMPONENT_INSTANCE => SourceKind::Frame,
        ElementKind::FRAGMENT => SourceKind::Fragment,
        ElementKind::TEXT => SourceKind::Text,
        ElementKind::IMAGE => SourceKind::Image,
        _ => SourceKind::Graphic,
    }
}

/// 상태·상태 머신·`MotionSpec`·Context 입력. 문서 계층이 IR 타입으로 옮긴 값입니다.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NativeBehaviors {
    /// 상태 변수 선언.
    pub states: Vec<StateDecl>,
    /// 선언형 상태 머신(XXII.1.5).
    pub machines: Vec<IrStateMachine>,
    /// `MotionSpec`(XXII.1.5).
    pub motions: Vec<IrMotion>,
    /// Context 해석 결과(XXII.1.6).
    pub contexts: IrContexts,
}

/// item longhand(범위 `node`)인지. 나머지 layout longhand는 container longhand(범위 `frame`)입니다.
fn is_item_longhand(name: &str) -> bool {
    matches!(
        name,
        "position"
            | "box-sizing"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "width"
            | "height"
            | "min-width"
            | "min-height"
            | "max-width"
            | "max-height"
            | "aspect-ratio"
            | "flex-grow"
            | "flex-shrink"
            | "flex-basis"
            | "align-self"
            | "justify-self"
            | "grid-column-start"
            | "grid-column-end"
            | "grid-row-start"
            | "grid-row-end"
    )
}

/// longhand의 값이 엔진 생략값과 다른지. 값 텍스트를 다시 읽지 않고 엔진 입력 필드를 비교합니다.
fn differs(style: &LayoutStyle, base: &LayoutStyle, name: &str) -> bool {
    let (t, r, b, l) = style.padding.components();
    let (bt, br, bb, bl) = base.padding.components();
    match name {
        "display" => style.display != base.display,
        "position" => style.position != base.position,
        "box-sizing" => style.box_sizing != base.box_sizing,
        "top" => style.top != base.top,
        "right" => style.right != base.right,
        "bottom" => style.bottom != base.bottom,
        "left" => style.left != base.left,
        "width" => style.width != base.width,
        "height" => style.height != base.height,
        "min-width" => style.min_width != base.min_width,
        "min-height" => style.min_height != base.min_height,
        "max-width" => style.max_width != base.max_width,
        "max-height" => style.max_height != base.max_height,
        "aspect-ratio" => style.aspect_ratio != base.aspect_ratio,
        "padding-top" => t != bt,
        "padding-right" => r != br,
        "padding-bottom" => b != bb,
        "padding-left" => l != bl,
        "flex-direction" => style.flex_direction != base.flex_direction,
        "flex-wrap" => style.flex_wrap != base.flex_wrap,
        "justify-content" => style.justify_content != base.justify_content,
        "align-content" => style.align_content != base.align_content,
        "align-items" => style.align_items != base.align_items,
        "justify-items" => style.justify_items != base.justify_items,
        "row-gap" => style.row_gap != base.row_gap,
        "column-gap" => style.column_gap != base.column_gap,
        "flex-grow" => style.flex_grow != base.flex_grow,
        "flex-shrink" => style.flex_shrink != base.flex_shrink,
        "flex-basis" => style.flex_basis != base.flex_basis,
        "align-self" => style.align_self != base.align_self,
        "justify-self" => style.justify_self != base.justify_self,
        "grid-template-columns" => style.grid_template_columns != base.grid_template_columns,
        "grid-template-rows" => style.grid_template_rows != base.grid_template_rows,
        "grid-auto-columns" => style.grid_auto_columns != base.grid_auto_columns,
        "grid-auto-rows" => style.grid_auto_rows != base.grid_auto_rows,
        "grid-auto-flow" => style.grid_auto_flow != base.grid_auto_flow,
        "grid-column-start" => style.grid_column_start != base.grid_column_start,
        "grid-column-end" => style.grid_column_end != base.grid_column_end,
        "grid-row-start" => style.grid_row_start != base.grid_row_start,
        "grid-row-end" => style.grid_row_end != base.grid_row_end,
        _ => false,
    }
}

/// node 하나의 CSS 기반 속성 사용(ADR-0121 결정 3). layout longhand는 CSS longhand 그대로라 매핑 상태가 `exact`입니다.
fn css_uses(
    style: &LayoutStyle,
    element: ElementKind,
    translation: (f32, f32),
) -> Vec<CssPropertyUse> {
    let base = LayoutStyle::default();
    let container = matches!(
        element,
        ElementKind::FRAME | ElementKind::COMPONENT_INSTANCE
    );
    let mut out: Vec<CssPropertyUse> = stellar_layout::declared::LONGHANDS
        .iter()
        .filter(|name| differs(style, &base, name))
        .filter_map(|name| {
            let scope = if is_item_longhand(name) {
                "node"
            } else if container {
                "frame"
            } else {
                return None;
            };
            Some(CssPropertyUse {
                scope,
                name,
                status: MappingInput::Exact,
                path: None,
            })
        })
        .collect();
    if element == ElementKind::FRAME && translation != (0.0, 0.0) {
        out.push(CssPropertyUse {
            scope: "frame",
            name: "transform",
            status: MappingInput::Exact,
            path: None,
        });
    }
    out
}

/// 입력에서 IR을 만듭니다(상태·상태 머신·motion·Context 없음).
pub fn build_native_ir(
    nodes: &[NativeNodeInput],
    profile: &TargetProfile,
) -> Result<CodegenIr, NativeInputError> {
    build_native_view_ir(nodes, &NativeBehaviors::default(), profile)
}

/// 입력과 상태·상태 머신·motion·Context에서 IR을 만듭니다.
pub fn build_native_view_ir(
    nodes: &[NativeNodeInput],
    behaviors: &NativeBehaviors,
    profile: &TargetProfile,
) -> Result<CodegenIr, NativeInputError> {
    let root = nodes.first().ok_or(NativeInputError::Empty)?.id;
    let mut children: BTreeMap<StableId, Vec<StableId>> = BTreeMap::new();
    let elements: BTreeMap<StableId, ElementKind> =
        nodes.iter().map(|node| (node.id, node.element)).collect();
    for node in nodes {
        if let Some(parent) = node.parent {
            children.entry(parent).or_default().push(node.id);
        }
    }
    // layout 자식은 Fragment를 펼친 자식 목록입니다(문서 순서).
    let flattened = |parent: StableId| -> Vec<StableId> {
        let mut out = Vec::new();
        let mut stack: Vec<StableId> = children.get(&parent).cloned().unwrap_or_default();
        stack.reverse();
        while let Some(id) = stack.pop() {
            if elements.get(&id) == Some(&ElementKind::FRAGMENT) {
                let mut inner = children.get(&id).cloned().unwrap_or_default();
                inner.reverse();
                stack.extend(inner);
            } else {
                out.push(id);
            }
        }
        out
    };
    let mut layout_nodes = Vec::new();
    let mut source = SourceTree::new();
    source.push_root(root);
    let mut texts = BTreeMap::new();
    let mut css_properties: BTreeMap<StableId, Vec<CssPropertyUse>> = BTreeMap::new();
    for node in nodes {
        let mut source_node = SourceNode::new(node.id, source_kind(node.element))
            .with_children(children.get(&node.id).cloned().unwrap_or_default());
        source_node.hidden = node.hidden;
        source
            .insert(source_node)
            .map_err(|e| NativeInputError::Accessibility(e.to_string()))?;
        if let Some(text) = &node.text {
            texts.insert(node.id, text.clone());
        }
        if node.element == ElementKind::FRAGMENT {
            continue;
        }
        let declarations: Vec<(&str, &str)> = node
            .declarations
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        let style =
            style_from_declarations(&declarations).map_err(|e| NativeInputError::Declarations {
                node: node.id,
                reason: e.to_string(),
            })?;
        let uses = css_uses(&style, node.element, node.translation);
        if !uses.is_empty() {
            css_properties.insert(node.id, uses);
        }
        let (tx, ty) = node.translation;
        let mut layout = LayoutNode::new(node.id).with_style(style).with_transform(
            Transform2D::new(1.0, 0.0, 0.0, 1.0, tx, ty).map_err(|_| {
                NativeInputError::Declarations {
                    node: node.id,
                    reason: "이동 값이 유한하지 않습니다".to_string(),
                }
            })?,
        );
        for (position, child) in flattened(node.id).into_iter().enumerate() {
            layout = layout.with_child(child, order_key(position));
        }
        layout_nodes.push(layout);
    }
    let tree = generate(&source).map_err(|e| NativeInputError::Accessibility(e.to_string()))?;
    let accessibility = AccessibilitySnapshot::from_tree(&tree)
        .map_err(|e| NativeInputError::Accessibility(e.to_string()))?;
    let view = ViewModel {
        nodes: nodes
            .iter()
            .map(|node| ViewNode {
                id: node.id,
                parent: node.parent,
                element: node.element,
                contracts: Vec::new(),
            })
            .collect(),
        ..ViewModel::default()
    };
    let layout = LayoutInput::new(root, layout_nodes);
    let catalog = builtin::catalog().map_err(|e| NativeInputError::Registry(format!("{e:?}")))?;
    let sources = CodegenSources {
        view: &view,
        capability_map: &NoCapabilities,
        variants: &VariantLayer::none(),
        layout: &layout,
        accessibility: &accessibility,
        texts: &texts,
        states: &behaviors.states,
        tokens: &[],
        token_bindings: &BTreeMap::new(),
        css_properties: &css_properties,
        machines: &behaviors.machines,
        motions: &behaviors.motions,
        contexts: &behaviors.contexts,
    };
    build_ir(&sources, profile, &catalog).map_err(NativeInputError::Ir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple::{export_apple_ui, AppleDialect, AppleOptions};
    use crate::fixture::{ios_profile, sid};

    fn declarations(overrides: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        stellar_layout::declared::LONGHANDS
            .iter()
            .map(|name| {
                let value = overrides
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_else(|| css_initial(name).to_string());
                (*name, value)
            })
            .collect()
    }

    fn css_initial(name: &str) -> &'static str {
        match name {
            "position" => "static",
            "box-sizing" => "content-box",
            "top" | "right" | "bottom" | "left" | "width" | "height" | "min-width"
            | "min-height" | "aspect-ratio" | "flex-basis" | "align-self" | "justify-self"
            | "grid-column-start" | "grid-column-end" | "grid-row-start" | "grid-row-end"
            | "grid-auto-columns" | "grid-auto-rows" => "auto",
            "max-width" | "max-height" | "grid-template-columns" | "grid-template-rows" => "none",
            "flex-grow" => "0",
            "flex-shrink" => "1",
            "display" => "block",
            "padding-top" | "padding-right" | "padding-bottom" | "padding-left" | "row-gap"
            | "column-gap" => "0px",
            "flex-direction" => "row",
            "flex-wrap" => "nowrap",
            "justify-content" | "align-content" | "align-items" | "justify-items" => "normal",
            "grid-auto-flow" => "row",
            _ => "auto",
        }
    }

    #[test]
    fn saved_view_projection_builds_ir_through_fragments() {
        // root Frame(flex column, 200px) ─ Fragment ─ Text "안녕"
        let nodes = vec![
            NativeNodeInput {
                id: sid(1),
                parent: None,
                element: ElementKind::FRAME,
                declarations: declarations(&[
                    ("display", "flex"),
                    ("flex-direction", "column"),
                    ("width", "200px"),
                ]),
                translation: (0.0, 0.0),
                text: None,
                hidden: false,
            },
            NativeNodeInput {
                id: sid(2),
                parent: Some(sid(1)),
                element: ElementKind::FRAGMENT,
                declarations: Vec::new(),
                translation: (0.0, 0.0),
                text: None,
                hidden: false,
            },
            NativeNodeInput {
                id: sid(3),
                parent: Some(sid(2)),
                element: ElementKind::TEXT,
                declarations: declarations(&[]),
                translation: (4.0, 0.0),
                text: Some("안녕".to_string()),
                hidden: false,
            },
        ];
        let ir = build_native_ir(&nodes, &ios_profile()).expect("IR");
        assert_eq!(ir.nodes.len(), 3);
        let export =
            export_apple_ui(&ir, &AppleOptions::new(AppleDialect::SwiftUi)).expect("export");
        assert!(export
            .source
            .contains("VStack(alignment: .leading, spacing: 0)"));
        assert!(export.source.contains("Text(verbatim: \"안녕\")"));
        assert!(export.source.contains(".offset(x: 4, y: 0)"));
        assert!(export
            .source
            .contains(".frame(width: 200, alignment: .topLeading)"));
        let broken = vec![NativeNodeInput {
            declarations: vec![("display", "flex".to_string())],
            ..nodes[0].clone()
        }];
        assert!(matches!(
            build_native_ir(&broken, &ios_profile()),
            Err(NativeInputError::Declarations { .. })
        ));
    }

    #[test]
    fn native_input_fills_css_uses_and_behaviors() {
        // ADR-0121 결정 3: 생략값과 다른 layout longhand와 시각 이동이 CSS 속성 사용이 되고, IR이 matrix 판정을
        // 붙입니다. 상태·motion·Context 입력은 그대로 IR에 들어갑니다.
        use crate::ir::{
            IrMotionStart, IrOwner, IrReducedMotion, IrTiming, MappingSource, MappingSubject,
        };
        use crate::matrix::css_entry;
        use stellar_core_types::typed_value::TypedValue;
        use stellar_target_profile::css_matrix::RegistryInput;
        use stellar_target_profile::profile::TargetProfileKind;

        let nodes = vec![
            NativeNodeInput {
                id: sid(1),
                parent: None,
                element: ElementKind::FRAME,
                declarations: declarations(&[
                    ("display", "flex"),
                    ("flex-direction", "column"),
                    ("flex-wrap", "wrap"),
                    ("width", "200px"),
                ]),
                translation: (2.0, 0.0),
                text: None,
                hidden: false,
            },
            NativeNodeInput {
                id: sid(3),
                parent: Some(sid(1)),
                element: ElementKind::TEXT,
                // Text의 container longhand(display)는 사용으로 보지 않습니다.
                declarations: declarations(&[("display", "flex"), ("flex-grow", "1")]),
                translation: (4.0, 0.0),
                text: Some("안녕".to_string()),
                hidden: false,
            },
        ];
        let state = sid(40);
        let behaviors = NativeBehaviors {
            states: vec![StateDecl {
                id: state,
                name: "open".to_string(),
                initial: TypedValue::Boolean(false),
            }],
            machines: Vec::new(),
            motions: vec![crate::ir::IrMotion {
                id: sid(41),
                owner: IrOwner::View(sid(1)),
                targets: vec![
                    PropertyPath::new(sid(3)).push(stellar_core_types::property::PropertyKey(4))
                ],
                start: IrMotionStart::Changes { state },
                duration_ms: 200.0,
                delay_ms: 0.0,
                timing: IrTiming::Linear,
                reduced_motion: IrReducedMotion::Instant,
            }],
            contexts: IrContexts::default(),
        };
        for profile in [ios_profile(), crate::fixture::android_profile()] {
            let ir = build_native_view_ir(&nodes, &behaviors, &profile).expect("IR");
            assert_eq!(ir.states.len(), 1);
            assert_eq!(ir.motions.len(), 1);
            let names = |index: usize| -> Vec<&str> {
                ir.nodes[index]
                    .mappings
                    .iter()
                    .filter(|m| m.source == MappingSource::Matrix)
                    .filter_map(|m| match m.subject {
                        MappingSubject::Css { name, .. } => Some(name),
                        _ => None,
                    })
                    .collect()
            };
            let root = names(0);
            for name in [
                "display",
                "flex-direction",
                "flex-wrap",
                "width",
                "transform",
            ] {
                assert!(root.contains(&name), "{name}: {root:?}");
            }
            assert!(
                !root.contains(&"grid-row-end"),
                "생략값 longhand는 사용이 아닙니다"
            );
            let text = names(1);
            assert!(text.contains(&"flex-grow"));
            // local transform registry 항목은 Frame(`frame/transform`)에만 있습니다.
            assert!(!text.contains(&"transform"));
            assert!(!text.contains(&"display"));
            // 판정은 matrix 그대로입니다(Apple은 flex-wrap Unsupported, Android는 Native).
            let kind = profile.kind();
            let wrap = ir.nodes[0]
                .mappings
                .iter()
                .find(|m| {
                    matches!(
                        m.subject,
                        MappingSubject::Css {
                            name: "flex-wrap",
                            ..
                        }
                    )
                })
                .expect("flex-wrap");
            let expected = css_entry(
                RegistryInput {
                    scope: "frame",
                    name: "flex-wrap",
                    status: MappingInput::Exact,
                },
                kind,
            );
            assert_eq!(wrap.status, expected.status);
            assert_eq!(wrap.rule, expected.rule);
            assert_eq!(
                wrap.status,
                Some(match kind {
                    TargetProfileKind::Android => {
                        stellar_target_profile::capability::CapabilityStatus::Native
                    }
                    _ => stellar_target_profile::capability::CapabilityStatus::Unsupported,
                })
            );
        }
        // 빈 입력은 상태·motion이 없고 CSS 속성 사용은 그대로 채웁니다.
        let plain = build_native_ir(&nodes, &ios_profile()).expect("IR");
        assert!(plain.motions.is_empty());
        assert!(!plain.nodes[0].mappings.is_empty());
    }
}
