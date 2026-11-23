//! Git conflict 세션: unmerged 경로의 세 stage를 Semantic Merge engine에 전달 (XIV.11.3, ADR-0081 결정 5).
//!
//! 작업 트리의 unmerged 경로(`git ls-files -u`)를 단위로 나눕니다.
//!
//! | 경로 | 단위 |
//! |---|---|
//! | `design/components/*.stellar.yaml` | 컴포넌트 원본 파일 하나(세 stage blob을 codec으로 읽음) |
//! | `design/current` | generation 문서 전체(세 stage pointer가 가리키는 세대를 작업 트리 파일에서 읽음) |
//! | 텍스트 원본 `design/{document,pages/**,assets/catalog,context,tokens/*}.stellar.yaml` | 텍스트 원본 문서 하나(그 경로 전체가 한 단위, ADR-0113 결정 5). 세 쪽 원본 파일 집합은 작업 트리의 나머지 원본과 unmerged 경로의 stage blob으로 만듭니다. 텍스트 원본이 있으면 `design/current`(로컬 cache)는 따로 병합하지 않고 이 단위의 결과로 다시 씁니다 |
//! | 그 밖의 `stellar.yaml`·`design/` 경로 | 의미 병합하지 않는 Stellar 경로(이유와 함께 보고) |
//! | 그 밖의 경로 | raw Git conflict(XIV.11.4, Semantic Merge로 해석하지 않음) |
//!
//! 단위마다 세 snapshot으로 [`CandidateSnapshot`]을 만들고, raw 경로 ↔ semantic 객체 대응을 함께 보관합니다:
//! 단위의 raw 경로는 그 단위 세 snapshot의 모든 객체를 담고, semantic conflict는 그 단위의 raw 경로로 돌아갑니다.
//! 세션은 작업 트리·index를 읽기만 합니다(해결은 C에만, XIV.12.2).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use stellar_project::private_temp::PrivateTempDir;
use stellar_project::saved_project::{SavedProject, SavedProjectLoader};
use stellar_semantic_diff::codec::{read_value, Reader};
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_merge::candidate::{
    allowed_resolutions, CandidateCommand, CandidateSnapshot, Resolution, ResolutionError,
    ResolutionKind,
};
use stellar_semantic_merge::model::{MergeInput, SemanticConflict};
use stellar_storage::chunk::path::ChunkObjectPath;
use stellar_storage::generation::loader::{manifest_path, read_manifest_file};
use stellar_storage::hash::ContentHash;

use super::component_file::{decode_component_file, is_component_source_path};
use super::git_port::{GitOperation, GitPortError, MergeGit, UnmergedEntry};
use crate::compare::ConflictEntry;
use crate::core_types::ids::StableId;
use crate::merge_text::{
    design_relative, is_text_source_path, prepare_text_merge, TextMergeBase, TextSides,
    DESIGN_PREFIX,
};
use stellar_project::document_io::source_v4::project::DOCUMENT_PATH as DOCUMENT_FILE;

/// 한 stage blob의 크기 상한(컴포넌트 원본·pointer 모두 이 안).
const MAX_STAGE_BYTES: u64 = 1024 * 1024;
/// generation current pointer 경로.
pub const CURRENT_POINTER: &str = "design/current";

/// 단위 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    /// 컴포넌트 원본 파일 하나.
    ComponentSource,
    /// generation 문서(`design/current`).
    Generation,
    /// 텍스트 원본 문서(ADR-0113 결정 5).
    TextSource,
}

impl UnitKind {
    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            UnitKind::ComponentSource => "component-source",
            UnitKind::Generation => "generation",
            UnitKind::TextSource => "text-source",
        }
    }
}

/// ours generation(쓰기 기준)과 그 임시 사본.
#[derive(Debug)]
pub(crate) struct GenerationBase {
    /// ours(stage 2) 세대 프로젝트.
    pub(crate) ours: SavedProject,
    _dir: PrivateTempDir,
}

/// 의미 병합 단위 하나.
#[derive(Debug)]
pub struct SessionUnit {
    path: PathBuf,
    kind: UnitKind,
    candidate: CandidateSnapshot,
    objects: BTreeSet<StableId>,
    observed: Option<ContentHash>,
    pub(crate) generation: Option<GenerationBase>,
    pub(crate) text: Option<TextUnit>,
}

/// 텍스트 원본 단위의 쓰기 기준(ADR-0113 결정 5).
#[derive(Debug)]
pub(crate) struct TextUnit {
    /// 결과를 원본으로 쓰는 데 필요한 병합한 문서 밖 의미와 ours 파일 집합.
    pub(crate) base: TextMergeBase,
    /// 이 단위가 resolved 처리하는 unmerged 경로(텍스트 원본, 그리고 cache `design/current`).
    pub(crate) paths: Vec<PathBuf>,
}

impl SessionUnit {
    /// raw conflict 경로(프로젝트 기준).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 단위 종류.
    pub fn kind(&self) -> UnitKind {
        self.kind
    }

    /// 단위 candidate.
    pub fn candidate(&self) -> &CandidateSnapshot {
        &self.candidate
    }

    /// raw 경로에 대응하는 semantic 객체(세 snapshot의 합집합).
    pub fn objects(&self) -> &BTreeSet<StableId> {
        &self.objects
    }

    /// 세션을 연 때 작업 트리 파일 bytes의 hash(없으면 `None`).
    pub fn observed(&self) -> Option<ContentHash> {
        self.observed
    }

    /// generation 단위의 ours(stage 2) 세대 번호. 쓰기는 이 세대를 기준으로 다음 세대를 만듭니다.
    pub fn ours_generation(&self) -> Option<u64> {
        self.generation
            .as_ref()
            .map(|base| base.ours.generation_number())
    }
}

/// 세션 열기 실패.
#[derive(Debug)]
pub enum SessionError {
    /// Git 실패.
    Git(GitPortError),
    /// 임시 폴더 입출력 실패.
    Io(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Git(e) => write!(f, "{e}"),
            SessionError::Io(e) => write!(f, "conflict 세션 입출력 실패: {e}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// 전역 번호로 가리킨 conflict 하나.
#[derive(Debug, Clone, Copy)]
pub struct SessionConflict<'a> {
    /// 세션 전체 번호(단위 경로 순서, 그 안에서 엔진 순서).
    pub index: usize,
    /// 단위.
    pub unit: &'a SessionUnit,
    /// conflict.
    pub conflict: &'a SemanticConflict,
}

impl SessionConflict<'_> {
    /// 표시용 conflict 항목.
    pub fn entry(&self) -> ConflictEntry {
        ConflictEntry::from(self.conflict)
    }

    /// 허용 해결 action 이름(XIV.12.1, code 순서).
    pub fn allowed(&self) -> Vec<&'static str> {
        allowed_resolutions(self.conflict)
            .into_iter()
            .map(ResolutionKind::name)
            .collect()
    }

    /// 적용한 해결 action 이름(미해결이면 `None`).
    pub fn resolution(&self) -> Option<&'static str> {
        self.unit
            .candidate
            .resolution(self.conflict.object, &self.conflict.target)
            .map(|r| r.kind().name())
    }
}

/// 단위의 쓰기 조건 상태(XIV.12.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitStatus {
    /// 미해결 conflict 수.
    pub unresolved: usize,
    /// 새 snapshot 위반 수.
    pub violations: usize,
}

impl UnitStatus {
    /// 쓸 수 있는지(둘 다 0).
    pub fn writable(&self) -> bool {
        self.unresolved == 0 && self.violations == 0
    }
}

impl SessionUnit {
    /// 쓰기 조건 상태.
    pub fn status(&self) -> UnitStatus {
        match self.candidate.validate() {
            Ok(_) => UnitStatus {
                unresolved: 0,
                violations: 0,
            },
            Err(blocked) => UnitStatus {
                unresolved: blocked.unresolved.len(),
                violations: blocked.violations.len(),
            },
        }
    }
}

/// 해결 action 표기를 읽습니다: `base`·`ours`·`theirs`·`delete`·`custom:<SemanticValue canonical bytes 16진수>`.
pub fn parse_resolution(text: &str) -> Result<Resolution, String> {
    Ok(match text {
        "base" => Resolution::ChooseBase,
        "ours" => Resolution::ChooseOurs,
        "theirs" => Resolution::ChooseTheirs,
        "delete" => Resolution::Delete,
        other => {
            let hex = other
                .strip_prefix("custom:")
                .ok_or_else(|| format!("알 수 없는 해결 action: {other}"))?;
            if hex.len() % 2 != 0 {
                return Err("custom 값 16진수 길이".into());
            }
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|_| "custom 값 16진수".to_string())?;
            let mut reader = Reader::new(&bytes);
            let value = read_value(&mut reader).map_err(|e| e.to_string())?;
            reader.finish().map_err(|e| e.to_string())?;
            Resolution::CustomValue(value)
        }
    })
}

/// Git conflict 세션.
#[derive(Debug)]
pub struct ConflictSession {
    root: PathBuf,
    entries: Vec<UnmergedEntry>,
    units: Vec<SessionUnit>,
    raw: Vec<PathBuf>,
    unsupported: Vec<(PathBuf, String)>,
    operation: GitOperation,
}

fn stage_bytes(
    git: &dyn MergeGit,
    root: &Path,
    entry: &UnmergedEntry,
    stage: usize,
) -> Result<Vec<u8>, SessionError> {
    match &entry.stages[stage] {
        Some(blob) => git
            .blob(root, &blob.oid, MAX_STAGE_BYTES)
            .map_err(SessionError::Git),
        None => Ok(Vec::new()),
    }
}

fn copy_into(root: &Path, side: &Path, relative: &Path) -> Result<(), String> {
    let from = root.join(relative);
    let to = side.join(relative);
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::copy(&from, &to)
        .map(|_| ())
        .map_err(|e| format!("{}: {e}", relative.display()))
}

/// stage pointer bytes가 가리키는 세대를 작업 트리 파일에서 비공개 사본으로 열어 문서 snapshot을 만듭니다.
fn open_generation_side(
    root: &Path,
    pointer: &[u8],
    side: &Path,
) -> Result<(SavedProject, SemanticSnapshot), String> {
    let line = std::str::from_utf8(pointer)
        .ok()
        .and_then(|text| text.strip_suffix('\n'))
        .ok_or("current pointer 형식")?;
    let hash: ContentHash = line.parse().map_err(|_| "current pointer hash 형식")?;
    std::fs::create_dir_all(side.join("design")).map_err(|e| e.to_string())?;
    copy_into(root, side, Path::new("stellar.yaml"))?;
    std::fs::write(side.join(CURRENT_POINTER), pointer).map_err(|e| e.to_string())?;
    let manifest = read_manifest_file(root, &hash)
        .map_err(|e| format!("세대 manifest가 작업 트리에 없거나 손상됨: {e}"))?;
    copy_into(root, side, &manifest_path(Path::new(""), &hash))?;
    for pointer in &manifest.chunk_pointers {
        copy_into(
            root,
            side,
            &ChunkObjectPath::for_hash(&pointer.content_hash).to_relative_path(),
        )?;
    }
    let project = SavedProjectLoader::for_materialized_snapshot()
        .open(side)
        .map_err(|e| e.to_string())?;
    let semantic = super::saved_semantic_snapshot(&project).map_err(|e| e.to_string())?;
    Ok((project, semantic))
}

fn objects_of(input: &MergeInput) -> BTreeSet<StableId> {
    [&input.base, &input.ours, &input.theirs]
        .into_iter()
        .flat_map(|side| side.objects().map(|o| o.id()))
        .collect()
}

fn observed_hash(root: &Path, relative: &Path) -> Option<ContentHash> {
    std::fs::read(root.join(relative))
        .ok()
        .map(|bytes| ContentHash::compute(&bytes))
}

impl ConflictSession {
    /// 프로젝트 루트 `root`(Git 작업 트리 안)의 conflict 세션을 엽니다. 작업 트리·index는 읽기만 합니다.
    pub fn open(git: &dyn MergeGit, root: &Path) -> Result<ConflictSession, SessionError> {
        let entries = git.unmerged(root).map_err(SessionError::Git)?;
        let operation = git.operation(root).map_err(SessionError::Git)?;
        let mut units = Vec::new();
        let mut raw = Vec::new();
        let mut unsupported = Vec::new();
        let text_paths: Vec<PathBuf> = entries
            .iter()
            .map(|entry| entry.path.clone())
            .filter(|path| is_text_source_path(path))
            .collect();
        let mut text_generation = false;
        // 텍스트 원본 프로젝트에서 cache pointer만 conflict여도(원본은 driver나 Git이 깨끗이 병합) 원본 결과로 cache를
        // 다시 씁니다(ADR-0113 결정 5).
        let text_project = root.join(DESIGN_PREFIX).join(DOCUMENT_FILE).is_file();
        let cache_conflict = entries
            .iter()
            .any(|entry| entry.path == Path::new(CURRENT_POINTER));
        if !text_paths.is_empty() || (text_project && cache_conflict) {
            match Self::text_source(git, root, &entries)? {
                Ok((input, base)) => {
                    let mut paths = text_paths.clone();
                    let mut generation = None;
                    if let Some(entry) = entries
                        .iter()
                        .find(|entry| entry.path == Path::new(CURRENT_POINTER))
                    {
                        text_generation = true;
                        match Self::generation(git, root, entry)? {
                            Ok((_, ours)) => {
                                generation = Some(ours);
                                paths.push(entry.path.clone());
                            }
                            Err(reason) => unsupported.push((
                                entry.path.clone(),
                                format!("텍스트 원본의 로컬 cache를 다시 쓸 수 없음: {reason}"),
                            )),
                        }
                    }
                    let path = text_paths
                        .first()
                        .cloned()
                        .unwrap_or_else(|| PathBuf::from(CURRENT_POINTER));
                    let mut unit = Self::unit(root, path, UnitKind::TextSource, input, generation)?;
                    unit.text = Some(TextUnit { base, paths });
                    units.push(unit);
                }
                Err(reason) => {
                    for path in &text_paths {
                        unsupported.push((path.clone(), reason.clone()));
                    }
                    if text_paths.is_empty() {
                        text_generation = true;
                        unsupported.push((PathBuf::from(CURRENT_POINTER), reason));
                    }
                }
            }
        }
        for entry in &entries {
            let path = entry.path.clone();
            if is_text_source_path(&path) || (text_generation && path == Path::new(CURRENT_POINTER))
            {
                continue;
            }
            if is_component_source_path(&path) {
                let name = path.file_name().unwrap_or_default();
                let mut sides = Vec::new();
                for stage in 0..3 {
                    let bytes = stage_bytes(git, root, entry, stage)?;
                    sides.push(decode_component_file(name, &bytes));
                }
                let mut sides = sides.into_iter();
                let (Some(Ok(base)), Some(Ok(ours)), Some(Ok(theirs))) =
                    (sides.next(), sides.next(), sides.next())
                else {
                    unsupported.push((path, "컴포넌트 원본 stage를 읽을 수 없음".into()));
                    continue;
                };
                units.push(Self::unit(
                    root,
                    path,
                    UnitKind::ComponentSource,
                    MergeInput { base, ours, theirs },
                    None,
                )?);
            } else if path == Path::new(CURRENT_POINTER) {
                match Self::generation(git, root, entry)? {
                    Ok((input, base)) => units.push(Self::unit(
                        root,
                        path,
                        UnitKind::Generation,
                        input,
                        Some(base),
                    )?),
                    Err(reason) => unsupported.push((path, reason)),
                }
            } else if path == Path::new("stellar.yaml") || path.starts_with("design") {
                unsupported.push((path, "의미 병합하지 않는 Stellar 저장 경로".into()));
            } else {
                raw.push(path);
            }
        }
        Ok(ConflictSession {
            root: root.to_path_buf(),
            entries,
            units,
            raw,
            unsupported,
            operation,
        })
    }

    fn unit(
        root: &Path,
        path: PathBuf,
        kind: UnitKind,
        input: MergeInput,
        generation: Option<GenerationBase>,
    ) -> Result<SessionUnit, SessionError> {
        let objects = objects_of(&input);
        let candidate =
            CandidateSnapshot::build(input).map_err(|e| SessionError::Io(e.to_string()))?;
        Ok(SessionUnit {
            observed: observed_hash(root, &path),
            path,
            kind,
            candidate,
            objects,
            generation,
            text: None,
        })
    }

    /// 텍스트 원본 단위의 세 쪽 파일 집합: 작업 트리의 원본(unmerged 경로 제외)에 unmerged 텍스트 원본·컴포넌트 원본의
    /// stage blob을 더합니다(ADR-0113 결정 5).
    #[allow(clippy::type_complexity)]
    fn text_source(
        git: &dyn MergeGit,
        root: &Path,
        entries: &[UnmergedEntry],
    ) -> Result<Result<(MergeInput, TextMergeBase), String>, SessionError> {
        let design = root.join(DESIGN_PREFIX.trim_end_matches('/'));
        let mut context = match stellar_project::document_io::session::read_source_dir(&design) {
            Ok(files) => files,
            Err(e) => return Ok(Err(format!("작업 트리 텍스트 원본을 읽을 수 없음: {e}"))),
        };
        let mut sides = TextSides::default();
        let unmerged: Vec<(String, &UnmergedEntry)> = entries
            .iter()
            .filter_map(|entry| design_relative(&entry.path).map(|relative| (relative, entry)))
            .collect();
        for (relative, _) in &unmerged {
            context.remove(relative);
        }
        sides.base = context.clone();
        sides.ours = context.clone();
        sides.theirs = context;
        for (relative, entry) in unmerged {
            for (stage, files) in [&mut sides.base, &mut sides.ours, &mut sides.theirs]
                .into_iter()
                .enumerate()
            {
                if entry.stages[stage].is_none() {
                    continue;
                }
                let bytes = stage_bytes(git, root, entry, stage)?;
                match String::from_utf8(bytes) {
                    Ok(text) => {
                        files.insert(relative.clone(), text);
                    }
                    Err(_) => return Ok(Err(format!("UTF-8이 아닌 원본 stage: {relative}"))),
                }
            }
        }
        Ok(prepare_text_merge(&sides).map_err(|e| e.to_string()))
    }

    #[allow(clippy::type_complexity)]
    fn generation(
        git: &dyn MergeGit,
        root: &Path,
        entry: &UnmergedEntry,
    ) -> Result<Result<(MergeInput, GenerationBase), String>, SessionError> {
        if entry.stages.iter().any(Option::is_none) {
            return Ok(Err("한쪽이 generation pointer를 지움".into()));
        }
        let dir = PrivateTempDir::create_in(&std::env::temp_dir(), "stellar-merge-generation")
            .map_err(|e| SessionError::Io(e.to_string()))?;
        let mut projects = Vec::new();
        for (stage, name) in ["base", "ours", "theirs"].into_iter().enumerate() {
            let pointer = stage_bytes(git, root, entry, stage)?;
            match open_generation_side(root, &pointer, &dir.path().join(name)) {
                Ok(side) => projects.push(side),
                Err(reason) => return Ok(Err(format!("{name}: {reason}"))),
            }
        }
        let mut projects = projects.into_iter();
        let (
            Some((base_project, base)),
            Some((ours_project, ours)),
            Some((theirs_project, theirs)),
        ) = (projects.next(), projects.next(), projects.next())
        else {
            return Ok(Err("세 stage가 필요합니다".into()));
        };
        if base_project.project_id != ours_project.project_id
            || base_project.project_id != theirs_project.project_id
        {
            return Ok(Err("세 stage의 project id가 다릅니다".into()));
        }
        Ok(Ok((
            MergeInput { base, ours, theirs },
            GenerationBase {
                ours: ours_project,
                _dir: dir,
            },
        )))
    }

    /// 프로젝트 루트.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 세션을 연 때의 unmerged 목록.
    pub fn entries(&self) -> &[UnmergedEntry] {
        &self.entries
    }

    /// 의미 병합 단위(Git index 순서).
    pub fn units(&self) -> &[SessionUnit] {
        &self.units
    }

    /// raw Git conflict(Stellar 파일이 아님, XIV.11.4).
    pub fn raw_conflicts(&self) -> &[PathBuf] {
        &self.raw
    }

    /// 의미 병합하지 못한 Stellar 경로와 이유.
    pub fn unsupported(&self) -> &[(PathBuf, String)] {
        &self.unsupported
    }

    /// 세션을 연 때 진행 중이던 Git 작업.
    pub fn operation(&self) -> GitOperation {
        self.operation
    }

    /// 모든 semantic conflict(전역 번호 순서).
    pub fn conflicts(&self) -> Vec<SessionConflict<'_>> {
        let mut out = Vec::new();
        for unit in &self.units {
            for conflict in unit.candidate.conflicts() {
                out.push(SessionConflict {
                    index: out.len(),
                    unit,
                    conflict,
                });
            }
        }
        out
    }

    /// 객체가 속한 raw conflict 경로(semantic conflict → raw 경로 대응).
    pub fn paths_for(&self, object: StableId) -> Vec<&Path> {
        self.units
            .iter()
            .filter(|unit| unit.objects.contains(&object))
            .map(|unit| unit.path.as_path())
            .collect()
    }

    /// raw 경로별 객체 대응(raw 경로 → semantic 객체).
    pub fn objects_by_path(&self) -> BTreeMap<&Path, &BTreeSet<StableId>> {
        self.units
            .iter()
            .map(|unit| (unit.path.as_path(), &unit.objects))
            .collect()
    }

    /// 전역 번호의 conflict를 해결합니다. C에만 적용하며 역 command를 돌려줍니다(XIV.12.2).
    pub fn resolve(
        &mut self,
        index: usize,
        resolution: Resolution,
    ) -> Result<CandidateCommand, ResolutionError> {
        let mut remaining = index;
        for unit in &mut self.units {
            let count = unit.candidate.conflicts().len();
            if remaining < count {
                let conflict = &unit.candidate.conflicts()[remaining];
                let command = CandidateCommand::Resolve {
                    object: conflict.object,
                    target: conflict.target.clone(),
                    resolution,
                };
                return unit.candidate.apply(&command);
            }
            remaining -= count;
        }
        Err(ResolutionError::UnknownConflict)
    }
}

/// 계획한 파일 쓰기 하나(프로젝트 기준 경로, `None` = 삭제).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedWrite {
    /// 프로젝트 기준 경로.
    pub path: PathBuf,
    /// 새 bytes(`None`이면 파일 삭제).
    pub bytes: Option<Vec<u8>>,
}

/// 검증한 모든 단위의 쓰기 계획(XIV.12.4). 파일은 아직 쓰지 않았습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePlan {
    /// 파일 쓰기(chunk → 컴포넌트 원본 → manifest → current pointer, 같은 등급은 경로 순서).
    pub writes: Vec<PlannedWrite>,
    /// resolved 처리할 경로(`git add`).
    pub add: Vec<PathBuf>,
}

/// 쓰기 계획 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// 의미 병합 단위가 없음.
    NothingToWrite,
    /// 단위가 쓰기 조건(미해결 0, 새 위반 0)을 통과하지 못함.
    Blocked {
        /// 단위 경로.
        path: PathBuf,
        /// 상태.
        status: UnitStatus,
    },
    /// 결과를 저장 형식으로 쓸 수 없음.
    Encode {
        /// 단위 경로.
        path: PathBuf,
        /// 원인.
        reason: String,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::NothingToWrite => f.write_str("의미 병합할 conflict 단위가 없습니다"),
            PlanError::Blocked { path, status } => write!(
                f,
                "{}: 미해결 conflict {}개, 새 snapshot 위반 {}개",
                path.display(),
                status.unresolved,
                status.violations
            ),
            PlanError::Encode { path, reason } => write!(f, "{}: {reason}", path.display()),
        }
    }
}

impl std::error::Error for PlanError {}

impl ConflictSession {
    /// 모든 단위를 검증하고(XIV.12.3) 쓸 파일을 계획합니다(XIV.12.4). 하나라도 막히면 아무것도 계획하지 않습니다.
    pub fn plan(&self) -> Result<WritePlan, PlanError> {
        if self.units.is_empty() {
            return Err(PlanError::NothingToWrite);
        }
        let mut writes = BTreeMap::new();
        let mut add = Vec::new();
        for unit in &self.units {
            let validated = unit.candidate.validate().map_err(|_| PlanError::Blocked {
                path: unit.path.clone(),
                status: unit.status(),
            })?;
            let encode = |reason: String| PlanError::Encode {
                path: unit.path.clone(),
                reason,
            };
            match unit.kind {
                UnitKind::ComponentSource => {
                    let name = unit.path.file_name().unwrap_or_default();
                    let bytes = super::component_file::encode_component_candidate(validated, name)
                        .map_err(|e| encode(e.to_string()))?;
                    writes.insert(unit.path.clone(), bytes);
                    add.push(unit.path.clone());
                }
                UnitKind::Generation => {
                    let base = unit
                        .generation
                        .as_ref()
                        .ok_or_else(|| encode("ours 세대가 없음".into()))?;
                    let files = super::serialize_merged_generation(&base.ours, validated)
                        .map_err(|e| encode(e.to_string()))?;
                    for (path, bytes) in files {
                        add.push(path.clone());
                        writes.insert(path, Some(bytes));
                    }
                }
                UnitKind::TextSource => {
                    let text = unit
                        .text
                        .as_ref()
                        .ok_or_else(|| encode("텍스트 원본 기준이 없음".into()))?;
                    let files = text
                        .base
                        .encode(validated.snapshot())
                        .map_err(|e| encode(e.to_string()))?;
                    let project_path =
                        |relative: &str| PathBuf::from(format!("{DESIGN_PREFIX}{relative}"));
                    for (relative, content) in &files {
                        let path = project_path(relative);
                        let unmerged = text.paths.contains(&path);
                        let current = std::fs::read(self.root.join(&path)).ok();
                        if unmerged || current.as_deref() != Some(content.as_bytes()) {
                            writes.insert(path.clone(), Some(content.clone().into_bytes()));
                            add.push(path);
                        }
                    }
                    for relative in text.base.ours().keys() {
                        let path = project_path(relative);
                        if !files.contains_key(relative)
                            && !relative.starts_with("components/")
                            && self.root.join(&path).exists()
                        {
                            writes.insert(path.clone(), None);
                            add.push(path);
                        }
                    }
                    for path in &text.paths {
                        add.push(path.clone());
                    }
                    if let Some(base) = &unit.generation {
                        // 로컬 cache(generation)도 같은 결과로 다시 씁니다. 원본이 Git 원본이라 cache는 따로 병합하지
                        // 않습니다.
                        let cache = super::serialize_merged_generation(&base.ours, validated)
                            .map_err(|e| encode(format!("로컬 cache: {e}")))?;
                        for (path, bytes) in cache {
                            add.push(path.clone());
                            writes.insert(path, Some(bytes));
                        }
                    }
                }
            }
        }
        add.sort();
        add.dedup();
        let mut writes: Vec<PlannedWrite> = writes
            .into_iter()
            .map(|(path, bytes)| PlannedWrite { path, bytes })
            .collect();
        // 세대 저장 순서(chunk → manifest → current pointer)를 지킵니다. 경로 순서로 두면 `design/current`가 먼저
        // 쓰여, 중간에 프로세스가 끝나면 pointer가 아직 없는 manifest를 가리킵니다.
        writes.sort_by_key(|write| write_rank(&write.path));
        Ok(WritePlan { writes, add })
    }
}

/// 쓰기 순서 등급: chunk 0, 그 밖(컴포넌트 원본) 1, generation manifest 2, current pointer 3. 같은 등급은 경로 순서입니다.
fn write_rank(path: &Path) -> u8 {
    if path == Path::new(CURRENT_POINTER) {
        3
    } else if path.starts_with("design/generations") {
        2
    } else if path.starts_with("design/objects") {
        0
    } else {
        1
    }
}

#[cfg(test)]
pub(crate) mod tests;
