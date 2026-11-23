//! 원본 없는 옛 chunk 프로젝트를 텍스트 원본으로 바꾸기 (V.10.7, ADR-0131 결정 3·4).
//!
//! ADR-0122 이전 프로젝트는 chunk generation을 `design/current`·`design/generations/`·`design/objects/`에 두고 Git이
//! 추적했습니다. 텍스트 원본(`design/**/*.stellar.yaml`)이 없는 이런 프로젝트는 옛 배치가 유일한 Git 원본입니다.
//! [`convert_legacy`]는 옛 배치를 읽기 전용 비공개 사본(옛 파일만 복사해 그 안에서 cache 위치로 옮김)에서 열어
//! generation 전체를 텍스트 원본으로 옮기고, 같은 의미임을 확인한 뒤에만 작업 트리에 씁니다.
//!
//! 순서와 각 단계가 실패했을 때:
//!
//! 1. 비공개 사본에서 generation을 열고 root 문서 전부·component library(Property·Slot·상태 머신 포함)·이미지 record를
//!    텍스트 원본 의미로 옮깁니다(`source_v4::project::from_saved_documents`: chunk pointer 전체 소비). 옮길 수 없으면
//!    [`CacheError::LegacyUnconvertible`]이고 작업 트리는 그대로입니다.
//! 2. 인코딩한 파일 집합을 다시 읽은 의미(첫 문서·프로젝트 의미·추가 root 문서)가 generation과 같아야 합니다.
//! 3. 변환 표시를 먼저 쓰고, 원본을 저장 journal 한 거래로 씁니다(기대 hash = 디스크의 기존 컴포넌트 원본, 없는 파일은
//!    없음). 다른 writer가 먼저 썼으면 journal이 거부하고 아무것도 쓰지 않습니다(원본이 없으면 표시도 거둠). 표시가 원본보다
//!    늦으면 그 사이 다른 관찰이 옛 배치를 작업 트리에서 옮길 수 있습니다.
//! 4. 옛 배치를 cache에 **복사**하고(옛 파일은 작업 트리에 둠) 원본·cache 짝(stamp)을 씁니다. 옛 배치는 변환한
//!    원본이 Git index에 들어갈 때(`cache::untrack_legacy`) 작업 트리에서 옮겨집니다. 그 전에 `git commit -a`를 해도
//!    추적하지 않는 새 원본 대신 옛 배치가 Git에 남으므로 Git 원본이 사라지지 않습니다.
//!
//! 앱 창 열기·CLI·MCP의 관찰(`document_io::session::observe`)과 `stellar cache migrate --convert`가 같은 함수를
//! 씁니다(`cache::migrate_working_tree_with`).

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use stellar_storage::generation::current::read_current_pointer;
use stellar_storage::generation::lazy::{LazyGeneration, NoopRecorder};
use stellar_storage::hash::ContentHash;
use stellar_storage::location::{
    transfer_legacy_layout, LegacyPointer, LegacyTransfer, CURRENT_FILE, LEGACY_DIR, LEGACY_PATHS,
    LOCAL_CACHE_DIR,
};

use crate::cache::{migrate_layout, project_id, CacheError};
use crate::document_io::component_source::read_project_library;
use crate::document_io::load::load_document_from_generation;
use crate::document_io::session::{
    commit_files, decode_extra_documents, decode_source, read_source, read_source_dir, write_stamp,
    SourceBase, SourceCommitError, SourceFiles, Stamp, DESIGN_DIR,
};
use crate::document_io::source_v4::documents;
use crate::document_io::source_v4::project::{encode, from_saved_documents};
use crate::path_resolver::ExternalAccess;
use crate::private_temp::PrivateTempDir;

/// 변환 표시 파일(로컬 cache 폴더 안, Git 밖).
pub const MARKER_FILE: &str = "legacy-converted";

/// 변환 표시 형식 첫 줄.
const MARKER_MAGIC: &str = "stellar-legacy-converted 1";

/// [`convert_legacy`] 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyConversion {
    /// 옮긴 옛 generation의 manifest hash(cache current).
    pub manifest: ContentHash,
    /// 옮긴 root 문서 수.
    pub documents: usize,
    /// 옮긴 컴포넌트 정의 수.
    pub components: usize,
    /// 쓴 원본 파일 수.
    pub written: usize,
}

fn marker_path(root: &Path) -> PathBuf {
    LOCAL_CACHE_DIR
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part))
        .join(MARKER_FILE)
}

/// 작업 트리 옛 pointer 파일(`design/current`) bytes의 hash. 없거나 정규 파일이 아니면 `None`입니다.
pub fn legacy_pointer_digest(root: &Path) -> Option<ContentHash> {
    let path = root.join(LEGACY_DIR).join(CURRENT_FILE);
    stellar_storage::source_file::read(&path, 1024)
        .ok()
        .map(|bytes| ContentHash::compute(&bytes))
}

/// 변환 표시: 변환했지만 원본이 아직 Git index에 들어가지 않았음을 뜻합니다. 값은 변환 때 옛 pointer bytes의 hash입니다.
/// 없거나 형식이 다르면 `None`입니다.
pub fn converted_marker(root: &Path) -> Option<ContentHash> {
    let bytes = stellar_storage::source_file::read(&marker_path(root), 512).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let mut lines = text.lines();
    if lines.next()? != MARKER_MAGIC {
        return None;
    }
    let pointer = lines.next()?.strip_prefix("pointer ")?.parse().ok()?;
    lines.next().is_none().then_some(pointer)
}

fn write_marker(root: &Path, pointer: ContentHash) -> io::Result<()> {
    stellar_storage::location::ensure_cache_ignore(root)?;
    stellar_storage::atomic_file::write_file_atomic(
        &marker_path(root),
        format!("{MARKER_MAGIC}\npointer {pointer}\n").as_bytes(),
    )
}

/// 변환 표시를 지웁니다. 없으면 아무것도 하지 않습니다.
pub fn clear_converted_marker(root: &Path) -> io::Result<()> {
    match std::fs::remove_file(marker_path(root)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// 옛 배치 파일만 `parent` 아래 비공개 임시 폴더로 복사하고 그 안에서 cache 위치로 옮깁니다. 원래 폴더는 읽기만
/// 합니다(symlink는 따르지 않음).
pub(crate) fn legacy_copy(root: &Path, parent: &Path) -> Result<PrivateTempDir, CacheError> {
    let temp = PrivateTempDir::create_in(parent, "stellar-legacy-view")
        .map_err(|error| CacheError::Write(error.to_string()))?;
    fn copy(from: &Path, to: &Path) -> io::Result<()> {
        let meta = std::fs::symlink_metadata(from)?;
        if meta.is_dir() {
            std::fs::create_dir_all(to)?;
            for entry in std::fs::read_dir(from)? {
                let entry = entry?;
                copy(&entry.path(), &to.join(entry.file_name()))?;
            }
        } else if meta.is_file() {
            std::fs::copy(from, to)?;
        }
        Ok(())
    }
    for relative in LEGACY_PATHS {
        let from = root.join(relative);
        if std::fs::symlink_metadata(&from).is_ok() {
            let to = temp.path().join(relative);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| CacheError::Write(e.to_string()))?;
            }
            copy(&from, &to).map_err(|e| CacheError::Write(e.to_string()))?;
        }
    }
    migrate_layout(temp.path())?;
    Ok(temp)
}

fn unconvertible(reason: impl std::fmt::Display) -> CacheError {
    CacheError::LegacyUnconvertible(reason.to_string())
}

/// 원본 없는 옛 chunk 프로젝트(작업 트리 `root`)를 텍스트 원본으로 바꿉니다(모듈 문서). 작업 트리에 텍스트 원본이
/// 이미 있거나 옛 pointer(`design/current`)가 없으면 거부합니다. 성공하면 원본·cache·stamp·변환 표시를 쓰고 옛 배치는
/// 그대로 둡니다. 실패하면 원본 쓰기 전에 멈추므로 작업 트리가 바뀌지 않습니다(원본 쓰기 뒤 cache 복사가 실패하면
/// 원본은 남고 다음 열기가 원본에서 cache를 다시 만듭니다).
pub fn convert_legacy(root: &Path) -> Result<LegacyConversion, CacheError> {
    if read_source(root)
        .map_err(|e| CacheError::Source(e.to_string()))?
        .is_some()
    {
        return Err(unconvertible("이미 텍스트 원본이 있습니다"));
    }
    let Some(pointer_digest) = legacy_pointer_digest(root) else {
        return Err(CacheError::LegacyWithoutSource);
    };
    let project = project_id(root)?;
    let view = legacy_copy(root, &std::env::temp_dir())?;
    let lazy =
        LazyGeneration::open(view.path(), project, &mut NoopRecorder).map_err(unconvertible)?;
    let schema = lazy.manifest().schema_version;
    if schema != crate::migration::PRODUCT_SCHEMA_VERSION {
        return Err(unconvertible(format!(
            "schema {schema} generation은 먼저 `stellar migrate`로 올려야 합니다"
        )));
    }
    let manifest = read_current_pointer(view.path()).map_err(unconvertible)?;
    let mut models = Vec::new();
    for id in lazy.manifest().root_document_ids.clone() {
        models.push(
            load_document_from_generation(&lazy, id, &mut NoopRecorder)
                .map_err(unconvertible)?
                .model,
        );
    }
    if models.is_empty() {
        return Err(unconvertible("root 문서가 없는 generation입니다"));
    }
    let (yaml, _) =
        read_project_library(root, ExternalAccess::MetadataOnly).map_err(unconvertible)?;
    let (text, extras) = from_saved_documents(&lazy, models, yaml).map_err(unconvertible)?;
    let primary = text.document.document_id();
    let mut files = encode(&text).map_err(unconvertible)?.files;
    let extra_files = documents::encode(primary, &extras).map_err(unconvertible)?;
    files.extend(extra_files);

    // 쓰기 전에 파일 집합 전체를 열기와 같은 함수로 다시 읽어 의미를 비교합니다.
    let check = SourceFiles {
        base: SourceBase::of(&files),
        files: files.clone(),
    };
    let decoded = decode_source(&check).map_err(unconvertible)?;
    if decoded.semantic_key().map_err(unconvertible)?
        != text.semantic_key().map_err(unconvertible)?
    {
        return Err(unconvertible(
            "다시 읽은 텍스트 원본의 의미가 generation과 다릅니다",
        ));
    }
    let again = decode_extra_documents(&check, primary).map_err(unconvertible)?;
    let same = again.len() == extras.len()
        && again.iter().zip(&extras).all(|(a, b)| {
            matches!((documents::semantic(a), documents::semantic(b)), (Ok(a), Ok(b)) if a == b)
        });
    if !same {
        return Err(unconvertible(
            "다시 읽은 추가 root 문서의 의미가 generation과 다릅니다",
        ));
    }

    // 기존 `design/` 원본(컴포넌트 원본)은 모두 변환 결과의 경로여야 합니다(그 파일을 기대 hash로 바꿔 씀).
    let design = root.join(DESIGN_DIR);
    let existing: BTreeMap<String, String> = match std::fs::symlink_metadata(&design) {
        Ok(_) => read_source_dir(&design).map_err(|e| CacheError::Source(e.to_string()))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
        Err(error) => return Err(CacheError::Source(error.to_string())),
    };
    if let Some(unknown) = existing.keys().find(|path| !files.contains_key(*path)) {
        return Err(unconvertible(format!(
            "텍스트 원본이 모르는 파일이 design/ 아래에 있습니다: {unknown}"
        )));
    }
    // 변환 표시는 원본보다 먼저 씁니다. 원본이 생긴 뒤 표시가 없는 순간이 있으면, 그 사이 다른 관찰(동시에 연 다른
    // 프로세스)이나 중단 뒤의 다음 열기가 "원본이 있는 작업 트리"로 보고 옛 배치를 작업 트리에서 옮깁니다. 그러면 아직
    // 추적하지 않는 새 원본 대신 옛 배치 삭제만 `git commit -a`에 담겨 Git 원본이 끊깁니다(ADR-0131 결정 4). 원본 없이
    // 표시만 남은 상태는 해가 없습니다(원본이 없으면 관찰이 다시 변환하고, index 정리는 원본이 없으면 아무것도 안 함).
    write_marker(root, pointer_digest).map_err(|e| CacheError::Write(e.to_string()))?;
    let committed = match commit_files(root, &SourceBase::of(&existing), &files) {
        Ok(commit) => Ok(commit.written),
        Err(SourceCommitError::Stale { paths }) => Err(CacheError::Write(format!(
            "변환하는 사이 다른 writer가 원본을 바꿨습니다: {}",
            paths.join(", ")
        ))),
        Err(error) => Err(CacheError::Write(error.to_string())),
    };
    let written = match committed {
        Ok(written) => written,
        Err(error) => {
            // 원본을 쓰지 못했으면 표시를 거둡니다. 다른 writer(동시 변환)가 원본을 썼으면 그 표시이므로 둡니다.
            if matches!(read_source(root), Ok(None)) {
                let _ = clear_converted_marker(root);
            }
            return Err(error);
        }
    };

    transfer_legacy_layout(root, LegacyTransfer::Copy, LegacyPointer::Replace)
        .map_err(CacheError::Legacy)?;
    let source = read_source(root)
        .map_err(|e| CacheError::Source(e.to_string()))?
        .ok_or_else(|| CacheError::Write("방금 쓴 원본이 없습니다".into()))?;
    write_stamp(
        root,
        &Stamp {
            source: source.base.digest(),
            manifest,
        },
    )
    .map_err(|e| CacheError::Write(e.to_string()))?;
    Ok(LegacyConversion {
        manifest,
        documents: 1 + extras.len(),
        components: text.components.len(),
        written,
    })
}
