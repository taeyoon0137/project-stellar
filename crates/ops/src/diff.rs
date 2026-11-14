//! 문서 레코드의 semantic diff 요약 (XXIV.4.4, XIV.3).
//!
//! 두 문서 레코드 집합을 semantic snapshot(XIV.3.1)으로 투영하고 diff(XIV.3.3~XIV.3.8)를 계산해,
//! 출력 형식과 무관한 [`DiffEntry`] 목록으로 바꿉니다. 순서는 semantic-diff의 canonical 순서
//! (XIV.3.8: 문서 → 객체 경로 → 속성 경로 → 변경 종류)를 그대로 따릅니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_document::records::DocumentRecords;
use stellar_semantic_diff::build::SnapshotBuilder;
use stellar_semantic_diff::change::{Position, SemanticChange};
use stellar_semantic_diff::diff::{diff, SemanticDiff};
use stellar_semantic_diff::snapshot::{ObjectKind, SemanticSnapshot};

use crate::core_types::ids::StableId;
use crate::query::{property_path_label, OpsValue};

/// semantic snapshot 구성 실패(레코드 불변식 위반).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionError(pub String);

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "semantic snapshot을 만들 수 없습니다: {}", self.0)
    }
}

impl std::error::Error for ProjectionError {}

/// 위치(부모와 OrderKey 16진수).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPosition {
    /// 부모(`None`이면 root).
    pub parent: Option<StableId>,
    /// OrderKey bytes 16진수(`None`이면 순서 없음).
    pub order_key_hex: Option<String>,
}

impl From<&Position> for DiffPosition {
    fn from(position: &Position) -> DiffPosition {
        DiffPosition {
            parent: position.parent,
            order_key_hex: position
                .order_key
                .as_ref()
                .map(|key| key.as_bytes().iter().map(|b| format!("{b:02x}")).collect()),
        }
    }
}

/// semantic 변경 한 건(표시용).
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    /// 변경 종류 이름(XIV.3.2, 예: `set-property`).
    pub kind: &'static str,
    /// 변경 종류 code.
    pub code: u8,
    /// 소유 문서(프로젝트 범위면 `None`).
    pub document: Option<StableId>,
    /// 정의나 그 tree의 속성이 속한 컴포넌트.
    pub component: Option<StableId>,
    /// 대상 객체.
    pub object: StableId,
    /// 대상 객체 종류(추가·삭제 변경에만, 예: `node:frame`).
    pub object_kind: Option<String>,
    /// 속성 경로([`property_path_label`], 속성 변경에만).
    pub path: Option<String>,
    /// 이전 값(속성 변경, 없으면 추가).
    pub old: Option<OpsValue>,
    /// 이후 값(속성 변경, 없으면 삭제).
    pub new: Option<OpsValue>,
    /// 이전 위치(위치 변경에만).
    pub from: Option<DiffPosition>,
    /// 이후 위치(위치 변경에만).
    pub to: Option<DiffPosition>,
}

/// 객체 종류 표시 이름.
pub fn object_kind_name(kind: ObjectKind) -> String {
    match kind {
        ObjectKind::Project => "project".to_string(),
        ObjectKind::Document => "document".to_string(),
        ObjectKind::Page => "page".to_string(),
        ObjectKind::View => "view".to_string(),
        ObjectKind::Node(kind) => format!("node:{}", kind.name()),
        ObjectKind::Modifier(kind) => format!("modifier:{}", kind.name()),
        ObjectKind::Component => "component".to_string(),
        ObjectKind::ComponentSlot => "component-slot".to_string(),
        ObjectKind::Token => "token".to_string(),
        ObjectKind::Asset => "asset".to_string(),
        ObjectKind::ExpressionNode => "expression-node".to_string(),
        ObjectKind::Variant => "variant".to_string(),
    }
}

impl From<&SemanticChange> for DiffEntry {
    fn from(change: &SemanticChange) -> DiffEntry {
        let kind = change.kind();
        let mut entry = DiffEntry {
            kind: kind.name(),
            code: kind.code(),
            document: change.document(),
            component: change.provenance().definition_component(),
            object: change.object_id(),
            object_kind: None,
            path: None,
            old: None,
            new: None,
            from: None,
            to: None,
        };
        match change {
            SemanticChange::AddObject(c)
            | SemanticChange::RemoveObject(c)
            | SemanticChange::AddToken(c)
            | SemanticChange::RemoveToken(c) => {
                entry.object_kind = Some(object_kind_name(c.object.kind()));
            }
            SemanticChange::MoveObject(c) | SemanticChange::ReorderObject(c) => {
                entry.from = Some(DiffPosition::from(&c.from));
                entry.to = Some(DiffPosition::from(&c.to));
            }
            SemanticChange::SetProperty(c)
            | SemanticChange::ChangeToken(c)
            | SemanticChange::ComponentInterfaceChange(c)
            | SemanticChange::AssetRefChange(c) => {
                entry.path = Some(property_path_label(&c.path));
                entry.old = c.old.as_ref().map(OpsValue::from);
                entry.new = c.new.as_ref().map(OpsValue::from);
            }
        }
        entry
    }
}

/// 문서 레코드들의 semantic snapshot(문서 범위 객체만).
pub fn snapshot_of(documents: &[&DocumentRecords]) -> Result<SemanticSnapshot, ProjectionError> {
    let mut builder = SnapshotBuilder::new();
    for records in documents {
        builder
            .add_document(records)
            .map_err(|e| ProjectionError(e.to_string()))?;
    }
    Ok(builder.finish())
}

/// semantic diff(XIV.3)와 표시용 목록.
#[derive(Debug, Clone)]
pub struct DiffReport {
    /// canonical 순서의 변경.
    pub diff: SemanticDiff,
    /// 표시용 목록(같은 순서).
    pub entries: Vec<DiffEntry>,
}

impl DiffReport {
    /// 변경 종류 이름별 개수(이름 순).
    pub fn counts(&self) -> BTreeMap<&'static str, usize> {
        let mut out = BTreeMap::new();
        for entry in &self.entries {
            *out.entry(entry.kind).or_default() += 1;
        }
        out
    }

    /// 변경이 없는지.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 두 snapshot의 semantic diff.
pub fn diff_snapshots(base: &SemanticSnapshot, target: &SemanticSnapshot) -> DiffReport {
    let diff = diff(base, target);
    let entries = diff.changes().iter().map(DiffEntry::from).collect();
    DiffReport { diff, entries }
}

/// 두 문서 레코드 집합의 semantic diff.
pub fn diff_records(
    base: &[&DocumentRecords],
    target: &[&DocumentRecords],
) -> Result<DiffReport, ProjectionError> {
    Ok(diff_snapshots(&snapshot_of(base)?, &snapshot_of(target)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::random_valid_document;

    #[test]
    fn identical_records_have_no_changes_and_visibility_change_is_one_property_change() {
        let model = random_valid_document(5);
        let records = model.to_records().unwrap();
        assert!(diff_records(&[&records], &[&records]).unwrap().is_empty());

        let mut changed = records.clone();
        let node = changed.nodes.first_mut().unwrap();
        node.visible = !node.visible;
        let target = node.id;
        let report = diff_records(&[&records], &[&changed]).unwrap();
        assert_eq!(report.entries.len(), 1);
        let entry = &report.entries[0];
        assert_eq!(entry.kind, "set-property");
        assert_eq!(entry.object, target);
        assert_eq!(entry.path.as_deref(), Some("0xe001"));
        assert_eq!(entry.document, Some(model.document_id()));
        assert_eq!(report.counts().get("set-property"), Some(&1));
    }
}
