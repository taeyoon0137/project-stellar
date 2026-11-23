//! 앱 창 session의 기본 원본: 텍스트 원본과 로컬 chunk cache (V.10.7, ADR-0088).
//!
//! 사용자 결정(2026-11-14)대로 Git 원본은 의미 단위 텍스트 파일(`design/**/*.stellar.yaml`, ADR-0077 v4)이고, chunk
//! generation은 빠른 열기·부분 로딩을 위한 파생 cache입니다. cache는 Git이 추적하지 않는 `.stellar/cache/chunks/`
//! ([`stellar_storage::location`], ADR-0122)에 있고, 수명(옛 배치 이동·다시 만들기·GC)은 [`crate::cache`]가 다룹니다. 이 모듈은 둘의
//! 관계를 정하는 한 곳입니다. 런타임 창 session(`stellar_runtime::window_session`)과 엔진 session
//! ([`crate::external_change::source::TextSourceSession`])이 같은 함수를 씁니다.
//!
//! # 원본과 cache의 짝(stamp)
//!
//! cache가 어느 원본에서 만들어졌는지는 `.stellar/cache/text-source.stamp`([`Stamp`])에 남깁니다. 한 줄에 원본
//! 파일 집합 digest([`SourceBase::digest`])와 그때의 current manifest hash를 씁니다. `.stellar/cache/`는 Git 밖
//! 로컬 데이터입니다(기술 아키텍처 §19). 열기는 이 짝과 디스크를 비교해 무엇을 읽을지 정합니다([`OpenPlan`]).
//!
//! | 원본 | stamp와 비교 | 계획 | 읽는 것 | 열 때 쓰는 것 |
//! |---|---|---|---|---|
//! | 있음 | 원본·cache 모두 같음 | [`OpenPlan::Cache`] | generation(문서 하나만, 부분 로딩) | 없음 |
//! | 있음 | 원본만 같고 cache가 움직임 | [`OpenPlan::Forward`] | generation(CLI·MCP가 쓴 새 cache) | 원본(cache → 원본) |
//! | 있음 | 원본이 다름(Git pull·직접 편집) 또는 stamp 없음 | [`OpenPlan::FromText`] | 원본 전체 | cache 다시 만들기 |
//! | 없음 | generation 있음 | [`OpenPlan::Migrate`] | generation | 원본(처음 전환) |
//! | 없음 | generation 없음 | [`OpenPlan::Empty`] | 없음 | 없음(첫 저장이 원본을 만듦) |
//!
//! 원본과 cache가 함께 움직였으면(둘 다 stamp와 다름) 원본이 이깁니다. 덮인 cache generation은 GC 보존 정책
//! ([`crate::cache::collect_garbage`])이 지우기 전까지 cache의 `generations/`에 남습니다.
//!
//! [`observe`]는 관찰 전에 옛 배치(`design/current`·`design/generations/`·`design/objects/`)를 cache 위치로 옮깁니다
//! ([`crate::cache::migrate_layout`]). 앱 열기·CLI·MCP·semantic Git이 모두 이 함수로 관찰하므로 옛 프로젝트도 처음
//! 관찰할 때 한 번 옮겨집니다(Git index 정리는 Git을 아는 호출자가 합니다).
//!
//! # 저장
//!
//! 저장은 원본을 먼저 씁니다([`commit`]: 바뀐 파일만 저장 journal 한 거래, 기대 hash = base). 그다음 cache
//! generation을 쓰고, 둘이 모두 맞으면 stamp를 씁니다. cache 쓰기가 실패해도 원본은 이미 저장되었고 stamp가 옛
//! 짝이라 다음 열기가 [`OpenPlan::FromText`]로 cache를 다시 만듭니다.
//!
//! # 원본 파일 집합
//!
//! [`read_source`]는 `design/` 아래 `.stellar.yaml` 가운데 기본값 세트 파일(`design/style-defaults/`, ADR-0082
//! 결정 4)을 뺀 나머지를 읽습니다. 기본값 세트는 `stellar.yaml`이 고르는 프로젝트 설정이고 텍스트 원본 codec이
//! 아는 경로가 아니기 때문입니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use stellar_assets::record::AssetRecord;
use stellar_core_types::ids::StableId;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::model::DocumentModel;
use stellar_storage::generation::current::{read_current_pointer, CurrentPointerError};
use stellar_storage::generation::lazy::LazyGeneration;
use stellar_storage::hash::ContentHash;
use stellar_storage::journal::{self, JournalChange, JournalError};

use super::component_source::read_project_library;
use super::source_v4::documents;
use super::source_v4::project::{
    decode, encode, from_saved_documents, read_dir, TextProject, DOCUMENT_PATH,
};
use super::source_v4::token::TokenSetSource;
use super::text_source::TextSourceError;
use crate::manifest::Manifest;
use crate::path_resolver::ExternalAccess;
use crate::root::MANIFEST_FILE;
use crate::yaml::load_restricted;
use crate::yaml::loader::MAX_INPUT_BYTES;

/// 프로젝트 root 기준 텍스트 원본 폴더.
pub const DESIGN_DIR: &str = crate::watcher::event::TEXT_SOURCE_DIR;

/// 원본 파일 집합에서 빼는 기본값 세트 폴더(`design/` 기준).
pub const STYLE_DEFAULTS_PREFIX: &str = "style-defaults/";

/// stamp 파일(프로젝트 root 기준).
pub const STAMP_PATH: [&str; 3] = [".stellar", "cache", "text-source.stamp"];

/// stamp 형식 첫 단어.
const STAMP_MAGIC: &str = "stellar-text-source-stamp 1";

/// 원본 파일(`design/` 기준 경로) → 마지막으로 읽거나 쓴 bytes hash.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceBase {
    files: BTreeMap<String, ContentHash>,
}

impl SourceBase {
    /// 읽은 파일 내용의 base.
    pub fn of(files: &BTreeMap<String, String>) -> SourceBase {
        SourceBase {
            files: files
                .iter()
                .map(|(path, text)| (path.clone(), ContentHash::compute(text.as_bytes())))
                .collect(),
        }
    }

    /// 원본이 하나도 없는지(처음 저장 전).
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// 파일별 hash.
    pub fn files(&self) -> &BTreeMap<String, ContentHash> {
        &self.files
    }

    /// 파일 집합의 digest: 경로와 파일 hash를 길이와 함께 차례로 이은 bytes의 hash. 같은 파일 집합·bytes는 같은
    /// 값입니다. stamp와 writer 출처 기록의 key입니다.
    pub fn digest(&self) -> ContentHash {
        let mut input = Vec::new();
        for (path, hash) in &self.files {
            input.extend_from_slice(&(path.len() as u64).to_le_bytes());
            input.extend_from_slice(path.as_bytes());
            input.extend_from_slice(hash.digest());
        }
        ContentHash::compute(&input)
    }

    /// `other`와 다른 경로가 모두 문서 밖 의미 파일([`is_library_path`])인지. 같으면 `false`입니다.
    pub fn differs_only_in_library(&self, other: &SourceBase) -> bool {
        let changed = self.changed_paths(other);
        !changed.is_empty() && changed.iter().all(|path| is_library_path(path))
    }

    /// `other`와 hash가 다른 경로(추가·삭제 포함).
    pub fn changed_paths(&self, other: &SourceBase) -> BTreeSet<String> {
        self.files
            .keys()
            .chain(other.files.keys())
            .filter(|path| self.files.get(*path) != other.files.get(*path))
            .cloned()
            .collect()
    }
}

/// 문서 밖 의미(컴포넌트 정의·token set)만 담는 원본 파일인지(`design/` 기준 경로). 이 파일들은 창 history 밖이라
/// (ADR-0079 결정 2) 미저장 편집이 있어도 디스크 것을 받습니다. 이미지·Text style catalog는 문서 Text style 표를 담으므로
/// 문서 파일로 봅니다.
pub fn is_library_path(path: &str) -> bool {
    path.starts_with("components/") || path.starts_with("tokens/")
}

/// 원본 읽기 결과: 파일 내용과 그 base.
#[derive(Debug, Clone)]
pub struct SourceFiles {
    /// `design/` 기준 경로 → 내용.
    pub files: BTreeMap<String, String>,
    /// 파일별 hash.
    pub base: SourceBase,
}

/// 프로젝트 `root`의 텍스트 원본을 읽습니다. `design/document.stellar.yaml`이 없으면 텍스트 원본 프로젝트가 아니므로
/// `Ok(None)`입니다(컴포넌트 원본만 있는 chunk 프로젝트 포함). 기본값 세트 파일은 뺍니다(모듈 문서).
pub fn read_source(root: &Path) -> Result<Option<SourceFiles>, TextSourceError> {
    let design = root.join(DESIGN_DIR);
    match std::fs::symlink_metadata(design.join(DOCUMENT_PATH)) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let files = read_source_dir(&design)?;
    Ok(Some(SourceFiles {
        base: SourceBase::of(&files),
        files,
    }))
}

/// `design` 폴더의 원본 파일 집합(기본값 세트 파일 제외). [`read_dir`]를 그대로 쓰고 결과만 거릅니다.
pub fn read_source_dir(design: &Path) -> Result<BTreeMap<String, String>, TextSourceError> {
    let mut files = read_dir(design)?;
    files.retain(|path, _| !path.starts_with(STYLE_DEFAULTS_PREFIX));
    Ok(files)
}

/// 원본이 담고 모델(`DocumentModel`)이 담지 않는 프로젝트 의미: 컴포넌트 정의, token set, 이미지 record.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceLibrary {
    /// 컴포넌트 정의.
    pub components: ComponentLibrary,
    /// token set.
    pub token_sets: Vec<TokenSetSource>,
    /// asset StableId → 이미지 record.
    pub images: BTreeMap<StableId, AssetRecord>,
}

impl SourceLibrary {
    /// 원본 프로젝트에서 문서 밖 의미를 떼어 냅니다.
    pub fn split(project: TextProject) -> (DocumentModel, SourceLibrary) {
        (
            project.document,
            SourceLibrary {
                components: project.components,
                token_sets: project.token_sets,
                images: project.images,
            },
        )
    }

    /// 모델과 합쳐 원본 프로젝트를 만듭니다.
    pub fn join(&self, document: DocumentModel) -> TextProject {
        TextProject {
            document,
            components: self.components.clone(),
            token_sets: self.token_sets.clone(),
            images: self.images.clone(),
        }
    }
}

/// 원본 파일 집합을 해석합니다(검증 포함). 추가 root 문서(ADR-0131 결정 3)도 함께 검증하지만 돌려주는 의미는 첫
/// 문서와 프로젝트 전체 의미입니다(추가 문서는 [`decode_extra_documents`]).
pub fn decode_source(files: &SourceFiles) -> Result<TextProject, TextSourceError> {
    let project = decode(&files.files)?;
    documents::decode(project.document.document_id(), &files.files)?;
    Ok(project)
}

/// 원본의 추가 root 문서(generation root 순서, 첫 문서 제외). 없으면 빈 목록입니다.
pub fn decode_extra_documents(
    files: &SourceFiles,
    primary: StableId,
) -> Result<Vec<DocumentModel>, TextSourceError> {
    documents::decode(primary, &files.files)
}

/// cache가 만들어진 원본·generation 짝.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// 원본 파일 집합 digest.
    pub source: ContentHash,
    /// 그때의 current manifest hash.
    pub manifest: ContentHash,
}

fn stamp_path(root: &Path) -> PathBuf {
    STAMP_PATH
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// stamp를 읽습니다. 없거나 형식이 맞지 않으면 `None`(짝을 모르는 cache로 봅니다).
pub fn read_stamp(root: &Path) -> Option<Stamp> {
    let bytes = stellar_storage::source_file::read(&stamp_path(root), 512).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let mut lines = text.lines();
    if lines.next()? != STAMP_MAGIC {
        return None;
    }
    let source = lines.next()?.strip_prefix("source ")?.parse().ok()?;
    let manifest = lines.next()?.strip_prefix("manifest ")?.parse().ok()?;
    lines.next().is_none().then_some(Stamp { source, manifest })
}

/// stamp를 원자적으로 씁니다. `.stellar/cache/.gitignore`(`*`)가 없으면 함께 만듭니다(chunk cache와 같은 함수,
/// [`stellar_storage::location::ensure_cache_ignore`]).
pub fn write_stamp(root: &Path, stamp: &Stamp) -> io::Result<()> {
    let path = stamp_path(root);
    stellar_storage::location::ensure_cache_ignore(root)?;
    let text = format!(
        "{STAMP_MAGIC}\nsource {}\nmanifest {}\n",
        stamp.source, stamp.manifest
    );
    stellar_storage::atomic_file::write_file_atomic(&path, text.as_bytes())
}

/// 열기에서 무엇을 읽고 무엇을 맞출지(모듈 문서 표).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenPlan {
    /// 원본과 cache가 stamp 짝과 같음. cache generation에서 문서만 읽습니다.
    Cache,
    /// 원본은 stamp와 같고 cache가 움직였음(CLI·MCP의 generation 저장). cache를 읽고 원본을 맞춰 씁니다.
    Forward,
    /// 원본이 stamp와 다름(또는 stamp·cache 없음). 원본을 읽고 cache를 다시 만듭니다.
    FromText,
    /// 원본이 없고 generation만 있음(기존 chunk 프로젝트). generation을 읽고 원본으로 옮깁니다.
    Migrate,
    /// 원본도 generation도 없음.
    Empty,
}

impl OpenPlan {
    /// 디스크 관찰값으로 계획을 정합니다. `source`는 원본 digest(원본이 없으면 `None`), `current`는 current
    /// manifest hash(없으면 `None`)입니다.
    pub fn decide(
        source: Option<ContentHash>,
        current: Option<ContentHash>,
        stamp: Option<Stamp>,
    ) -> OpenPlan {
        match (source, current) {
            (None, None) => OpenPlan::Empty,
            (None, Some(_)) => OpenPlan::Migrate,
            (Some(_), None) => OpenPlan::FromText,
            (Some(source), Some(current)) => match stamp {
                Some(stamp) if stamp.source == source && stamp.manifest == current => {
                    OpenPlan::Cache
                }
                Some(stamp) if stamp.source == source => OpenPlan::Forward,
                _ => OpenPlan::FromText,
            },
        }
    }

    /// 안정 code(보고·trace용).
    pub fn code(self) -> &'static str {
        match self {
            OpenPlan::Cache => "cache",
            OpenPlan::Forward => "forward",
            OpenPlan::FromText => "from-text",
            OpenPlan::Migrate => "migrate",
            OpenPlan::Empty => "empty",
        }
    }
}

/// 원본 저장 실패.
#[derive(Debug)]
pub enum SourceCommitError {
    /// 모델·library를 원본으로 쓸 수 없습니다(표현할 수 없는 의미).
    Encode(TextSourceError),
    /// base를 읽은 뒤 외부가 바꾼 파일이 있어 아무것도 쓰지 않았습니다(`design/` 기준 경로).
    Stale {
        /// 바뀐 파일.
        paths: Vec<String>,
    },
    /// 저장 journal 실패(잠금·I/O·복구).
    Journal(JournalError),
}

impl fmt::Display for SourceCommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceCommitError::Encode(error) => {
                write!(f, "텍스트 원본으로 쓸 수 없습니다: {error}")
            }
            SourceCommitError::Stale { paths } => {
                write!(f, "외부에서 바뀐 원본이 있습니다: {}", paths.join(", "))
            }
            SourceCommitError::Journal(error) => write!(f, "저장 journal 오류: {error}"),
        }
    }
}

impl std::error::Error for SourceCommitError {}

/// 원본 저장 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCommit {
    /// 저장 뒤 base(디스크와 같음).
    pub base: SourceBase,
    /// 새로 쓰거나 바꾼 파일 수.
    pub written: usize,
    /// 지운 파일 수.
    pub deleted: usize,
}

/// 인코딩한 파일 집합을 base 위에 쓸 journal 변경 목록. base와 같은 파일은 빼고, base에만 있는 파일은 지웁니다.
/// 다만 추가 root 문서 파일(`documents.stellar.yaml`·`documents/`)은 `files`에 없어도 지우지 않습니다. 첫 문서만
/// 다루는 저장(앱 창·CLI·MCP)이 다른 root 문서를 지우지 않게 하기 위함입니다.
pub fn plan_changes(base: &SourceBase, files: &BTreeMap<String, String>) -> Vec<JournalChange> {
    let mut changes = Vec::new();
    for (path, text) in files {
        let hash = ContentHash::compute(text.as_bytes());
        if base.files.get(path) == Some(&hash) {
            continue;
        }
        changes.push(JournalChange::write(
            format!("{DESIGN_DIR}/{path}"),
            text.as_bytes().to_vec(),
            base.files.get(path).copied(),
        ));
    }
    for (path, hash) in &base.files {
        // 추가 root 문서는 첫 문서만 다루는 인코딩 결과에 없습니다. 그 경로는 지우지 않습니다(ADR-0131 결정 3).
        if !files.contains_key(path) && !documents::is_documents_path(path) {
            changes.push(JournalChange::delete(format!("{DESIGN_DIR}/{path}"), *hash));
        }
    }
    changes
}

/// 원본 프로젝트를 인코딩해 `base` 위에 저장 journal 한 거래로 씁니다(바뀐 파일만). 바뀐 것이 없으면 쓰지 않고
/// 같은 base를 돌려줍니다. 기대 hash는 base이므로 그 사이 외부가 바꾼 파일이 있으면 [`SourceCommitError::Stale`]
/// 이고 아무것도 쓰지 않습니다.
pub fn commit(
    root: &Path,
    base: &SourceBase,
    project: &TextProject,
) -> Result<SourceCommit, SourceCommitError> {
    let encoded = encode(project).map_err(SourceCommitError::Encode)?;
    commit_files(root, base, &encoded.files)
}

/// 이미 인코딩한 파일 집합을 [`commit`]과 같은 규칙으로 씁니다.
pub fn commit_files(
    root: &Path,
    base: &SourceBase,
    files: &BTreeMap<String, String>,
) -> Result<SourceCommit, SourceCommitError> {
    let next = SourceBase::of(files);
    let changes = plan_changes(base, files);
    if changes.is_empty() {
        return Ok(SourceCommit {
            base: next,
            written: 0,
            deleted: 0,
        });
    }
    let report = match journal::commit(root, &changes, MAX_INPUT_BYTES as u64) {
        Ok(report) => report,
        Err(JournalError::Stale { paths }) => {
            let prefix = format!("{DESIGN_DIR}/");
            return Err(SourceCommitError::Stale {
                paths: paths
                    .into_iter()
                    .map(|path| {
                        path.strip_prefix(&prefix)
                            .map_or(path.clone(), str::to_owned)
                    })
                    .collect(),
            });
        }
        Err(error) => return Err(SourceCommitError::Journal(error)),
    };
    Ok(SourceCommit {
        base: next,
        written: report.written,
        deleted: report.deleted,
    })
}

/// `stellar.yaml`의 활성 기본값 세트 이름(ADR-0082 결정 4). 매니페스트를 읽을 수 없으면 오류입니다.
pub fn active_style_defaults(root: &Path) -> Result<String, String> {
    let bytes =
        stellar_storage::source_file::read(&root.join(MANIFEST_FILE), MAX_INPUT_BYTES as u64)
            .map_err(|error| error.to_string())?;
    let text = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
    let value = load_restricted(text).map_err(|error| error.to_string())?;
    let manifest = Manifest::from_yaml(&value).map_err(|error| error.to_string())?;
    Ok(crate::manifest_style_defaults::active_name(&manifest).to_owned())
}

/// 열기 계획을 정하는 디스크 관찰값(ADR-0113 결정 1). 앱 창 session·CLI·MCP·semantic Git이 같은 함수로 봅니다.
#[derive(Debug, Clone)]
pub struct DiskSource {
    /// 텍스트 원본(없으면 `None`).
    pub text: Option<SourceFiles>,
    /// current manifest hash(cache가 없으면 `None`).
    pub current: Option<ContentHash>,
    /// 원본·cache 짝.
    pub stamp: Option<Stamp>,
    /// 고른 계획.
    pub plan: OpenPlan,
}

impl DiskSource {
    /// 원본 revision([`source_revision`]). 원본이 없으면 `None`입니다.
    pub fn source_revision(&self) -> Option<ContentHash> {
        self.text
            .as_ref()
            .map(|text| source_revision(self.current, &text.base))
    }
}

/// 디스크 관찰 실패.
#[derive(Debug)]
pub enum ObserveError {
    /// 원본을 읽을 수 없습니다.
    Source(TextSourceError),
    /// current pointer가 손상됐거나 읽을 수 없습니다.
    Pointer(CurrentPointerError),
}

impl fmt::Display for ObserveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObserveError::Source(error) => write!(f, "{error}"),
            ObserveError::Pointer(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ObserveError {}

/// 프로젝트 `root`의 원본·cache·stamp를 읽고 계획을 정합니다. 원본·cache 의미는 쓰지 않습니다. 옛 배치가 있으면
/// 먼저 cache 위치로 옮깁니다(모듈 문서). 옮기지 못하면 원본이 있을 때는 cache 없이 관찰하고(원본이 Git 원본),
/// 원본이 없으면 cache를 읽을 수 없으므로 [`ObserveError::Pointer`]입니다.
pub fn observe(root: &Path) -> Result<DiskSource, ObserveError> {
    let migrated = crate::cache::migrate_working_tree(root);
    let text = read_source(root).map_err(ObserveError::Source)?;
    if let (Err(error), None) = (&migrated, &text) {
        return Err(ObserveError::Pointer(CurrentPointerError::Io {
            path: crate::cache::cache_dir(root),
            source: io::Error::other(error.to_string()),
        }));
    }
    let current = match read_current_pointer(root) {
        Ok(current) => Some(current),
        Err(CurrentPointerError::Missing { .. }) => None,
        Err(error) => return Err(ObserveError::Pointer(error)),
    };
    let stamp = read_stamp(root);
    let plan = OpenPlan::decide(text.as_ref().map(|text| text.base.digest()), current, stamp);
    Ok(DiskSource {
        text,
        current,
        stamp,
        plan,
    })
}

/// 원본이 있는 프로젝트 상태의 revision hash(ADR-0113 결정 2): `stellar-text-source-revision/1`, current manifest
/// hash(없으면 빈 값), 원본 digest를 길이와 함께 이은 bytes의 hash. 원본이나 cache 어느 쪽이 바뀌어도 값이 바뀝니다.
pub fn source_revision(current: Option<ContentHash>, base: &SourceBase) -> ContentHash {
    let mut input = b"stellar-text-source-revision/1".to_vec();
    let current = current
        .map(|hash| hash.digest().to_vec())
        .unwrap_or_default();
    input.extend_from_slice(&(current.len() as u64).to_le_bytes());
    input.extend_from_slice(&current);
    input.extend_from_slice(base.digest().digest());
    ContentHash::compute(&input)
}

/// generation 문서를 텍스트 원본 의미로 옮길 수 있는지 검증하고 원본 base와 문서 밖 의미를 돌려줍니다(ADR-0088 결정 2,
/// ADR-0113 결정 1, ADR-0131 결정 1·3). 앱 창 session과 CLI·MCP 쓰기가 같은 함수를 씁니다.
///
/// generation의 모든 chunk를 원본이 소비하고([`from_saved_documents`]), 인코딩한 원본을 다시 읽은 의미가 같아야
/// 합니다. 원본이 이미 있으면(`text`, Forward) 컴포넌트 정의(interface 포함)와 token set은 원본 것을 쓰고, generation의
/// component library chunk는 그 정의와 같아야 합니다. root 문서가 둘 이상이면 `model`이 첫 root 문서여야 하고, 나머지
/// root 문서는 원본의 추가 root 문서와 의미가 같아야 합니다(이 경로는 첫 문서만 원본으로 옮기므로). 원본이 없으면
/// (Migrate) root 문서는 하나여야 하고(둘 이상은 [`crate::cache::convert_legacy`]의 변환 경로), `design/` 아래 기존
/// `.stellar.yaml`(컴포넌트 원본)이 모두 인코딩 결과의 경로여야 합니다. 실패는 이유 문장입니다.
pub fn convert_saved(
    root: &Path,
    lazy: &LazyGeneration,
    model: &DocumentModel,
    text: Option<&SourceFiles>,
) -> Result<(SourceBase, SourceLibrary), String> {
    let decoded = text
        .map(decode_source)
        .transpose()
        .map_err(|error| error.to_string())?;
    let components = match &decoded {
        Some(project) => project.components.clone(),
        None => {
            read_project_library(root, ExternalAccess::MetadataOnly)
                .map_err(|error| error.to_string())?
                .0
        }
    };
    let roots = &lazy.manifest().root_document_ids;
    if roots.first() != Some(&model.document_id()) {
        return Err("옮길 문서가 generation의 첫 root 문서가 아닙니다".into());
    }
    let mut documents = vec![documents::copy_model(model).map_err(|error| error.to_string())?];
    for id in &roots[1..] {
        let extra = super::load::load_document_from_generation(
            lazy,
            *id,
            &mut stellar_storage::generation::lazy::NoopRecorder,
        )
        .map_err(|error| error.to_string())?;
        documents.push(extra.model);
    }
    let (mut project, extras) =
        from_saved_documents(lazy, documents, components).map_err(|error| error.to_string())?;
    if !extras.is_empty() {
        let Some(text) = text else {
            return Err(
                "root 문서가 둘 이상인 generation은 옛 배치 변환(stellar cache migrate --convert)으로 옮깁니다"
                    .into(),
            );
        };
        let source =
            decode_extra_documents(text, model.document_id()).map_err(|error| error.to_string())?;
        let same = source.len() == extras.len()
            && source.iter().zip(&extras).all(|(a, b)| {
                matches!(
                    (documents::semantic(a), documents::semantic(b)),
                    (Ok(a), Ok(b)) if a == b
                )
            });
        if !same {
            return Err("원본의 추가 root 문서와 generation의 다른 root 문서가 다릅니다".into());
        }
    }
    if let Some(decoded) = &decoded {
        project.token_sets = decoded.token_sets.clone();
    }
    let encoded = encode(&project).map_err(|error| error.to_string())?;
    let decoded_again = decode(&encoded.files).map_err(|error| error.to_string())?;
    if decoded_again
        .semantic_key()
        .map_err(|error| error.to_string())?
        != project.semantic_key().map_err(|error| error.to_string())?
    {
        return Err("다시 읽은 텍스트 원본의 의미가 generation과 다릅니다".into());
    }
    let base = match text {
        Some(text) => text.base.clone(),
        None => {
            let design = root.join(DESIGN_DIR);
            let existing = match std::fs::symlink_metadata(&design) {
                Ok(_) => read_source_dir(&design).map_err(|error| error.to_string())?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
                Err(error) => return Err(error.to_string()),
            };
            if let Some(unknown) = existing
                .keys()
                .find(|path| !encoded.files.contains_key(*path))
            {
                return Err(format!(
                    "텍스트 원본이 모르는 파일이 design/ 아래에 있습니다: {unknown}"
                ));
            }
            SourceBase::of(&existing)
        }
    };
    let (_, library) = SourceLibrary::split(project);
    Ok((base, library))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(n: u8) -> ContentHash {
        ContentHash::compute(&[n])
    }

    #[test]
    fn open_plan_follows_the_stamp_table() {
        let stamp = Some(Stamp {
            source: hash(1),
            manifest: hash(2),
        });
        assert_eq!(OpenPlan::decide(None, None, stamp), OpenPlan::Empty);
        assert_eq!(
            OpenPlan::decide(None, Some(hash(2)), stamp),
            OpenPlan::Migrate
        );
        assert_eq!(
            OpenPlan::decide(Some(hash(1)), None, stamp),
            OpenPlan::FromText
        );
        assert_eq!(
            OpenPlan::decide(Some(hash(1)), Some(hash(2)), stamp),
            OpenPlan::Cache
        );
        assert_eq!(
            OpenPlan::decide(Some(hash(1)), Some(hash(3)), stamp),
            OpenPlan::Forward
        );
        // 원본이 움직였으면(둘 다 움직였어도) 원본이 이깁니다.
        assert_eq!(
            OpenPlan::decide(Some(hash(4)), Some(hash(2)), stamp),
            OpenPlan::FromText
        );
        assert_eq!(
            OpenPlan::decide(Some(hash(4)), Some(hash(3)), stamp),
            OpenPlan::FromText
        );
        assert_eq!(
            OpenPlan::decide(Some(hash(1)), Some(hash(2)), None),
            OpenPlan::FromText
        );
    }

    #[test]
    fn stamp_round_trips_and_rejects_other_bytes() {
        let dir = std::env::temp_dir().join(format!(
            "stellar-stamp-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_stamp(&dir), None);
        let stamp = Stamp {
            source: hash(7),
            manifest: hash(8),
        };
        write_stamp(&dir, &stamp).unwrap();
        assert_eq!(read_stamp(&dir), Some(stamp));
        assert_eq!(
            std::fs::read(dir.join(".stellar/cache/.gitignore")).unwrap(),
            b"*\n"
        );
        std::fs::write(stamp_path(&dir), b"stellar-text-source-stamp 1\nsource x\n").unwrap();
        assert_eq!(read_stamp(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn digest_and_changes_cover_added_changed_and_removed_files() {
        let a: BTreeMap<String, String> = [("a.stellar.yaml", "1"), ("b.stellar.yaml", "2")]
            .into_iter()
            .map(|(p, t)| (p.to_owned(), t.to_owned()))
            .collect();
        let mut b = a.clone();
        b.insert("b.stellar.yaml".into(), "3".into());
        b.remove("a.stellar.yaml");
        b.insert("c.stellar.yaml".into(), "4".into());
        let (base_a, base_b) = (SourceBase::of(&a), SourceBase::of(&b));
        assert_eq!(base_a.digest(), SourceBase::of(&a.clone()).digest());
        assert_ne!(base_a.digest(), base_b.digest());
        assert_eq!(
            base_a
                .changed_paths(&base_b)
                .into_iter()
                .collect::<Vec<_>>(),
            ["a.stellar.yaml", "b.stellar.yaml", "c.stellar.yaml"]
        );
        let changes = plan_changes(&base_a, &b);
        assert_eq!(changes.len(), 3);
        assert!(plan_changes(&base_a, &a).is_empty());
        assert!(!base_a.differs_only_in_library(&base_b));
        assert!(!base_a.differs_only_in_library(&base_a));
        let mut c = a.clone();
        c.insert("components/x-0123456789ab.stellar.yaml".into(), "5".into());
        c.insert("tokens/t.stellar.yaml".into(), "6".into());
        assert!(base_a.differs_only_in_library(&SourceBase::of(&c)));
        c.insert("assets/catalog.stellar.yaml".into(), "7".into());
        assert!(!base_a.differs_only_in_library(&SourceBase::of(&c)));
    }
}
