//! 프로젝트 사이의 semantic diff와 3-way semantic merge (XXIV.1.3).
//!
//! 각 프로젝트 폴더를 snapshot(XXIV.4.2)으로 열고 모든 문서를 semantic snapshot(XIV.3.1)으로
//! 투영합니다. diff는 XIV.3, merge는 XIV.10 엔진을 그대로 씁니다. 두 기능 모두 읽기 전용이며 파일을
//! 쓰지 않습니다. merge 결과를 프로젝트 파일로 기록하는 일(XIV.12.4)은 이 모듈의 범위가 아닙니다.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::Path;

use stellar_project::document_io::component_source::{
    project_source_index, source_set_hash, ProjectSourceIndex,
};
use stellar_semantic_diff::build::SnapshotBuilder;
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_merge::candidate::CandidateSnapshot;
use stellar_semantic_merge::model::{ConflictTarget, MergeInput, SemanticConflict, SideState};
use stellar_storage::hash::ContentHash;

use crate::core_types::ids::StableId;
use crate::diff::{
    add_contexts, diff_snapshots, object_kind_name, projectable_context, DiffPosition, DiffReport,
    DocumentSource,
};
use crate::error::{DocumentReadError, OpenError};
use crate::query::{property_path_label, OpsValue};
use crate::revision::ProjectRevision;
use crate::snapshot::{open_snapshot, ProjectSnapshot};
use stellar_document::context::set::ContextSet;

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
    /// 입력의 프로젝트 컴포넌트 원본을 읽을 수 없습니다.
    ComponentSource {
        /// 입력 이름.
        side: &'static str,
        /// 원본 검증 오류.
        error: crate::text_source::TextSourceOperationError,
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
            CompareError::ComponentSource { side, error } => write!(f, "{side} 컴포넌트: {error}"),
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
    let (mut builder, documents) = document_builder(&snapshot, side)?;
    let (library, _) = crate::component_source::project_library(&snapshot)
        .map_err(|error| CompareError::ComponentSource { side, error })?;
    builder
        .add_project_component_library(&library)
        .map_err(|error| CompareError::Projection(error.to_string()))?;
    add_contexts(
        &mut builder,
        documents.iter().map(DocumentSource::input),
        Some(&library),
    )
    .map_err(|error| CompareError::Projection(error.0))?;
    Ok(Side {
        snapshot,
        semantic: builder.finish(),
    })
}

/// 문서를 넣은 builder와 문서(레코드 + Context). Context는 컴포넌트 library를 넣은 뒤 [`add_contexts`]로 넣습니다
/// (requirement·Slot 공개 목록이 Component·Slot 객체에 붙음, ADR-0114 결정 2).
fn document_builder(
    snapshot: &ProjectSnapshot,
    side: &'static str,
) -> Result<(SnapshotBuilder, Vec<DocumentSource>), CompareError> {
    let mut builder = SnapshotBuilder::new();
    let mut documents = Vec::new();
    for definition in snapshot.documents() {
        let model = snapshot
            .document_model(definition.id)
            .map_err(|error| CompareError::Document { side, error })?;
        let document =
            DocumentSource::of_model(&model).map_err(|error| CompareError::Projection(error.0))?;
        builder
            .add_document(&document.records)
            .map_err(|error| CompareError::Projection(error.to_string()))?;
        documents.push(document);
    }
    Ok((builder, documents))
}

/// Context에서 requirement·Slot 공개 목록만 남긴 사본(문서별). 단일 컴포넌트 비교가 그 컴포넌트 객체에 다시 붙입니다.
fn context_members(documents: &[DocumentSource]) -> Vec<(StableId, ContextSet)> {
    documents
        .iter()
        .filter_map(|document| {
            let mut members = ContextSet::new();
            for (component, keys) in document.context.all_requirements() {
                for key in keys {
                    members.add_requirement(component, *key);
                }
            }
            for (slot, keys) in document.context.all_slot_keys() {
                for key in keys {
                    members.add_slot_key(slot, *key);
                }
            }
            (!members.is_empty()).then_some((document.records.definition.id, members))
        })
        .collect()
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

/// YAML을 해석하지 않고 파일 이름·바이트의 집합을 관측합니다.
pub fn working_components(snapshot: &ProjectSnapshot) -> Result<ProjectSourceIndex, CompareError> {
    let access = crate::component_source::external_access(snapshot.project_id(), snapshot.root());
    project_source_index(snapshot.root(), access).map_err(|error| CompareError::ComponentSource {
        side: "target",
        error: error.into(),
    })
}

/// 현재 컴포넌트 원본 집합의 hash만 필요한 호출자.
pub fn working_component_hash(snapshot: &ProjectSnapshot) -> Result<ContentHash, CompareError> {
    Ok(working_components(snapshot)?.hash())
}

/// 불변 Git commit에서 한 번 읽고 검증한 semantic 비교 기준.
#[derive(Debug)]
pub struct HeadDiffBase {
    commit: String,
    root: std::path::PathBuf,
    project: StableId,
    manifest_hash: ContentHash,
    components: BTreeMap<OsString, (StableId, ContentHash)>,
    /// 문서별 Context requirement·Slot 공개 목록(단일 컴포넌트 비교용).
    members: Vec<(StableId, ContextSet)>,
    semantic: SemanticSnapshot,
}

impl HeadDiffBase {
    /// 고정한 commit의 문서·컴포넌트 ID·원본을 모두 검증합니다.
    pub fn open(
        head: &crate::trusted::ProjectHead,
        target: &ProjectSnapshot,
    ) -> Result<Self, CompareError> {
        let unavailable = || {
            CompareError::Projection("HEAD에서 같은 프로젝트의 저장 세대를 열 수 없습니다".into())
        };
        if head.root() != target.root() || target.generation().is_none() {
            return Err(unavailable());
        }
        let base = head.saved(target.project_id()).map_err(|_| unavailable())?;
        let saved = base.project();
        let mut builder = SnapshotBuilder::new();
        let mut documents = Vec::new();
        for document in saved.document_ids().map_err(|_| unavailable())? {
            let loaded = saved.load_document(document).map_err(|_| unavailable())?;
            let document = DocumentSource::of_model(&loaded.model).map_err(|_| unavailable())?;
            builder
                .add_document(&document.records)
                .map_err(|_| unavailable())?;
            documents.push(document);
        }
        builder
            .add_project_component_library(base.components())
            .map_err(|_| unavailable())?;
        add_contexts(
            &mut builder,
            documents.iter().map(DocumentSource::input),
            Some(base.components()),
        )
        .map_err(|error| CompareError::Projection(error.0))?;
        Ok(Self {
            commit: head.commit().into(),
            root: target.root().into(),
            project: target.project_id(),
            manifest_hash: base.manifest_hash,
            components: base
                .component_files
                .into_iter()
                .map(|(id, (path, hash))| {
                    (path.file_name().expect("검증한 원본").into(), (id, hash))
                })
                .collect(),
            members: context_members(&documents),
            semantic: builder.finish(),
        })
    }

    /// 이 비교 기준의 commit ID.
    pub fn commit(&self) -> &str {
        &self.commit
    }

    /// 동일한 원본 집합이면 HEAD에서 검증한 YAML 투영을 재사용합니다.
    pub fn compare(
        &self,
        target: &ProjectSnapshot,
        observed: &ProjectSourceIndex,
    ) -> Result<(DiffReport, ContentHash), CompareError> {
        if target.root() != self.root
            || observed.project_root() != target.root()
            || target.project_id() != self.project
            || target.generation().is_none()
        {
            return Err(CompareError::Projection(
                "다른 프로젝트의 HEAD 비교 기준입니다".into(),
            ));
        }
        // 세트 선택을 더하지 않은 원본·cache revision으로 HEAD generation과 같은지 봅니다(ADR-0128 결정 2).
        if target.storage_revision().hash() == Some(self.manifest_hash)
            && observed.entries().len() == self.components.len()
        {
            // 같은 파일명이 모두 있으면 정의간 이름 충돌과 기존 generation의 경계도 동일합니다.
            // 한 파일의 변경만 독립 비교하고 나머지는 기존 전체 조립 경로로 보냅니다.
            let mut edited = None;
            let mut single_edit = true;
            for (name, (_, hash)) in observed.entries() {
                let Some((owner, before)) = self.components.get(name) else {
                    single_edit = false;
                    break;
                };
                if before != hash && edited.replace((name, owner)).is_some() {
                    single_edit = false;
                    break;
                }
            }
            if single_edit {
                match edited {
                    None => return Ok((DiffReport::default(), observed.hash())),
                    Some((name, owner)) => return self.compare_component(observed, name, *owner),
                }
            }
        }
        // 문서는 전체 조립 경로에서만 읽습니다. 위의 단일 파일 경로는 저장 generation의 manifest
        // hash가 HEAD와 같다는 것을 이미 확인했으므로 문서 투영이 HEAD와 같아 다시 읽지 않습니다.
        let (mut working, documents) = document_builder(target, "target")?;
        let (library, files) =
            crate::component_source::project_library(target).map_err(|error| {
                CompareError::ComponentSource {
                    side: "target",
                    error,
                }
            })?;
        working
            .add_project_component_library(&library)
            .map_err(|e| CompareError::Projection(e.to_string()))?;
        add_contexts(
            &mut working,
            documents.iter().map(DocumentSource::input),
            Some(&library),
        )
        .map_err(|error| CompareError::Projection(error.0))?;
        let source_hash = source_set_hash(files.into_values()).map_err(|error| {
            CompareError::ComponentSource {
                side: "target",
                error: error.into(),
            }
        })?;
        Ok((
            diff_snapshots(&self.semantic, &working.finish()),
            source_hash,
        ))
    }

    fn compare_component(
        &self,
        observed: &ProjectSourceIndex,
        name: &OsStr,
        owner: StableId,
    ) -> Result<(DiffReport, ContentHash), CompareError> {
        let library =
            observed
                .library_for(name)
                .map_err(|error| CompareError::ComponentSource {
                    side: "target",
                    error: error.into(),
                })?;
        let mut builder = SnapshotBuilder::new();
        builder
            .add_project_component_library(&library)
            .map_err(|e| CompareError::Projection(e.to_string()))?;
        // 문서가 HEAD와 같으므로 문서 Context의 requirement·Slot 공개 목록도 같습니다. 이 컴포넌트 투영에 남은
        // Component·Slot 객체에 다시 붙여 base와 같은 범위로 비교합니다.
        // 여러 문서가 같은 컴포넌트·Slot에 항목을 두면 그 객체 하나에 합집합으로 붙습니다(`add_contexts`와 같은 규칙).
        let mut union = ContextSet::new();
        for (_, members) in &self.members {
            let mine = projectable_context(members, Some(&library));
            for (component, keys) in mine.all_requirements() {
                for key in keys {
                    union.add_requirement(component, *key);
                }
            }
            for (slot, keys) in mine.all_slot_keys() {
                for key in keys {
                    union.add_slot_key(slot, *key);
                }
            }
        }
        if let Some((anchor, _)) = self.members.first().filter(|_| !union.is_empty()) {
            builder
                .add_context(*anchor, &union)
                .map_err(|e| CompareError::Projection(e.to_string()))?;
        }
        let working = builder.finish();
        let mut base = SemanticSnapshot::new();
        for object in self
            .semantic
            .objects()
            .filter(|o| o.origin().component() == Some(owner))
        {
            base.insert(object.clone())
                .map_err(|e| CompareError::Projection(e.to_string()))?;
        }
        for object in working.objects() {
            if self
                .semantic
                .get(object.id())
                .is_some_and(|old| old.origin().component() != Some(owner))
            {
                return Err(CompareError::Projection(
                    "다른 정의 또는 문서와 StableId가 겹칩니다".into(),
                ));
            }
        }
        Ok((diff_snapshots(&base, &working), observed.hash()))
    }
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
    /// 정의나 그 tree의 충돌이 속한 컴포넌트.
    pub component: Option<StableId>,
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
            component: conflict.provenance.definition_component(),
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
    let opened = candidate_from_projects(base, ours, theirs)?;
    let candidate = &opened.candidate;
    let changes = diff_snapshots(&candidate.input().base, candidate.snapshot());
    Ok(ProjectMerge {
        base_revision: opened.base_revision,
        ours_revision: opened.ours_revision,
        theirs_revision: opened.theirs_revision,
        changes,
        conflicts: candidate
            .conflicts()
            .iter()
            .map(ConflictEntry::from)
            .collect(),
    })
}

/// 세 프로젝트 폴더에서 만든 독립 CandidateSnapshot C(XXIII.2.1).
#[derive(Debug, Clone)]
pub struct ProjectCandidate {
    /// base revision.
    pub base_revision: ProjectRevision,
    /// Source A(ours) revision.
    pub ours_revision: ProjectRevision,
    /// Source B(theirs) revision.
    pub theirs_revision: ProjectRevision,
    /// 병합 candidate. 입력 폴더의 파일은 읽기만 합니다.
    pub candidate: CandidateSnapshot,
}

/// 같은 프로젝트의 base·Source A(ours)·Source B(theirs) 폴더를 읽기 전용으로 열어 C를 만듭니다(XXIII.2.1).
/// 어느 폴더의 파일도 만들거나 바꾸지 않습니다.
pub fn candidate_from_projects(
    base: &Path,
    ours: &Path,
    theirs: &Path,
) -> Result<ProjectCandidate, CompareError> {
    let base = open_side(base, "base")?;
    let ours = open_side(ours, "ours")?;
    let theirs = open_side(theirs, "theirs")?;
    same_project(&base, &ours, "ours")?;
    same_project(&base, &theirs, "theirs")?;
    let input = MergeInput {
        base: base.semantic,
        ours: ours.semantic,
        theirs: theirs.semantic,
    };
    let candidate =
        CandidateSnapshot::build(input).map_err(|e| CompareError::Merge(e.to_string()))?;
    Ok(ProjectCandidate {
        base_revision: base.snapshot.revision(),
        ours_revision: ours.snapshot.revision(),
        theirs_revision: theirs.snapshot.revision(),
        candidate,
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
    fn candidate_from_projects_leaves_source_files_unchanged() {
        let (_dir, roots, document, children) = three_copies(7);
        edit(
            &roots[1],
            document,
            CommandInput::Rename {
                node: children[0],
                name: "A".into(),
            },
        );
        edit(
            &roots[2],
            document,
            CommandInput::Rename {
                node: children[1],
                name: "B".into(),
            },
        );
        let before: Vec<_> = roots.iter().map(|root| file_digest(root)).collect();
        let opened = candidate_from_projects(&roots[0], &roots[1], &roots[2]).unwrap();
        assert_eq!(opened.candidate.unresolved_count(), 0);
        let after: Vec<_> = roots.iter().map(|root| file_digest(root)).collect();
        assert_eq!(before, after, "A·B·base 파일 bytes 미변경");
        assert_eq!(opened.ours_revision.generation(), 2);
        // C는 두 변경을 모두 담고 A·B snapshot과 다릅니다.
        let input = opened.candidate.input();
        assert_ne!(opened.candidate.snapshot(), &input.ours);
        assert_ne!(opened.candidate.snapshot(), &input.theirs);
    }

    /// seed 문서에 key 하나(이름 `name`)와 View root provision을 더해 `root`에 저장합니다.
    fn save_with_context(root: &Path, seed: u64, keys: &[(u64, &str)]) {
        use stellar_document::context::fixtures::{number, number_type};
        use stellar_document::context::key::{ContextKey, ContextScope};
        use stellar_document::context::provision::{ContextOwner, ContextProvision};
        let mut model = random_valid_document(seed);
        let view = model
            .definition()
            .pages
            .iter()
            .flat_map(|page| page.views.iter())
            .next()
            .expect("View가 있는 seed")
            .clone();
        let mut set = stellar_document::context::set::ContextSet::new();
        for (n, name) in keys {
            let key = StableId::new_v7(90_000 + n, 0x0c, *n);
            set.insert_key(
                ContextKey::new(
                    key,
                    *name,
                    ContextScope::Document(model.document_id()),
                    number_type(),
                    number(91_000 + n, 1.0),
                )
                .unwrap(),
            )
            .unwrap();
            set.insert_provision(
                ContextProvision::new(
                    StableId::new_v7(92_000 + n, 0x0c, *n),
                    ContextOwner::View(view.id),
                    view.root_frame,
                    key,
                    number(93_000 + n, 2.0),
                )
                .unwrap(),
            )
            .unwrap();
        }
        model.replace_context(set);
        save_documents(root, &[&model]);
    }

    #[test]
    fn project_diff_and_merge_carry_the_document_context() {
        // 검증 기준(XXX.7.10 남은 범위, ADR-0114 결정 2): `stellar diff`·compare·merge에 Context 변경이 나타납니다.
        let (_dir, roots, _, _) = three_copies(7);
        save_with_context(&roots[1], 7, &[(1, "theme")]);
        let report = semantic_diff_projects(&roots[0], &roots[1]).unwrap().report;
        let kinds: Vec<Option<&str>> = report
            .entries
            .iter()
            .map(|e| e.object_kind.as_deref())
            .collect();
        assert!(kinds.contains(&Some("context-key")), "{kinds:?}");
        assert!(kinds.contains(&Some("context-provision")), "{kinds:?}");
        // 두 사본이 서로 다른 key를 더하면 자동 병합 결과에 둘 다 있습니다.
        save_with_context(&roots[2], 7, &[(2, "spacing")]);
        let merged = semantic_merge_projects(&roots[0], &roots[1], &roots[2]).unwrap();
        assert!(merged.is_clean(), "{:?}", merged.conflicts);
        let added: Vec<StableId> = merged
            .changes
            .entries
            .iter()
            .filter(|e| e.object_kind.as_deref() == Some("context-key"))
            .map(|e| e.object)
            .collect();
        assert_eq!(
            added,
            [
                StableId::new_v7(90_001, 0x0c, 1),
                StableId::new_v7(90_002, 0x0c, 2)
            ]
        );
        // 같은 key를 서로 다른 이름으로 더하면 conflict입니다.
        save_with_context(&roots[2], 7, &[(1, "accent")]);
        let conflicted = semantic_merge_projects(&roots[0], &roots[1], &roots[2]).unwrap();
        assert!(!conflicted.is_clean());
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
