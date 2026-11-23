//! 컴포넌트 공개 interface 원본 v4(`design/interfaces/<이름>-<hash12>.stellar.yaml`, ADR-0131 결정 1).
//!
//! 컴포넌트 정의 파일(`design/components/`, [`ComponentSource`](super::super::component_source::ComponentSource))은
//! 정의 이름·root·local token과 정의 tree를 담습니다. 이 파일은 같은 컴포넌트의 Property·Slot·상태 머신을 담습니다.
//! 파일 이름은 정의 파일과 같고(이름과 StableId 접미사), interface가 비어 있으면 파일을 쓰지 않습니다.
//!
//! ```yaml
//! format: stellar-component-interface
//! schema_version: "4"
//! component: "<컴포넌트 id>"
//! properties:            # 정의의 선언 순서
//!   - id: "<id>"
//!     name: "label"
//!     visibility: public            # public | private
//!     value_type: string            # 값 타입 이름(Context 원본과 같은 문법)
//!     default_expression: "<id>"    # 리터럴 하나면 default_expression·default_type·default,
//!     default_type: string          #   그 밖이면 default_ast(canonical AST bytes hex)
//!     default: "OK"
//! slots:                 # 정의의 선언 순서
//!   - id: "<id>"
//!     host: "<host Frame id>"
//!     kinds: "frame text"           # 허용 node 종류(종류 번호 순, 공백 구분)
//!     min: "0"
//!     max: none                     # none 또는 정수
//!     content: "<id> <id>"          # default content(시각 순서)
//! actions:               # semantic action(id 순)
//!   - id: "<id>"
//!     name: "pressed"
//! variables:             # 상태 변수(id 순)
//!   - id: "<id>"
//!     name: "on"
//!     value_type: boolean
//!     initial_ast: "…"              # 또는 initial_expression·initial_type·initial
//! transitions:           # 우선순위 순
//!   - id: "<id>"
//!     trigger: "event activate <node id>"   # 또는 "action <nested instance id> <action id>"
//!     condition_ast: "…"            # 없으면 항상 참(condition_expression·condition_type·condition도 받음)
//!     actions:
//!       - set: "<변수 id>"
//!         to_ast: "…"                   # 또는 to_expression·to_type·to
//!       - emit: "<action id>"
//! ```
//!
//! 식은 Component Context 원본(ADR-0099)과 같은 문법입니다. 읽은 결과는 library 검증(`ComponentLibrary::decode`의
//! slot·Property·override·상태 머신 규칙)을 그대로 통과해야 합니다. 쓰기는 다시 읽은 library bytes가 같을 때만 합니다
//! (손실 저장 없음, 다중 파일 원본 [`super::project::encode`]의 의미 비교).

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentEntry, ComponentLibrary, ComponentPropertyId, ComponentSlotId,
};
use stellar_document::component::event::EventKind;
use stellar_document::component::property::{ComponentProperty, PropertyVisibility};
use stellar_document::component::slot::{ComponentSlot, SlotCardinality};
use stellar_document::component::state_machine::{
    ActionSpec, PropertyScope, SemanticAction, SemanticActionId, StateMachine, StateVariable,
    StateVariableId, TransitionAction, TransitionId, TransitionSpec, Trigger,
};
use stellar_document::expressions::ast::ExpressionAst;
use stellar_document::node_kind::NodeKind;

use super::super::component_source::source_filename;
use super::super::text_codec::{field, fields, scalar, text};
use super::super::text_source::TextSourceError;
use super::context::{expression_fields, parse_expression, parse_type, type_name};
use super::css::{self, invalid, unsupported};
use super::VERSION;
use crate::yaml::{load_restricted, write_canonical_checked, YamlValue};

/// interface 파일 폴더(`design/` 기준, 끝 `/` 포함).
pub const INTERFACES_DIR: &str = "interfaces/";
/// 파일 형식 이름.
pub const INTERFACE_FORMAT: &str = "stellar-component-interface";

/// `design/` 기준 경로가 interface 파일인지(폴더 바로 아래 `.stellar.yaml`).
pub fn is_interface_path(path: &str) -> bool {
    path.strip_prefix(INTERFACES_DIR)
        .is_some_and(|name| name.ends_with(".stellar.yaml") && !name.contains('/'))
}

/// 컴포넌트 `entry`의 interface 파일 경로(`design/` 기준).
pub fn interface_path(definition: &ComponentDefinition) -> Result<String, TextSourceError> {
    Ok(format!(
        "{INTERFACES_DIR}{}",
        source_filename(definition.display_name(), definition.id)?
    ))
}

/// interface가 비었는지(Property·Slot·상태 머신이 모두 없음).
pub fn is_empty(entry: &ComponentEntry) -> bool {
    entry.definition().properties().is_empty()
        && entry.definition().slots().is_empty()
        && entry.slots().next().is_none()
        && entry.state_machine().is_empty()
}

fn map(entries: Vec<(String, YamlValue)>) -> YamlValue {
    YamlValue::Mapping(entries.into_iter().map(|(k, v)| (scalar(k), v)).collect())
}

fn id_entry(key: &str, id: StableId) -> (String, YamlValue) {
    (key.to_owned(), scalar(css::id(id)))
}

/// 식 필드에 `prefix`를 붙입니다: 리터럴은 `<prefix>_expression`·`<prefix>_type`·`<prefix>`, 그 밖은 `<prefix>_ast`.
fn prefixed(
    prefix: &str,
    ast: &ExpressionAst,
) -> Result<Vec<(String, YamlValue)>, TextSourceError> {
    Ok(expression_fields(ast)?
        .into_iter()
        .map(|(key, value)| {
            let name = match key {
                "value" => prefix.to_owned(),
                other => format!("{prefix}_{other}"),
            };
            (name, value)
        })
        .collect())
}

/// `item`에서 `prefix` 식 필드 이름(있는 형식 기준).
fn prefixed_keys(item: &YamlValue, prefix: &str) -> Vec<String> {
    if item.get(&format!("{prefix}_ast")).is_some() {
        vec![format!("{prefix}_ast")]
    } else {
        vec![
            format!("{prefix}_expression"),
            format!("{prefix}_type"),
            prefix.to_owned(),
        ]
    }
}

/// `item`의 `prefix` 식을 읽습니다.
fn parse_prefixed(item: &YamlValue, prefix: &str) -> Result<ExpressionAst, TextSourceError> {
    let entries = ["expression", "type", "value", "ast"]
        .into_iter()
        .filter_map(|key| {
            let name = if key == "value" {
                prefix.to_owned()
            } else {
                format!("{prefix}_{key}")
            };
            item.get(&name).map(|value| (scalar(key), value.clone()))
        })
        .collect();
    parse_expression(&YamlValue::Mapping(entries))
}

fn check_fields(item: &YamlValue, keys: &[String]) -> Result<(), TextSourceError> {
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    fields(item, &keys)
}

fn list<'a>(value: &'a YamlValue, key: &str) -> Result<&'a [YamlValue], TextSourceError> {
    field(value, key)?
        .as_sequence()
        .ok_or_else(|| invalid(format!("{key}는 sequence여야 합니다")))
}

fn visibility_text(visibility: PropertyVisibility) -> &'static str {
    match visibility {
        PropertyVisibility::Public => "public",
        PropertyVisibility::Private => "private",
    }
}

fn parse_visibility(value: &str) -> Result<PropertyVisibility, TextSourceError> {
    match value {
        "public" => Ok(PropertyVisibility::Public),
        "private" => Ok(PropertyVisibility::Private),
        other => Err(invalid(format!("모르는 Property 공개 여부 {other}"))),
    }
}

fn parse_kind(name: &str) -> Result<NodeKind, TextSourceError> {
    NodeKind::ALL
        .into_iter()
        .find(|kind| kind.name() == name)
        .ok_or_else(|| invalid(format!("모르는 node 종류 {name}")))
}

fn parse_event(name: &str) -> Result<EventKind, TextSourceError> {
    EventKind::ALL
        .into_iter()
        .find(|kind| kind.name() == name)
        .ok_or_else(|| invalid(format!("모르는 이벤트 종류 {name}")))
}

fn trigger_text(trigger: Trigger) -> String {
    match trigger {
        Trigger::Event { kind, source } => format!("event {} {}", kind.name(), css::id(source)),
        Trigger::NestedAction { instance, action } => format!(
            "action {} {}",
            css::id(instance),
            css::id(action.stable_id())
        ),
    }
}

fn parse_trigger(value: &str) -> Result<Trigger, TextSourceError> {
    let parts: Vec<&str> = value.split(' ').collect();
    match parts.as_slice() {
        ["event", kind, source] => Ok(Trigger::Event {
            kind: parse_event(kind)?,
            source: css::parse_id(source)?,
        }),
        ["action", instance, action] => Ok(Trigger::NestedAction {
            instance: css::parse_id(instance)?,
            action: SemanticActionId::new(css::parse_id(action)?),
        }),
        _ => Err(invalid(format!("모르는 transition trigger {value}"))),
    }
}

/// 컴포넌트 `entry`의 interface를 canonical 텍스트로 씁니다. interface가 비었으면 `None`입니다.
pub fn encode(entry: &ComponentEntry) -> Result<Option<String>, TextSourceError> {
    if is_empty(entry) {
        return Ok(None);
    }
    let definition = entry.definition();
    let mut properties = Vec::new();
    for id in definition.properties() {
        let property = entry
            .property(*id)
            .ok_or_else(|| invalid("선언한 Property의 record가 없습니다"))?;
        let mut entries = vec![
            id_entry("id", id.stable_id()),
            ("name".into(), scalar(property.name())),
            (
                "visibility".into(),
                scalar(visibility_text(property.visibility())),
            ),
            (
                "value_type".into(),
                scalar(type_name(property.value_type())),
            ),
        ];
        entries.extend(prefixed("default", property.default_expression().ast())?);
        properties.push(map(entries));
    }
    let mut slots = Vec::new();
    for id in definition.slots() {
        let slot = entry
            .slot(*id)
            .ok_or_else(|| invalid("선언한 Slot의 record가 없습니다"))?;
        let kinds: Vec<&str> = slot.allowed_kinds().map(NodeKind::name).collect();
        slots.push(map(vec![
            id_entry("id", id.stable_id()),
            id_entry("host", slot.host.id()),
            ("kinds".into(), scalar(kinds.join(" "))),
            ("min".into(), scalar(slot.cardinality.min().to_string())),
            (
                "max".into(),
                scalar(
                    slot.cardinality
                        .max()
                        .map_or_else(|| "none".to_owned(), |max| max.to_string()),
                ),
            ),
            (
                "content".into(),
                scalar(css::id_list(slot.default_content().iter().map(|n| n.id()))),
            ),
        ]));
    }
    let machine = entry.state_machine();
    let actions = machine
        .actions()
        .map(|action| {
            map(vec![
                id_entry("id", action.id().stable_id()),
                ("name".into(), scalar(action.name())),
            ])
        })
        .collect();
    let mut variables = Vec::new();
    for variable in machine.variables() {
        let mut entries = vec![
            id_entry("id", variable.id().stable_id()),
            ("name".into(), scalar(variable.name())),
            (
                "value_type".into(),
                scalar(type_name(variable.value_type())),
            ),
        ];
        entries.extend(prefixed("initial", variable.initial().ast())?);
        variables.push(map(entries));
    }
    let mut transitions = Vec::new();
    for transition in machine.transitions() {
        let mut entries = vec![
            id_entry("id", transition.id().stable_id()),
            ("trigger".into(), scalar(trigger_text(transition.trigger()))),
        ];
        if let Some(condition) = transition.condition() {
            entries.extend(prefixed("condition", condition.ast())?);
        }
        let mut actions = Vec::new();
        for action in transition.actions() {
            actions.push(match action {
                TransitionAction::SetState { variable, value } => {
                    let mut entries = vec![id_entry("set", variable.stable_id())];
                    entries.extend(prefixed("to", value.ast())?);
                    map(entries)
                }
                TransitionAction::Emit { action } => {
                    map(vec![id_entry("emit", action.stable_id())])
                }
            });
        }
        entries.push(("actions".into(), YamlValue::Sequence(actions)));
        transitions.push(map(entries));
    }
    let value = map(vec![
        ("format".into(), scalar(INTERFACE_FORMAT)),
        ("schema_version".into(), scalar(VERSION)),
        id_entry("component", definition.id),
        ("properties".into(), YamlValue::Sequence(properties)),
        ("slots".into(), YamlValue::Sequence(slots)),
        ("actions".into(), YamlValue::Sequence(actions)),
        ("variables".into(), YamlValue::Sequence(variables)),
        ("transitions".into(), YamlValue::Sequence(transitions)),
    ]);
    write_canonical_checked(&value).map(Some).map_err(invalid)
}

/// 정렬 검사: id 오름차순·중복 없음.
fn ascending(ids: &[StableId], what: &str) -> Result<(), TextSourceError> {
    if ids.windows(2).any(|w| w[0] >= w[1]) {
        return Err(invalid(format!("{what}는 id 오름차순, 중복 없이 씁니다")));
    }
    Ok(())
}

/// interface 텍스트 하나를 `library`의 해당 컴포넌트에 적용합니다. 컴포넌트가 없거나, interface가 비었거나, 이미
/// interface가 있거나, `path`가 그 컴포넌트의 interface 파일 이름이 아니면 거부합니다. 적용 뒤 library 전체 검증은
/// [`validate_library`]가 합니다.
pub fn apply(
    library: &mut ComponentLibrary,
    path: &str,
    source: &str,
) -> Result<(), TextSourceError> {
    let value = load_restricted(source).map_err(|e| invalid(format!("{path}: {e}")))?;
    fields(
        &value,
        &[
            "format",
            "schema_version",
            "component",
            "properties",
            "slots",
            "actions",
            "variables",
            "transitions",
        ],
    )?;
    if text(field(&value, "format")?)? != INTERFACE_FORMAT {
        return Err(unsupported("컴포넌트 interface 원본 형식"));
    }
    if text(field(&value, "schema_version")?)? != VERSION {
        return Err(unsupported("컴포넌트 interface 원본 버전"));
    }
    let component = ComponentRef::new(css::parse_id(text(field(&value, "component")?)?)?);
    let entry = library
        .get(component)
        .ok_or_else(|| invalid(format!("{path}: interface의 컴포넌트 정의가 없습니다")))?;
    if interface_path(entry.definition())? != path {
        return Err(invalid(format!(
            "{path}: interface 파일 이름이 컴포넌트 정의 파일 이름과 다릅니다"
        )));
    }
    if !is_empty(entry) {
        return Err(invalid(format!(
            "{path}: 같은 컴포넌트의 interface가 둘입니다"
        )));
    }
    let mut definition = entry.definition().clone();
    let tree = entry.tree().clone();

    let mut properties = Vec::new();
    for item in list(&value, "properties")? {
        let mut keys: Vec<String> = ["id", "name", "visibility", "value_type"]
            .map(str::to_owned)
            .to_vec();
        keys.extend(prefixed_keys(item, "default"));
        check_fields(item, &keys)?;
        let get = |key| text(field(item, key)?);
        let id = ComponentPropertyId::new(css::parse_id(get("id")?)?);
        definition.add_property(id).map_err(invalid)?;
        properties.push(
            ComponentProperty::new(
                id,
                get("name")?.to_owned(),
                parse_type(get("value_type")?)?,
                parse_prefixed(item, "default")?,
                parse_visibility(get("visibility")?)?,
            )
            .map_err(invalid)?,
        );
    }
    let mut slots = Vec::new();
    for item in list(&value, "slots")? {
        fields(item, &["id", "host", "kinds", "min", "max", "content"])?;
        let get = |key| text(field(item, key)?);
        let id = ComponentSlotId::new(css::parse_id(get("id")?)?);
        definition.add_slot(id).map_err(invalid)?;
        let kinds = get("kinds")?
            .split_whitespace()
            .map(parse_kind)
            .collect::<Result<Vec<_>, _>>()?;
        if kinds.windows(2).any(|w| w[0] >= w[1]) {
            return Err(invalid("Slot kinds는 종류 번호 순, 중복 없이 씁니다"));
        }
        let max = match get("max")? {
            "none" => None,
            other => Some(css::parse_u32(other)?),
        };
        slots.push(ComponentSlot::new(
            id,
            NodeRef::new(css::parse_id(get("host")?)?),
            kinds,
            SlotCardinality::new(css::parse_u32(get("min")?)?, max).map_err(invalid)?,
            css::parse_id_list(get("content")?)?
                .into_iter()
                .map(NodeRef::new)
                .collect(),
        ));
    }
    library.remove(component).map_err(invalid)?;
    library.insert(definition, tree).map_err(invalid)?;
    for property in properties {
        library.add_property(component, property).map_err(invalid)?;
    }
    for slot in slots {
        library.add_slot(component, slot).map_err(invalid)?;
    }

    let entry = library
        .get(component)
        .ok_or_else(|| invalid("방금 등록한 컴포넌트가 없습니다"))?;
    let scope = PropertyScope::new(entry);
    let mut machine = StateMachine::new();
    let mut ids = Vec::new();
    for item in list(&value, "actions")? {
        fields(item, &["id", "name"])?;
        let id = css::parse_id(text(field(item, "id")?)?)?;
        ids.push(id);
        machine
            .add_action(
                SemanticAction::new(
                    SemanticActionId::new(id),
                    text(field(item, "name")?)?.to_owned(),
                )
                .map_err(invalid)?,
            )
            .map_err(invalid)?;
    }
    ascending(&ids, "semantic action")?;
    let mut ids = Vec::new();
    for item in list(&value, "variables")? {
        let mut keys: Vec<String> = ["id", "name", "value_type"].map(str::to_owned).to_vec();
        keys.extend(prefixed_keys(item, "initial"));
        check_fields(item, &keys)?;
        let get = |key| text(field(item, key)?);
        let id = css::parse_id(get("id")?)?;
        ids.push(id);
        machine
            .add_variable(
                StateVariable::new(
                    StateVariableId::new(id),
                    get("name")?.to_owned(),
                    parse_type(get("value_type")?)?,
                    parse_prefixed(item, "initial")?,
                    &scope,
                )
                .map_err(invalid)?,
            )
            .map_err(invalid)?;
    }
    ascending(&ids, "상태 변수")?;
    for item in list(&value, "transitions")? {
        let has_condition = ["condition", "condition_ast", "condition_expression"]
            .iter()
            .any(|key| item.get(key).is_some());
        let mut keys: Vec<String> = ["id", "trigger", "actions"].map(str::to_owned).to_vec();
        if has_condition {
            keys.extend(prefixed_keys(item, "condition"));
        }
        check_fields(item, &keys)?;
        let mut actions = Vec::new();
        for action in list(item, "actions")? {
            if action.get("emit").is_some() {
                fields(action, &["emit"])?;
                actions.push(ActionSpec::Emit {
                    action: SemanticActionId::new(css::parse_id(text(field(action, "emit")?)?)?),
                });
            } else {
                let mut keys = vec!["set".to_owned()];
                keys.extend(prefixed_keys(action, "to"));
                check_fields(action, &keys)?;
                actions.push(ActionSpec::SetState {
                    variable: StateVariableId::new(css::parse_id(text(field(action, "set")?)?)?),
                    value: parse_prefixed(action, "to")?,
                });
            }
        }
        let spec = TransitionSpec {
            id: TransitionId::new(css::parse_id(text(field(item, "id")?)?)?),
            trigger: parse_trigger(text(field(item, "trigger")?)?)?,
            condition: if has_condition {
                Some(parse_prefixed(item, "condition")?)
            } else {
                None
            },
            actions,
        };
        machine.add_transition(entry, spec).map_err(invalid)?;
    }
    if machine.is_empty()
        && library.get(component).is_some_and(|entry| {
            entry.definition().properties().is_empty() && entry.definition().slots().is_empty()
        })
    {
        return Err(invalid(format!("{path}: 빈 interface는 쓰지 않습니다")));
    }
    library
        .set_state_machine(component, machine)
        .map_err(invalid)?;
    Ok(())
}

/// interface를 모두 적용한 library를 library 저장 규칙 그대로 다시 검증합니다(선언 Slot·Property의 record, 정의 tree
/// 안 nested 인스턴스의 override 규칙, 상태 머신). 같은 검증을 두 곳에 두지 않으려고 canonical bytes를
/// [`ComponentLibrary::decode`]로 다시 읽습니다.
pub fn validate_library(library: ComponentLibrary) -> Result<ComponentLibrary, TextSourceError> {
    ComponentLibrary::decode(&library.encode()).map_err(invalid)
}
