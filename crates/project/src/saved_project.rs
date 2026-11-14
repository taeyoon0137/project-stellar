//! 디스크에 저장된 프로젝트 loader (XIV.2.7, XIV.11.2 연결).
//!
//! Git snapshot provider(XIV.2)의 [`DiskProjectLoader`]를 Stellar 프로젝트 열기 경로로 구현합니다.
//! WorkingTree snapshot([`read_working_tree_project`])과 conflict 세 snapshot(base·ours·theirs)은 모두
//! 이 loader로 **디스크에 저장된 파일**만 읽어 엽니다. 편집기의 저장 대기 메모리 상태(autosave 전 편집)는
//! loader 입력에 존재하지 않으므로, snapshot과 Git status는 같은 기준
//! ([`SnapshotBasis::SavedFilesOnDisk`](stellar_git_porcelain::snapshot::working_tree::SnapshotBasis))입니다.
//!
//! 열기 순서([`SavedProjectLoader::open`]):
//! 1. `stellar.yaml`을 제한 YAML(IV.1.2)로 읽고 스키마(IV.1.1)를 검증해 project id(IV.2.3)를 얻습니다.
//! 2. 저장소가 읽을 파일(current pointer, generation manifest, manifest가 가리키는 모든 chunk)의 종류와
//!    크기를 **읽기 전에** 검사합니다(아래 "비신뢰 입력").
//! 3. current pointer → manifest → chunk 존재 순으로 검증하고(V.4.4) DocumentManifest chunk만
//!    읽습니다(V.6.6 lazy open).
//! 4. 문서 내용(ViewTree·subtree)은 요청할 때 [`SavedProject::load_document`]로 읽습니다(VI.10.3).
//!
//! `.git`은 요구하지 않습니다. materialize된 virtual project(XIV.2.6)에는 `.git`이 없기 때문입니다.
//!
//! # 비신뢰 입력
//!
//! conflict의 theirs side처럼 materialize된 snapshot은 사용자가 checkout하지 않은 Git 객체에서 온
//! 비신뢰 입력입니다. 저장소 파일이 장치·FIFO이거나 거대하면 읽기가 멈추거나 메모리를 무한히 쓰므로:
//!
//! - `stellar.yaml`은 symlink를 따르지 않고, 정규 파일만 loader 상한([`MAX_INPUT_BYTES`])까지만
//!   읽습니다. 검사한 파일과 연 파일이 다르면(검사 뒤 교체) 거부합니다. 검사 뒤 FIFO·장치로 바뀌어도
//!   열기에서 멈추지 않도록 macOS에서는 non-blocking·no-follow로 엽니다(macOS 밖 unix는 flag 없이 열어
//!   그 사이 FIFO로 바뀌면 열기가 writer를 기다릴 수 있음, Linux 등은 아직 지원 대상이 아님).
//! - 저장소 파일은 정규 파일이어야 하고 [`StorageReadLimits`] 상한 이하여야 합니다. 디렉터리 구성
//!   요소는 디렉터리여야 합니다.
//! - [`StorageLinkPolicy::Reject`](materialize된 snapshot)는 `design/` 아래 저장소 경로의 어느 구성
//!   요소든 symlink면 거부합니다. snapshot 밖(절대 경로 대상 포함) 파일을 읽지 않게 합니다.
//!   [`StorageLinkPolicy::Follow`](사용자 작업 트리)는 symlink를 전면 허용하는 기술 아키텍처 19.3에
//!   따라 symlink를 따르되 최종 대상의 종류·크기는 같은 규칙으로 검사합니다.
//!
//! 저장소 파일은 저장소 crate가 경로로 다시 열기 때문에 검사와 읽기 사이의 교체(TOCTOU)는 막지
//! 못합니다. materialize된 snapshot은 소유자 전용 session 디렉터리(XVI.7.5) 안에 있어 같은 사용자의
//! 프로세스만 이 틈을 쓸 수 있습니다. 검사한 manifest와 실제로 연 manifest가 다르면
//! [`SavedProjectError::StorageChanged`]로 거부합니다. 검사 단계가 manifest를 한 번 더 읽으므로 열기
//! 비용은 manifest 읽기·디코딩 한 번만큼 늘어납니다.
//!
//! 이 loader는 디스크 bytes를 바꾸지 않습니다(읽기 전용).

pub mod head;

use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use stellar_core_types::ids::StableId;
use stellar_git_porcelain::ops::GitOpError;
use stellar_git_porcelain::snapshot::working_tree::{
    read_working_tree_snapshot, DiskProjectLoader, WorkingTreeSnapshot,
};
use stellar_git_porcelain::trust::GatedGitRunner;
use stellar_storage::chunk::path::ChunkObjectPath;
use stellar_storage::generation::current::{current_pointer_path, read_current_pointer};
use stellar_storage::generation::lazy::{LazyGeneration, LazyLoadError, NoopRecorder};
use stellar_storage::generation::loader::{manifest_path, verify_generation, LoadError};
use stellar_storage::generation::manifest::GenerationManifest;

use crate::document_io::load::{load_document_from_generation, LoadDocumentError, LoadedDocument};
use crate::document_io::manifest::DocumentManifestChunk;
use crate::identity::parse_project_id;
use crate::manifest::{Manifest, ManifestError};
use crate::probe::read_at_most;
use crate::root::MANIFEST_FILE;
use crate::yaml::loader::MAX_INPUT_BYTES;
use crate::yaml::{load_restricted, LoaderError, LoaderErrorKind};

/// 저장소 파일 크기 상한 기본값: current pointer(형식상 정확히 72 bytes).
pub const DEFAULT_MAX_CURRENT_POINTER_BYTES: u64 = 1024;

/// 저장소 파일 크기 상한 기본값: generation manifest 한 개.
///
/// 제품 예산이 아니라 비신뢰 입력 방어 상한입니다. chunk pointer 하나가 수십 bytes라 백만 개 이상의
/// chunk를 담을 수 있습니다.
pub const DEFAULT_MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

/// 저장소 파일 크기 상한 기본값: chunk 파일 한 개.
///
/// 제품 예산이 아니라 비신뢰 입력 방어 상한입니다. chunk 분할 임계치(ADR-0012, 256 KiB)의 1,000배
/// 이상이라 분할할 수 없는 큰 의미 단위도 담습니다.
pub const DEFAULT_MAX_CHUNK_BYTES: u64 = 256 * 1024 * 1024;

/// 저장소(`design/`) 경로의 symlink 처리 규칙.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageLinkPolicy {
    /// symlink를 따릅니다(사용자 작업 트리, 기술 아키텍처 19.3). 최종 대상의 종류·크기는 검사합니다.
    Follow,
    /// 저장소 경로 구성 요소가 symlink면 거부합니다(materialize된 snapshot, 비신뢰 Git 객체).
    Reject,
}

/// loader가 읽는 저장소 파일의 크기 상한(bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageReadLimits {
    /// `design/current` 상한.
    pub current_pointer_bytes: u64,
    /// generation manifest 파일 하나의 상한.
    pub manifest_bytes: u64,
    /// chunk 파일 하나의 상한.
    pub chunk_bytes: u64,
}

impl Default for StorageReadLimits {
    fn default() -> StorageReadLimits {
        StorageReadLimits {
            current_pointer_bytes: DEFAULT_MAX_CURRENT_POINTER_BYTES,
            manifest_bytes: DEFAULT_MAX_MANIFEST_BYTES,
            chunk_bytes: DEFAULT_MAX_CHUNK_BYTES,
        }
    }
}

/// 디스크에서 연 저장된 프로젝트.
#[derive(Debug)]
pub struct SavedProject {
    /// 검증한 `stellar.yaml`.
    pub manifest: Manifest,
    /// 매니페스트의 project id.
    pub project_id: StableId,
    /// 검증한 current generation(DocumentManifest chunk까지 읽음).
    pub generation: LazyGeneration,
}

impl SavedProject {
    /// current generation 번호.
    pub fn generation_number(&self) -> u64 {
        self.generation.manifest().generation
    }

    /// 저장된 문서 id(DocumentManifest chunk 순서).
    pub fn document_ids(&self) -> Result<Vec<StableId>, LoadDocumentError> {
        self.generation
            .document_manifest_records()
            .map(|(pointer, payload)| {
                DocumentManifestChunk::decode_for_schema(pointer.schema_version, payload)
                    .map(|manifest| manifest.definition.id)
                    .map_err(LoadDocumentError::Manifest)
            })
            .collect()
    }

    /// 저장된 문서 하나를 디스크 chunk에서 로드합니다(그 문서의 chunk만 읽음).
    pub fn load_document(&self, document: StableId) -> Result<LoadedDocument, LoadDocumentError> {
        load_document_from_generation(&self.generation, document, &mut NoopRecorder)
    }
}

/// 저장소 파일·디렉터리 검사 실패 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageEntryProblem {
    /// symlink라 따르지 않음([`StorageLinkPolicy::Reject`]).
    Symlink,
    /// 디렉터리여야 하는 구성 요소가 디렉터리가 아님.
    NotDirectory,
    /// 정규 파일이 아님(디렉터리·FIFO·장치·socket 등).
    NotFile,
    /// 크기 상한 초과.
    TooLarge {
        /// 파일 크기.
        len: u64,
        /// 상한.
        limit: u64,
    },
    /// metadata 읽기 실패.
    Io {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
}

impl fmt::Display for StorageEntryProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageEntryProblem::Symlink => f.write_str("symlink라 따르지 않습니다"),
            StorageEntryProblem::NotDirectory => f.write_str("디렉터리가 아닙니다"),
            StorageEntryProblem::NotFile => f.write_str("정규 파일이 아닙니다"),
            StorageEntryProblem::TooLarge { len, limit } => {
                write!(f, "크기 {len} bytes가 상한 {limit} bytes를 넘습니다")
            }
            StorageEntryProblem::Io { kind } => write!(f, "metadata 읽기 실패: {kind:?}"),
        }
    }
}

/// 저장된 프로젝트 열기 실패.
#[derive(Debug)]
pub enum SavedProjectError {
    /// `stellar.yaml`이 symlink라 따르지 않음.
    ManifestSymlink,
    /// `stellar.yaml`이 정규 파일이 아님(디렉터리·FIFO·장치 등).
    ManifestNotFile,
    /// 검사한 `stellar.yaml`과 연 파일이 다름(검사 뒤 교체).
    ManifestChanged,
    /// `stellar.yaml` 읽기 실패.
    ManifestRead {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// `stellar.yaml` 제한 YAML 문법 오류(크기 상한 초과 포함).
    ManifestSyntax(LoaderError),
    /// 매니페스트 스키마 오류.
    ManifestSchema(ManifestError),
    /// `project.id`가 UUID 형식이 아님.
    InvalidProjectId {
        /// 매니페스트의 원문 값.
        id: String,
    },
    /// 저장소 파일·디렉터리가 읽기 규칙에 맞지 않음(읽기 전 거부).
    StorageEntry {
        /// 프로젝트 root 기준 상대 경로(`/` 구분).
        path: String,
        /// 거부 이유.
        problem: StorageEntryProblem,
    },
    /// 검사한 generation과 연 generation이 다름(검사 중 저장소 변경).
    StorageChanged,
    /// 저장소(generation) 열기 실패(V.4.4).
    Storage(LazyLoadError),
}

impl fmt::Display for SavedProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SavedProjectError::ManifestSymlink => {
                write!(f, "{MANIFEST_FILE}이 symlink라 따르지 않습니다")
            }
            SavedProjectError::ManifestNotFile => {
                write!(f, "{MANIFEST_FILE}이 정규 파일이 아닙니다")
            }
            SavedProjectError::ManifestChanged => {
                write!(f, "{MANIFEST_FILE}이 검사 뒤 다른 파일로 바뀌었습니다")
            }
            SavedProjectError::ManifestRead { kind } => {
                write!(f, "{MANIFEST_FILE} 읽기 실패: {kind:?}")
            }
            SavedProjectError::ManifestSyntax(e) => write!(f, "{MANIFEST_FILE} 손상: {e}"),
            SavedProjectError::ManifestSchema(e) => write!(f, "매니페스트 스키마 오류: {e}"),
            SavedProjectError::InvalidProjectId { id } => {
                write!(f, "project.id가 UUID가 아닙니다: {id:?}")
            }
            SavedProjectError::StorageEntry { path, problem } => {
                write!(f, "저장소 항목 {path}: {problem}")
            }
            SavedProjectError::StorageChanged => {
                f.write_str("저장소가 검사 중 바뀌어 검사한 generation과 다릅니다")
            }
            SavedProjectError::Storage(e) => write!(f, "저장소 열기 실패: {e}"),
        }
    }
}

impl std::error::Error for SavedProjectError {}

/// 디스크에 저장된 파일로 프로젝트를 여는 loader(읽기 전용).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedProjectLoader {
    links: StorageLinkPolicy,
    limits: StorageReadLimits,
}

impl Default for SavedProjectLoader {
    fn default() -> SavedProjectLoader {
        SavedProjectLoader::new()
    }
}

impl SavedProjectLoader {
    /// 사용자 작업 트리용 loader를 만듭니다: symlink를 따르고([`StorageLinkPolicy::Follow`]) 기본
    /// 크기 상한을 씁니다.
    pub fn new() -> SavedProjectLoader {
        SavedProjectLoader {
            links: StorageLinkPolicy::Follow,
            limits: StorageReadLimits::default(),
        }
    }

    /// materialize된 snapshot(비신뢰 Git 객체)용 loader를 만듭니다: 저장소 경로의 symlink를
    /// 거부합니다([`StorageLinkPolicy::Reject`]).
    pub fn for_materialized_snapshot() -> SavedProjectLoader {
        SavedProjectLoader {
            links: StorageLinkPolicy::Reject,
            limits: StorageReadLimits::default(),
        }
    }

    /// 크기 상한을 바꾼 loader.
    pub fn with_limits(self, limits: StorageReadLimits) -> SavedProjectLoader {
        SavedProjectLoader { limits, ..self }
    }

    /// symlink 규칙.
    pub fn link_policy(&self) -> StorageLinkPolicy {
        self.links
    }

    /// 크기 상한.
    pub fn limits(&self) -> StorageReadLimits {
        self.limits
    }

    /// `root`의 저장된 프로젝트를 엽니다: 매니페스트 → 저장소 파일 검사 → current generation(V.4.4).
    pub fn open(&self, root: &Path) -> Result<SavedProject, SavedProjectError> {
        let manifest = read_manifest(root)?;
        let project_id = parse_project_id(&manifest.project.id).ok_or_else(|| {
            SavedProjectError::InvalidProjectId {
                id: manifest.project.id.clone(),
            }
        })?;
        let checked = self.check_storage(root, project_id)?;
        let generation = LazyGeneration::open(root, project_id, &mut NoopRecorder)
            .map_err(SavedProjectError::Storage)?;
        if generation.manifest() != &checked {
            return Err(SavedProjectError::StorageChanged);
        }
        Ok(SavedProject {
            manifest,
            project_id,
            generation,
        })
    }

    /// 저장소가 읽을 파일을 읽기 전에 검사하고, 검사한 current manifest를 돌려줍니다.
    ///
    /// 파일이 없으면 그 항목 검사를 건너뛰고 저장소의 typed 오류(V.4.4: pointer·manifest·chunk 누락)를
    /// 그대로 씁니다. 저장소 검증 실패도 [`SavedProjectError::Storage`]로 보고합니다.
    fn check_storage(
        &self,
        root: &Path,
        project_id: StableId,
    ) -> Result<GenerationManifest, SavedProjectError> {
        let load_error = |e: LoadError| SavedProjectError::Storage(LazyLoadError::Load(e));
        let mut guard = StorageGuard::new(root, self.links);
        guard.check_file(
            &storage_components(&current_pointer_path(Path::new(""))),
            self.limits.current_pointer_bytes,
        )?;
        let manifest_hash = read_current_pointer(root)
            .map_err(LoadError::CurrentPointer)
            .map_err(load_error)?;
        guard.check_file(
            &storage_components(&manifest_path(Path::new(""), &manifest_hash)),
            self.limits.manifest_bytes,
        )?;
        let loaded = verify_generation(root, &manifest_hash, project_id).map_err(load_error)?;
        for pointer in &loaded.manifest.chunk_pointers {
            let chunk = ChunkObjectPath::for_hash(&pointer.content_hash).to_relative_path();
            guard.check_file(&storage_components(&chunk), self.limits.chunk_bytes)?;
        }
        Ok(loaded.manifest)
    }
}

impl DiskProjectLoader for SavedProjectLoader {
    type Project = SavedProject;
    type Error = SavedProjectError;

    fn load_saved_project(&self, worktree_root: &Path) -> Result<SavedProject, SavedProjectError> {
        self.open(worktree_root)
    }
}

/// 저장소 상대 경로를 구성 요소 문자열 목록으로 바꿉니다(저장소 경로는 hash 기반 ASCII뿐).
fn storage_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// 저장소 경로 검사기. 같은 디렉터리 구성 요소는 한 번만 검사합니다.
struct StorageGuard<'a> {
    root: &'a Path,
    links: StorageLinkPolicy,
    checked_dirs: BTreeSet<String>,
}

impl<'a> StorageGuard<'a> {
    fn new(root: &'a Path, links: StorageLinkPolicy) -> StorageGuard<'a> {
        StorageGuard {
            root,
            links,
            checked_dirs: BTreeSet::new(),
        }
    }

    /// `components`(root 기준) 경로가 `limit` bytes 이하 정규 파일인지 검사합니다.
    ///
    /// 항목이 없으면 통과합니다(저장소가 누락을 typed 오류로 보고). [`StorageLinkPolicy::Reject`]는
    /// 디렉터리 구성 요소마다 symlink를 따르지 않고 디렉터리인지 검사합니다.
    fn check_file(&mut self, components: &[String], limit: u64) -> Result<(), SavedProjectError> {
        let Some((last, dirs)) = components.split_last() else {
            return Ok(());
        };
        let mut relative = String::new();
        let mut path = self.root.to_path_buf();
        for dir in dirs {
            push_component(&mut relative, &mut path, dir);
            if self.links == StorageLinkPolicy::Follow || self.checked_dirs.contains(&relative) {
                continue;
            }
            let Some(meta) = self.metadata(&path, &relative)? else {
                return Ok(());
            };
            if !meta.is_dir() {
                return Err(entry_error(&relative, StorageEntryProblem::NotDirectory));
            }
            self.checked_dirs.insert(relative.clone());
        }
        push_component(&mut relative, &mut path, last);
        let Some(meta) = self.metadata(&path, &relative)? else {
            return Ok(());
        };
        if !meta.is_file() {
            return Err(entry_error(&relative, StorageEntryProblem::NotFile));
        }
        if meta.len() > limit {
            return Err(entry_error(
                &relative,
                StorageEntryProblem::TooLarge {
                    len: meta.len(),
                    limit,
                },
            ));
        }
        Ok(())
    }

    /// 규칙에 맞는 metadata(Reject는 symlink를 따르지 않음). 없으면 `None`.
    fn metadata(
        &self,
        path: &Path,
        relative: &str,
    ) -> Result<Option<std::fs::Metadata>, SavedProjectError> {
        let result = match self.links {
            StorageLinkPolicy::Follow => std::fs::metadata(path),
            StorageLinkPolicy::Reject => std::fs::symlink_metadata(path),
        };
        match result {
            Ok(meta) if meta.file_type().is_symlink() => {
                Err(entry_error(relative, StorageEntryProblem::Symlink))
            }
            Ok(meta) => Ok(Some(meta)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(entry_error(
                relative,
                StorageEntryProblem::Io { kind: e.kind() },
            )),
        }
    }
}

fn push_component(relative: &mut String, path: &mut PathBuf, component: &str) {
    if !relative.is_empty() {
        relative.push('/');
    }
    relative.push_str(component);
    path.push(component);
}

fn entry_error(relative: &str, problem: StorageEntryProblem) -> SavedProjectError {
    SavedProjectError::StorageEntry {
        path: relative.to_string(),
        problem,
    }
}

/// `stellar.yaml`을 symlink를 따르지 않고 loader 상한까지만 읽어 검증합니다.
///
/// 정규 파일만 엽니다(FIFO·장치에서 멈추지 않음). 검사(lstat)와 열기 사이에 FIFO·장치·symlink로 바뀌어도
/// 열기가 멈추지 않도록 macOS에서는 `O_NONBLOCK | O_NOFOLLOW`로 엽니다([`open_manifest`]). 연 handle이
/// 검사한 파일과 다르면(검사 뒤 교체) 거부하고, 크기 확인 뒤 파일이 자라도 상한 + 1 바이트에서 읽기를
/// 멈춥니다.
fn read_manifest(root: &Path) -> Result<Manifest, SavedProjectError> {
    read_manifest_checked(root, |_| {})
}

/// [`read_manifest`] 본체. `after_check`는 lstat 검사와 열기 사이에 불립니다(테스트가 그 사이 교체를
/// 재현하는 지점이며, 실제 경로는 아무것도 하지 않음).
fn read_manifest_checked(
    root: &Path,
    after_check: impl FnOnce(&Path),
) -> Result<Manifest, SavedProjectError> {
    let read_error = |e: io::Error| SavedProjectError::ManifestRead { kind: e.kind() };
    let path = root.join(MANIFEST_FILE);
    let meta = std::fs::symlink_metadata(&path).map_err(read_error)?;
    if meta.file_type().is_symlink() {
        return Err(SavedProjectError::ManifestSymlink);
    }
    if !meta.is_file() {
        return Err(SavedProjectError::ManifestNotFile);
    }
    let limit = u64::try_from(MAX_INPUT_BYTES).unwrap_or(u64::MAX);
    if meta.len() > limit {
        return Err(manifest_too_large(Some(meta.len())));
    }
    after_check(&path);
    let file = match open_manifest(&path) {
        Ok(file) => file,
        // 검사 때 정규 파일이던 경로가 symlink로 바뀜.
        Err(e) if is_symlink_refusal(&e) => return Err(SavedProjectError::ManifestChanged),
        Err(e) => return Err(read_error(e)),
    };
    let opened = file.metadata().map_err(read_error)?;
    if !opened.is_file() || !same_file(&meta, &opened) {
        return Err(SavedProjectError::ManifestChanged);
    }
    let bytes = read_at_most(file, MAX_INPUT_BYTES)
        .map_err(read_error)?
        .ok_or_else(|| manifest_too_large(None))?;
    parse_manifest_bytes(bytes)
}

fn parse_manifest_bytes(bytes: Vec<u8>) -> Result<Manifest, SavedProjectError> {
    let text = String::from_utf8(bytes).map_err(|_| SavedProjectError::ManifestRead {
        kind: io::ErrorKind::InvalidData,
    })?;
    let value = load_restricted(&text).map_err(SavedProjectError::ManifestSyntax)?;
    Manifest::from_yaml(&value).map_err(SavedProjectError::ManifestSchema)
}

/// macOS `<sys/fcntl.h>`·`<sys/errno.h>` 값(SDK header로 확인).
#[cfg(target_os = "macos")]
mod open_flags {
    pub const O_NONBLOCK: i32 = 0x0000_0004;
    pub const O_NOFOLLOW: i32 = 0x0000_0100;
    pub const ELOOP: i32 = 62;
}

/// `stellar.yaml`을 읽기 전용으로 엽니다.
///
/// macOS에서는 `O_NONBLOCK`(FIFO·장치로 바뀌어도 writer·장치를 기다리지 않음, 정규 파일 읽기에는 영향
/// 없음)과 `O_NOFOLLOW`(마지막 구성 요소 symlink를 따르지 않음)를 줍니다. 다른 unix는 flag 값이
/// 플랫폼·아키텍처마다 달라 주지 않고, 열기 뒤 handle 검사만 합니다(모듈 문서의 남은 한계).
fn open_manifest(path: &Path) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(open_flags::O_NONBLOCK | open_flags::O_NOFOLLOW);
    }
    options.open(path)
}

/// [`open_manifest`]가 symlink 때문에 거부한 오류인지(macOS `ELOOP`).
fn is_symlink_refusal(e: &io::Error) -> bool {
    #[cfg(target_os = "macos")]
    {
        e.raw_os_error() == Some(open_flags::ELOOP)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = e;
        false
    }
}

/// 검사한 metadata와 연 handle의 metadata가 같은 파일인지(unix: device + inode).
#[cfg(unix)]
fn same_file(checked: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    checked.dev() == opened.dev() && checked.ino() == opened.ino()
}

/// 비-unix(Windows, ADR-0006 보류)는 파일 식별 비교를 하지 않습니다.
#[cfg(not(unix))]
fn same_file(_checked: &std::fs::Metadata, _opened: &std::fs::Metadata) -> bool {
    true
}

/// loader와 같은 `input-too-large` 오류.
fn manifest_too_large(declared: Option<u64>) -> SavedProjectError {
    let message = match declared {
        Some(bytes) => format!("입력이 {bytes} bytes로 상한 {MAX_INPUT_BYTES} bytes를 넘습니다"),
        None => format!("입력이 상한 {MAX_INPUT_BYTES} bytes를 넘습니다"),
    };
    SavedProjectError::ManifestSyntax(LoaderError {
        kind: LoaderErrorKind::InputTooLarge,
        line: 0,
        col: 0,
        message,
    })
}

/// 디스크 기준 WorkingTree snapshot을 실제 프로젝트 loader로 읽습니다(XIV.2.7).
///
/// 파일 목록은 Git이 아는 작업 트리 파일, 프로젝트는 [`SavedProjectLoader::new`]가 같은 root에서 연
/// 저장된 프로젝트입니다. 둘 다 디스크에 저장된 파일 기준이며 편집기 메모리 상태는 포함하지 않습니다.
pub fn read_working_tree_project(
    git: &GatedGitRunner,
    worktree_root: &Path,
) -> Result<WorkingTreeSnapshot<SavedProject, SavedProjectError>, GitOpError> {
    read_working_tree_snapshot(git, worktree_root, &SavedProjectLoader::new())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document_io::serialize::serialize_document;
    use crate::manifest::{ProjectMeta, CURRENT_SCHEMA_VERSION};
    use crate::yaml::write_canonical;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_document::model::DocumentModel;
    use stellar_document::test_support::random_document::random_valid_document;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_storage::generation::loader::{load_current_generation, LoadError};
    use stellar_storage::save::cancel::{run_save, CancelToken};
    use stellar_storage::save::commit::SavedGenerationState;
    use stellar_storage::save::plan::build_save_plan;

    pub(crate) const PROJECT_ID_TEXT: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

    pub(crate) struct TempDir {
        pub(crate) path: PathBuf,
    }
    impl TempDir {
        pub(crate) fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-saved-project-test-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    pub(crate) fn project_id() -> StableId {
        parse_project_id(PROJECT_ID_TEXT).unwrap()
    }

    /// 유효한 `stellar.yaml`을 씁니다.
    pub(crate) fn write_manifest(root: &Path, id: &str) {
        let manifest = Manifest {
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectMeta {
                id: id.to_string(),
                name: "Saved".to_string(),
            },
            target_profiles: Vec::new(),
            dependencies: Vec::new(),
        };
        std::fs::write(
            root.join(MANIFEST_FILE),
            write_canonical(&manifest.to_yaml()),
        )
        .unwrap();
    }

    /// 문서들을 실제 저장 파이프라인(V.5)으로 current generation 위에 저장합니다.
    pub(crate) fn save_documents(root: &Path, documents: &[&DocumentModel]) {
        let base = load_current_generation(root, project_id()).ok();
        let serialized: Vec<_> = documents
            .iter()
            .map(|model| serialize_document(model).unwrap())
            .collect();
        let states: Vec<_> = serialized
            .iter()
            .flat_map(|d| d.to_dirty_states())
            .collect();
        let roots = serialized.iter().map(|d| d.document_id).collect();
        let plan = build_save_plan(base.as_ref(), project_id(), 1, &states, roots).unwrap();
        let mut memory = SavedGenerationState::new(
            base.as_ref()
                .map(|b| (b.manifest.generation, b.manifest_hash)),
        );
        run_save(root, &plan, &mut memory, &CancelToken::new()).unwrap();
    }

    #[test]
    fn opens_manifest_and_current_generation_and_loads_documents() {
        let dir = TempDir::new();
        write_manifest(&dir.path, PROJECT_ID_TEXT);
        let first = random_valid_document(11);
        let second = random_valid_document(12);
        save_documents(&dir.path, &[&first, &second]);

        let project = SavedProjectLoader::new().open(&dir.path).unwrap();
        assert_eq!(project.project_id, project_id());
        assert_eq!(project.manifest.project.name, "Saved");
        assert_eq!(project.generation_number(), 1);
        let mut ids = project.document_ids().unwrap();
        ids.sort();
        let mut expected = vec![first.document_id(), second.document_id()];
        expected.sort();
        assert_eq!(ids, expected);
        for model in [&first, &second] {
            let loaded = project.load_document(model.document_id()).unwrap();
            assert_eq!(
                model_snapshot_json(&loaded.model).unwrap(),
                model_snapshot_json(model).unwrap()
            );
        }
        // DiskProjectLoader로도 같은 결과(.git 없이 열림).
        let via_trait = SavedProjectLoader::new()
            .load_saved_project(&dir.path)
            .unwrap();
        assert_eq!(via_trait.generation_number(), 1);
        // materialize된 snapshot 규칙(symlink 거부)으로도 정상 프로젝트는 그대로 열림.
        let via_trait = SavedProjectLoader::for_materialized_snapshot()
            .load_saved_project(&dir.path)
            .unwrap();
        assert_eq!(via_trait.generation_number(), 1);
    }

    #[test]
    fn invalid_projects_are_rejected_with_typed_errors() {
        let loader = SavedProjectLoader::new();
        let dir = TempDir::new();
        // 매니페스트 없음.
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::ManifestRead {
                kind: io::ErrorKind::NotFound
            })
        ));
        // 문법 오류(anchor 금지).
        std::fs::write(dir.path.join(MANIFEST_FILE), "schema_version: &a 1\n").unwrap();
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::ManifestSyntax(_))
        ));
        // 스키마 오류.
        std::fs::write(dir.path.join(MANIFEST_FILE), "schema_version: 1\n").unwrap();
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::ManifestSchema(_))
        ));
        // UUID가 아닌 project id.
        write_manifest(&dir.path, "not-a-uuid");
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::InvalidProjectId { id }) if id == "not-a-uuid"
        ));
        // 저장된 generation 없음(current pointer 없음).
        write_manifest(&dir.path, PROJECT_ID_TEXT);
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::Storage(LazyLoadError::Load(
                LoadError::CurrentPointer(_)
            )))
        ));
        // 매니페스트와 저장소의 project id가 다름.
        save_documents(&dir.path, &[&random_valid_document(3)]);
        write_manifest(&dir.path, "0195c530-3acd-7bc1-8414-000000000000");
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::Storage(LazyLoadError::Load(
                LoadError::WrongProjectId { .. }
            )))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_manifest_is_not_followed() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        write_manifest(&outside.path, PROJECT_ID_TEXT);
        save_documents(&dir.path, &[&random_valid_document(4)]);
        std::os::unix::fs::symlink(
            outside.path.join(MANIFEST_FILE),
            dir.path.join(MANIFEST_FILE),
        )
        .unwrap();
        assert!(matches!(
            SavedProjectLoader::new().open(&dir.path),
            Err(SavedProjectError::ManifestSymlink)
        ));
    }

    /// 저장된 정상 프로젝트 하나.
    fn saved_project_dir(seed: u64) -> TempDir {
        let dir = TempDir::new();
        write_manifest(&dir.path, PROJECT_ID_TEXT);
        save_documents(&dir.path, &[&random_valid_document(seed)]);
        dir
    }

    /// `design/objects` 아래 첫 shard 디렉터리 이름.
    fn first_shard(root: &Path) -> String {
        let mut names: Vec<String> = std::fs::read_dir(root.join("design").join("objects"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names.remove(0)
    }

    fn storage_problem(
        result: Result<SavedProject, SavedProjectError>,
    ) -> (String, StorageEntryProblem) {
        match result {
            Err(SavedProjectError::StorageEntry { path, problem }) => (path, problem),
            other => panic!("StorageEntry 오류가 아님: {other:?}"),
        }
    }

    /// `relative` 항목을 `outside`로 옮기고 그 자리에 절대 경로 symlink를 둡니다.
    #[cfg(unix)]
    fn replace_with_absolute_symlink(root: &Path, relative: &[&str], outside: &Path) {
        let mut path = root.to_path_buf();
        for component in relative {
            path.push(component);
        }
        let target = outside.join(relative.join("-"));
        std::fs::rename(&path, &target).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_loader_rejects_storage_symlinks_and_working_tree_follows_them() {
        let snapshot = SavedProjectLoader::for_materialized_snapshot();
        let working_tree = SavedProjectLoader::new();

        // current pointer가 snapshot 밖 절대 경로 symlink.
        let dir = saved_project_dir(21);
        let outside = TempDir::new();
        replace_with_absolute_symlink(&dir.path, &["design", "current"], &outside.path);
        assert_eq!(
            storage_problem(snapshot.open(&dir.path)),
            ("design/current".to_string(), StorageEntryProblem::Symlink)
        );
        assert_eq!(working_tree.open(&dir.path).unwrap().generation_number(), 1);

        // chunk shard 디렉터리가 symlink.
        let dir = saved_project_dir(22);
        let outside = TempDir::new();
        let shard = first_shard(&dir.path);
        replace_with_absolute_symlink(&dir.path, &["design", "objects", &shard], &outside.path);
        assert_eq!(
            storage_problem(snapshot.open(&dir.path)),
            (
                format!("design/objects/{shard}"),
                StorageEntryProblem::Symlink
            )
        );
        assert_eq!(working_tree.open(&dir.path).unwrap().generation_number(), 1);

        // design 디렉터리 자체가 symlink.
        let dir = saved_project_dir(23);
        let outside = TempDir::new();
        replace_with_absolute_symlink(&dir.path, &["design"], &outside.path);
        assert_eq!(
            storage_problem(snapshot.open(&dir.path)),
            ("design".to_string(), StorageEntryProblem::Symlink)
        );
        assert_eq!(working_tree.open(&dir.path).unwrap().generation_number(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn non_regular_storage_entries_are_rejected_before_reading() {
        // 장치 파일을 가리키는 pointer: 따라가도 정규 파일이 아니라 읽지 않음(무한 읽기 없음).
        let dir = saved_project_dir(24);
        let pointer = current_pointer_path(&dir.path);
        std::fs::remove_file(&pointer).unwrap();
        std::os::unix::fs::symlink("/dev/zero", &pointer).unwrap();
        assert_eq!(
            storage_problem(SavedProjectLoader::new().open(&dir.path)),
            ("design/current".to_string(), StorageEntryProblem::NotFile)
        );
        assert_eq!(
            storage_problem(SavedProjectLoader::for_materialized_snapshot().open(&dir.path)),
            ("design/current".to_string(), StorageEntryProblem::Symlink)
        );

        // pointer 자리에 디렉터리.
        std::fs::remove_file(&pointer).unwrap();
        std::fs::create_dir(&pointer).unwrap();
        for loader in [
            SavedProjectLoader::new(),
            SavedProjectLoader::for_materialized_snapshot(),
        ] {
            assert_eq!(
                storage_problem(loader.open(&dir.path)),
                ("design/current".to_string(), StorageEntryProblem::NotFile)
            );
        }

        // manifest 디렉터리 자리에 정규 파일(snapshot 규칙은 디렉터리 구성 요소도 읽기 전에 검사).
        let dir = saved_project_dir(25);
        let generations = dir.path.join("design").join("generations");
        std::fs::remove_dir_all(&generations).unwrap();
        std::fs::write(&generations, b"not a directory").unwrap();
        assert_eq!(
            storage_problem(SavedProjectLoader::for_materialized_snapshot().open(&dir.path)),
            (
                "design/generations".to_string(),
                StorageEntryProblem::NotDirectory
            )
        );

        // chunk shard 자리에 정규 파일: 저장소의 chunk 존재 확인(읽기 없음)이 먼저 누락으로 보고.
        let dir = saved_project_dir(27);
        let shard = first_shard(&dir.path);
        let shard_path = dir.path.join("design").join("objects").join(&shard);
        std::fs::remove_dir_all(&shard_path).unwrap();
        std::fs::write(&shard_path, b"not a directory").unwrap();
        assert!(matches!(
            SavedProjectLoader::for_materialized_snapshot().open(&dir.path),
            Err(SavedProjectError::Storage(LazyLoadError::Load(
                LoadError::MissingChunk { .. }
            )))
        ));
    }

    #[test]
    fn oversized_storage_files_are_rejected_before_reading() {
        let dir = saved_project_dir(26);
        let limits = StorageReadLimits::default();
        // pointer 상한(형식상 72 bytes).
        let loader = SavedProjectLoader::new().with_limits(StorageReadLimits {
            current_pointer_bytes: 71,
            ..limits
        });
        assert_eq!(
            storage_problem(loader.open(&dir.path)),
            (
                "design/current".to_string(),
                StorageEntryProblem::TooLarge { len: 72, limit: 71 }
            )
        );
        // manifest 상한.
        let loader =
            SavedProjectLoader::for_materialized_snapshot().with_limits(StorageReadLimits {
                manifest_bytes: 8,
                ..limits
            });
        let (path, problem) = storage_problem(loader.open(&dir.path));
        assert!(path.starts_with("design/generations/"), "{path}");
        assert!(matches!(
            problem,
            StorageEntryProblem::TooLarge { limit: 8, .. }
        ));
        // chunk 상한.
        let loader = SavedProjectLoader::new().with_limits(StorageReadLimits {
            chunk_bytes: 8,
            ..limits
        });
        let (path, problem) = storage_problem(loader.open(&dir.path));
        assert!(path.starts_with("design/objects/"), "{path}");
        assert!(matches!(
            problem,
            StorageEntryProblem::TooLarge { limit: 8, .. }
        ));
        // 기본 상한은 정상 프로젝트를 그대로 엶.
        assert_eq!(
            SavedProjectLoader::for_materialized_snapshot()
                .with_limits(limits)
                .open(&dir.path)
                .unwrap()
                .generation_number(),
            1
        );
    }

    #[test]
    fn manifest_must_be_a_bounded_regular_file() {
        use std::io::{Seek, SeekFrom, Write};
        let loader = SavedProjectLoader::for_materialized_snapshot();
        // 디렉터리인 stellar.yaml.
        let dir = TempDir::new();
        std::fs::create_dir(dir.path.join(MANIFEST_FILE)).unwrap();
        assert!(matches!(
            loader.open(&dir.path),
            Err(SavedProjectError::ManifestNotFile)
        ));
        // 상한의 64배 sparse 파일: 본문을 읽지 않고 input-too-large(마지막 바이트는 비-UTF-8).
        let dir = TempDir::new();
        let size = u64::try_from(MAX_INPUT_BYTES).unwrap() * 64;
        let mut file = std::fs::File::create(dir.path.join(MANIFEST_FILE)).unwrap();
        file.set_len(size).unwrap();
        file.seek(SeekFrom::Start(size - 1)).unwrap();
        file.write_all(&[0xFF]).unwrap();
        drop(file);
        match loader.open(&dir.path) {
            Err(SavedProjectError::ManifestSyntax(error)) => {
                assert_eq!(error.kind, LoaderErrorKind::InputTooLarge, "{error}");
            }
            other => panic!("input-too-large가 아님: {other:?}"),
        }
    }

    /// `mkfifo`로 FIFO를 만듭니다. 도구가 없으면 `false`.
    #[cfg(unix)]
    fn make_fifo(path: &Path) -> bool {
        std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .is_ok_and(|status| status.success())
    }

    #[cfg(unix)]
    #[test]
    fn manifest_swapped_for_a_fifo_after_check_does_not_block_open() {
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = TempDir::new();
        write_manifest(&dir.path, PROJECT_ID_TEXT);
        let root = dir.path.clone();
        let (sender, receiver) = mpsc::channel();
        let fifo_ready = mpsc::channel::<bool>();
        let worker = std::thread::spawn({
            let fifo_ready = fifo_ready.0;
            move || {
                // lstat 검사(정규 파일) 뒤, 열기 전에 같은 경로를 FIFO로 바꿉니다.
                let result = read_manifest_checked(&root, |path| {
                    std::fs::remove_file(path).unwrap();
                    fifo_ready.send(make_fifo(path)).unwrap();
                });
                sender.send(result.err()).unwrap();
            }
        });
        if !fifo_ready.1.recv().unwrap() {
            return; // mkfifo 없는 환경 건너뜀.
        }
        let outcome = receiver.recv_timeout(Duration::from_secs(5));
        if outcome.is_err() {
            // 열기가 writer를 기다리며 멈춤: writer 쪽을 열어 worker를 풀어 준 뒤 실패로 보고합니다.
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .open(dir.path.join(MANIFEST_FILE));
            let _ = worker.join();
            panic!("검사 뒤 FIFO로 바뀐 stellar.yaml 열기가 멈췄습니다");
        }
        worker.join().unwrap();
        assert!(
            matches!(outcome, Ok(Some(SavedProjectError::ManifestChanged))),
            "{outcome:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn manifest_swapped_for_a_symlink_after_check_is_rejected() {
        let dir = TempDir::new();
        write_manifest(&dir.path, PROJECT_ID_TEXT);
        let outside = TempDir::new();
        write_manifest(&outside.path, PROJECT_ID_TEXT);
        let target = outside.path.join(MANIFEST_FILE);
        let result = read_manifest_checked(&dir.path, |path| {
            std::fs::remove_file(path).unwrap();
            std::os::unix::fs::symlink(&target, path).unwrap();
        });
        assert!(
            matches!(result, Err(SavedProjectError::ManifestChanged)),
            "{result:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fifo_manifest_is_rejected_without_blocking() {
        let dir = TempDir::new();
        let path = dir.path.join(MANIFEST_FILE);
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .is_ok_and(|status| status.success());
        if !made {
            return; // mkfifo 없는 환경 건너뜀.
        }
        assert!(matches!(
            SavedProjectLoader::new().open(&dir.path),
            Err(SavedProjectError::ManifestNotFile)
        ));
    }
}
