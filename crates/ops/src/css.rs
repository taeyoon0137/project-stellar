//! headless CSS 선언 블록 복사·붙여넣기 (IX.11.15, ADR-0075 결정 6).
//!
//! 앱 속성 패널과 같은 core API(`stellar_commands::css_value`)로 저장된 node 하나의 속성을 longhand 선언 블록으로
//! 복사하고, 선언 블록을 선택 node에 붙여넣습니다. 붙여넣기는 받은 선언을 command로 바꿔 [`crate::apply::apply`]의
//! 한 transaction(base revision 검사, 저장 잠금, 세대 전환 저장)으로 적용하고, 건너뛴 선언은 이유 코드와 함께
//! 돌려줍니다.

use std::cell::RefCell;
use std::io;
use std::path::Path;

use stellar_commands::codec::encode_command;
use stellar_commands::css_value::{
    copy_declarations, declaration_block_text, paste_declarations, AppliedDeclaration,
    CopiedDeclaration, NoTokens, PasteError, SkippedDeclaration,
};
use stellar_project::new_ids::new_ids;

use crate::apply::{apply, ApplyError, ApplyMode, ApplyOutcome, ApplyRequest, CommandInput};
use crate::core_types::ids::StableId;
use crate::error::{DocumentReadError, OpenError};
use crate::lock::LockPolicy;
use crate::revision::ProjectRevision;
use crate::snapshot::open_snapshot;

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
    let declarations =
        copy_declarations(&model, node).ok_or(CssError::UnknownNode { document, node })?;
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
    let plan =
        paste_declarations(&model, nodes, text, &NoTokens, &mut new_id).map_err(CssError::Paste)?;
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
        assert_eq!(pasted.applied.len(), 2, "{:?}", pasted.skipped);
        assert_eq!(pasted.skipped.len(), 4, "padding 4변");
        let outcome = pasted.outcome.expect("저장");
        assert_eq!(outcome.command_count, 2);
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
}
