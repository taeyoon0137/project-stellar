//! headless CSS 선언 블록 복사·붙여넣기 (IX.11.15, ADR-0075 결정 6).
//!
//! 앱 속성 패널과 같은 core API(`stellar_commands::css_value`)로 저장된 node 하나의 속성을 longhand 선언 블록으로
//! 복사하고, 선언 블록을 선택 node에 붙여넣습니다. 붙여넣기는 받은 선언을 command로 바꿔 [`crate::apply::apply`]의
//! 한 transaction(base revision 검사, 저장 잠금, 세대 전환 저장)으로 적용하고, 건너뛴 선언은 이유 코드와 함께
//! 돌려줍니다.
//!
//! **명시 layout longhand(ADR-0086)**: 속성 패널 행이 없는 CSS layout longhand(`display`, `padding-*`, `flex-*`,
//! `gap`의 longhand, `grid-*`, `position`, `right`·`bottom`, Text·Path·Line·ComponentInstance의 `width`·`height` 등)는
//! node의 layout record에 명시 값으로 둡니다([`paste_plan`]). 값은 텍스트 원본 `layout` 줄과 같은 문서 parser
//! (`LayoutLonghand::parse`)로 읽고, 받을 수 없는 node 종류와 잠긴 node는 건너뜁니다. 속성 패널 행이 선택한 모든
//! node에 있는 이름(`left`·`top`은 위치 transform, payload 크기 종류의 `width`·`height`)은 지금처럼 속성 패널 경로가
//! 받습니다(위치·transform 분리는 IX.11.9). 복사([`copy`])도 같은 규칙으로 명시 layout longhand를 함께 냅니다.
//!
//! 선언 블록 크기 상한은 [`MAX_DECLARATION_BYTES`] 하나이며 CLI·MCP가 이 값을 씁니다.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use stellar_commands::codec::encode_command;
use stellar_commands::command::Command;
use stellar_commands::css_value::{
    copy_declarations, declaration_block_text, paste_declarations, AppliedDeclaration,
    CopiedDeclaration, NoTokens, PasteError, PastePlan, SkippedDeclaration,
};
use stellar_commands::inspector::{InspectorProperty, NodeClass};
use stellar_commands::lock::{EditIntent, NodeAccess};
use stellar_commands::style_defaults::SetLayoutProperty;
use stellar_document::css::declaration::{format_block, parse_block, BlockItem, MAX_BLOCK_LEN};
use stellar_document::css::shorthand;
use stellar_document::layout_record::{
    format_value, longhand_violation, LayoutLonghand, LayoutRecord,
};
use stellar_document::model::DocumentModel;
use stellar_project::new_ids::new_ids;

use crate::apply::{apply, ApplyError, ApplyMode, ApplyOutcome, ApplyRequest, CommandInput};
use crate::core_types::ids::StableId;
use crate::error::{DocumentReadError, OpenError};
use crate::lock::LockPolicy;
use crate::revision::ProjectRevision;
use crate::snapshot::open_snapshot;

/// 선언 블록 최대 byte 수. 문서 선언 블록 parser 상한(`MAX_BLOCK_LEN`)과 같고, `stellar css paste` 입력과 MCP
/// `document.propose_css_paste` 인자가 이 값을 씁니다.
pub const MAX_DECLARATION_BYTES: usize = MAX_BLOCK_LEN;

/// 이름 `name`을 `nodes` 모두에서 명시 layout longhand로 받는지. 속성 패널 행이 모든 node에 있으면 속성 패널
/// 경로가 받습니다. 그 밖의 layout longhand 이름은 layout record로 갑니다(종류에 붙지 않으면 그 단계에서 건너뜀).
fn layout_longhand(
    model: &DocumentModel,
    nodes: &[StableId],
    name: &str,
) -> Option<LayoutLonghand> {
    let longhand = LayoutLonghand::from_name(name)?;
    let inspector = InspectorProperty::from_name(name).is_some_and(|property| {
        nodes.iter().all(|node| {
            model
                .payload(*node)
                .is_some_and(|payload| property.applies(NodeClass::of(payload)))
        })
    });
    (!inspector).then_some(longhand)
}

fn skipped(name: &str, value: &str, code: &'static str, message: String) -> SkippedDeclaration {
    SkippedDeclaration {
        name: name.to_string(),
        value: value.to_string(),
        code,
        message,
    }
}

/// 선언 하나를 `nodes`의 layout record에 둘 수 있는지 보고, 받으면 node별 새 값을 `pending`에 씁니다.
fn plan_layout(
    model: &DocumentModel,
    nodes: &[StableId],
    longhand: LayoutLonghand,
    value: &str,
    pending: &mut BTreeMap<StableId, LayoutRecord>,
) -> Result<AppliedDeclaration, SkippedDeclaration> {
    let name = longhand.name();
    let parsed = longhand
        .parse(value)
        .map_err(|e| skipped(name, value, e.code, e.message))?;
    for node in nodes {
        let payload = model.payload(*node).ok_or_else(|| {
            skipped(
                name,
                value,
                "rejected",
                format!("문서에 없는 레이어: {node}"),
            )
        })?;
        if let Some(violation) = longhand_violation(*node, longhand, payload) {
            return Err(skipped(
                name,
                value,
                "not-applicable",
                violation.to_string(),
            ));
        }
        let access = NodeAccess::of(model, *node).ok_or_else(|| {
            skipped(
                name,
                value,
                "rejected",
                format!("문서에 없는 레이어: {node}"),
            )
        })?;
        access
            .permits(EditIntent::Modify(longhand.key_for(payload)))
            .map_err(|blocked| {
                skipped(
                    name,
                    value,
                    "rejected",
                    format!("{blocked:?}: 이 레이어의 속성을 바꿀 수 없습니다"),
                )
            })?;
    }
    for node in nodes {
        let record = pending.entry(*node).or_insert_with(|| {
            model
                .layout_record(*node)
                .cloned()
                .unwrap_or_else(|| LayoutRecord::new(*node))
        });
        record
            .set(longhand, parsed.clone())
            .map_err(|e| skipped(name, value, "invalid-value", e.to_string()))?;
    }
    Ok(AppliedDeclaration {
        name,
        value: format_value(&parsed),
    })
}

/// 선언 블록 `text`를 `nodes`에 붙여넣을 command 계획을 만듭니다. 속성 패널 경로(`paste_declarations`)가 받는
/// 선언은 그 경로로, 명시 layout longhand는 layout record로 바꿉니다(모듈 설명). shorthand는 먼저 펼쳐 longhand마다
/// 판정하고, 펼친 longhand가 하나도 layout record로 가지 않으면 원래 선언 그대로 속성 패널 경로에 넘깁니다. 같은
/// longhand가 두 번 오면 CSS처럼 뒤 선언이 이깁니다. layout 결과(command·적용·건너뜀)는 속성 패널 경로 결과 뒤에
/// 붙습니다. 값이 이미 같은 명시 값이면 command가 없습니다(세트 값과 같아도 명시로 둡니다).
pub fn paste_plan(
    model: &DocumentModel,
    nodes: &[StableId],
    text: &str,
    new_id: &mut dyn FnMut() -> StableId,
) -> Result<PastePlan, PasteError> {
    // 선택 검사는 속성 패널 경로와 같은 code입니다. 모든 선언이 layout record로 가서 그 경로를 부르지 않아도
    // 빈 선택·없는 레이어를 성공으로 보고하지 않습니다.
    if nodes.is_empty() {
        return Err(PasteError {
            code: "empty-selection",
            message: "선택한 레이어가 없습니다".to_string(),
        });
    }
    if let Some(node) = nodes.iter().find(|node| model.payload(**node).is_none()) {
        return Err(PasteError {
            code: "unknown-node",
            message: format!("문서에 없는 레이어: {node}"),
        });
    }
    let items = parse_block(text).map_err(|e| PasteError {
        code: e.code,
        message: e.message,
    })?;
    let mut rest: Vec<(String, String)> = Vec::new();
    let mut applied = Vec::new();
    let mut skips = Vec::new();
    let mut pending: BTreeMap<StableId, LayoutRecord> = BTreeMap::new();
    for item in items {
        let declaration = match item {
            BlockItem::Declaration(declaration) => declaration,
            // 형식이 틀린 조각은 속성 패널 경로와 같은 내용으로 건너뜁니다.
            BlockItem::Malformed(malformed) => {
                skips.push(SkippedDeclaration {
                    name: String::new(),
                    value: malformed.text,
                    code: malformed.code,
                    message: malformed.message,
                });
                continue;
            }
        };
        let longhands: Vec<(String, String)> =
            match shorthand::expand(&declaration.name, &declaration.value) {
                Some(Ok(expanded))
                    if expanded
                        .iter()
                        .any(|l| layout_longhand(model, nodes, l.name).is_some()) =>
                {
                    expanded
                        .into_iter()
                        .map(|l| (l.name.to_string(), l.value))
                        .collect()
                }
                _ => vec![(declaration.name.clone(), declaration.value.clone())],
            };
        for (name, value) in longhands {
            match layout_longhand(model, nodes, &name) {
                Some(longhand) => match plan_layout(model, nodes, longhand, &value, &mut pending) {
                    Ok(done) => applied.push(done),
                    Err(skip) => skips.push(skip),
                },
                None => rest.push((name, value)),
            }
        }
    }
    let mut plan = if rest.is_empty() {
        PastePlan::default()
    } else {
        let block = format_block(rest.iter().map(|(n, v)| (n.as_str(), v.as_str())));
        paste_declarations(model, nodes, &block, &NoTokens, new_id)?
    };
    for record in pending.into_values() {
        let current = model.layout_record(record.node);
        for (longhand, value) in record.iter() {
            if current.and_then(|c| c.get(longhand)) != Some(value) {
                plan.commands
                    .push(Command::SetLayoutProperty(SetLayoutProperty {
                        node: record.node,
                        longhand,
                        value: value.clone(),
                    }));
            }
        }
    }
    plan.applied.extend(applied);
    plan.skipped.extend(skips);
    Ok(plan)
}

/// 복사 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssCopy {
    /// 읽은 revision.
    pub revision: ProjectRevision,
    /// 문서.
    pub document: StableId,
    /// node.
    pub node: StableId,
    /// 선언(registry 순서의 longhand).
    pub declarations: Vec<CopiedDeclaration>,
    /// 선언 블록 텍스트.
    pub text: String,
}

/// 붙여넣기 결과.
#[derive(Debug)]
pub struct CssPaste {
    /// 붙여넣기 전 revision.
    pub base_revision: ProjectRevision,
    /// 적용한 선언.
    pub applied: Vec<AppliedDeclaration>,
    /// 건너뛴 선언.
    pub skipped: Vec<SkippedDeclaration>,
    /// 바꿀 것이 있었으면 적용 결과(dry-run이면 diff만).
    pub outcome: Option<ApplyOutcome>,
}

/// 복사·붙여넣기 실패.
#[derive(Debug)]
pub enum CssError {
    /// 프로젝트를 열 수 없습니다.
    Open(OpenError),
    /// 문서를 읽을 수 없습니다.
    Document(DocumentReadError),
    /// 문서에 node가 없습니다.
    UnknownNode {
        /// 문서.
        document: StableId,
        /// node.
        node: StableId,
    },
    /// 선언 블록 전체가 잘못되었습니다.
    Paste(PasteError),
    /// 새 modifier ID를 만들지 못했습니다(OS 난수).
    Ids(io::Error),
    /// 적용에 실패했습니다(프로젝트는 바뀌지 않음).
    Apply(ApplyError),
}

/// `document`의 `node` 속성을 선언 블록으로 복사합니다.
pub fn copy(path: &Path, document: StableId, node: StableId) -> Result<CssCopy, CssError> {
    let snapshot = open_snapshot(path).map_err(CssError::Open)?;
    let model = snapshot
        .document_model(document)
        .map_err(CssError::Document)?;
    let mut declarations =
        copy_declarations(&model, node).ok_or(CssError::UnknownNode { document, node })?;
    // 명시 layout longhand는 붙여넣기가 layout record로 받는 이름만 냅니다(같은 이름의 속성 패널 행과 겹치지 않음).
    if let Some(record) = model.layout_record(node) {
        declarations.extend(
            record
                .iter()
                .filter(|(longhand, _)| {
                    layout_longhand(&model, &[node], longhand.name()) == Some(*longhand)
                })
                .map(|(longhand, value)| CopiedDeclaration {
                    name: longhand.name(),
                    value: format_value(value),
                    stellar_only: false,
                }),
        );
    }
    Ok(CssCopy {
        revision: snapshot.revision(),
        document,
        node,
        text: declaration_block_text(&declarations),
        declarations,
    })
}

/// 선언 블록 `text`를 `nodes`에 붙여넣습니다. 받은 선언은 모두 한 transaction이고, 건너뛴 선언은 문서를 바꾸지
/// 않습니다. 바꿀 것이 없으면 저장하지 않습니다(`outcome`이 `None`).
pub fn paste(
    path: &Path,
    document: StableId,
    nodes: &[StableId],
    text: &str,
    mode: ApplyMode,
    lock: &LockPolicy,
) -> Result<CssPaste, CssError> {
    let snapshot = open_snapshot(path).map_err(CssError::Open)?;
    let model = snapshot
        .document_model(document)
        .map_err(CssError::Document)?;
    if let Some(node) = nodes.iter().find(|n| model.record(**n).is_none()) {
        return Err(CssError::UnknownNode {
            document,
            node: *node,
        });
    }
    let id_error: RefCell<Option<io::Error>> = RefCell::new(None);
    let mut new_id = || match new_ids::<1>() {
        Ok([id]) => id,
        Err(error) => {
            id_error.borrow_mut().get_or_insert(error);
            StableId::NIL
        }
    };
    let plan = paste_plan(&model, nodes, text, &mut new_id).map_err(CssError::Paste)?;
    if let Some(error) = id_error.into_inner() {
        return Err(CssError::Ids(error));
    }
    let base_revision = snapshot.revision();
    let outcome = if plan.commands.is_empty() {
        None
    } else {
        let request = ApplyRequest {
            base_revision,
            document,
            commands: plan
                .commands
                .iter()
                .map(|command| CommandInput::Canonical {
                    kind: command.kind().name().to_string(),
                    bytes: encode_command(command),
                })
                .collect(),
        };
        Some(apply(path, &request, mode, lock).map_err(CssError::Apply)?)
    };
    Ok(CssPaste {
        base_revision,
        applied: plan.applied,
        skipped: plan.skipped,
        outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{large_document, save_documents, scaffold_project, TempDir};
    use stellar_document::records::NodePayload;

    #[test]
    fn copy_then_paste_through_saved_project_writes_one_transaction() {
        let tmp = TempDir::new("css-copy-paste");
        let root = tmp.path.join("project");
        scaffold_project(&root);
        let model = large_document(4);
        save_documents(&root, &[&model]);
        let document = model.document_id();
        let frame = model
            .to_records()
            .unwrap()
            .nodes
            .iter()
            .find(|n| matches!(n.payload, NodePayload::Frame(_)) && !n.locked)
            .map(|n| n.id)
            .expect("Frame");
        let copied = copy(&root, document, frame).unwrap();
        assert!(copied.text.contains("width: "), "{}", copied.text);
        assert!(copied
            .declarations
            .iter()
            .all(|d| !d.name.contains('.') && d.name != "size"));
        let pasted = paste(
            &root,
            document,
            &[frame],
            "width: 321px; background-color: #ff0000; padding: 4px",
            ApplyMode::Write,
            &LockPolicy::None,
        )
        .unwrap();
        // `padding`은 4변 longhand로 펼쳐 Frame의 명시 layout record에 둡니다(ADR-0086 개정 5).
        assert_eq!(pasted.applied.len(), 6, "{:?}", pasted.skipped);
        assert!(pasted.skipped.is_empty(), "{:?}", pasted.skipped);
        let outcome = pasted.outcome.expect("저장");
        assert_eq!(outcome.command_count, 6);
        assert_ne!(outcome.revision, pasted.base_revision);
        let after = copy(&root, document, frame).unwrap();
        assert!(after.text.contains("width: 321px;\n"), "{}", after.text);
        assert!(after.text.contains("background-color: #ff0000;\n"));
        // 같은 블록을 다시 붙이면 바꿀 것이 없어 저장하지 않습니다.
        let again = paste(
            &root,
            document,
            &[frame],
            &after.text,
            ApplyMode::Write,
            &LockPolicy::None,
        )
        .unwrap();
        assert!(again.outcome.is_none());
        assert!(again.skipped.is_empty(), "{:?}", again.skipped);
        assert!(matches!(
            copy(&root, document, StableId::new_v7(1, 2, 3)),
            Err(CssError::UnknownNode { .. })
        ));
    }

    #[test]
    fn layout_longhands_paste_into_layout_records_and_copy_back() {
        use stellar_document::layout_record::LayoutLonghand;
        use stellar_document::records::NodeRecord;
        use stellar_document::text::TextPayload;

        let tmp = TempDir::new("css-layout-paste");
        let root = tmp.path.join("project");
        scaffold_project(&root);
        let mut model = large_document(4);
        let records = model.to_records().unwrap();
        let frame = records
            .nodes
            .iter()
            .find(|n| n.parent.is_none())
            .unwrap()
            .id;
        let text = StableId::new_v7(1_900_000_000_500, 0x0123, 500);
        model
            .insert_node(NodeRecord {
                id: text,
                parent: Some(frame),
                order_key: model.next_child_order_key(frame),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload: NodePayload::Text(TextPayload::empty(0)),
            })
            .unwrap();
        save_documents(&root, &[&model]);
        let document = model.document_id();
        let paste_into = |nodes: &[StableId], text: &str| {
            paste(
                &root,
                document,
                nodes,
                text,
                ApplyMode::Write,
                &LockPolicy::None,
            )
            .unwrap()
        };
        // shorthand(`padding`, `gap`)는 longhand로 펼쳐 layout record에 둡니다. Frame `width`는 그대로 payload입니다.
        let pasted = paste_into(
            &[frame],
            "display: flex; padding: 4px 8px; gap: 2px; width: 300px; background-color: #00ff00",
        );
        assert!(pasted.skipped.is_empty(), "{:?}", pasted.skipped);
        let names: Vec<&str> = pasted.applied.iter().map(|a| a.name).collect();
        assert_eq!(
            names,
            [
                "width",
                "background-color",
                "display",
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
                "row-gap",
                "column-gap"
            ]
        );
        let reopened = open_snapshot(&root)
            .unwrap()
            .document_model(document)
            .unwrap();
        let record = reopened.layout_record(frame).unwrap();
        assert_eq!(record.len(), 7);
        assert_eq!(
            record.get(LayoutLonghand::PaddingRight),
            Some(&LayoutLonghand::PaddingRight.parse("8px").unwrap())
        );
        // Text의 `width`는 record 크기이고, container longhand는 Text에 붙지 않아 건너뜁니다.
        let pasted = paste_into(&[text], "width: 120px; display: grid; flex-grow: 2");
        assert_eq!(pasted.applied.len(), 2, "{:?}", pasted.skipped);
        assert_eq!(pasted.skipped.len(), 1);
        assert_eq!(pasted.skipped[0].name, "display");
        assert_eq!(pasted.skipped[0].code, "not-applicable");
        // 값 오류는 문서 parser의 code로 건너뜁니다.
        let pasted = paste_into(&[frame], "column-gap: 1em; flex-direction: sideways");
        assert!(pasted.outcome.is_none());
        assert_eq!(pasted.skipped.len(), 2);
        // 복사는 명시 layout longhand를 함께 내고, 그 블록을 다시 붙이면 바꿀 것이 없습니다.
        let copied = copy(&root, document, frame).unwrap();
        assert!(copied.text.contains("display: flex;\n"), "{}", copied.text);
        assert!(
            copied.text.contains("padding-left: 8px;\n"),
            "{}",
            copied.text
        );
        let again = paste_into(&[frame], &copied.text);
        assert!(again.outcome.is_none(), "{:?}", again.applied);
        let copied = copy(&root, document, text).unwrap();
        assert!(copied.text.contains("width: 120px;\n"), "{}", copied.text);
        assert!(copied.text.contains("flex-grow: 2;\n"), "{}", copied.text);
        // layout longhand만 있는 블록도 빈 선택·없는 레이어는 속성 패널 경로와 같은 오류입니다.
        let mut ids = || StableId::NIL;
        assert_eq!(
            paste_plan(&reopened, &[], "display: flex", &mut ids)
                .unwrap_err()
                .code,
            "empty-selection"
        );
        let missing = StableId::new_v7(1_900_000_000_900, 0x0123, 900);
        assert_eq!(
            paste_plan(&reopened, &[frame, missing], "display: flex", &mut ids)
                .unwrap_err()
                .code,
            "unknown-node"
        );
    }
}
