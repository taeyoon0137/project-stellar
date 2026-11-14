//! 프로젝트 사이의 semantic diff와 3-way semantic merge (XXIV.1.3).
//!
//! 각 프로젝트 폴더를 snapshot(XXIV.4.2)으로 열고 모든 문서를 semantic snapshot(XIV.3.1)으로
//! 투영합니다. diff는 XIV.3, merge는 XIV.10 엔진을 그대로 씁니다. 두 기능 모두 읽기 전용이며 파일을
//! 쓰지 않습니다. merge 결과를 프로젝트 파일로 기록하는 일(XIV.12.4)은 이 모듈의 범위가 아닙니다.

use std::fmt;
use std::path::Path;

use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_merge::merge::merge;
use stellar_semantic_merge::model::{ConflictTarget, MergeInput, SemanticConflict, SideState};

use crate::core_types::ids::StableId;
use crate::diff::{diff_snapshots, object_kind_name, snapshot_of, DiffPosition, DiffReport};
use crate::error::{DocumentReadError, OpenError};
use crate::query::{property_path_label, OpsValue};
use crate::revision::ProjectRevision;
use crate::snapshot::{open_snapshot, ProjectSnapshot};

/// 비교·병합 실패.
#[derive(Debug)]
pub enum CompareError {
    /// 입력 프로젝트를 열 수 없습니다(`side`: base·target·ours·theirs).
    Open {
        /// 입력 이름.
        side: &'static str,
        /// 원인.
        error: OpenError,
    },
    /// 입력 프로젝트의 문서를 읽을 수 없습니다.
    Document {
        /// 입력 이름.
        side: &'static str,
        /// 원인.
        error: DocumentReadError,
    },
    /// 입력들이 같은 프로젝트(project id)가 아닙니다.
    ProjectMismatch {
        /// 기준 project id.
        expected: StableId,
        /// 다른 입력의 project id.
        found: StableId,
        /// 다른 입력 이름.
        side: &'static str,
    },
    /// semantic snapshot을 만들 수 없습니다.
    Projection(String),
    /// merge 엔진 실패(XIV.10).
    Merge(String),
}

impl fmt::Display for CompareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompareError::Open { side, error } => write!(f, "{side} 프로젝트: {error}"),
            CompareError::Document { side, error } => write!(f, "{side} 프로젝트: {error}"),
            CompareError::ProjectMismatch {
                expected,
                found,
                side,
            } => write!(
                f,
                "{side} 프로젝트의 project id {found}가 기준 {expected}와 다릅니다"
            ),
            CompareError::Projection(e) => write!(f, "semantic snapshot 실패: {e}"),
            CompareError::Merge(e) => write!(f, "semantic merge 실패: {e}"),
        }
    }
}

impl std::error::Error for CompareError {}

/// 열린 입력 하나.
struct Side {
    snapshot: ProjectSnapshot,
    semantic: SemanticSnapshot,
}

fn open_side(path: &Path, side: &'static str) -> Result<Side, CompareError> {
    let snapshot = open_snapshot(path).map_err(|error| CompareError::Open { side, error })?;
    let semantic = project_semantic_snapshot(&snapshot, side)?;
    Ok(Side { snapshot, semantic })
}

/// snapshot의 모든 문서를 semantic snapshot 하나로 투영합니다.
fn project_semantic_snapshot(
    snapshot: &ProjectSnapshot,
    side: &'static str,
) -> Result<SemanticSnapshot, CompareError> {
    let mut records = Vec::new();
    for definition in snapshot.documents() {
        records.push(
            snapshot
                .document_records(definition.id)
                .map_err(|error| CompareError::Document { side, error })?,
        );
    }
    let refs: Vec<_> = records.iter().collect();
    snapshot_of(&refs).map_err(|e| CompareError::Projection(e.0))
}

fn same_project(base: &Side, other: &Side, side: &'static str) -> Result<(), CompareError> {
    if base.snapshot.project_id() != other.snapshot.project_id() {
        return Err(CompareError::ProjectMismatch {
            expected: base.snapshot.project_id(),
            found: other.snapshot.project_id(),
            side,
        });
    }
    Ok(())
}

/// 두 프로젝트의 semantic diff 결과.
#[derive(Debug, Clone)]
pub struct ProjectDiff {
    /// base revision.
    pub base_revision: ProjectRevision,
    /// target revision.
    pub target_revision: ProjectRevision,
    /// base → target 변경.
    pub report: DiffReport,
}

/// 같은 프로젝트의 두 상태(예: 두 worktree·checkout) 사이 semantic diff.
pub fn semantic_diff_projects(base: &Path, target: &Path) -> Result<ProjectDiff, CompareError> {
    let base = open_side(base, "base")?;
    let target = open_side(target, "target")?;
    same_project(&base, &target, "target")?;
    Ok(ProjectDiff {
        base_revision: base.snapshot.revision(),
        target_revision: target.snapshot.revision(),
        report: diff_snapshots(&base.semantic, &target.semantic),
    })
}

/// conflict 한쪽의 표시용 상태.
#[derive(Debug, Clone, PartialEq)]
pub enum SideView {
    /// 객체가 없음.
    Missing,
    /// 객체가 있음(종류 이름).
    Object(String),
    /// 객체 위치.
    Position(DiffPosition),
    /// 속성 값(`None` = 속성 없음).
    Value(Option<OpsValue>),
}

impl From<&SideState> for SideView {
    fn from(state: &SideState) -> SideView {
        match state {
            SideState::Missing => SideView::Missing,
            SideState::Object(object) => SideView::Object(object_kind_name(object.kind())),
            SideState::Position(position) => SideView::Position(DiffPosition::from(position)),
            SideState::Value(value) => SideView::Value(value.as_ref().map(OpsValue::from)),
        }
    }
}

/// semantic conflict 한 건(표시용).
#[derive(Debug, Clone, PartialEq)]
pub struct ConflictEntry {
    /// conflict 종류 이름(XIV.10, 예: `property-value`).
    pub kind: &'static str,
    /// 소유 문서(프로젝트 범위면 `None`).
    pub document: Option<StableId>,
    /// 대상 객체.
    pub object: StableId,
    /// 대상 종류: `object`, `position`, `property`.
    pub target: &'static str,
    /// 속성 경로(속성 conflict에만).
    pub path: Option<String>,
    /// base 상태.
    pub base: SideView,
    /// ours 상태.
    pub ours: SideView,
    /// theirs 상태.
    pub theirs: SideView,
}

impl From<&SemanticConflict> for ConflictEntry {
    fn from(conflict: &SemanticConflict) -> ConflictEntry {
        let (target, path) = match &conflict.target {
            ConflictTarget::Object => ("object", None),
            ConflictTarget::Position => ("position", None),
            ConflictTarget::Property(path) => ("property", Some(property_path_label(path))),
        };
        ConflictEntry {
            kind: conflict.kind.name(),
            document: conflict.provenance.document(),
            object: conflict.object,
            target,
            path,
            base: SideView::from(&conflict.base),
            ours: SideView::from(&conflict.ours),
            theirs: SideView::from(&conflict.theirs),
        }
    }
}

/// 3-way merge 결과.
#[derive(Debug, Clone)]
pub struct ProjectMerge {
    /// base revision.
    pub base_revision: ProjectRevision,
    /// ours revision.
    pub ours_revision: ProjectRevision,
    /// theirs revision.
    pub theirs_revision: ProjectRevision,
    /// base → 자동 병합 결과 변경(conflict 대상은 base 유지).
    pub changes: DiffReport,
    /// 자동 병합하지 못한 conflict(엔진 순서).
    pub conflicts: Vec<ConflictEntry>,
}

impl ProjectMerge {
    /// conflict가 없는지.
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// 같은 프로젝트의 base·ours·theirs 상태를 3-way semantic merge합니다(XIV.10, 읽기 전용).
pub fn semantic_merge_projects(
    base: &Path,
    ours: &Path,
    theirs: &Path,
) -> Result<ProjectMerge, CompareError> {
    let base = open_side(base, "base")?;
    let ours = open_side(ours, "ours")?;
    let theirs = open_side(theirs, "theirs")?;
    same_project(&base, &ours, "ours")?;
    same_project(&base, &theirs, "theirs")?;
    let input = MergeInput {
        base: base.semantic.clone(),
        ours: ours.semantic,
        theirs: theirs.semantic,
    };
    let result = merge(&input).map_err(|e| CompareError::Merge(e.to_string()))?;
    let changes = diff_snapshots(&base.semantic, &result.merged);
    Ok(ProjectMerge {
        base_revision: base.snapshot.revision(),
        ours_revision: ours.snapshot.revision(),
        theirs_revision: theirs.snapshot.revision(),
        changes,
        conflicts: result.conflicts.iter().map(ConflictEntry::from).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
    use crate::lock::LockPolicy;
    use crate::test_support::{
        file_digest, random_valid_document, save_documents, scaffold_project, TempDir,
    };

    /// 같은 문서 seed로 저장한 세 사본(base·ours·theirs).
    fn three_copies(seed: u64) -> (TempDir, Vec<std::path::PathBuf>, StableId, Vec<StableId>) {
        let dir = TempDir::new("compare");
        let mut roots = Vec::new();
        for name in ["base", "ours", "theirs"] {
            let root = dir.path.join(name);
            scaffold_project(&root);
            save_documents(&root, &[&random_valid_document(seed)]);
            roots.push(root);
        }
        let model = random_valid_document(seed);
        let children: Vec<StableId> = model
            .to_records()
            .unwrap()
            .nodes
            .iter()
            .filter(|n| n.parent.is_some())
            .map(|n| n.id)
            .collect();
        (dir, roots, model.document_id(), children)
    }

    fn edit(root: &Path, document: StableId, command: CommandInput) {
        let base = open_snapshot(root).unwrap().revision();
        let request = ApplyRequest {
            base_revision: base,
            document,
            commands: vec![command],
        };
        apply(root, &request, ApplyMode::Write, &LockPolicy::None).unwrap();
    }

    #[test]
    fn diff_between_copies_reports_only_the_edit() {
        let (_dir, roots, document, children) = three_copies(7);
        assert!(semantic_diff_projects(&roots[0], &roots[1])
            .unwrap()
            .report
            .is_empty());
        edit(
            &roots[1],
            document,
            CommandInput::Rename {
                node: children[0],
                name: "ours".into(),
            },
        );
        let before = file_digest(&roots[0]);
        let diff = semantic_diff_projects(&roots[0], &roots[1]).unwrap();
        assert_eq!(diff.report.entries.len(), 1);
        assert_eq!(diff.report.entries[0].object, children[0]);
        assert_eq!(diff.target_revision.generation(), 2);
        assert_eq!(file_digest(&roots[0]), before);
    }

    #[test]
    fn non_overlapping_edits_merge_cleanly_and_same_property_conflicts() {
        let (_dir, roots, document, children) = three_copies(7);
        assert!(children.len() >= 2);
        edit(
            &roots[1],
            document,
            CommandInput::Rename {
                node: children[0],
                name: "ours".into(),
            },
        );
        edit(
            &roots[2],
            document,
            CommandInput::SetLocked {
                node: children[1],
                locked: true,
            },
        );
        let clean = semantic_merge_projects(&roots[0], &roots[1], &roots[2]).unwrap();
        assert!(clean.is_clean(), "{:?}", clean.conflicts);
        assert_eq!(clean.changes.entries.len(), 2);

        // 같은 속성을 다른 값으로 바꾸면 conflict이고 세 값이 모두 남습니다.
        edit(
            &roots[2],
            document,
            CommandInput::Rename {
                node: children[0],
                name: "theirs".into(),
            },
        );
        let conflicted = semantic_merge_projects(&roots[0], &roots[1], &roots[2]).unwrap();
        assert_eq!(conflicted.conflicts.len(), 1);
        let conflict = &conflicted.conflicts[0];
        assert_eq!(conflict.kind, "property-value");
        assert_eq!(conflict.object, children[0]);
        assert_eq!(conflict.target, "property");
        assert_eq!(
            conflict.ours,
            SideView::Value(Some(OpsValue::Text("ours".into())))
        );
        assert_eq!(
            conflict.theirs,
            SideView::Value(Some(OpsValue::Text("theirs".into())))
        );
    }

    #[test]
    fn different_projects_are_rejected() {
        let (dir, roots, _, _) = three_copies(7);
        let other = dir.path.join("other");
        std::fs::create_dir_all(&other).unwrap();
        stellar_project::scaffold::write_default_scaffold(
            &other,
            "Other",
            "0195c530-3acd-7bc1-8414-c213f61cb111",
        )
        .unwrap();
        std::fs::create_dir_all(other.join(".git")).unwrap();
        assert!(matches!(
            semantic_diff_projects(&roots[0], &other),
            Err(CompareError::ProjectMismatch { side: "target", .. })
        ));
    }
}
