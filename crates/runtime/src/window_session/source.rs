//! 창 session의 디스크 읽기: 텍스트 원본(기본)과 chunk cache(V.10.7, ADR-0088).
//!
//! [`read_disk_state`]가 열기·다시 불러오기·외부 변경 반영·UseCurrentWindow의 유일한 디스크 읽기입니다. stamp
//! 짝으로 계획([`OpenPlan`])을 정하고 그 계획대로 작업 문서와 base(generation·원본)를 만듭니다. 쓰기는 하지 않으며,
//! 계획이 요구하는 맞춤 쓰기(원본 전환·cache 다시 만들기)는 호출자가 session 저장 경로로 합니다
//! ([`super::ProjectWindowSession::reconcile`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use stellar_document::model::DocumentModel;
use stellar_project::document_io::component_source::read_project_library;
use stellar_project::document_io::load::LoadDocumentError;
use stellar_project::document_io::session::{
    active_style_defaults, decode_source, read_source, read_source_dir, read_stamp, OpenPlan,
    SourceBase, SourceFiles, SourceLibrary, DESIGN_DIR,
};
use stellar_project::document_io::source_v4::project::{decode, encode, from_saved};
use stellar_project::document_io::text_source::TextSourceError;
use stellar_project::external_change::source::diagnose;
use stellar_project::identity::ProjectIdentity;
use stellar_project::path_resolver::ExternalAccess;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::KIND_COMPONENT_LIBRARY;
use stellar_storage::generation::current::{read_current_pointer, CurrentPointerError};
use stellar_storage::generation::lazy::{LazyGeneration, LazyLoadError, NoopRecorder};
use stellar_storage::generation::loader::{load_current_generation, LoadError, LoadedGeneration};
use stellar_storage::hash::ContentHash;

use super::{read_disk_document, DocumentPick, WindowSessionLoadError};

/// 창 session의 원본 상태.
#[derive(Debug, Clone)]
pub(crate) enum SourceMode {
    /// 텍스트 원본이 Git 원본입니다(기본).
    Text {
        /// 마지막으로 읽거나 쓴 원본 파일 hash.
        base: SourceBase,
        /// 원본의 문서 밖 의미. cache로 빠르게 연 경우 처음 저장할 때 writer가 원본에서 읽습니다(`None`).
        library: Option<Arc<SourceLibrary>>,
    },
    /// 텍스트 원본으로 검증된 변환이 없어 chunk generation이 원본으로 남습니다(ADR-0088 결정 2). 이유는 진단용입니다.
    ChunkOnly {
        /// 변환하지 못한 이유.
        reason: String,
    },
}

impl SourceMode {
    /// 텍스트 원본 base(chunk 전용이면 `None`).
    pub(crate) fn base(&self) -> Option<&SourceBase> {
        match self {
            SourceMode::Text { base, .. } => Some(base),
            SourceMode::ChunkOnly { .. } => None,
        }
    }
}

/// 디스크 읽기 결과(아무것도 쓰지 않음).
#[derive(Debug)]
pub(crate) struct DiskState {
    /// 고른 계획.
    pub(crate) plan: OpenPlan,
    /// current generation(cache). 없으면 `None`.
    pub(crate) loaded: Option<LoadedGeneration>,
    /// 작업 문서가 current generation에서 차지한 chunk.
    pub(crate) owned: BTreeSet<ChunkId>,
    /// 작업 문서. 고를 문서가 없으면 `None`.
    pub(crate) model: Option<DocumentModel>,
    /// 원본 상태.
    pub(crate) source: SourceMode,
    /// `stellar.yaml`의 활성 기본값 세트 이름(읽지 못하면 내장 기본 세트).
    pub(crate) style_defaults: String,
}

impl DiskState {
    /// 계획이 열기 뒤 맞춤 쓰기를 요구하는지(원본 전환·cache 다시 만들기).
    pub(crate) fn needs_reconcile(&self) -> bool {
        matches!(self.source, SourceMode::Text { .. })
            && matches!(
                self.plan,
                OpenPlan::Forward | OpenPlan::FromText | OpenPlan::Migrate
            )
    }
}

fn source_error(files: Option<&SourceFiles>, error: &TextSourceError) -> WindowSessionLoadError {
    let empty = BTreeMap::new();
    let files = files.map_or(&empty, |files| &files.files);
    let all: BTreeSet<String> = files.keys().cloned().collect();
    WindowSessionLoadError::Source(diagnose(files, &all, error))
}

fn pointer_error(error: CurrentPointerError) -> WindowSessionLoadError {
    WindowSessionLoadError::Generation(LazyLoadError::Load(LoadError::CurrentPointer(error)))
}

/// 디스크를 읽어 `pick` 문서의 상태를 만듭니다(모듈 문서). `chunk_only`이면 원본을 보지 않고 generation만 읽습니다
/// (chunk 전용 session의 다시 읽기).
pub(crate) fn read_disk_state(
    identity: &ProjectIdentity,
    pick: DocumentPick,
    chunk_only: Option<&str>,
) -> Result<DiskState, WindowSessionLoadError> {
    let root = identity.canonical_target();
    let style_defaults = active_style_defaults(root)
        .unwrap_or_else(|_| stellar_document::css::style_defaults::DEFAULT_SET.to_owned());
    if let Some(reason) = chunk_only {
        let disk = read_disk_document(identity, pick)?;
        return Ok(from_generation(
            OpenPlan::Cache,
            disk,
            SourceMode::ChunkOnly {
                reason: reason.to_owned(),
            },
            style_defaults,
        ));
    }
    let text = read_source(root).map_err(|error| source_error(None, &error))?;
    let current = match read_current_pointer(root) {
        Ok(current) => Some(current),
        Err(CurrentPointerError::Missing { .. }) => None,
        Err(error) => return Err(pointer_error(error)),
    };
    let plan = OpenPlan::decide(
        text.as_ref().map(|text| text.base.digest()),
        current,
        read_stamp(root),
    );
    match plan {
        OpenPlan::Empty => Ok(DiskState {
            plan,
            loaded: None,
            owned: BTreeSet::new(),
            model: None,
            source: SourceMode::Text {
                base: SourceBase::default(),
                library: Some(Arc::default()),
            },
            style_defaults,
        }),
        OpenPlan::Cache => {
            let disk = read_disk_document(identity, pick)?;
            let base = text.map(|text| text.base).unwrap_or_default();
            Ok(from_generation(
                plan,
                disk,
                SourceMode::Text {
                    base,
                    library: None,
                },
                style_defaults,
            ))
        }
        OpenPlan::Forward | OpenPlan::Migrate => {
            let disk = read_disk_document(identity, pick)?;
            let source = match disk.as_ref().and_then(|disk| disk.document.as_ref()) {
                Some((model, _)) => match convert(identity, model, text.as_ref()) {
                    Ok((base, library)) => SourceMode::Text {
                        base,
                        library: Some(Arc::new(library)),
                    },
                    Err(reason) => SourceMode::ChunkOnly { reason },
                },
                // generation에 고를 문서가 없으면(문서 없는 generation) 원본을 처음 저장할 때 만듭니다.
                None => SourceMode::Text {
                    base: text.map(|text| text.base).unwrap_or_default(),
                    library: None,
                },
            };
            Ok(from_generation(plan, disk, source, style_defaults))
        }
        OpenPlan::FromText => {
            let text = text.expect("FromText는 원본이 있을 때만");
            let project =
                decode_source(&text).map_err(|error| source_error(Some(&text), &error))?;
            let (model, library) = SourceLibrary::split(project);
            let document_id = model.document_id();
            if let DocumentPick::Document(expected) = pick {
                if expected != document_id {
                    return Err(WindowSessionLoadError::Document(
                        LoadDocumentError::UnknownDocument { document: expected },
                    ));
                }
            }
            let (loaded, owned) = if current.is_some() {
                match read_disk_document(identity, DocumentPick::Document(document_id)) {
                    Ok(Some(disk)) => (
                        Some(disk.loaded),
                        disk.document.map(|(_, owned)| owned).unwrap_or_default(),
                    ),
                    Ok(None) => (None, BTreeSet::new()),
                    Err(WindowSessionLoadError::Document(LoadDocumentError::UnknownDocument {
                        ..
                    })) => (
                        Some(
                            load_current_generation(root, identity.project_id()).map_err(
                                |error| {
                                    WindowSessionLoadError::Generation(LazyLoadError::Load(error))
                                },
                            )?,
                        ),
                        BTreeSet::new(),
                    ),
                    Err(error) => return Err(error),
                }
            } else {
                (None, BTreeSet::new())
            };
            Ok(DiskState {
                plan,
                loaded,
                owned,
                model: Some(model),
                source: SourceMode::Text {
                    base: text.base,
                    library: Some(Arc::new(library)),
                },
                style_defaults,
            })
        }
    }
}

fn from_generation(
    plan: OpenPlan,
    disk: Option<super::DiskDocument>,
    source: SourceMode,
    style_defaults: String,
) -> DiskState {
    let (loaded, document) = match disk {
        Some(disk) => (Some(disk.loaded), disk.document),
        None => (None, None),
    };
    let (model, owned) = match document {
        Some((model, owned)) => (Some(model), owned),
        None => (None, BTreeSet::new()),
    };
    DiskState {
        plan,
        loaded,
        owned,
        model,
        source,
        style_defaults,
    }
}

/// generation 문서를 텍스트 원본 의미로 옮길 수 있는지 검증하고 원본 base와 문서 밖 의미를 돌려줍니다(ADR-0088
/// 결정 2). generation의 모든 chunk를 원본이 소비하고(문서 하나, component library chunk 없음), 인코딩한 원본을 다시
/// 읽은 의미가 같아야 합니다. 원본이 이미 있으면(Forward) token set은 원본 것을 씁니다. 원본이 없으면(Migrate)
/// `design/` 아래 기존 `.stellar.yaml`(컴포넌트 원본)이 모두 인코딩 결과의 경로여야 합니다. 실패는 이유 문장입니다.
fn convert(
    identity: &ProjectIdentity,
    model: &DocumentModel,
    text: Option<&SourceFiles>,
) -> Result<(SourceBase, SourceLibrary), String> {
    let root = identity.canonical_target();
    let lazy = LazyGeneration::open(root, identity.project_id(), &mut NoopRecorder)
        .map_err(|error| error.to_string())?;
    if lazy
        .manifest()
        .chunk_pointers
        .iter()
        .any(|pointer| pointer.kind == KIND_COMPONENT_LIBRARY)
    {
        return Err("component library chunk는 텍스트 원본으로 옮기지 않습니다".into());
    }
    let (yaml, _) = read_project_library(root, ExternalAccess::MetadataOnly)
        .map_err(|error| error.to_string())?;
    let records = model.to_records().map_err(|error| error.to_string())?;
    let (mut copy, _) = DocumentModel::from_records(records).map_err(|error| error.to_string())?;
    // Context는 레코드 밖에 있어 함께 옮깁니다(ADR-0099).
    copy.replace_context(model.context().live(model));
    let mut project = from_saved(&lazy, copy, yaml).map_err(|error| error.to_string())?;
    if let Some(text) = text {
        project.token_sets = decode_source(text)
            .map_err(|error| error.to_string())?
            .token_sets;
    }
    let encoded = encode(&project).map_err(|error| error.to_string())?;
    let decoded = decode(&encoded.files).map_err(|error| error.to_string())?;
    if decoded.semantic_key().map_err(|error| error.to_string())?
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
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
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

/// 디스크 원본 관찰값(stale 판정).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceMark {
    /// 원본 파일 집합 digest(원본이 없으면 빈 집합의 digest).
    Digest(ContentHash),
    /// 원본을 읽지 못했습니다(I/O·symlink 등). 외부 writer로 봅니다.
    Unreadable,
}

/// 디스크 원본의 문서 파일이 `base`와 다르면 그 관찰값을 돌려줍니다. 문서 밖 의미 파일(컴포넌트·token)만 바뀌었으면
/// `None`입니다(그 변화는 [`library_moved`]가 받음, ADR-0088 결정 5).
pub(crate) fn document_source_moved(
    identity: &ProjectIdentity,
    base: &SourceBase,
) -> Option<(SourceMark, SourceBase)> {
    source_moved(identity, base).filter(|(mark, now)| {
        !(matches!(mark, SourceMark::Digest(_)) && base.differs_only_in_library(now))
    })
}

/// 디스크 원본이 문서 밖 의미 파일에서만 `base`와 다르면 지금 원본을 돌려줍니다.
pub(crate) fn library_moved(identity: &ProjectIdentity, base: &SourceBase) -> Option<SourceFiles> {
    let files = read_source(identity.canonical_target()).ok().flatten()?;
    base.differs_only_in_library(&files.base).then_some(files)
}

/// 디스크 원본이 `base`와 다르면 그 관찰값을 돌려줍니다.
pub(crate) fn source_moved(
    identity: &ProjectIdentity,
    base: &SourceBase,
) -> Option<(SourceMark, SourceBase)> {
    match read_source(identity.canonical_target()) {
        Ok(text) => {
            let now = text.map(|text| text.base).unwrap_or_default();
            (now != *base).then(|| (SourceMark::Digest(now.digest()), now))
        }
        Err(_) => Some((SourceMark::Unreadable, SourceBase::default())),
    }
}
