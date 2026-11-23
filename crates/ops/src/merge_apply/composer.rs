//! Merge Composer 결과 적용 (XXIII.3.3·XXIII.3.4, ADR-0110 결정 5·6).
//!
//! 순서:
//!
//! 1. [`compose`]: Source A·B를 해석하고(XXIII.1.1) 공통 이력 base를 구해(XXIII.1.2) 세 snapshot을 임시 디렉터리에
//!    materialize합니다(XXIII.1.3). 각 snapshot의 저장 generation을 무손실 semantic snapshot으로 올려 C를 만듭니다
//!    (XXIII.2.1). A·B의 해석 결과(commit OID·snapshot 원천)와 내용 지문(path·mode·blob ID)을 기록합니다.
//! 2. 사용자는 [`Composition::candidate_mut`]로 conflict를 해결합니다(XXIII.2.3).
//! 3. [`apply_composition`]: C가 쓰기 조건(unresolved 0·새 위반 0, XXIII.2.4)을 통과하는지, C의 문서를 무손실로 되돌릴
//!    수 있는지 확인합니다. 그다음 **A·B를 다시 해석·지문 계산해 기록과 비교**하고 다르면 아무것도 쓰지 않습니다
//!    (XXIII.3.4). 새 branch+worktree target이면 이때 시스템 Git으로 만듭니다(XXIII.3.2). 마지막으로 C의 문서를
//!    target 프로젝트에 generation save합니다(XXIII.3.3): 저장 잠금 → target revision 재확인 → 문서 재직렬화 →
//!    저장 파이프라인(V.5, optimistic base check). 문서가 아닌 chunk(project metadata·컴포넌트·token·asset)는 target
//!    것을 그대로 잇습니다.
//!
//! **Git 상태**: 이 경로는 `git commit`·`merge`·`add`·`reset`·`checkout`을 부르지 않습니다. 적용 뒤 target worktree의
//! HEAD·index는 그대로이고 C는 작업 트리의 변경으로만 남습니다. 사용자가 결과를 보고 직접 stage·commit합니다.
//!
//! **지원 범위**: 문서 의미(generation)만 병합합니다. 컴포넌트 원본(`design/components`)이 base와 다른 source는
//! 병합하지 않고 [`ComposeError::ComponentSources`]입니다(조용히 버리지 않음). base가 하나가 아닌 비교(2-way·
//! criss-cross)는 3-way 병합 입력이 없으므로 [`ComposeError::NoCommonBase`]입니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;
use stellar_git_porcelain::composer::{
    materialize_source, materialize_sources, resolve_source, ComparisonMode, ComposerSnapshotError,
    ResolvedSource, SourceError, SourceRef, SourceSnapshot,
};
use stellar_git_porcelain::object::ObjectId;
use stellar_git_porcelain::snapshot::{SnapshotEntry, SnapshotSource};
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_project::document_io::component_source::project_source_index;
use stellar_project::migration::SchemaDecision;
use stellar_project::path_resolver::ExternalAccess;
use stellar_project::saved_project::SavedProjectLoader;
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_merge::candidate::{CandidateBlocked, CandidateSnapshot};
use stellar_semantic_merge::model::MergeInput;
use stellar_storage::chunk::kinds::KIND_ASSET_METADATA;

use super::document::{project_documents, DocumentMergeError};
use super::target::{create_branch_worktree, project_location, TargetError, ValidatedTarget};
use crate::candidate::saved_semantic_snapshot;
use crate::core_types::ids::StableId;
use crate::diff::DocumentSource;
use crate::error::OpenError;
use crate::lock::{LockError, LockPolicy, SaveLock};
use crate::revision::ProjectRevision;
use crate::snapshot::open_snapshot;
use crate::snapshot::save::{root_order, save_documents, SaveError};

/// 적용 전 재검사의 기준이 되는 source 기록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFingerprint {
    /// 해석 결과(commit OID·snapshot 원천·worktree).
    pub resolved: ResolvedSource,
    /// materialize한 내용 지문(path·mode·blob ID).
    pub entries: Vec<SnapshotEntry>,
}

impl SourceFingerprint {
    fn of(snapshot: &SourceSnapshot) -> SourceFingerprint {
        SourceFingerprint {
            resolved: snapshot.resolved.clone(),
            entries: snapshot.entries.clone(),
        }
    }
}

/// Source A·B·Base에서 만든 C와 그 source 기록.
#[derive(Debug, Clone)]
pub struct Composition {
    project_root: PathBuf,
    project_id: StableId,
    a: SourceFingerprint,
    b: SourceFingerprint,
    base: ObjectId,
    candidate: CandidateSnapshot,
}

impl Composition {
    /// Merge Composer 프로젝트 root(정규화).
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// 프로젝트 id.
    pub fn project_id(&self) -> StableId {
        self.project_id
    }

    /// Source A 기록.
    pub fn source_a(&self) -> &SourceFingerprint {
        &self.a
    }

    /// Source B 기록.
    pub fn source_b(&self) -> &SourceFingerprint {
        &self.b
    }

    /// merge base commit.
    pub fn base(&self) -> &ObjectId {
        &self.base
    }

    /// C.
    pub fn candidate(&self) -> &CandidateSnapshot {
        &self.candidate
    }

    /// conflict 해결용 C(XXIII.2.3).
    pub fn candidate_mut(&mut self) -> &mut CandidateSnapshot {
        &mut self.candidate
    }
}

/// C 만들기 실패. 원본 worktree는 바뀌지 않습니다.
#[derive(Debug)]
pub enum ComposeError {
    /// 프로젝트 위치·열기 실패.
    Target(TargetError),
    /// source 해석 실패.
    Source(SourceError),
    /// snapshot materialize 실패.
    Snapshot(ComposerSnapshotError),
    /// 공통 base가 하나가 아닙니다(`two-way`·`criss-cross`).
    NoCommonBase(&'static str),
    /// source 쪽 프로젝트를 읽을 수 없습니다(`side`: `a`·`b`·`base`).
    Project {
        /// 입력 이름.
        side: &'static str,
        /// 원인.
        error: String,
    },
    /// source 쪽 프로젝트 id가 다릅니다.
    ProjectMismatch {
        /// 입력 이름.
        side: &'static str,
    },
    /// source의 컴포넌트 원본이 base와 다릅니다(이 경로가 병합하지 않는 의미).
    ComponentSources {
        /// 다른 입력 이름.
        side: &'static str,
    },
    /// 병합 엔진 실패.
    Merge(String),
}

impl fmt::Display for ComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComposeError::Target(e) => write!(f, "{e}"),
            ComposeError::Source(e) => write!(f, "source 해석 실패: {e}"),
            ComposeError::Snapshot(e) => write!(f, "snapshot 실패: {e}"),
            ComposeError::NoCommonBase(mode) => {
                write!(
                    f,
                    "공통 base가 하나가 아니라({mode}) 3-way 병합할 수 없습니다"
                )
            }
            ComposeError::Project { side, error } => write!(f, "{side} 프로젝트: {error}"),
            ComposeError::ProjectMismatch { side } => {
                write!(f, "{side} 프로젝트의 project id가 다릅니다")
            }
            ComposeError::ComponentSources { side } => write!(
                f,
                "{side}의 컴포넌트 원본이 base와 달라 병합하지 않습니다(문서 의미만 병합)"
            ),
            ComposeError::Merge(e) => write!(f, "semantic merge 실패: {e}"),
        }
    }
}

impl std::error::Error for ComposeError {}

/// materialize한 snapshot 하나의 프로젝트 문서 semantic과 컴포넌트 원본 hash.
fn side_semantic(
    snapshot: &SourceSnapshot,
    relative: &Path,
    project_id: StableId,
    side: &'static str,
) -> Result<(SemanticSnapshot, stellar_storage::hash::ContentHash), ComposeError> {
    let root = snapshot.materialized.root().join(relative);
    let project_error = |error: String| ComposeError::Project { side, error };
    let components = project_source_index(&root, ExternalAccess::MetadataOnly)
        .map_err(|e| project_error(e.to_string()))?
        .hash();
    let project = SavedProjectLoader::for_materialized_snapshot()
        .open(&root)
        .map_err(|e| project_error(e.to_string()))?;
    if project.project_id != project_id {
        return Err(ComposeError::ProjectMismatch { side });
    }
    // 텍스트 원본이 있으면 원본이 Git 원본입니다(ADR-0113 결정 6). cache generation은 project id 확인에만 씁니다.
    if let Some(semantic) = crate::snapshot::source_semantic(&root).map_err(project_error)? {
        return Ok((semantic, components));
    }
    let semantic = saved_semantic_snapshot(&project).map_err(|e| project_error(e.to_string()))?;
    Ok((semantic, components))
}

/// Source A(ours)·B(theirs)로 C를 만듭니다(모듈 문서 1). `scratch`는 임시 snapshot을 둘 디렉터리이며 worktree 밖이어야
/// 합니다. 원본 worktree·index·ref를 바꾸지 않습니다.
pub fn compose(
    git: &GatedGitRunner,
    project_root: &Path,
    a: &SourceRef,
    b: &SourceRef,
    scratch: &Path,
) -> Result<Composition, ComposeError> {
    let (current, relative) = project_location(git, project_root).map_err(ComposeError::Target)?;
    let project = current.join(&relative);
    let project_id = open_snapshot(&project)
        .map_err(|e| ComposeError::Target(TargetError::Project(e)))?
        .project_id();
    let a = resolve_source(git, &current, a).map_err(ComposeError::Source)?;
    let b = resolve_source(git, &current, b).map_err(ComposeError::Source)?;
    let snapshots =
        materialize_sources(git, &current, &a, &b, scratch).map_err(ComposeError::Snapshot)?;
    let base_commit = match &snapshots.mode {
        ComparisonMode::ThreeWay { base } => base.clone(),
        ComparisonMode::CrissCross { .. } => return Err(ComposeError::NoCommonBase("criss-cross")),
        ComparisonMode::TwoWay { .. } => return Err(ComposeError::NoCommonBase("two-way")),
    };
    let Some(base_snapshot) = snapshots.base.as_ref() else {
        return Err(ComposeError::NoCommonBase("two-way"));
    };
    let (base, base_components) = side_semantic(base_snapshot, &relative, project_id, "base")?;
    let (ours, ours_components) = side_semantic(&snapshots.a, &relative, project_id, "a")?;
    let (theirs, theirs_components) = side_semantic(&snapshots.b, &relative, project_id, "b")?;
    for (side, hash) in [("a", ours_components), ("b", theirs_components)] {
        if hash != base_components {
            return Err(ComposeError::ComponentSources { side });
        }
    }
    let candidate = CandidateSnapshot::build(MergeInput { base, ours, theirs })
        .map_err(|e| ComposeError::Merge(e.to_string()))?;
    Ok(Composition {
        project_root: project,
        project_id,
        a: SourceFingerprint::of(&snapshots.a),
        b: SourceFingerprint::of(&snapshots.b),
        base: base_commit,
        candidate,
    })
}

/// 적용 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerApplyReport {
    /// target 이름(`current-worktree` 등).
    pub target: &'static str,
    /// target 프로젝트 root.
    pub project_root: PathBuf,
    /// 새 branch를 만들었으면 그 이름.
    pub created_branch: Option<String>,
    /// 쓰기 전 target revision.
    pub base_revision: ProjectRevision,
    /// 쓴 뒤 target revision(쓸 것이 없었으면 `base_revision`).
    pub revision: ProjectRevision,
    /// 새로 쓴 chunk 수.
    pub written_chunks: usize,
    /// 결과 문서 수.
    pub documents: usize,
}

/// 적용 실패. [`ComposerApplyError::Target`] 가운데 새 worktree를 만든 뒤의 실패를 빼면 아무것도 만들거나 쓰지
/// 않았습니다.
#[derive(Debug)]
pub enum ComposerApplyError {
    /// C가 쓰기 조건을 통과하지 못했습니다(unresolved conflict·새 위반).
    Blocked(CandidateBlocked),
    /// C의 문서를 무손실로 되돌릴 수 없습니다.
    Result(DocumentMergeError),
    /// C를 만든 뒤 source가 바뀌었습니다(XXIII.3.4). `side`는 `a`·`b`입니다.
    StaleSource {
        /// 바뀐 source.
        side: &'static str,
        /// 바뀐 내용(`commit`·`content`, 또는 `resolve`·`materialize` 실패 원인).
        reason: String,
    },
    /// target 검증·생성 실패.
    Target(TargetError),
    /// 저장 잠금 실패.
    Lock(LockError),
    /// target 프로젝트 열기 실패.
    Open(OpenError),
    /// target이 검증 뒤 바뀌었습니다.
    StaleTarget {
        /// 검증 때 revision.
        expected: Option<ProjectRevision>,
        /// 지금 revision.
        found: ProjectRevision,
    },
    /// 이 경로가 쓰지 않는 target 상태(generation 없음·schema·없는 asset 참조 등).
    Unsupported(&'static str),
    /// 계획과 교체 사이에 다른 writer가 current pointer를 바꿨습니다.
    ConcurrentWrite,
    /// 직렬화·저장 실패.
    Save(String),
}

impl fmt::Display for ComposerApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComposerApplyError::Blocked(e) => write!(f, "C를 쓸 수 없습니다: {e}"),
            ComposerApplyError::Result(e) => write!(f, "{e}"),
            ComposerApplyError::StaleSource { side, reason } => write!(
                f,
                "C를 만든 뒤 source {side}가 바뀌었습니다({reason}). 다시 만드세요"
            ),
            ComposerApplyError::Target(e) => write!(f, "{e}"),
            ComposerApplyError::Lock(e) => write!(f, "{e}"),
            ComposerApplyError::Open(e) => write!(f, "target 프로젝트: {e}"),
            ComposerApplyError::StaleTarget { expected, found } => write!(
                f,
                "target이 검증 뒤 바뀌었습니다(기대 {}, 지금 {found})",
                expected.map_or_else(|| "없음".to_owned(), |r| r.to_string())
            ),
            ComposerApplyError::Unsupported(reason) => write!(f, "지원하지 않는 target: {reason}"),
            ComposerApplyError::ConcurrentWrite => {
                f.write_str("다른 writer가 target을 먼저 저장했습니다")
            }
            ComposerApplyError::Save(e) => write!(f, "저장 실패: {e}"),
        }
    }
}

impl std::error::Error for ComposerApplyError {}

/// C를 만든 뒤 source가 바뀌었는지 다시 검사합니다(XXIII.3.4). A·B를 같은 요청으로 다시 해석해 commit OID·snapshot
/// 원천이 같은지 보고, index·작업 트리 source는 같은 materialize 경로로 내용 지문을 다시 계산해 비교합니다. commit
/// 계열 source의 내용은 OID가 고정하므로 다시 읽지 않습니다. 원본을 바꾸지 않습니다.
pub fn recheck_sources(
    git: &GatedGitRunner,
    composition: &Composition,
    scratch: &Path,
) -> Result<(), ComposerApplyError> {
    let (current, _) =
        project_location(git, &composition.project_root).map_err(ComposerApplyError::Target)?;
    for (side, recorded) in [("a", &composition.a), ("b", &composition.b)] {
        let stale = |reason: String| ComposerApplyError::StaleSource { side, reason };
        let now = resolve_source(git, &current, &recorded.resolved.source)
            .map_err(|e| stale(format!("resolve: {e}")))?;
        if now != recorded.resolved {
            return Err(stale("commit".into()));
        }
        if matches!(
            now.snapshot,
            SnapshotSource::Index | SnapshotSource::WorkingTree
        ) {
            let fresh = materialize_source(git, &now, scratch)
                .map_err(|e| stale(format!("materialize: {e}")))?;
            if fresh.entries != recorded.entries {
                return Err(stale("content".into()));
            }
        }
    }
    Ok(())
}

/// C를 `target`에 적용합니다(모듈 문서 3). `target`은 [`super::target::validate_target`] 결과입니다.
pub fn apply_composition(
    git: &GatedGitRunner,
    composition: &Composition,
    target: ValidatedTarget,
    lock: &LockPolicy,
    scratch: &Path,
) -> Result<ComposerApplyReport, ComposerApplyError> {
    let validated = composition
        .candidate
        .validate()
        .map_err(ComposerApplyError::Blocked)?;
    let documents = project_documents(validated.snapshot()).map_err(ComposerApplyError::Result)?;
    recheck_sources(git, composition, scratch)?;
    let created_branch = match target.target() {
        super::target::ResultTarget::NewBranchWorktree { branch, .. } => Some(branch.clone()),
        _ => None,
    };
    let target = create_branch_worktree(
        git,
        &composition.project_root,
        composition.project_id,
        target,
    )
    .map_err(ComposerApplyError::Target)?;
    let (base_revision, revision, written_chunks) = write_documents(
        target.project_root(),
        target.expected_revision(),
        composition.project_id,
        &documents,
        lock,
    )?;
    Ok(ComposerApplyReport {
        target: target.target().name(),
        project_root: target.project_root().to_path_buf(),
        created_branch,
        base_revision,
        revision,
        written_chunks,
        documents: documents.len(),
    })
}

/// 문서 레코드가 참조하는 이미지 asset.
fn referenced_assets<'a>(
    documents: impl Iterator<Item = &'a DocumentSource>,
) -> BTreeSet<StableId> {
    documents
        .flat_map(|document| document.records.nodes.iter())
        .filter_map(|node| match &node.payload {
            NodePayload::Image(image) => Some(image.asset.id()),
            _ => None,
        })
        .collect()
}

/// `documents`를 target 프로젝트의 다음 generation으로 저장합니다(XXIII.3.3). 반환: (쓰기 전 revision, 쓴 뒤
/// revision, 새 chunk 수).
fn write_documents(
    root: &Path,
    expected: Option<ProjectRevision>,
    project_id: StableId,
    documents: &[DocumentSource],
    lock: &LockPolicy,
) -> Result<(ProjectRevision, ProjectRevision, usize), ComposerApplyError> {
    let root = std::fs::canonicalize(root)
        .map_err(|e| ComposerApplyError::Open(OpenError::Path { kind: e.kind() }))?;
    let _guard = match lock {
        LockPolicy::Required { dir } => {
            Some(SaveLock::try_acquire(dir, project_id, &root).map_err(ComposerApplyError::Lock)?)
        }
        LockPolicy::None => None,
    };
    let snapshot = open_snapshot(&root).map_err(ComposerApplyError::Open)?;
    let found = snapshot.revision();
    if Some(found) != expected {
        return Err(ComposerApplyError::StaleTarget { expected, found });
    }
    if snapshot.schema() != SchemaDecision::Current {
        return Err(ComposerApplyError::Unsupported(
            "현재 schema가 아닌 generation",
        ));
    }
    let generation = snapshot
        .generation()
        .ok_or(ComposerApplyError::Unsupported(
            "저장된 generation이 없는 target",
        ))?;
    let manifest = generation.manifest();
    // 결과가 target 문서와 같으면 쓰지 않습니다.
    let mut current = BTreeMap::new();
    for definition in snapshot.documents() {
        let model = snapshot
            .document_model(definition.id)
            .map_err(|e| ComposerApplyError::Save(e.to_string()))?;
        let document =
            DocumentSource::of_model(&model).map_err(|e| ComposerApplyError::Save(e.0))?;
        current.insert(definition.id, document);
    }
    // 결과가 새로 참조하는 이미지 asset은 target에 그 metadata chunk가 있어야 합니다(target 문서가 이미 참조하던
    // asset은 target 상태 그대로라 따지지 않습니다). 없는 asset을 새로 참조하는 결과는 쓰지 않습니다.
    let mut known: BTreeSet<StableId> = manifest
        .chunk_pointers
        .iter()
        .filter(|p| p.kind == KIND_ASSET_METADATA)
        .map(|p| p.chunk_id.stable_id())
        .collect();
    known.extend(referenced_assets(current.values()));
    if !referenced_assets(documents.iter()).is_subset(&known) {
        return Err(ComposerApplyError::Unsupported(
            "target에 없는 이미지 asset을 새로 참조",
        ));
    }
    let wanted: BTreeMap<StableId, &DocumentSource> = documents
        .iter()
        .map(|document| (document.records.definition.id, document))
        .collect();
    if current.len() == wanted.len()
        && current
            .iter()
            .all(|(id, records)| wanted.get(id) == Some(&records))
    {
        return Ok((found, found, 0));
    }

    // root 순서: target에 남는 문서는 target 순서, 새 문서는 결과 순서(StableId 오름차순)로 뒤에 둡니다.
    let mut roots: Vec<StableId> = root_order(&snapshot)
        .into_iter()
        .filter(|id| wanted.contains_key(id))
        .collect();
    for id in wanted.keys() {
        if !roots.contains(id) {
            roots.push(*id);
        }
    }
    // 원본·cache 저장(ADR-0113 결정 3). Component Context는 병합 결과의 것을 씁니다(ADR-0114).
    let mut models = Vec::with_capacity(roots.len());
    for id in &roots {
        let (mut model, _) = DocumentModel::from_records(wanted[id].records.clone())
            .map_err(|e| ComposerApplyError::Save(format!("{e:?}")))?;
        model.replace_context(wanted[id].context.clone());
        if let Ok(target) = snapshot.document_model(*id) {
            model.set_style_defaults(target.style_defaults());
        }
        models.push(model);
    }
    let saved = save_documents(&snapshot, roots, models).map_err(|e| match e {
        SaveError::ConcurrentWrite => ComposerApplyError::ConcurrentWrite,
        other => ComposerApplyError::Save(other.to_string()),
    })?;
    let (revision, written) = (saved.revision, saved.written_chunks);
    Ok((found, revision, written))
}
