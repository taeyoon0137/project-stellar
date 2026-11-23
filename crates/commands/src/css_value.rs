//! CSS 값 parser·formatter 재수출과 선언 블록 복사·붙여넣기 core API (UX.2.2, IX.11.13·IX.11.14·IX.11.15).
//!
//! 값 문법 구현은 registry와 같은 crate의 `stellar_document::css::value`에 있습니다(ADR-0075 결정 5). 속성 패널·
//! 선언 블록·CLI·MCP가 같은 parser·formatter를 쓰도록 여기서 그대로 다시 내보냅니다.
//!
//! **복사**([`copy_declarations`]): node 하나의 속성을 속성 패널과 같은 이름·정규 값으로 registry(표시) 순서의
//! longhand 선언 목록으로 냅니다. 묶음 행(`border-radius`)은 longhand로 펼치고, 레이어 속성(`name`·`visible`·
//! `locked`)은 넣지 않습니다. Stellar 전용 속성은 표시를 달아 함께 냅니다. 속성 패널 행이 아직 없는 registry
//! 속성(`transform` 선형 부분, `stroke-linecap`, `image-rendering`, Shadow·Blur·Blend·Opacity modifier 속성)도 그 뒤에
//! registry 이름과 정규 값으로 냅니다(아래 "속성 패널 행이 없는 registry 속성" 절, ADR-0075 개정).
//!
//! **붙여넣기**([`paste_declarations`]): 선언 블록을 해석하고 shorthand를 longhand로 펼친 뒤, 받을 수 있는 선언을
//! 속성 패널과 같은 command로 바꿔 **CommandTransaction 하나**에 담을 command 목록을 만듭니다. 알 수 없는 속성과
//! 받을 수 없는 값은 이유 코드와 함께 [`PastePlan::skipped`]에 남기고 조용히 버리지 않습니다(부분 실패 없음:
//! 받은 선언은 모두 같은 transaction이고 건너뛴 선언은 문서를 바꾸지 않음). 클립보드 연결은 셸이 맡습니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_document::css::declaration::{format_block, parse_block, BlockItem};
use stellar_document::css::shorthand;
use stellar_document::model::DocumentModel;

pub use stellar_document::css::value::*;

use stellar_core_types::property::PropertyKey;
use stellar_core_types::value::{FiniteF32, Transform2D};
use stellar_document::css::mapping::is_known_longhand;
use stellar_document::css::registry::{self, PropertyScope as RegistryScope, RegistryEntry};
use stellar_document::frame::keys as frame_keys;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload};
use stellar_document::modifier_metadata::{
    keys as modifier_keys, modifier_property, ModifierPropertyType,
};
use stellar_document::payload_metadata::{image_keys, shape_keys};
use stellar_document::records::NodePayload;
use stellar_document::shape::ShapeGeometry;

use crate::command::Command;
use crate::id::{CommandId, CommandOrigin};
use crate::inspector::{
    edit_commands, flags, has_fill, inspector_rows, position_modifier, InspectorInput,
    InspectorProperty, NodeClass,
};
use crate::lock::{EditIntent, NodeAccess};
use crate::modifier_ops::{
    css_insert_order, payload_with, read_payload, InsertModifier, SetModifierProperty,
};
use crate::property::{blend_mode_from_tag, read_property, PropertyValue, SetProperty};
use crate::request::CommandRequest;
use crate::state::DocumentState;
use crate::transaction::CommandTransaction;

/// 복사한 선언 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopiedDeclaration {
    /// CSS longhand 이름(Stellar 전용 속성은 registry 이름).
    pub name: &'static str,
    /// 정규 값 텍스트.
    pub value: String,
    /// CSS 대응이 없는 Stellar 전용 속성인지(Web 붙여넣기에서는 무시됨).
    pub stellar_only: bool,
}

/// registry가 저장 위치를 가진 이름인지(registry 이름이거나 매핑 대상 CSS longhand). 속성 패널 행이 없어
/// 붙여넣기 경로가 아직 없는 속성(`opacity`·`transform`·`stroke-linecap` …)을 `unknown-property`와 구분합니다.
fn registry_knows(name: &str) -> bool {
    stellar_document::css::registry::entries()
        .iter()
        .any(|e| e.name == name || e.css.longhands.contains(&name))
}

/// 레이어 메타데이터 속성(복사·붙여넣기 대상 아님).
fn is_layer_property(property: InspectorProperty) -> bool {
    matches!(
        property,
        InspectorProperty::Name | InspectorProperty::Visible | InspectorProperty::Locked
    )
}

/// node 하나의 속성을 선언 목록으로 복사합니다. 문서에 없는 node면 `None`.
pub fn copy_declarations(model: &DocumentModel, node: StableId) -> Option<Vec<CopiedDeclaration>> {
    let snapshot = inspector_rows(model, &[node]);
    if snapshot.count == 0 {
        return None;
    }
    // Text `color`는 Fill이 없으면 문서 기본 글자색을 물려받은 표시 값이라 명시 선언이 아닙니다. 복사는 명시 값만
    // 내므로 넣지 않습니다(붙여넣은 쪽에서 기본값과 같은 색을 명시로 바꾸지 않음, ADR-0082 결정 5).
    let inherited_color = !has_fill(model, node);
    Some(
        snapshot
            .rows
            .into_iter()
            .filter(|row| !is_layer_property(row.property) && row.flags & flags::GROUP == 0)
            .filter(|row| !(row.property == InspectorProperty::Color && inherited_color))
            .map(|row| CopiedDeclaration {
                name: row.property.name(),
                value: row.value,
                stellar_only: row.flags & flags::STELLAR_ONLY != 0,
            })
            .chain(copy_extras(model, node))
            .collect(),
    )
}

/// 복사한 선언 목록의 선언 블록 텍스트(`property: value;` 한 줄에 하나).
pub fn declaration_block_text(declarations: &[CopiedDeclaration]) -> String {
    format_block(declarations.iter().map(|d| (d.name, d.value.as_str())))
}

/// 건너뛴 선언 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedDeclaration {
    /// 속성 이름(형식이 틀린 조각이면 빈 문자열).
    pub name: String,
    /// 값 텍스트(형식이 틀린 조각이면 원문).
    pub value: String,
    /// 이유 코드: 값 오류 코드(`unsupported-unit`·`unsupported-value`·`unknown-token`·`forbidden-value`·
    /// `invalid-value`), `invalid-declaration`, `unknown-property`(Stellar에 저장 위치가 없음),
    /// `unsupported-property`(저장 위치는 있지만 붙여넣기 경로가 아직 없음),
    /// `not-applicable`(선택한 node 종류에 없는 속성이거나 레이어 속성), `rejected`(편집 규칙이 거부).
    pub code: &'static str,
    /// 사람이 읽는 이유.
    pub message: String,
}

/// 적용한 선언 하나(shorthand는 펼친 longhand).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedDeclaration {
    /// longhand 이름.
    pub name: &'static str,
    /// 정규 값 텍스트.
    pub value: String,
}

/// 붙여넣기 결과.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PastePlan {
    /// 한 transaction에 순서대로 담을 command.
    pub commands: Vec<Command>,
    /// 적용한 선언(입력 순서).
    pub applied: Vec<AppliedDeclaration>,
    /// 건너뛴 선언(입력 순서).
    pub skipped: Vec<SkippedDeclaration>,
}

impl PastePlan {
    /// command를 CommandTransaction 하나로 묶습니다. 바꿀 것이 없으면 `None`.
    pub fn transaction(
        &self,
        id: CommandId,
        origin: CommandOrigin,
        base: DocumentRevision,
    ) -> Option<CommandTransaction> {
        if self.commands.is_empty() {
            return None;
        }
        let mut transaction = CommandTransaction::new(id, origin);
        for command in &self.commands {
            transaction = transaction.with_request(CommandRequest::new(base, command.clone()));
        }
        Some(transaction)
    }
}

/// 붙여넣기 전체 실패(블록 형식, 선택 없음, 문서 복제 실패).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteError {
    /// 진단 코드.
    pub code: &'static str,
    /// 이유.
    pub message: String,
}

fn paste_error(code: &'static str, message: impl Into<String>) -> PasteError {
    PasteError {
        code,
        message: message.into(),
    }
}

fn skip(plan: &mut PastePlan, name: &str, value: &str, code: &'static str, message: String) {
    plan.skipped.push(SkippedDeclaration {
        name: name.to_string(),
        value: value.to_string(),
        code,
        message,
    });
}

/// `selection`에 선언 블록 `text`를 붙여넣을 command를 만듭니다. `scope`는 `var(--name)`을 문서 토큰으로 풉니다.
///
/// 선언은 입력 순서대로 문서 사본 위에서 하나씩 적용해 다음 선언의 기준 상태로 씁니다(같은 속성이 두 번 오면
/// CSS처럼 뒤 선언이 이기고, 새 modifier 순서 key가 겹치지 않습니다). `new_id`는 새 modifier(Fill·Stroke·위치
/// Transform) ID 공급자입니다. 사본은 문서 레코드에서 만들므로 비용은 문서 크기에 선형입니다.
pub fn paste_declarations(
    model: &DocumentModel,
    selection: &[StableId],
    text: &str,
    scope: &dyn TokenScope,
    new_id: &mut dyn FnMut() -> StableId,
) -> Result<PastePlan, PasteError> {
    if selection.is_empty() {
        return Err(paste_error("empty-selection", "선택한 레이어가 없습니다"));
    }
    let items = parse_block(text).map_err(|e| paste_error(e.code, e.message))?;
    let records = model
        .to_records()
        .map_err(|e| paste_error("document-error", format!("{e:?}")))?;
    let (scratch, _) = DocumentModel::from_records(records)
        .map_err(|e| paste_error("document-error", format!("{e:?}")))?;
    let mut scratch = DocumentState::new(scratch);
    let classes: Vec<NodeClass> = selection
        .iter()
        .map(|id| {
            scratch
                .model()
                .payload(*id)
                .map(NodeClass::of)
                .ok_or_else(|| paste_error("unknown-node", format!("문서에 없는 레이어: {id}")))
        })
        .collect::<Result<_, _>>()?;
    let mut plan = PastePlan::default();
    for item in items {
        let declaration = match item {
            BlockItem::Malformed(malformed) => {
                plan.skipped.push(SkippedDeclaration {
                    name: String::new(),
                    value: malformed.text,
                    code: malformed.code,
                    message: malformed.message,
                });
                continue;
            }
            BlockItem::Declaration(declaration) => declaration,
        };
        // shorthand는 펼친 longhand가 모두 속성 패널 속성이면 longhand로 적용합니다(`border-radius` → 4 모서리).
        // 그렇지 않고 같은 이름의 속성 패널 행이 있으면(`border`, `overflow` 등 Stellar가 묶어 저장하는 값) 그 행으로
        // 적용하고, 둘 다 아니면 펼친 longhand를 하나씩 판정해 저장 위치가 없는 것을 건너뜁니다.
        let expanded = shorthand::expand(&declaration.name, &declaration.value);
        let direct = InspectorProperty::from_name(&declaration.name).is_some();
        let longhands: Vec<(String, String)> = match expanded {
            Some(Ok(expanded))
                if expanded
                    .iter()
                    .all(|l| InspectorProperty::from_name(l.name).is_some())
                    || !direct =>
            {
                expanded
                    .into_iter()
                    .map(|l| (l.name.to_string(), l.value))
                    .collect()
            }
            Some(Err(error)) if !direct => {
                skip(
                    &mut plan,
                    &declaration.name,
                    &declaration.value,
                    error.code,
                    error.message,
                );
                continue;
            }
            _ => vec![(declaration.name.clone(), declaration.value.clone())],
        };
        for (name, value) in longhands {
            let Some(property) = InspectorProperty::from_name(&name) else {
                if let Some(extra) = Extra::by_name(&name) {
                    paste_extra(
                        &mut plan,
                        &mut scratch,
                        selection,
                        extra,
                        (&name, &value),
                        scope,
                        new_id,
                    );
                    continue;
                }
                if registry_knows(&name) {
                    skip(
                        &mut plan,
                        &name,
                        &value,
                        "unsupported-property",
                        format!("{name}은 Stellar가 저장하지만 선언 블록 붙여넣기로는 아직 받지 않는 속성입니다"),
                    );
                } else {
                    skip(
                        &mut plan,
                        &name,
                        &value,
                        "unknown-property",
                        format!("{name}은 Stellar에 저장 위치가 없는 속성입니다"),
                    );
                }
                continue;
            };
            if is_layer_property(property) {
                skip(
                    &mut plan,
                    &name,
                    &value,
                    "not-applicable",
                    "레이어 속성(이름·표시·잠금)은 붙여넣지 않습니다".into(),
                );
                continue;
            }
            if let Some(class) = classes.iter().find(|c| !property.applies(**c)) {
                skip(
                    &mut plan,
                    &name,
                    &value,
                    "not-applicable",
                    format!("{class:?} 레이어에는 {name} 속성이 없습니다"),
                );
                continue;
            }
            let parsed = match parse_with(property.grammar(), &value, scope) {
                Ok(CssValue::Var(var)) => {
                    skip(
                        &mut plan,
                        &name,
                        &value,
                        "unsupported-value",
                        format!(
                            "토큰 참조 {}는 아직 붙여넣기로 연결하지 않습니다(값 binding 필요)",
                            var.name
                        ),
                    );
                    continue;
                }
                Ok(parsed) => parsed,
                Err(error) => {
                    skip(&mut plan, &name, &value, error.code, error.message);
                    continue;
                }
            };
            let canonical = format(&parsed, property.grammar());
            let commands = match edit_commands(
                scratch.model(),
                selection,
                property,
                &InspectorInput::Set(parsed),
                new_id,
            ) {
                Ok(commands) => commands,
                Err(error) => {
                    skip(&mut plan, &name, &value, "rejected", error.0);
                    continue;
                }
            };
            if !commands.is_empty() {
                let mut transaction =
                    CommandTransaction::new(CommandId::from_bytes([0; 16]), CommandOrigin::User);
                for command in &commands {
                    transaction = transaction.with_command(scratch.revision(), command.clone());
                }
                if let Err(error) = scratch.execute_transaction(&transaction) {
                    skip(
                        &mut plan,
                        &name,
                        &value,
                        "rejected",
                        format!("{:?}", error.error),
                    );
                    continue;
                }
                plan.commands.extend(commands);
            }
            plan.applied.push(AppliedDeclaration {
                name: property.name(),
                value: canonical,
            });
        }
    }
    Ok(plan)
}

// ───────────── 속성 패널 행이 없는 registry 속성(IX.11.15 보완, ADR-0075 개정) ─────────────
//
// 속성 패널 행이 아직 없는 registry 속성도 같은 이름·정규 값으로 복사·붙여넣기합니다. 이름·값 문법은 registry
// 항목(`css::registry`)에서 읽고, 값은 registry 값 문법의 parser·formatter를 씁니다.
//
// - payload keyword(`stroke-linecap`, `image-rendering`): `SetProperty`(registry key, keyword index).
// - modifier 속성(`shadow-*`, `blur-radius`, `mix-blend-mode`, `opacity`): 그 종류의 첫 활성 modifier가 대상입니다.
//   없으면 기본 payload에 값을 쓴 modifier를 CSS 고정 순서 위치(속성 모델 9.1)에 넣습니다. 같은 종류 modifier가
//   여럿이면 첫 활성 하나만 선언으로 나타냅니다.
// - `transform`: 선형 부분 `matrix(a, b, c, d, 0, 0)`만 다룹니다. 이동 성분은 위치(`left`·`top`)입니다(속성 모델 5장
//   `transform` 행). Frame은 payload transform, 그 밖의 node는 위치 Transform modifier(`position_modifier`)의 선형
//   부분이고 이동 성분은 그대로 둡니다. 이동 성분이 있는 값은 `unsupported-value`입니다.

/// 속성 패널 행이 없는 registry 속성의 저장 위치.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Extra {
    /// node payload keyword(registry 범위, key).
    Payload(RegistryScope, PropertyKey),
    /// modifier payload 속성(종류, key).
    Modifier(ModifierKind, PropertyKey),
    /// 선형 transform(Frame payload 또는 위치 Transform modifier).
    Transform,
}

/// registry 순서(범위 순, key 순)의 추가 속성.
const EXTRAS: [Extra; 10] = [
    Extra::Transform,
    Extra::Payload(RegistryScope::Shape, shape_keys::STROKE_LINECAP),
    Extra::Payload(RegistryScope::Image, image_keys::IMAGE_RENDERING),
    Extra::Modifier(ModifierKind::Shadow, modifier_keys::SHADOW_COLOR),
    Extra::Modifier(ModifierKind::Shadow, modifier_keys::SHADOW_BLUR),
    Extra::Modifier(ModifierKind::Shadow, modifier_keys::SHADOW_OFFSET_X),
    Extra::Modifier(ModifierKind::Shadow, modifier_keys::SHADOW_OFFSET_Y),
    Extra::Modifier(ModifierKind::Blur, modifier_keys::BLUR_RADIUS),
    Extra::Modifier(ModifierKind::Blend, modifier_keys::BLEND_MODE),
    Extra::Modifier(ModifierKind::Opacity, modifier_keys::OPACITY_VALUE),
];

impl Extra {
    fn entry(self) -> RegistryEntry {
        let (scope, key) = match self {
            Extra::Payload(scope, key) => (scope, key),
            Extra::Modifier(_, key) => (RegistryScope::Modifier, key),
            Extra::Transform => (RegistryScope::Frame, frame_keys::TRANSFORM),
        };
        registry::entry(scope, key).expect("추가 속성은 registry 항목입니다")
    }

    fn name(self) -> &'static str {
        self.entry().name
    }

    fn grammar(self) -> CssGrammar {
        self.entry()
            .css
            .grammar
            .expect("추가 속성은 값 문법을 가집니다")
    }

    fn by_name(name: &str) -> Option<Extra> {
        EXTRAS.into_iter().find(|extra| extra.name() == name)
    }

    /// 이 node 종류에 저장 위치가 있는지.
    fn applies(self, payload: &NodePayload) -> bool {
        match self {
            Extra::Payload(_, shape_keys::STROKE_LINECAP) => {
                matches!(payload, NodePayload::Shape(ShapeGeometry::Line(_)))
            }
            Extra::Payload(_, _) => matches!(payload, NodePayload::Image(_)),
            Extra::Transform => !matches!(payload, NodePayload::Fragment(_)),
            Extra::Modifier(..) => true,
        }
    }

    /// 권한 판정에 쓰는 key.
    fn key(self, payload: &NodePayload) -> PropertyKey {
        match (self, payload) {
            (Extra::Payload(_, key) | Extra::Modifier(_, key), _) => key,
            (Extra::Transform, NodePayload::Frame(_)) => frame_keys::TRANSFORM,
            (Extra::Transform, _) => modifier_keys::TRANSFORM_MATRIX,
        }
    }
}

/// 첫 활성 `kind` modifier.
fn first_enabled(
    model: &DocumentModel,
    node: StableId,
    kind: ModifierKind,
) -> Option<ModifierEntry> {
    model
        .modifier_stack(node)?
        .entries()
        .iter()
        .find(|e| e.enabled && e.kind == kind)
        .cloned()
}

fn linear(transform: Transform2D) -> [f32; 6] {
    let (a, b, c, d, _, _) = transform.components();
    [a, b, c, d, 0., 0.]
}

/// 저장 값을 CSS 값으로 읽습니다. 선언이 없으면(그 종류 modifier 없음, 종류 밖) `None`.
fn read_extra(model: &DocumentModel, node: StableId, extra: Extra) -> Option<CssValue> {
    let record = model.record(node)?;
    if !extra.applies(&record.payload) {
        return None;
    }
    match extra {
        Extra::Payload(_, key) => match read_property(&record, key)? {
            PropertyValue::Keyword(tag) => Some(CssValue::Keyword(usize::from(tag))),
            _ => None,
        },
        // 위치 Transform modifier가 없으면 선형 부분은 단위 행렬(`none`)입니다.
        Extra::Transform => Some(CssValue::Transform(match &record.payload {
            NodePayload::Frame(frame) => linear(frame.transform),
            _ => match position_modifier(model, node).map(|entry| entry.payload) {
                Some(ModifierPayload::Transform(t)) => linear(t),
                _ => linear(Transform2D::identity()),
            },
        })),
        Extra::Modifier(kind, key) => {
            let entry = first_enabled(model, node, kind)?;
            Some(match read_payload(&entry.payload, key)? {
                PropertyValue::UnitInterval(v) => CssValue::Number(v.get()),
                PropertyValue::BlendMode(mode) => CssValue::Keyword(mode as usize),
                PropertyValue::Color(color) => CssValue::Color(Some(color)),
                PropertyValue::NonNegative(v) | PropertyValue::Number(v) => {
                    CssValue::Length(v.get())
                }
                _ => return None,
            })
        }
    }
}

/// 추가 속성 선언(registry 순서). 속성 패널 행 뒤에 붙습니다.
fn copy_extras(model: &DocumentModel, node: StableId) -> Vec<CopiedDeclaration> {
    EXTRAS
        .into_iter()
        .filter_map(|extra| {
            let value = read_extra(model, node, extra)?;
            let name = extra.name();
            Some(CopiedDeclaration {
                name,
                value: format(&value, extra.grammar()),
                stellar_only: !is_known_longhand(name),
            })
        })
        .collect()
}

/// CSS 값을 modifier 속성 값으로 바꿉니다(registry 값 타입 기준).
fn modifier_value(key: PropertyKey, value: &CssValue) -> Result<PropertyValue, String> {
    let metadata = modifier_property(key).ok_or("registry에 없는 modifier 속성입니다")?;
    let finite = |v: f32| FiniteF32::new(v).map_err(|_| "유한한 수가 아닙니다".to_string());
    Ok(match (metadata.value_type, value) {
        (ModifierPropertyType::UnitInterval, CssValue::Number(v)) => {
            PropertyValue::UnitInterval(finite(*v)?)
        }
        (ModifierPropertyType::NonNegative, CssValue::Length(v)) if *v >= 0. => {
            PropertyValue::NonNegative(finite(*v)?)
        }
        (ModifierPropertyType::Coordinate, CssValue::Length(v)) => {
            PropertyValue::Number(finite(*v)?)
        }
        (ModifierPropertyType::Color, CssValue::Color(Some(color))) => PropertyValue::Color(*color),
        (ModifierPropertyType::BlendMode, CssValue::Keyword(index)) => PropertyValue::BlendMode(
            u8::try_from(*index)
                .ok()
                .and_then(blend_mode_from_tag)
                .ok_or("알 수 없는 합성 방식입니다")?,
        ),
        _ => return Err("값이 이 속성의 문법과 맞지 않습니다".into()),
    })
}

/// 한 node에 추가 속성 값을 쓰는 command(값이 같으면 빈 목록).
fn extra_commands(
    model: &DocumentModel,
    node: StableId,
    extra: Extra,
    value: &CssValue,
    new_id: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, (&'static str, String)> {
    let rejected = |message: String| ("rejected", message);
    let record = model
        .record(node)
        .ok_or_else(|| rejected(format!("문서에 없는 레이어: {node}")))?;
    let access = NodeAccess::of(model, node)
        .ok_or_else(|| rejected(format!("문서에 없는 레이어: {node}")))?;
    access
        .permits(EditIntent::Modify(extra.key(&record.payload)))
        .map_err(|blocked| rejected(format!("{blocked:?}: 이 레이어의 속성을 바꿀 수 없습니다")))?;
    if read_extra(model, node, extra).as_ref() == Some(value) {
        return Ok(Vec::new());
    }
    let set = |key, value| Command::SetProperty(SetProperty { node, key, value });
    Ok(match (extra, value) {
        (Extra::Payload(_, key), CssValue::Keyword(index)) => {
            let tag = u8::try_from(*index).map_err(|_| rejected("keyword 범위 초과".into()))?;
            vec![set(key, PropertyValue::Keyword(tag))]
        }
        (Extra::Transform, CssValue::Transform([a, b, c, d, e, f])) => {
            if *e != 0. || *f != 0. {
                return Err((
                    "unsupported-value",
                    "transform 이동 성분은 위치(left·top)로 지정하세요".into(),
                ));
            }
            let with = |t: Transform2D| {
                let (_, _, _, _, x, y) = t.components();
                Transform2D::new(*a, *b, *c, *d, x, y)
                    .map_err(|_| rejected("유한하지 않은 transform입니다".into()))
            };
            match &record.payload {
                NodePayload::Frame(frame) => vec![set(
                    frame_keys::TRANSFORM,
                    PropertyValue::Transform(with(frame.transform)?),
                )],
                _ => match position_modifier(model, node) {
                    Some(entry) => {
                        let ModifierPayload::Transform(current) = entry.payload else {
                            return Err(rejected("위치 modifier가 Transform이 아닙니다".into()));
                        };
                        vec![Command::SetModifierProperty(SetModifierProperty {
                            node,
                            modifier: entry.id,
                            key: modifier_keys::TRANSFORM_MATRIX,
                            value: PropertyValue::Transform(with(current)?),
                        })]
                    }
                    None => vec![insert_modifier(
                        model,
                        node,
                        ModifierPayload::Transform(with(Transform2D::identity())?),
                        new_id,
                    )],
                },
            }
        }
        (Extra::Modifier(kind, key), value) => {
            let value = modifier_value(key, value).map_err(|m| ("invalid-value", m))?;
            match first_enabled(model, node, kind) {
                Some(entry) => vec![Command::SetModifierProperty(SetModifierProperty {
                    node,
                    modifier: entry.id,
                    key,
                    value,
                })],
                None => {
                    let payload = payload_with(kind, key, &value)
                        .ok_or_else(|| rejected("modifier 기본값에 값을 쓸 수 없습니다".into()))?;
                    vec![insert_modifier(model, node, payload, new_id)]
                }
            }
        }
        _ => {
            return Err((
                "invalid-value",
                "값이 이 속성의 문법과 맞지 않습니다".into(),
            ))
        }
    })
}

/// CSS 고정 순서 위치에 새 활성 modifier를 넣는 command.
fn insert_modifier(
    model: &DocumentModel,
    node: StableId,
    payload: ModifierPayload,
    new_id: &mut dyn FnMut() -> StableId,
) -> Command {
    let kind = payload.kind();
    Command::InsertModifier(InsertModifier {
        node,
        entry: ModifierEntry {
            id: ModifierId::new(new_id()),
            order: css_insert_order(model, node, kind),
            kind,
            enabled: true,
            payload,
        },
    })
}

/// 추가 속성 선언 하나를 모든 선택 node에 붙여넣습니다(사본 문서에 바로 적용).
fn paste_extra(
    plan: &mut PastePlan,
    scratch: &mut DocumentState,
    selection: &[StableId],
    extra: Extra,
    (name, value): (&str, &str),
    scope: &dyn TokenScope,
    new_id: &mut dyn FnMut() -> StableId,
) {
    for node in selection {
        let applies = scratch
            .model()
            .payload(*node)
            .is_some_and(|payload| extra.applies(payload));
        if !applies {
            let class = scratch.model().payload(*node).map(NodeClass::of);
            skip(
                plan,
                name,
                value,
                "not-applicable",
                format!("{class:?} 레이어에는 {name} 속성이 없습니다"),
            );
            return;
        }
    }
    let grammar = extra.grammar();
    let parsed = match parse_with(grammar, value, scope) {
        Ok(CssValue::Var(var)) => {
            skip(
                plan,
                name,
                value,
                "unsupported-value",
                format!(
                    "토큰 참조 {}는 아직 붙여넣기로 연결하지 않습니다(값 binding 필요)",
                    var.name
                ),
            );
            return;
        }
        Ok(parsed) => parsed,
        Err(error) => {
            skip(plan, name, value, error.code, error.message);
            return;
        }
    };
    let mut commands = Vec::new();
    for node in selection {
        match extra_commands(scratch.model(), *node, extra, &parsed, new_id) {
            Ok(more) => commands.extend(more),
            Err((code, message)) => {
                skip(plan, name, value, code, message);
                return;
            }
        }
    }
    if !commands.is_empty() {
        let mut transaction =
            CommandTransaction::new(CommandId::from_bytes([0; 16]), CommandOrigin::User);
        for command in &commands {
            transaction = transaction.with_command(scratch.revision(), command.clone());
        }
        if let Err(error) = scratch.execute_transaction(&transaction) {
            skip(plan, name, value, "rejected", format!("{:?}", error.error));
            return;
        }
        plan.commands.extend(commands);
    }
    plan.applied.push(AppliedDeclaration {
        name: extra.name(),
        value: format(&parsed, grammar),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::tests::seven_kind_model;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;

    fn new_ids(base: u64) -> impl FnMut() -> StableId {
        let mut n = 0;
        move || {
            n += 1;
            StableId::new_v7(base * 1_000 + n, 0, n)
        }
    }

    fn execute(state: &mut DocumentState, plan: &PastePlan) -> CommandTransaction {
        let transaction = plan
            .transaction(
                CommandId::from_bytes([7; 16]),
                CommandOrigin::User,
                state.revision(),
            )
            .expect("바꿀 것이 있음");
        state.execute_transaction(&transaction).unwrap().inverse
    }

    /// node 하나의 속성(레이어 속성 제외) 선언 블록.
    fn style_of(model: &DocumentModel, node: StableId) -> String {
        declaration_block_text(&copy_declarations(model, node).unwrap())
    }

    fn copy_model(model: &DocumentModel) -> DocumentState {
        DocumentState::new(
            DocumentModel::from_records(model.to_records().unwrap())
                .unwrap()
                .0,
        )
    }

    #[test]
    fn copy_emits_longhands_in_registry_order() {
        let (model, ids) = seven_kind_model();
        let rect = copy_declarations(&model, ids[1]).unwrap();
        let names: Vec<&str> = rect.iter().map(|d| d.name).collect();
        assert!(names.contains(&"border-top-left-radius"));
        assert!(!names.contains(&"border-radius"), "묶음 행은 longhand로");
        assert!(!names.contains(&"name") && !names.contains(&"locked"));
        let text = declaration_block_text(&rect);
        assert!(text.contains("width: 80px;\n"), "{text}");
        assert!(text.lines().all(|l| l.ends_with(';')));
        let polygon = copy_declarations(&model, ids[4]).unwrap();
        assert!(polygon
            .iter()
            .any(|d| d.name == "polygon-sides" && d.stellar_only));
        assert!(copy_declarations(&model, StableId::new_v7(99, 9, 99)).is_none());
    }

    #[test]
    fn copy_paste_round_trip_reproduces_the_style_and_undoes_once() {
        // 검증 기준(IX.11.15): copy→paste 왕복 후 semantic snapshot 동일, undo 한 번으로 전체 복원.
        let (model, ids) = seven_kind_model();
        let mut state = DocumentState::new(model);
        // 같은 node에 붙여넣으면 아무것도 바뀌지 않습니다(건너뜀 0, command 0).
        for id in ids {
            let text = style_of(state.model(), id);
            let plan = paste_declarations(state.model(), &[id], &text, &NoTokens, &mut new_ids(1))
                .unwrap();
            assert!(plan.skipped.is_empty(), "{id}: {:?}", plan.skipped);
            assert!(plan.commands.is_empty(), "{id}: 같은 값은 command 없음");
        }
        let source = ids[1];
        let copied = style_of(state.model(), source);
        let before = model_snapshot_json(state.model()).unwrap();
        let edit = paste_declarations(
            state.model(),
            &[source],
            "width: 200px; border-radius: 9px; fill: #00ff00; left: 7px",
            &NoTokens,
            &mut new_ids(2),
        )
        .unwrap();
        assert!(edit.skipped.is_empty(), "{:?}", edit.skipped);
        let undo = execute(&mut state, &edit);
        assert_ne!(style_of(state.model(), source), copied);
        // 복사해 둔 블록을 다시 붙이면 style이 복사 시점과 같아집니다.
        let mut pasted = copy_model(state.model());
        let restore = paste_declarations(
            pasted.model(),
            &[source],
            &copied,
            &NoTokens,
            &mut new_ids(3),
        )
        .unwrap();
        assert!(restore.skipped.is_empty(), "{:?}", restore.skipped);
        execute(&mut pasted, &restore);
        assert_eq!(style_of(pasted.model(), source), copied);
        // undo 한 번(transaction 하나)으로 semantic snapshot 전체가 처음과 같습니다.
        state
            .execute_transaction(&undo.rebased(state.revision()))
            .unwrap();
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
    }

    #[test]
    fn registry_properties_without_panel_rows_round_trip_through_copy_and_paste() {
        // IX.11.15 보완: opacity·mix-blend-mode·transform·stroke-linecap·image-rendering·shadow·blur도 같은 이름·정규
        // 값으로 복사되고, 붙여넣으면 semantic snapshot이 복사 시점과 같아집니다.
        let (model, ids) = seven_kind_model();
        let (rect, ellipse, line, image) = (ids[1], ids[2], ids[3], ids[6]);
        let mut state = DocumentState::new(model);
        let effects =
            "transform: matrix(0, 1, -1, 0, 0, 0); shadow-color: #ff000080; shadow-blur: 4px; \
                       shadow-offset-x: 2px; shadow-offset-y: -3px; blur-radius: 1.5px; \
                       mix-blend-mode: multiply; opacity: 0.25";
        let plan = paste_declarations(state.model(), &[rect], effects, &NoTokens, &mut new_ids(20))
            .unwrap();
        assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
        execute(&mut state, &plan);
        // 새 modifier는 CSS 고정 순서(transform → paint → filter → opacity → mix-blend-mode)로 들어갑니다.
        let kinds: Vec<ModifierKind> = state
            .model()
            .modifier_stack(rect)
            .unwrap()
            .entries()
            .iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                ModifierKind::Transform,
                ModifierKind::Shadow,
                ModifierKind::Blur,
                ModifierKind::Opacity,
                ModifierKind::Blend,
            ]
        );
        let copied = style_of(state.model(), rect);
        for line in [
            "transform: matrix(0, 1, -1, 0, 0, 0);",
            "shadow-color: #ff000080;",
            "shadow-blur: 4px;",
            "shadow-offset-x: 2px;",
            "shadow-offset-y: -3px;",
            "blur-radius: 1.5px;",
            "mix-blend-mode: multiply;",
            "opacity: 0.25;",
        ] {
            assert!(copied.lines().any(|l| l == line), "{line}\n{copied}");
        }
        let stellar_only: Vec<&str> = copy_declarations(state.model(), rect)
            .unwrap()
            .into_iter()
            .filter(|d| d.stellar_only)
            .map(|d| d.name)
            .collect();
        assert!(stellar_only.contains(&"shadow-blur") && !stellar_only.contains(&"opacity"));
        // 같은 node에 다시 붙이면 바꿀 것이 없습니다.
        let again =
            paste_declarations(state.model(), &[rect], &copied, &NoTokens, &mut new_ids(21))
                .unwrap();
        assert!(
            again.skipped.is_empty() && again.commands.is_empty(),
            "{again:?}"
        );
        // 값을 바꾼 뒤 복사해 둔 블록을 붙이면 semantic snapshot이 복사 시점과 같습니다.
        let before = model_snapshot_json(state.model()).unwrap();
        let change = paste_declarations(
            state.model(),
            &[rect],
            "transform: none; shadow-blur: 0px; mix-blend-mode: normal; opacity: 1",
            &NoTokens,
            &mut new_ids(22),
        )
        .unwrap();
        execute(&mut state, &change);
        assert_ne!(model_snapshot_json(state.model()).unwrap(), before);
        let restore =
            paste_declarations(state.model(), &[rect], &copied, &NoTokens, &mut new_ids(23))
                .unwrap();
        assert!(restore.skipped.is_empty(), "{:?}", restore.skipped);
        execute(&mut state, &restore);
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
        // 효과 없는 다른 도형에 붙이면 같은 효과 선언이 생깁니다(도형 전용 행은 not-applicable).
        let onto = paste_declarations(
            state.model(),
            &[ellipse],
            &copied,
            &NoTokens,
            &mut new_ids(24),
        )
        .unwrap();
        assert!(onto
            .skipped
            .iter()
            .all(|s| s.code == "not-applicable" && s.name.starts_with("border-")));
        execute(&mut state, &onto);
        let effects_of = |model: &DocumentModel, node| {
            copy_extras(model, node)
                .into_iter()
                .map(|d| (d.name, d.value))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            effects_of(state.model(), ellipse),
            effects_of(state.model(), rect)
        );
        // payload keyword: Line `stroke-linecap`, Image `image-rendering`.
        let rendering = Extra::by_name("image-rendering").unwrap();
        let CssGrammar::Keyword(choices) = rendering.grammar() else {
            panic!("keyword 문법");
        };
        let block = format!("image-rendering: {}", choices[1]);
        let keyword =
            paste_declarations(state.model(), &[image], &block, &NoTokens, &mut new_ids(25))
                .unwrap();
        assert!(keyword.skipped.is_empty(), "{:?}", keyword.skipped);
        execute(&mut state, &keyword);
        assert!(style_of(state.model(), image).contains(&format!("{block};\n")));
        let cap = paste_declarations(
            state.model(),
            &[line],
            "stroke-linecap: square",
            &NoTokens,
            &mut new_ids(26),
        )
        .unwrap();
        execute(&mut state, &cap);
        assert!(style_of(state.model(), line).contains("stroke-linecap: square;\n"));
        let wrong = paste_declarations(
            state.model(),
            &[rect],
            "stroke-linecap: round; transform: translate(4px)",
            &NoTokens,
            &mut new_ids(27),
        )
        .unwrap();
        let codes: Vec<&str> = wrong.skipped.iter().map(|s| s.code).collect();
        assert_eq!(codes, ["not-applicable", "unsupported-value"]);
        // 효과 붙여넣기 transaction 하나는 undo 한 번으로 되돌아갑니다.
        let mut fresh = DocumentState::new(seven_kind_model().0);
        let original = model_snapshot_json(fresh.model()).unwrap();
        let plan = paste_declarations(fresh.model(), &[rect], effects, &NoTokens, &mut new_ids(28))
            .unwrap();
        let inverse = execute(&mut fresh, &plan);
        fresh
            .execute_transaction(&inverse.rebased(fresh.revision()))
            .unwrap();
        assert_eq!(model_snapshot_json(fresh.model()).unwrap(), original);
    }

    #[test]
    fn inherited_text_color_is_not_copied_but_an_explicit_equal_color_stays_explicit() {
        // ADR-0082 결정 5: 명시한 값은 기본값과 같아도 명시로 남고, 복사는 명시 값만 냅니다.
        let (model, ids) = seven_kind_model();
        let text = ids[5];
        let mut state = DocumentState::new(model);
        let copied = copy_declarations(state.model(), text).unwrap();
        assert!(
            copied.iter().all(|d| d.name != "color"),
            "물려받은 글자색은 선언이 아님: {copied:?}"
        );
        // 기본 글자색(검정)과 같은 색을 붙여넣어도 명시 Fill이 생깁니다.
        let plan = paste_declarations(
            state.model(),
            &[text],
            "color: #000000",
            &NoTokens,
            &mut new_ids(9),
        )
        .unwrap();
        assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
        assert_eq!(plan.commands.len(), 1, "명시 Fill 추가");
        assert!(matches!(plan.commands[0], Command::InsertModifier(_)));
        execute(&mut state, &plan);
        let copied = copy_declarations(state.model(), text).unwrap();
        assert!(copied
            .iter()
            .any(|d| d.name == "color" && d.value == "#000000"));
        // 같은 블록을 다시 붙이면 바꿀 것이 없습니다.
        let again = paste_declarations(
            state.model(),
            &[text],
            &declaration_block_text(&copied),
            &NoTokens,
            &mut new_ids(10),
        )
        .unwrap();
        assert!(
            again.commands.is_empty() && again.skipped.is_empty(),
            "{copied:?} {:?} {:?}",
            again.commands,
            again.skipped
        );
    }

    #[test]
    fn style_copied_from_one_shape_applies_to_another() {
        let (model, ids) = seven_kind_model();
        let mut state = DocumentState::new(model);
        let text = style_of(state.model(), ids[1]);
        // Rectangle style을 Polygon에 붙이면 모서리 반경은 Polygon에 없어 건너뜁니다.
        let plan = paste_declarations(state.model(), &[ids[4]], &text, &NoTokens, &mut new_ids(4))
            .unwrap();
        assert!(plan
            .skipped
            .iter()
            .any(|s| s.name == "border-top-left-radius" && s.code == "not-applicable"));
        if !plan.commands.is_empty() {
            execute(&mut state, &plan);
        }
        let polygon = style_of(state.model(), ids[4]);
        let rect = style_of(state.model(), ids[1]);
        for line in rect.lines().filter(|l| !l.contains("radius")) {
            assert!(polygon.contains(line), "{line} ∉ {polygon}");
        }
    }

    #[test]
    fn skipped_declarations_are_reported_with_reasons() {
        // 검증 기준(IX.11.15): 건너뜀 보고 fixture. Constraint CenterX 형태(calc(50% + offset))도 건너뜀.
        let (model, ids) = seven_kind_model();
        let frame = ids[0];
        let token = StableId::new_v7(70, 0, 70);
        let scope: std::collections::BTreeMap<String, StableId> =
            [("--brand".to_string(), token)].into();
        let plan = paste_declarations(
            &model,
            &[frame],
            "left: calc(50% + 4px); padding: 4px; margin: 0; width: 2rem; \
             background-color: var(--brand); background-color: var(--missing); \
             border-radius: 4px / 2px; object-fit: cover; name: x; junk; \
             transform: rotate(30deg); width: 120px",
            &scope,
            &mut new_ids(5),
        )
        .unwrap();
        let skipped: Vec<(&str, &str)> = plan
            .skipped
            .iter()
            .map(|s| (s.name.as_str(), s.code))
            .collect();
        assert_eq!(
            skipped,
            vec![
                ("left", "unsupported-value"),
                ("padding-top", "unknown-property"),
                ("padding-right", "unknown-property"),
                ("padding-bottom", "unknown-property"),
                ("padding-left", "unknown-property"),
                ("margin", "unknown-property"),
                ("width", "unsupported-unit"),
                ("background-color", "unsupported-value"),
                ("background-color", "unknown-token"),
                ("border-radius", "unsupported-value"),
                ("object-fit", "not-applicable"),
                ("name", "not-applicable"),
                ("", "invalid-declaration"),
                ("transform", "unsupported-value"),
            ]
        );
        assert!(plan.skipped.iter().all(|s| !s.message.is_empty()));
        assert_eq!(
            plan.applied,
            vec![AppliedDeclaration {
                name: "width",
                value: "120px".into()
            }]
        );
        assert_eq!(plan.commands.len(), 1);
        // 속성 패널 행이 없는 registry 속성도 붙여넣습니다. Stellar가 저장하지만 붙여넣기 경로가 아직 없는 속성은
        // 저장 위치가 없는 속성과 다른 코드입니다.
        let stored = paste_declarations(
            &model,
            &[ids[3]],
            "opacity: 0.5; mix-blend-mode: multiply; stroke-linecap: round; transform-origin: 0 0; \
             margin-top: 1px",
            &NoTokens,
            &mut new_ids(7),
        )
        .unwrap();
        let codes: Vec<(&str, &str)> = stored
            .skipped
            .iter()
            .map(|s| (s.name.as_str(), s.code))
            .collect();
        assert_eq!(
            codes,
            vec![
                ("transform-origin", "unsupported-property"),
                ("margin-top", "unknown-property"),
            ]
        );
        let applied: Vec<&str> = stored.applied.iter().map(|a| a.name).collect();
        assert_eq!(applied, ["opacity", "mix-blend-mode", "stroke-linecap"]);
        // 블록 전체 오류는 command를 만들지 않습니다.
        let error = paste_declarations(&model, &[frame], "a { b: c", &NoTokens, &mut new_ids(6))
            .unwrap_err();
        assert_eq!(error.code, "invalid-declaration-block");
        assert_eq!(
            paste_declarations(&model, &[], "width: 1px", &NoTokens, &mut new_ids(6))
                .unwrap_err()
                .code,
            "empty-selection"
        );
    }

    #[test]
    fn later_declarations_win_and_modifier_inserts_do_not_collide() {
        let (model, ids) = seven_kind_model();
        let mut state = DocumentState::new(model);
        let ellipse = ids[2];
        let plan = paste_declarations(
            state.model(),
            &[ellipse],
            "fill: red; stroke: blue; stroke-width: 3px; left: 5px; fill: #00ff00",
            &NoTokens,
            &mut new_ids(8),
        )
        .unwrap();
        assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
        execute(&mut state, &plan);
        let style = style_of(state.model(), ellipse);
        assert!(style.contains("fill: #00ff00;"), "{style}");
        assert!(style.contains("stroke: #0000ff;"));
        assert!(style.contains("stroke-width: 3px;"));
        assert!(style.contains("left: 5px;"));
        // Fill·Stroke·Transform modifier가 있는 node에 같은 블록을 다시 붙이면 빈 변경 command가 없습니다.
        let again = paste_declarations(
            state.model(),
            &[ellipse],
            &style,
            &NoTokens,
            &mut new_ids(11),
        )
        .unwrap();
        assert!(again.skipped.is_empty(), "{:?}", again.skipped);
        assert!(again.commands.is_empty(), "{:?}", again.commands);
    }
}
