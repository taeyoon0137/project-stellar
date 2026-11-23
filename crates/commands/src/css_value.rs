//! CSS 값 parser·formatter 재수출과 선언 블록 복사·붙여넣기 core API (UX.2.2, IX.11.13·IX.11.14·IX.11.15).
//!
//! 값 문법 구현은 registry와 같은 crate의 `stellar_document::css::value`에 있습니다(ADR-0075 결정 5). 속성 패널·
//! 선언 블록·CLI·MCP가 같은 parser·formatter를 쓰도록 여기서 그대로 다시 내보냅니다.
//!
//! **복사**([`copy_declarations`]): node 하나의 속성을 속성 패널과 같은 이름·정규 값으로 registry(표시) 순서의
//! longhand 선언 목록으로 냅니다. 묶음 행(`border-radius`)은 longhand로 펼치고, 레이어 속성(`name`·`visible`·
//! `locked`)은 넣지 않습니다. Stellar 전용 속성은 표시를 달아 함께 냅니다.
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

use crate::command::Command;
use crate::id::{CommandId, CommandOrigin};
use crate::inspector::{
    edit_commands, flags, has_fill, inspector_rows, InspectorInput, InspectorProperty, NodeClass,
};
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
                skip(
                    &mut plan,
                    &name,
                    &value,
                    "unknown-property",
                    format!("{name}은 Stellar에 저장 위치가 없는 속성입니다"),
                );
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
                ("transform", "unknown-property"),
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
