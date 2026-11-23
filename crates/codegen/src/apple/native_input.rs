//! 저장 View 투영 → CodegenIR 입력 (ADR-0107 결정 8).
//!
//! 문서 계층(ops)이 저장 View를 Web 투영과 같은 규칙으로 읽어 node마다 [`NativeNodeInput`]을 넘기면, 이 함수가
//! [`CodegenSources`]를 채워 [`build_ir`]를 부릅니다. layout 값은 문서 정규 formatter가 쓴 40개 longhand 선언 목록이고
//! [`stellar_layout::declared::style_from_declarations`]로만 엔진 입력이 됩니다(값 parser를 새로 두지 않음). 접근성은
//! [`stellar_accessibility::generate`]로 만듭니다.
//!
//! 문서가 아직 복원하지 못하는 control contract·상태·token·Variant·target capability 속성은 채우지 않습니다(ADR-0049,
//! XXII.1.1). 그래서 이 입력의 IR은 구조·layout·텍스트·기본 접근성만 담습니다.

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

use crate::ir::{build_ir, CodegenIr, CodegenSources, IrError};

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

/// 입력에서 IR을 만듭니다.
pub fn build_native_ir(
    nodes: &[NativeNodeInput],
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
        states: &[],
        tokens: &[],
        token_bindings: &BTreeMap::new(),
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
}
