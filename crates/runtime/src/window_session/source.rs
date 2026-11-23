//! 창 session의 디스크 읽기: 텍스트 원본(기본)과 chunk cache(V.10.7, ADR-0088).
//!
//! [`read_disk_state`]가 열기·다시 불러오기·외부 변경 반영·UseCurrentWindow의 유일한 디스크 읽기입니다. stamp
//! 짝으로 계획([`OpenPlan`])을 정하고 그 계획대로 작업 문서와 base(generation·원본)를 만듭니다. 쓰기는 하지 않으며,
//! 계획이 요구하는 맞춤 쓰기(원본 전환·cache 다시 만들기)는 호출자가 session 저장 경로로 합니다
//! ([`super::ProjectWindowSession::reconcile`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use stellar_document::model::DocumentModel;
use stellar_project::document_io::load::LoadDocumentError;
use stellar_project::document_io::session::{
    active_style_defaults, convert_saved, decode_source, observe, read_source, DiskSource,
    ObserveError, OpenPlan, SourceBase, SourceFiles, SourceLibrary,
};
use stellar_project::document_io::text_source::TextSourceError;
use stellar_project::external_change::source::diagnose;
use stellar_project::identity::ProjectIdentity;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::generation::current::CurrentPointerError;
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

    /// 원본의 문서 밖 의미(chunk 전용이거나 cache로 열어 아직 읽지 않았으면 `None`). 에이전트 관찰(ADR-0132 결정 4)이 읽습니다.
    pub(crate) fn library(&self) -> Option<&Arc<SourceLibrary>> {
        match self {
            SourceMode::Text { library, .. } => library.as_ref(),
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
    // 열기 계획은 CLI·MCP와 같은 관찰 함수 하나로 정합니다(ADR-0113 결정 1).
    let DiskSource {
        text,
        current,
        plan,
        ..
    } = observe(root).map_err(|error| match error {
        ObserveError::Source(error) => source_error(None, &error),
        ObserveError::Pointer(error) => pointer_error(error),
    })?;
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

/// generation 문서를 텍스트 원본 의미로 옮길 수 있는지 검증합니다. 판정은 CLI·MCP와 같은
/// [`stellar_project::document_io::session::convert_saved`] 하나입니다(ADR-0088 결정 2, ADR-0113 결정 1).
fn convert(
    identity: &ProjectIdentity,
    model: &DocumentModel,
    text: Option<&SourceFiles>,
) -> Result<(SourceBase, SourceLibrary), String> {
    let root = identity.canonical_target();
    let lazy = LazyGeneration::open(root, identity.project_id(), &mut NoopRecorder)
        .map_err(|error| error.to_string())?;
    convert_saved(root, &lazy, model, text)
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
