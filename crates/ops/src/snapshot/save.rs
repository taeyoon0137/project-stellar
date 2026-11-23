//! headless 쓰기의 원본·cache 저장(ADR-0113 결정 3).
//!
//! CLI·MCP의 쓰기(`stellar apply`, `stellar css paste`, Merge Composer 결과 적용)는 앱 창 저장(ADR-0088 결정 3)과 같은
//! 순서로 씁니다. 호출자는 저장 잠금([`crate::lock::SaveLock`])을 쥔 채 부릅니다.
//!
//! 1. 텍스트 원본 대상 정하기: snapshot이 원본을 봤으면(Cache·Forward·FromText) 그 원본 base 위에 씁니다. 문서 밖
//!    의미(컴포넌트·token·이미지 record)는 원본에서 읽은 것을 씁니다(FromText는 열 때 해석한 것, 아니면 열 때 본 원본
//!    bytes를 해석). 텍스트 원본이 없는 chunk 프로젝트(Migrate)와 root 문서가 하나가 아닌 snapshot은 chunk 전용
//!    저장입니다([`SourceWrite::ChunkOnly`], 원본을 쓰지 않음). 원본 전환은 앱 창 열기의 검증된 변환(ADR-0088 결정 2,
//!    `session::convert_saved`)과 새 프로젝트 생성이 합니다. 원본이 있는데 읽거나 해석할 수 없으면 chunk 전용으로
//!    물러서지 않고 아무것도 쓰지 않습니다([`SaveError::Source`]).
//! 2. 결과 문서를 원본으로 인코딩합니다. 원본이 담을 수 없는 결과는 아무것도 쓰지 않고 [`SaveError::Encode`]입니다
//!    (Git 원본과 cache가 갈라지지 않게 함).
//! 3. current pointer가 열 때와 같은지 확인합니다(다르면 [`SaveError::ConcurrentWrite`], 아무것도 쓰지 않음).
//! 4. 바뀐 원본 파일만 저장 journal 한 거래로 씁니다(기대 hash = 열 때 base). 그 사이 외부가 바꾼 원본 파일이 있으면
//!    [`SaveError::ConcurrentWrite`]이고 아무것도 쓰지 않습니다.
//! 5. cache generation을 세대 전환 저장으로 씁니다(문서가 아닌 chunk는 base pointer 재사용).
//! 6. stamp(원본 digest, 새 manifest hash)를 씁니다.
//!
//! 원본을 쓴 뒤 cache나 stamp 쓰기가 실패하면 저장은 성공이고 [`SaveOutcome::cache_stale`]입니다. revision은 그
//! 디스크 상태를 다시 열었을 때와 같은 값(원본 revision)입니다. 다음 열기가 원본에서 읽고, 다음 쓰기가 cache를 다시
//! 맞춥니다.

use std::fmt;

use stellar_document::model::DocumentModel;
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::document_io::session::{
    commit_files, decode_source, write_stamp, SourceBase, SourceCommitError, SourceLibrary, Stamp,
};
use stellar_project::document_io::source_v4::project::encode;
use stellar_storage::chunk::kinds::{KIND_CONTEXT_SET, KIND_DOCUMENT_MANIFEST, KIND_VIEW_TREE};
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::generation::current::{read_current_pointer, CurrentPointerError};
use stellar_storage::generation::loader::LoadedGeneration;
use stellar_storage::save::cancel::{run_save, CancelToken, SaveRunError};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::executor::SaveExecError;
use stellar_storage::save::optimistic::BaseCheckError;
use stellar_storage::save::plan::build_save_plan;

use super::{text_revision, ProjectSnapshot};
use crate::core_types::ids::StableId;
use crate::revision::ProjectRevision;

/// 원본 쓰기 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceWrite {
    /// 텍스트 원본에 썼습니다.
    Text {
        /// 새로 쓰거나 바꾼 원본 파일 수.
        written: usize,
        /// 지운 원본 파일 수.
        deleted: usize,
    },
    /// 텍스트 원본으로 옮길 수 없어 chunk generation만 썼습니다(ADR-0088 결정 2).
    ChunkOnly {
        /// 이유.
        reason: String,
    },
}

/// 저장 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveOutcome {
    /// 저장 뒤 프로젝트 revision(다시 열었을 때와 같은 값, 세트 선택 포함).
    pub revision: ProjectRevision,
    /// 새로 기록한 chunk 수.
    pub written_chunks: usize,
    /// 원본 쓰기 결과.
    pub source: SourceWrite,
    /// 원본은 썼지만 cache나 stamp를 쓰지 못했습니다(다음 쓰기가 맞춤).
    pub cache_stale: bool,
}

/// 저장 실패. 어느 경우든 원본과 current generation은 바뀌지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// 열 뒤 다른 writer가 원본이나 current generation을 바꿨습니다.
    ConcurrentWrite,
    /// 결과를 텍스트 원본으로 쓸 수 없습니다.
    Encode(String),
    /// 텍스트 원본이 있지만 읽거나 해석할 수 없습니다(파일·줄 진단). cache만 쓰면 Git 원본과 cache가 갈라지므로
    /// 아무것도 쓰지 않습니다.
    Source(String),
    /// 문서 상태를 만들거나 직렬화할 수 없습니다.
    Invalid(String),
    /// 파일 쓰기 실패.
    Write(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::ConcurrentWrite => f.write_str(
                "연 뒤 다른 writer가 원본이나 current generation을 바꿨습니다(아무것도 쓰지 않음)",
            ),
            SaveError::Encode(e) => write!(f, "결과를 텍스트 원본으로 쓸 수 없습니다: {e}"),
            SaveError::Source(e) => write!(f, "텍스트 원본을 읽을 수 없습니다: {e}"),
            SaveError::Invalid(e) => write!(f, "{e}"),
            SaveError::Write(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// snapshot의 root 문서 순서. 원본에서 읽은 snapshot은 원본 문서 하나, 아니면 cache manifest 순서입니다.
pub fn root_order(snapshot: &ProjectSnapshot) -> Vec<StableId> {
    match snapshot.generation() {
        Some(generation) if !snapshot.reads_from_text() => {
            generation.manifest().root_document_ids.clone()
        }
        _ => snapshot.documents().iter().map(|d| d.id).collect(),
    }
}

/// 원본 대상: base와 문서 밖 의미. 안쪽 `Err`는 chunk 전용 이유입니다. 원본이 있는데 읽거나 해석할 수 없으면 chunk
/// 전용으로 물러서지 않고 [`SaveError::Source`]입니다(cache만 앞서 Git 원본과 갈라지지 않게 함).
#[allow(clippy::type_complexity)]
fn source_target(
    snapshot: &ProjectSnapshot,
    models: &[DocumentModel],
) -> Result<Result<(SourceBase, SourceLibrary), String>, SaveError> {
    if let Some(diagnostic) = snapshot.source_diagnostic() {
        return Err(SaveError::Source(diagnostic.to_string()));
    }
    let Some(files) = snapshot.source_files() else {
        // 텍스트 원본이 없는 chunk 프로젝트는 chunk 전용으로 씁니다. 원본 전환은 앱 창 열기의 검증된 변환
        // (ADR-0088 결정 2)이나 새 프로젝트 생성이 합니다(ADR-0113 결정 3).
        return Ok(Err("텍스트 원본이 없는 chunk 프로젝트입니다".into()));
    };
    if models.len() != 1 {
        return Ok(Err("텍스트 원본은 root 문서 하나만 담습니다".into()));
    }
    let library = match snapshot.decoded_library() {
        Some(library) => library.clone(),
        None => {
            let project = decode_source(files).map_err(|e| {
                let all = files.files.keys().cloned().collect();
                SaveError::Source(
                    stellar_project::external_change::source::diagnose(&files.files, &all, &e)
                        .to_string(),
                )
            })?;
            SourceLibrary::split(project).1
        }
    };
    Ok(Ok((files.base.clone(), library)))
}

fn write_error(error: SaveRunError) -> SaveError {
    match error {
        SaveRunError::Exec(SaveExecError::BaseCheck(BaseCheckError::StaleBase { .. })) => {
            SaveError::ConcurrentWrite
        }
        other => SaveError::Write(other.to_string()),
    }
}

/// `roots` 순서의 문서 전체(`models`, 바꾸지 않은 문서 포함)를 원본과 cache에 씁니다(모듈 문서 순서).
///
/// 결과 revision은 원본·cache revision에 열 때 본 활성 기본값 세트 선택을 더한 프로젝트 revision입니다(ADR-0128
/// 결정 2). 같은 쓰기에서 세트도 바꾸는 호출자는 [`save_documents_selecting`]을 씁니다.
pub fn save_documents(
    snapshot: &ProjectSnapshot,
    roots: Vec<StableId>,
    models: Vec<DocumentModel>,
) -> Result<SaveOutcome, SaveError> {
    save_documents_selecting(snapshot, roots, models, snapshot.style_defaults_name())
}

/// [`save_documents`]와 같고, 결과 revision에 더할 세트 선택이 `selection`입니다. 이 함수는 `stellar.yaml`을 쓰지
/// 않습니다. 호출자가 저장 뒤 그 선택을 씁니다(`ops::apply`, ADR-0087 결정 2).
pub fn save_documents_selecting(
    snapshot: &ProjectSnapshot,
    roots: Vec<StableId>,
    models: Vec<DocumentModel>,
    selection: &str,
) -> Result<SaveOutcome, SaveError> {
    let mut outcome = write_documents(snapshot, roots, models)?;
    outcome.revision = outcome.revision.with_style_defaults(selection);
    Ok(outcome)
}

/// 원본과 cache를 씁니다. 결과 revision은 세트 선택을 더하지 않은 원본·cache revision입니다.
fn write_documents(
    snapshot: &ProjectSnapshot,
    roots: Vec<StableId>,
    models: Vec<DocumentModel>,
) -> Result<SaveOutcome, SaveError> {
    let root = snapshot.root();
    // 1·2. 원본 대상과 인코딩(쓰기 전).
    let text = match source_target(snapshot, &models)? {
        Ok((base, library)) => {
            let model = models.first().expect("source_target이 문서 하나를 확인함");
            let records = model
                .to_records()
                .map_err(|e| SaveError::Invalid(e.to_string()))?;
            let (mut copy, _) = DocumentModel::from_records(records)
                .map_err(|e| SaveError::Invalid(format!("{e:?}")))?;
            copy.replace_context(model.context().live(model));
            copy.set_style_defaults(model.style_defaults());
            let encoded =
                encode(&library.join(copy)).map_err(|e| SaveError::Encode(e.to_string()))?;
            Ok((base, encoded.files))
        }
        Err(reason) => Err(reason),
    };

    // 5번 입력: cache 저장 계획(쓰기 전).
    let base = snapshot.generation().map(|generation| LoadedGeneration {
        manifest_hash: snapshot
            .current_manifest()
            .expect("generation이 있으면 current가 있음"),
        manifest: generation.manifest().clone(),
    });
    let mut states: Vec<LogicalChunkState> = base
        .iter()
        .flat_map(|base| base.manifest.chunk_pointers.iter())
        // 문서 Context chunk(context-set)도 문서마다 다시 직렬화하므로 base pointer를 재사용하지 않습니다(ADR-0114).
        // Context가 비면 그 문서는 context-set chunk를 쓰지 않으므로 옛 chunk도 남기지 않습니다.
        .filter(|p| {
            p.kind != KIND_DOCUMENT_MANIFEST
                && p.kind != KIND_VIEW_TREE
                && p.kind != KIND_CONTEXT_SET
        })
        .map(|p| LogicalChunkState::Clean {
            chunk_id: p.chunk_id,
        })
        .collect();
    for model in &models {
        states.extend(
            serialize_document(model)
                .map_err(|e| SaveError::Invalid(e.to_string()))?
                .to_dirty_states(),
        );
    }
    let schema_version = base
        .as_ref()
        .map_or(crate::migrate::CURRENT_SCHEMA_VERSION, |base| {
            base.manifest.schema_version
        });
    let plan = build_save_plan(
        base.as_ref(),
        snapshot.project_id(),
        schema_version,
        &states,
        roots,
    )
    .map_err(|e| SaveError::Invalid(e.to_string()))?;
    let written_chunks = plan.dirty_chunks.len();

    // 3. current pointer 확인.
    let current = match read_current_pointer(root) {
        Ok(current) => Some(current),
        Err(CurrentPointerError::Missing { .. }) => None,
        Err(e) => return Err(SaveError::Write(e.to_string())),
    };
    if current != snapshot.current_manifest() {
        return Err(SaveError::ConcurrentWrite);
    }

    // 4. 원본.
    let (source, new_base) = match text {
        Ok((base, files)) => {
            let commit = commit_files(root, &base, &files).map_err(commit_error)?;
            (
                SourceWrite::Text {
                    written: commit.written,
                    deleted: commit.deleted,
                },
                Some(commit.base),
            )
        }
        Err(reason) => (SourceWrite::ChunkOnly { reason }, None),
    };

    // 5. cache.
    let cache_generation = base.as_ref().map_or(0, |base| base.manifest.generation);
    let mut memory = SavedGenerationState::new(
        base.as_ref()
            .map(|base| (base.manifest.generation, base.manifest_hash)),
    );
    let saved = run_save(root, &plan, &mut memory, &CancelToken::new())
        .map_err(write_error)
        .and_then(|_| {
            memory
                .saved()
                .ok_or_else(|| SaveError::Write("저장 결과 generation이 없습니다".into()))
        });
    let (generation, manifest_hash) = match (saved, &new_base) {
        (Ok(saved), _) => saved,
        (Err(error), None) => return Err(error),
        (Err(_), Some(new_base)) => {
            return Ok(SaveOutcome {
                revision: text_revision(cache_generation, current, Some(new_base)),
                written_chunks: 0,
                source,
                cache_stale: true,
            })
        }
    };

    // 6. stamp.
    let mut cache_stale = false;
    let mut revision = ProjectRevision::Saved {
        generation,
        manifest_hash,
    };
    if let Some(new_base) = &new_base {
        let stamp = Stamp {
            source: new_base.digest(),
            manifest: manifest_hash,
        };
        if write_stamp(root, &stamp).is_err() {
            cache_stale = true;
            revision = text_revision(generation, Some(manifest_hash), Some(new_base));
        }
    }
    Ok(SaveOutcome {
        revision,
        written_chunks,
        source,
        cache_stale,
    })
}

/// 텍스트 원본으로 옮긴 결과(`stellar text-source adopt`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptOutcome {
    /// 옮긴 뒤 revision(원본과 cache가 짝이라 cache revision과 같음).
    pub revision: ProjectRevision,
    /// 새로 쓴 원본 파일 수.
    pub written: usize,
    /// 원본 파일 수.
    pub files: usize,
}

/// 옮기지 못한 이유.
#[derive(Debug)]
pub enum AdoptError {
    /// 이미 텍스트 원본이 있습니다.
    AlreadyText,
    /// 저장된 generation이 없습니다.
    Unsaved,
    /// 검증된 변환이 없습니다(ADR-0088 결정 2의 이유 문장).
    Unsupported(String),
    /// 쓰기 실패·경합.
    Save(SaveError),
    /// 프로젝트를 열 수 없습니다.
    Open(crate::error::OpenError),
    /// 저장 잠금을 잡지 못했습니다.
    Lock(crate::lock::LockError),
}

/// 저장 잠금을 잡고 `path` 프로젝트를 텍스트 원본으로 옮깁니다([`adopt_text_source`]).
pub fn adopt(
    path: &std::path::Path,
    lock: &crate::lock::LockPolicy,
) -> Result<AdoptOutcome, AdoptError> {
    use crate::error::OpenError;
    let root = std::fs::canonicalize(path)
        .map_err(|e| AdoptError::Open(OpenError::Path { kind: e.kind() }))?;
    let _guard = match lock {
        crate::lock::LockPolicy::Required { dir } => {
            let probe = super::probe(&root).map_err(AdoptError::Open)?;
            let project_id =
                stellar_project::identity::parse_project_id(&probe.manifest.project.id)
                    .ok_or_else(|| {
                        AdoptError::Open(OpenError::InvalidProjectId {
                            id: probe.manifest.project.id.clone(),
                        })
                    })?;
            Some(
                crate::lock::SaveLock::try_acquire(dir, project_id, &root)
                    .map_err(AdoptError::Lock)?,
            )
        }
        crate::lock::LockPolicy::None => None,
    };
    let snapshot = super::open_snapshot(&root).map_err(AdoptError::Open)?;
    adopt_text_source(&snapshot)
}

impl fmt::Display for AdoptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdoptError::Open(e) => write!(f, "{e}"),
            AdoptError::Lock(e) => write!(f, "{e}"),
            AdoptError::AlreadyText => f.write_str("이미 텍스트 원본이 있는 프로젝트입니다"),
            AdoptError::Unsaved => f.write_str("저장된 generation이 없습니다"),
            AdoptError::Unsupported(reason) => {
                write!(f, "텍스트 원본으로 옮길 수 없습니다: {reason}")
            }
            AdoptError::Save(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AdoptError {}

fn commit_error(error: SourceCommitError) -> SaveError {
    match error {
        SourceCommitError::Stale { .. } => SaveError::ConcurrentWrite,
        SourceCommitError::Encode(e) => SaveError::Encode(e.to_string()),
        SourceCommitError::Journal(e) => SaveError::Write(e.to_string()),
    }
}

/// 텍스트 원본이 없는 chunk 프로젝트를 앱 창 열기와 같은 검증된 변환(`session::convert_saved`, ADR-0088 결정 2)으로
/// 텍스트 원본으로 옮기고 stamp를 씁니다(ADR-0113 결정 3). generation은 그대로 두므로 revision도 그대로입니다. 호출자는
/// 저장 잠금을 쥔 채 부릅니다.
pub fn adopt_text_source(snapshot: &ProjectSnapshot) -> Result<AdoptOutcome, AdoptError> {
    if snapshot.source_files().is_some() || snapshot.source_diagnostic().is_some() {
        return Err(AdoptError::AlreadyText);
    }
    let generation = snapshot.generation().ok_or(AdoptError::Unsaved)?;
    let [definition] = snapshot.documents() else {
        return Err(AdoptError::Unsupported(
            "텍스트 원본은 root 문서 하나만 담습니다".into(),
        ));
    };
    let model = snapshot
        .document_model(definition.id)
        .map_err(|e| AdoptError::Unsupported(e.to_string()))?;
    let (base, library) = stellar_project::document_io::session::convert_saved(
        snapshot.root(),
        generation,
        &model,
        None,
    )
    .map_err(AdoptError::Unsupported)?;
    let encoded = encode(&library.join(model))
        .map_err(|e| AdoptError::Save(SaveError::Encode(e.to_string())))?;
    let current = read_current_pointer(snapshot.root())
        .map_err(|e| AdoptError::Save(SaveError::Write(e.to_string())))?;
    if Some(current) != snapshot.current_manifest() {
        return Err(AdoptError::Save(SaveError::ConcurrentWrite));
    }
    let commit = commit_files(snapshot.root(), &base, &encoded.files)
        .map_err(|e| AdoptError::Save(commit_error(e)))?;
    write_stamp(
        snapshot.root(),
        &Stamp {
            source: commit.base.digest(),
            manifest: current,
        },
    )
    .map_err(|e| AdoptError::Save(SaveError::Write(e.to_string())))?;
    Ok(AdoptOutcome {
        revision: snapshot.revision(),
        written: commit.written,
        files: encoded.files.len(),
    })
}
