//! [`CodegenIr`] semantic snapshot 텍스트 (XXII.1.1).
//!
//! fixture 비교·리뷰용 결정론 텍스트입니다(파일 형식 아님). 같은 IR은 항상 같은 텍스트입니다. 수는 Rust `f32`
//! 최단 왕복 10진 표기(지수 없음), 문자열은 escape한 따옴표 표기, StableId는 하이픈 표기입니다.
//!
//! ```text
//! codegen-ir v3
//! target <profile id> <kind> <spec> registry=<release>
//! tokens <n>
//!   token <id> <name> = <value>
//! states <n>
//!   state <id> <name> = <initial>
//! nodes <n>
//!   [i] <id> <element> contracts=[..] semantics=a11y[j]     (깊이마다 두 칸 들여쓰기)
//!     layout {<CSS 생략값과 다른 longhand; ...>} [transform=..] [content=..]
//!     text <string>
//!     property <path> = <value> origin=<origin> [token=<id>] [capability=<name>]
//!     mapping <validator|matrix> <subject> <status> rule=<rule> [diagnostic=<code>] [replacement=<subject>] [path=<path>]
//! accessibility <n>
//!   [j] node=[i] <role> [label=..] [value=..] [states{..}] [actions{..}]   (깊이마다 두 칸 들여쓰기)
//! ```

use std::fmt::Write as _;

use stellar_accessibility::node::{AccessibilityBinding, ActionTarget};
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PathSegment, PropertyPath};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::Transform2D;
use stellar_layout::input::LayoutContent;
use stellar_layout::intrinsic::MeasuredContent;
use stellar_target_profile::builtin::property_name;
use stellar_target_profile::capability::{CapabilitySubject, ContractCode, ElementKind};
use stellar_target_profile::overrides::ValueOrigin;
use stellar_target_profile::spec::ProfileSpec;

use super::{CodegenIr, IrLayout, MappingSubject};
use crate::text::num;

/// snapshot 형식 머리.
pub const SNAPSHOT_HEADER: &str = "codegen-ir v3";

impl CodegenIr {
    /// 결정론 semantic snapshot 텍스트(모듈 문서 형식).
    pub fn semantic_snapshot(&self) -> String {
        let mut out = String::new();
        out.push_str(SNAPSHOT_HEADER);
        out.push('\n');
        let profile = &self.target.profile;
        let _ = writeln!(
            out,
            "target {} {} {} registry={}",
            profile.id.as_str(),
            profile.kind().name(),
            spec_text(&profile.spec),
            self.target.registry_version.get()
        );
        let _ = writeln!(out, "tokens {}", self.tokens.len());
        for token in &self.tokens {
            let _ = writeln!(
                out,
                "  token {} {:?} = {}",
                token.id,
                token.name,
                value_text(&token.value)
            );
        }
        let _ = writeln!(out, "states {}", self.states.len());
        for state in &self.states {
            let _ = writeln!(
                out,
                "  state {} {:?} = {}",
                state.id,
                state.name,
                value_text(&state.initial)
            );
        }
        let _ = writeln!(out, "nodes {}", self.nodes.len());
        for (index, node) in self.nodes.iter().enumerate() {
            let indent = "  ".repeat(node.depth as usize + 1);
            let _ = write!(
                out,
                "{indent}[{index}] {} {}",
                node.id,
                element_name(node.element)
            );
            if !node.contracts.is_empty() {
                let names: Vec<String> = node.contracts.iter().map(|c| contract_name(*c)).collect();
                let _ = write!(out, " contracts=[{}]", names.join(","));
            }
            if let Some(entry) = node.semantics {
                let _ = write!(out, " semantics=a11y[{entry}]");
            }
            out.push('\n');
            if let Some(layout) = &node.layout {
                let _ = writeln!(out, "{indent}  layout {}", layout_text(layout));
            }
            if let Some(text) = &node.text {
                let _ = writeln!(out, "{indent}  text {text:?}");
            }
            for property in &node.properties {
                let _ = write!(
                    out,
                    "{indent}  property {} = {} origin={}",
                    path_text(&property.path),
                    value_text(&property.value),
                    origin_text(property.origin)
                );
                if let Some(token) = property.token {
                    let _ = write!(out, " token={token}");
                }
                if let Some(capability) = property.capability {
                    let _ = write!(
                        out,
                        " capability={}",
                        subject_text(CapabilitySubject::Property(capability))
                    );
                }
                out.push('\n');
            }
            for mapping in &node.mappings {
                let status = mapping.status.map_or("not_registered", |s| s.name());
                let subject = match mapping.subject {
                    MappingSubject::Capability(subject) => subject_text(subject),
                    MappingSubject::Css { scope, name } => format!("css:{scope}/{name}"),
                };
                let _ = write!(
                    out,
                    "{indent}  mapping {} {subject} {status} rule={}",
                    mapping.source.name(),
                    mapping.rule
                );
                if let Some(code) = mapping.diagnostic {
                    let _ = write!(out, " diagnostic={code}");
                }
                if let Some(replacement) = mapping.replacement {
                    let _ = write!(out, " replacement={}", subject_text(replacement));
                }
                if let Some(path) = &mapping.path {
                    let _ = write!(out, " path={}", path_text(path));
                }
                out.push('\n');
            }
        }
        let _ = writeln!(out, "accessibility {}", self.accessibility.len());
        for (index, entry) in self.accessibility.iter().enumerate() {
            let indent = "  ".repeat(entry.depth as usize + 1);
            let semantics = &entry.semantics;
            let _ = write!(
                out,
                "{indent}[{index}] node=[{}] {}",
                entry.node, semantics.role
            );
            if let Some(label) = &semantics.label {
                let _ = write!(out, " label={}", binding_text(label));
            }
            if let Some(value) = &semantics.value {
                let _ = write!(out, " value={}", binding_text(value));
            }
            if !semantics.states.is_empty() {
                let states: Vec<String> = semantics
                    .states
                    .iter()
                    .map(|(state, binding)| format!("{state}={}", binding_text(binding)))
                    .collect();
                let _ = write!(out, " states{{{}}}", states.join(","));
            }
            if !semantics.actions.is_empty() {
                let actions: Vec<String> = semantics
                    .actions
                    .iter()
                    .map(|(action, target)| format!("{action}={}", target_text(*target)))
                    .collect();
                let _ = write!(out, " actions{{{}}}", actions.join(","));
            }
            out.push('\n');
        }
        out
    }
}

fn spec_text(spec: &ProfileSpec) -> String {
    match spec {
        ProfileSpec::Web(web) => format!(
            "baseline={} capability_set={}",
            web.baseline.year(),
            web.capability_set_version.get()
        ),
        ProfileSpec::Apple(apple) => format!(
            "platform={} minimum_os={}.{}.{}",
            apple.platform.name(),
            apple.minimum_os.major(),
            apple.minimum_os.minor(),
            apple.minimum_os.patch()
        ),
        ProfileSpec::Android(android) => format!(
            "api_level={} ui_system={}.{}.{}",
            android.api_level.get(),
            android.ui_system_version.major(),
            android.ui_system_version.minor(),
            android.ui_system_version.patch()
        ),
    }
}

/// element 종류의 안정 이름.
pub fn element_name(kind: ElementKind) -> String {
    match kind {
        ElementKind::FRAME => "frame".to_string(),
        ElementKind::FRAGMENT => "fragment".to_string(),
        ElementKind::SHAPE => "shape".to_string(),
        ElementKind::PATH => "path".to_string(),
        ElementKind::TEXT => "text".to_string(),
        ElementKind::IMAGE => "image".to_string(),
        ElementKind::COMPONENT_INSTANCE => "component_instance".to_string(),
        other => format!("element:{}", other.0),
    }
}

/// contract 종류의 안정 이름.
pub fn contract_name(code: ContractCode) -> String {
    match code {
        ContractCode::BUTTON => "button".to_string(),
        ContractCode::TEXT_INPUT => "text_input".to_string(),
        ContractCode::MODAL => "modal".to_string(),
        ContractCode::LIST => "list".to_string(),
        ContractCode::SINGLE_SELECTION_GROUP => "single_selection_group".to_string(),
        other => format!("contract:{}", other.0),
    }
}

fn subject_text(subject: CapabilitySubject) -> String {
    match subject {
        CapabilitySubject::Element(kind) => format!("element:{}", element_name(kind)),
        CapabilitySubject::Contract(code) => format!("contract:{}", contract_name(code)),
        CapabilitySubject::Property(id) => match property_name(id) {
            Some(name) => format!("property:{name}"),
            None => format!("property:{:#010x}", id.0),
        },
    }
}

fn origin_text(origin: ValueOrigin) -> String {
    match origin {
        ValueOrigin::Common => "common".to_string(),
        ValueOrigin::Profile => "profile".to_string(),
        ValueOrigin::Variant(id) => format!("variant({id})"),
        ValueOrigin::ProfileVariant(id) => format!("profile_variant({id})"),
    }
}

/// 속성 경로 표기: `<root>.<key>...[index]` 또는 `<root>.<key>...["key"]`.
pub fn path_text(path: &PropertyPath) -> String {
    let mut out = path.root().to_string();
    for key in path.keys() {
        let _ = write!(out, ".{}", key.0);
    }
    match path.tail() {
        Some(PathSegment::Index(index)) => {
            let _ = write!(out, "[{index}]");
        }
        Some(PathSegment::Key(key)) => {
            let _ = write!(out, "[{key:?}]");
        }
        None => {}
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn color_text(color: &Color) -> String {
    let space = match color.space() {
        ColorSpace::Srgb => "srgb",
        ColorSpace::DisplayP3 => "display-p3",
    };
    format!(
        "{space}({} {} {} / {})",
        num(color.r()),
        num(color.g()),
        num(color.b()),
        num(color.alpha())
    )
}

/// 값 표기.
pub fn value_text(value: &TypedValue) -> String {
    match value {
        TypedValue::Boolean(flag) => flag.to_string(),
        TypedValue::Number(number) => num(number.get()),
        TypedValue::String(text) => format!("{text:?}"),
        TypedValue::Enum(value) => format!("enum({:#010x}:{})", value.enum_type(), value.variant()),
        TypedValue::Length(length) => format!("{}px", num(length.get())),
        TypedValue::Percentage(percentage) => format!("{}%", num(percentage.get())),
        TypedValue::Angle(angle) => format!("{}deg", num(angle.get())),
        TypedValue::Duration(duration) => format!("duration({})", num(duration.get())),
        TypedValue::Color(color) => color_text(color),
        TypedValue::ResourceRef(resource) => format!("resource({})", resource.id()),
        TypedValue::Gradient(_) | TypedValue::Shadow(_) | TypedValue::Typography(_) => {
            format!(
                "value:{:#04x}({})",
                value.type_tag(),
                hex(&value.to_bytes())
            )
        }
    }
}

fn binding_text(binding: &AccessibilityBinding) -> String {
    match binding {
        AccessibilityBinding::Constant(value) => value_text(value),
        AccessibilityBinding::Property(path) => format!("property({})", path_text(path)),
        AccessibilityBinding::ComponentProperty(id) => format!("component_property({id})"),
        AccessibilityBinding::State(id) => format!("state({id})"),
        AccessibilityBinding::NodeText(id) => format!("text({id})"),
        AccessibilityBinding::SelectionMatches { selection, item } => {
            format!("selection({selection}=={item})")
        }
    }
}

fn target_text(target: ActionTarget) -> String {
    match target {
        ActionTarget::None => "none".to_string(),
        ActionTarget::SemanticAction(id) => format!("action({id})"),
        ActionTarget::State(id) => format!("state({id})"),
    }
}

fn content_text(content: &LayoutContent) -> Option<String> {
    match content {
        LayoutContent::None => None,
        LayoutContent::Intrinsic(intrinsic) => Some(format!(
            "intrinsic({}x{}{})",
            num(intrinsic.width.get()),
            num(intrinsic.height.get()),
            intrinsic
                .baseline
                .map(|b| format!(" baseline={}", num(b.get())))
                .unwrap_or_default()
        )),
        LayoutContent::Measured(measured) => Some(match measured {
            MeasuredContent::Shape(shape) => format!(
                "measured(shape {}x{})",
                num(shape.width.get()),
                num(shape.height.get())
            ),
            MeasuredContent::Text(text) => format!(
                "measured(text words={} line_height={})",
                text.words.len(),
                num(text.line_height.get())
            ),
            MeasuredContent::Image(image) => format!(
                "measured(image {}x{})",
                num(image.width.get()),
                num(image.height.get())
            ),
            MeasuredContent::ComponentInstance(_) => "measured(component_instance)".to_string(),
        }),
    }
}

fn layout_text(layout: &IrLayout) -> String {
    let mut out = format!("{{{}}}", layout.style.declaration_text());
    if layout.transform != Transform2D::identity() {
        let (a, b, c, d, tx, ty) = layout.transform.components();
        let _ = write!(
            out,
            " transform=({},{},{},{},{},{})",
            num(a),
            num(b),
            num(c),
            num(d),
            num(tx),
            num(ty)
        );
    }
    if let Some(content) = content_text(&layout.content) {
        let _ = write!(out, " content={content}");
    }
    out
}

/// StableId 짧은 표기(adapter 식별자용 32자리 소문자 16진).
pub fn id_hex(id: StableId) -> String {
    hex(&id.to_bytes())
}
