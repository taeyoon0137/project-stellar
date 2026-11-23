//! 프로토타입 socket 종류와 속성 입력 포트 타입 (XXX.1.2, ADR-0025 결정 1.2·2.2, ADR-0094 결정 2).
//!
//! socket은 **State**(시간에 걸쳐 유지되는 `TypedValue`, expression으로 낮춤)와 **Pulse**(한 번 발생하는 사건,
//! EventKind·semantic emit으로 낮춤) 두 종류입니다. 두 종류는 직접 연결하지 못하고, 등록된 변환 node
//! ([`super::node_kind::ConversionKind`])만 종류를 바꿉니다([`check_socket_kinds`]).
//!
//! 속성 입력 포트의 State 타입은 registry 항목의 CSS 값 문법을 `ExpressionType`으로 바꾼 **집합**입니다
//! ([`property_port`]). 포트 하나는 CSS longhand 하나이므로, 같은 longhand(`width` 등)는 저장 위치(payload 크기
//! key 또는 layout record key)와 관계없이 layout longhand 문법([`LayoutLonghand::grammar`])을 씁니다. 합집합
//! 문법(`auto | <length> | <percentage> | …`)은 구성 타입별 `ExpressionType`의 집합이고, keyword 구성 요소는 그
//! longhand의 keyword만 담은 Enum 하나로 모읍니다([`keyword_enum_type`]). 대응이 없는 문법(`<track-size>`,
//! `<transform-list>`, grid line 등)의 속성은 포트로 노출하지 않고 [`PortDiagnostic`]으로 진단합니다.
//!
//! 연결 검사([`check_state_connection`])는 연결된 식의 **정적 타입**이 집합에 속하는지만 봅니다. 저장된 값의
//! variant를 보지 않으므로 값을 Length에서 Percentage로 바꿔도 결과가 같습니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::property::PropertyKey;
use stellar_expressions::types::{EnumTypeId, ExpressionType, TypeSpec};

use crate::css::grammar::CssGrammar;
use crate::css::mapping::MappingStatus;
use crate::css::registry::{self, PropertyScope, RegistryEntry};
use crate::layout_record::{LayoutGrammar, LayoutLonghand};
use crate::node_kind::NodeKind;

/// socket 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SocketKind {
    /// 시간에 걸쳐 유지되는 값.
    State,
    /// 한 번 발생하는 사건.
    Pulse,
}

impl SocketKind {
    /// 안정 tag.
    pub const fn tag(self) -> u8 {
        match self {
            SocketKind::State => 0x01,
            SocketKind::Pulse => 0x02,
        }
    }

    /// 표시 이름.
    pub const fn name(self) -> &'static str {
        match self {
            SocketKind::State => "state",
            SocketKind::Pulse => "pulse",
        }
    }
}

impl fmt::Display for SocketKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 포트 대상의 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetKind {
    /// node 속성(root = node StableId).
    Node(NodeKind),
    /// modifier 속성(root = modifier StableId).
    Modifier,
}

/// keyword Enum 타입 번호 namespace(ADR-0094 결정 2). 하위 16비트는 longhand 번호입니다.
pub const KEYWORD_ENUM_NAMESPACE: u32 = 0xC000_0000;

/// longhand 번호 `id`의 keyword Enum 타입.
///
/// layout longhand는 [`LayoutLonghand::id`], 그 밖은 registry key 값을 씁니다. 같은 longhand는 저장 위치와 관계없이
/// 같은 번호입니다(`width`는 언제나 [`LayoutLonghand::Width`]).
pub const fn keyword_enum_type(id: u16) -> EnumTypeId {
    EnumTypeId::new(KEYWORD_ENUM_NAMESPACE | id as u32)
}

/// 속성 입력 포트 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyPort {
    /// registry key(저장 위치).
    pub key: PropertyKey,
    /// CSS longhand 이름(또는 Stellar 이름).
    pub name: &'static str,
    /// 받는 `ExpressionType` 집합(비어 있지 않음).
    pub types: BTreeSet<ExpressionType>,
    /// keyword 구성 요소(목록 index = Enum variant). 없으면 빈 slice.
    pub keywords: &'static [&'static str],
}

impl PropertyPort {
    /// 타입 집합에 `value_type`이 있는지.
    pub fn accepts(&self, value_type: ExpressionType) -> bool {
        self.types.contains(&value_type)
    }

    /// keyword Enum 타입(keyword 구성 요소가 있을 때).
    pub fn keyword_type(&self) -> Option<EnumTypeId> {
        self.types.iter().find_map(|t| match t {
            ExpressionType::Enum(id) => Some(*id),
            _ => None,
        })
    }

    /// 포트 타입 golden 표기: `name: type | type …`(타입 순서는 `ExpressionType` 순).
    pub fn golden_line(&self) -> String {
        let types: Vec<String> = self
            .types
            .iter()
            .map(|t| match t {
                ExpressionType::Enum(_) => format!("enum({})", self.keywords.join(" ")),
                other => type_name(*other).to_string(),
            })
            .collect();
        format!("{}: {}", self.name, types.join(" | "))
    }
}

/// `ExpressionType`의 안정 표시 이름.
pub const fn type_name(value_type: ExpressionType) -> &'static str {
    match value_type {
        ExpressionType::Boolean => "boolean",
        ExpressionType::Number => "number",
        ExpressionType::String => "string",
        ExpressionType::Enum(_) => "enum",
        ExpressionType::Length => "length",
        ExpressionType::Percentage => "percentage",
        ExpressionType::Angle => "angle",
        ExpressionType::Duration => "duration",
        ExpressionType::Color => "color",
        ExpressionType::Gradient => "gradient",
        ExpressionType::Shadow => "shadow",
        ExpressionType::Typography => "typography",
        ExpressionType::ResourceRef => "resource-ref",
    }
}

/// 포트로 노출하지 않는 속성의 진단.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDiagnostic {
    /// 안정 진단 코드.
    pub code: &'static str,
    /// 대상 key.
    pub key: PropertyKey,
    /// 사람이 읽는 이유.
    pub message: String,
}

impl fmt::Display for PortDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (key 0x{:04x}): {}",
            self.code, self.key.0, self.message
        )
    }
}

impl std::error::Error for PortDiagnostic {}

/// `Size` 문법의 keyword.
pub const SIZE_KEYWORDS: &[&str] = &["auto", "min-content", "max-content", "fit-content"];
/// `MaxSize` 문법의 keyword.
pub const MAX_SIZE_KEYWORDS: &[&str] = &["none", "min-content", "max-content", "fit-content"];
/// `Inset` 문법의 keyword.
pub const INSET_KEYWORDS: &[&str] = &["auto"];
/// `FlexBasis` 문법의 keyword.
pub const FLEX_BASIS_KEYWORDS: &[&str] = &["auto", "content"];
/// `Gap` 문법의 keyword.
pub const GAP_KEYWORDS: &[&str] = &["normal"];

fn set(types: &[ExpressionType]) -> BTreeSet<ExpressionType> {
    types.iter().copied().collect()
}

fn unexposed(key: PropertyKey, code: &'static str, message: String) -> PortDiagnostic {
    PortDiagnostic { code, key, message }
}

/// layout longhand 문법의 포트 타입.
fn layout_port(key: PropertyKey, longhand: LayoutLonghand) -> Result<PropertyPort, PortDiagnostic> {
    use ExpressionType as T;
    let enum_type = T::Enum(keyword_enum_type(longhand.id()));
    let (types, keywords): (BTreeSet<ExpressionType>, &'static [&'static str]) =
        match longhand.grammar() {
            LayoutGrammar::Keyword(list) => (set(&[enum_type]), list),
            LayoutGrammar::Inset => (set(&[enum_type, T::Length, T::Percentage]), INSET_KEYWORDS),
            LayoutGrammar::Size => (set(&[enum_type, T::Length, T::Percentage]), SIZE_KEYWORDS),
            LayoutGrammar::MaxSize => (
                set(&[enum_type, T::Length, T::Percentage]),
                MAX_SIZE_KEYWORDS,
            ),
            LayoutGrammar::FlexBasis => (
                set(&[enum_type, T::Length, T::Percentage]),
                FLEX_BASIS_KEYWORDS,
            ),
            LayoutGrammar::Gap => (set(&[enum_type, T::Length]), GAP_KEYWORDS),
            LayoutGrammar::NonNegativeNumber => (set(&[T::Number]), &[]),
            LayoutGrammar::NonNegativeLength => (set(&[T::Length]), &[]),
            LayoutGrammar::AspectRatio => {
                return Err(unexposed(
                    key,
                    "port-unmapped-grammar",
                    format!(
                        "{}: `<number> / <number>`에 대응하는 값 타입이 없습니다",
                        longhand.name()
                    ),
                ))
            }
            LayoutGrammar::GridLine => {
                return Err(unexposed(
                    key,
                    "port-unmapped-grammar",
                    format!(
                        "{}: grid line(`span <integer>`)에 대응하는 값 타입이 없습니다",
                        longhand.name()
                    ),
                ))
            }
            LayoutGrammar::TrackList | LayoutGrammar::AutoTracks => {
                return Err(unexposed(
                    key,
                    "port-unmapped-grammar",
                    format!(
                        "{}: `<track-size>`에 대응하는 값 타입이 없습니다",
                        longhand.name()
                    ),
                ))
            }
        };
    Ok(PropertyPort {
        key,
        name: longhand.name(),
        types,
        keywords,
    })
}

/// registry 문법의 포트 타입.
fn registry_port(entry: &RegistryEntry) -> Result<PropertyPort, PortDiagnostic> {
    use ExpressionType as T;
    let key = entry.key;
    // 같은 CSS longhand는 layout longhand 문법 하나를 씁니다(payload 크기 `width`·`height`). 다른 값을 같은 CSS
    // 속성으로 바꿔 쓰는 항목(`visible` → `display`)은 자기 문법을 씁니다.
    let sizing = matches!(
        entry.css.grammar,
        Some(CssGrammar::Length { sizing: true, .. })
    );
    if let ([name], true) = (entry.css.longhands, sizing) {
        if let Some(longhand) = LayoutLonghand::from_name(name) {
            return layout_port(key, longhand).map(|port| PropertyPort {
                key,
                name: entry.name,
                ..port
            });
        }
    }
    if !entry.bundle.is_empty() {
        return Err(unexposed(
            key,
            "port-bundle-key",
            format!(
                "{}: 묶음 key는 나뉜 longhand 포트로만 연결합니다",
                entry.name
            ),
        ));
    }
    if entry.scope == PropertyScope::Node && entry.css.status == MappingStatus::StellarOnly {
        return Err(unexposed(
            key,
            "port-editor-metadata",
            format!("{}: 편집기 전용 필드는 포트가 아닙니다", entry.name),
        ));
    }
    let Some(grammar) = entry.css.grammar else {
        return Err(unexposed(
            key,
            "port-no-value-grammar",
            format!("{}: 값 문법이 없는 참조·구조 값입니다", entry.name),
        ));
    };
    let id = key.0 as u16;
    let (types, keywords): (BTreeSet<ExpressionType>, &'static [&'static str]) = match grammar {
        CssGrammar::Length { .. } => (set(&[T::Length]), &[]),
        CssGrammar::Percentage => (set(&[T::Percentage]), &[]),
        CssGrammar::Angle => (set(&[T::Angle]), &[]),
        CssGrammar::Duration => (set(&[T::Duration]), &[]),
        CssGrammar::Number | CssGrammar::UnitInterval | CssGrammar::Count { .. } => {
            (set(&[T::Number]), &[])
        }
        CssGrammar::Color { .. } => (set(&[T::Color]), &[]),
        CssGrammar::Keyword(list) => (set(&[T::Enum(keyword_enum_type(id))]), list),
        CssGrammar::Bool => (set(&[T::Boolean]), &[]),
        CssGrammar::Text => (set(&[T::String]), &[]),
        CssGrammar::Transform | CssGrammar::Border | CssGrammar::Corners => {
            return Err(unexposed(
                key,
                "port-unmapped-grammar",
                format!(
                    "{}: `<{}>`에 대응하는 값 타입이 없습니다",
                    entry.name,
                    grammar.name()
                ),
            ))
        }
    };
    Ok(PropertyPort {
        key,
        name: entry.name,
        types,
        keywords,
    })
}

fn scope_of(kind: NodeKind) -> Option<PropertyScope> {
    match kind {
        NodeKind::Frame => Some(PropertyScope::Frame),
        NodeKind::Shape => Some(PropertyScope::Shape),
        NodeKind::Path => Some(PropertyScope::Path),
        NodeKind::Text => Some(PropertyScope::Text),
        NodeKind::Image => Some(PropertyScope::Image),
        NodeKind::Fragment | NodeKind::ComponentInstance => None,
    }
}

/// `target`의 `key` 속성 입력 포트.
///
/// 찾는 순서: layout record key(그 종류에 붙는 longhand) → node 공통 registry → 종류별 registry(modifier는 modifier
/// registry). 어디에도 없으면 `port-unknown-property`입니다.
pub fn property_port(target: TargetKind, key: PropertyKey) -> Result<PropertyPort, PortDiagnostic> {
    match target {
        TargetKind::Modifier => match registry::entry(PropertyScope::Modifier, key) {
            Some(entry) => registry_port(&entry),
            None => Err(unknown(key)),
        },
        TargetKind::Node(kind) => {
            // Frame·Image의 `width`·`height`는 payload 크기 key가 저장 위치이므로 record key로는 찾지 않습니다(ADR-0086).
            let payload_sized = matches!(kind, NodeKind::Frame | NodeKind::Image);
            if let Some(longhand) = LayoutLonghand::ALL
                .iter()
                .copied()
                .filter(|l| {
                    !(payload_sized && matches!(l, LayoutLonghand::Width | LayoutLonghand::Height))
                })
                .find(|l| l.record_key() == key && l.applies_to(kind))
            {
                return layout_port(key, longhand);
            }
            if let Some(entry) = registry::entry(PropertyScope::Node, key) {
                return registry_port(&entry);
            }
            match scope_of(kind).and_then(|scope| registry::entry(scope, key)) {
                Some(entry) => registry_port(&entry),
                None => Err(unknown(key)),
            }
        }
    }
}

fn unknown(key: PropertyKey) -> PortDiagnostic {
    unexposed(
        key,
        "port-unknown-property",
        "대상 종류에 없는 속성입니다".to_string(),
    )
}

/// 연결 검사 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionError {
    /// State와 Pulse를 직접 연결했습니다.
    SocketKindMismatch {
        /// 출력 socket 종류.
        from: SocketKind,
        /// 입력 socket 종류.
        to: SocketKind,
    },
    /// 식의 정적 타입이 포트 집합에 없습니다.
    TypeNotAccepted {
        /// 식의 정적 타입.
        found: TypeSpec,
        /// 포트.
        port: &'static str,
    },
    /// nullable 식은 속성 포트에 연결할 수 없습니다.
    Nullable {
        /// 식의 정적 타입.
        found: TypeSpec,
    },
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectionError::SocketKindMismatch { from, to } => write!(
                f,
                "{from} socket을 {to} socket에 직접 연결할 수 없습니다(변환 node가 필요합니다)"
            ),
            ConnectionError::TypeNotAccepted { found, port } => {
                write!(f, "{port} 포트는 {found:?} 식을 받지 않습니다")
            }
            ConnectionError::Nullable { found } => {
                write!(f, "nullable 식({found:?})은 속성 포트에 연결할 수 없습니다")
            }
        }
    }
}

impl std::error::Error for ConnectionError {}

/// 출력 socket 종류 `from`을 입력 socket 종류 `to`에 바로 연결할 수 있는지.
pub fn check_socket_kinds(from: SocketKind, to: SocketKind) -> Result<(), ConnectionError> {
    if from == to {
        Ok(())
    } else {
        Err(ConnectionError::SocketKindMismatch { from, to })
    }
}

/// 정적 타입 `found`인 식을 속성 포트 `port`에 연결해도 되는지. 통과하면 저장 값 variant의 타입을 돌려줍니다.
pub fn check_state_connection(
    port: &PropertyPort,
    found: TypeSpec,
) -> Result<ExpressionType, ConnectionError> {
    let TypeSpec::Required(value_type) = found else {
        return Err(ConnectionError::Nullable { found });
    };
    if port.accepts(value_type) {
        Ok(value_type)
    } else {
        Err(ConnectionError::TypeNotAccepted {
            found,
            port: port.name,
        })
    }
}

/// 모든 node 종류·modifier의 포트 golden 텍스트(노출 포트와 진단 모두, 결정론적 순서).
pub fn port_golden_text() -> String {
    let mut lines = Vec::new();
    let mut targets: Vec<(String, TargetKind, Vec<PropertyKey>)> = Vec::new();
    for kind in NodeKind::ALL {
        let mut keys: Vec<PropertyKey> = LayoutLonghand::ALL
            .iter()
            .filter(|l| l.applies_to(kind))
            .map(|l| l.record_key())
            .collect();
        keys.extend(
            registry::entries()
                .into_iter()
                .filter(|e| e.scope == PropertyScope::Node || Some(e.scope) == scope_of(kind))
                .map(|e| e.key),
        );
        keys.sort();
        keys.dedup();
        targets.push((
            format!("{kind:?}").to_lowercase(),
            TargetKind::Node(kind),
            keys,
        ));
    }
    let modifier_keys = registry::entries()
        .into_iter()
        .filter(|e| e.scope == PropertyScope::Modifier)
        .map(|e| e.key)
        .collect();
    targets.push(("modifier".into(), TargetKind::Modifier, modifier_keys));
    for (label, target, keys) in targets {
        for key in keys {
            match property_port(target, key) {
                Ok(port) => lines.push(format!("{label} 0x{:04x} {}", key.0, port.golden_line())),
                Err(diag) => lines.push(format!("{label} 0x{:04x} ! {}", key.0, diag.code)),
            }
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_record::LayoutValue;

    fn node(kind: NodeKind) -> TargetKind {
        TargetKind::Node(kind)
    }

    #[test]
    fn port_types_match_golden() {
        // 검증 기준(XXX.1.2): 값 문법별 포트 타입 golden.
        let golden = include_str!("port_golden.txt");
        let text = port_golden_text();
        if text != golden {
            panic!("포트 golden이 다릅니다. 새 값:\n{text}");
        }
    }

    #[test]
    fn width_accepts_length_and_percentage_but_not_color() {
        // Frame payload width(0x0108)와 Rectangle record 없는 width가 같은 longhand 문법을 씁니다.
        let port = property_port(node(NodeKind::Frame), PropertyKey(0x0108)).expect("width");
        assert_eq!(port.name, "width");
        let length = TypeSpec::required(ExpressionType::Length);
        let percentage = TypeSpec::required(ExpressionType::Percentage);
        let color = TypeSpec::required(ExpressionType::Color);
        assert_eq!(
            check_state_connection(&port, length),
            Ok(ExpressionType::Length)
        );
        assert_eq!(
            check_state_connection(&port, percentage),
            Ok(ExpressionType::Percentage)
        );
        assert!(matches!(
            check_state_connection(&port, color),
            Err(ConnectionError::TypeNotAccepted { .. })
        ));
        let text_width =
            property_port(node(NodeKind::Text), LayoutLonghand::Width.record_key()).expect("width");
        assert_eq!(text_width.types, port.types);
        assert_eq!(
            port.keyword_type(),
            Some(keyword_enum_type(LayoutLonghand::Width.id()))
        );
    }

    #[test]
    fn unmapped_grammar_is_diagnosed() {
        let tracks = property_port(
            node(NodeKind::Frame),
            LayoutLonghand::GridTemplateColumns.record_key(),
        );
        assert_eq!(tracks.map_err(|d| d.code), Err("port-unmapped-grammar"));
        let transform = property_port(node(NodeKind::Frame), PropertyKey(0x0102));
        assert_eq!(transform.map_err(|d| d.code), Err("port-unmapped-grammar"));
        let size = property_port(node(NodeKind::Frame), PropertyKey(0x0101));
        assert_eq!(size.map_err(|d| d.code), Err("port-bundle-key"));
        let visible = property_port(node(NodeKind::Frame), PropertyKey(0x0001)).expect("visible");
        assert!(visible.accepts(ExpressionType::Boolean));
        let locked = property_port(node(NodeKind::Frame), PropertyKey(0x0002));
        assert_eq!(locked.map_err(|d| d.code), Err("port-editor-metadata"));
        let unknown = property_port(node(NodeKind::Frame), PropertyKey(0x7777));
        assert_eq!(unknown.map_err(|d| d.code), Err("port-unknown-property"));
    }

    #[test]
    fn state_and_pulse_do_not_connect_directly() {
        assert_eq!(
            check_socket_kinds(SocketKind::State, SocketKind::State),
            Ok(())
        );
        assert_eq!(
            check_socket_kinds(SocketKind::Pulse, SocketKind::Pulse),
            Ok(())
        );
        assert!(check_socket_kinds(SocketKind::State, SocketKind::Pulse).is_err());
        assert!(check_socket_kinds(SocketKind::Pulse, SocketKind::State).is_err());
    }

    #[test]
    fn nullable_expression_is_rejected() {
        let port = property_port(node(NodeKind::Frame), PropertyKey(0x0108)).expect("width");
        let found = TypeSpec::optional(ExpressionType::Length);
        assert!(matches!(
            check_state_connection(&port, found),
            Err(ConnectionError::Nullable { .. })
        ));
    }

    #[test]
    fn union_keywords_parse_with_layout_grammar() {
        // keyword 목록이 layout record parser와 갈라지지 않는지 확인합니다.
        for longhand in LayoutLonghand::ALL {
            let Ok(port) = property_port(node(NodeKind::Frame), longhand.record_key()) else {
                continue;
            };
            for keyword in port.keywords {
                assert_eq!(
                    longhand.parse(keyword),
                    Ok(LayoutValue::Keyword(keyword)),
                    "{} {keyword}",
                    longhand.name()
                );
            }
        }
    }
}
