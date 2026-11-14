//! opt-in 단일 View 텍스트 원본 export·검사·명령 적용 (ADR-0059).
//!
//! 기존 generation을 변경하지 않습니다. export는 재직렬화한 문서의 chunk 집합이 저장된
//! generation 전체와 정확히 같을 때만 새 파일을 만듭니다. 미지원 chunk를 버리지 않습니다.

use std::fmt;
use std::io::ErrorKind;
use std::path::Path;

use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::request::CommandRequest;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::document_io::text_source::{self, TextSourceError};
use stellar_project::identity::ProjectIdentity;
use stellar_storage::chunk::pointer::ChunkPointer;
pub use stellar_storage::hash::ContentHash;

use crate::apply::{decode_commands, ApplyError, ApplyMode, CommandInput};
use crate::core_types::ids::StableId;
use crate::core_types::revision::DocumentRevision;
use crate::diff::{diff_records, DiffReport};
use crate::error::{DocumentReadError, OpenError};
use crate::lock::{LockError, LockPolicy, SaveLock};
use crate::snapshot::open_snapshot;

/// export·검사 결과. 경로와 프로젝트 내용은 포함하지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSourceOutcome {
    /// 실제 원본 문서의 StableId.
    pub document: StableId,
    /// 파일의 실제 텍스트 bytes hash.
    pub source_hash: ContentHash,
    /// 검증한 문서의 node 수.
    pub node_count: usize,
}

/// 텍스트 원본 작업 실패.
#[derive(Debug)]
pub enum TextSourceOperationError {
    /// 입력 프로젝트 열기 실패.
    Open(OpenError),
    /// 저장 문서 읽기 실패.
    Document(DocumentReadError),
    /// 지원 범위 밖의 프로젝트·문서 의미.
    Unsupported(String),
    /// 문서 또는 텍스트 형식 오류.
    Invalid(String),
    /// 작업 중 파일이 바뀌었습니다.
    Changed,
    /// 파일 접근 실패. 경로는 출력 계층이 별도로 다룹니다.
    Io(ErrorKind),
    /// 기존 command 검증·실행·저장 잠금 오류.
    Apply(ApplyError),
}

impl fmt::Display for TextSourceOperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(error) => write!(f, "{error}"),
            Self::Document(error) => write!(f, "{error}"),
            Self::Unsupported(message) => write!(f, "지원하지 않는 텍스트 원본 의미: {message}"),
            Self::Invalid(message) => write!(f, "텍스트 원본 검증: {message}"),
            Self::Changed => f.write_str("작업 중 텍스트 원본이 변경되었습니다"),
            Self::Io(kind) => write!(f, "텍스트 원본 I/O: {kind:?}"),
            Self::Apply(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for TextSourceOperationError {}

impl From<ApplyError> for TextSourceOperationError {
    fn from(error: ApplyError) -> Self {
        Self::Apply(error)
    }
}

impl From<TextSourceError> for TextSourceOperationError {
    fn from(error: TextSourceError) -> Self {
        match error {
            TextSourceError::Invalid(message) => Self::Invalid(message),
            TextSourceError::Unsupported(message) => Self::Unsupported(message),
            TextSourceError::Changed => Self::Changed,
            TextSourceError::Io(error) => Self::Io(error.kind()),
        }
    }
}

/// 저장된 단일 문서·page·View 프로젝트를 새 텍스트 파일로 내보냅니다.
/// 기존 출력 파일은 덮어쓰지 않습니다.
pub fn export(
    project: &Path,
    output: &Path,
) -> Result<TextSourceOutcome, TextSourceOperationError> {
    let snapshot = open_snapshot(project).map_err(TextSourceOperationError::Open)?;
    let generation = snapshot.generation().ok_or_else(|| {
        TextSourceOperationError::Unsupported("저장된 generation이 필요합니다".into())
    })?;
    let [document] = snapshot.documents() else {
        return Err(TextSourceOperationError::Unsupported(
            "정확히 한 문서가 필요합니다".into(),
        ));
    };
    let manifest = generation.manifest();
    if manifest.schema_version != crate::migrate::CURRENT_SCHEMA_VERSION
        || manifest.root_document_ids.as_slice() != [document.id]
    {
        return Err(TextSourceOperationError::Unsupported(
            "지원하는 generation 스키마와 단일 root 문서가 필요합니다".into(),
        ));
    }
    let model = snapshot
        .document_model(document.id)
        .map_err(TextSourceOperationError::Document)?;
    let serialized = serialize_document(&model)
        .map_err(|error| TextSourceOperationError::Invalid(error.to_string()))?;
    let mut expected: Vec<_> = serialized
        .chunks()
        .into_iter()
        .map(|chunk| ChunkPointer {
            chunk_id: chunk.chunk_id,
            content_hash: ContentHash::compute(&chunk.payload),
            kind: chunk.kind,
            schema_version: chunk.schema_version,
        })
        .collect();
    expected.sort();
    let mut saved = manifest.chunk_pointers.clone();
    saved.sort();
    if saved != expected {
        return Err(TextSourceOperationError::Unsupported(
            "문서 밖 chunk 또는 재직렬화로 보존할 수 없는 chunk가 있습니다".into(),
        ));
    }
    Ok(TextSourceOutcome {
        document: model.document_id(),
        source_hash: text_source::save_new(output, &model)?,
        node_count: model.node_count(),
    })
}

/// 텍스트 원본을 읽기 전용으로 열고 문서 불변식을 검사합니다.
pub fn check(path: &Path) -> Result<TextSourceOutcome, TextSourceOperationError> {
    let opened = text_source::open(path)?;
    Ok(TextSourceOutcome {
        document: opened.model.document_id(),
        source_hash: opened.source_hash,
        node_count: opened.model.node_count(),
    })
}

/// 단일 텍스트 원본의 명령 요청. generation revision과 구분합니다.
#[derive(Debug, Clone)]
pub struct TextSourceApplyRequest {
    /// 읽은 파일의 실제 bytes hash. 주석·공백 변경도 불일치입니다.
    pub base_source_hash: ContentHash,
    /// 대상 문서 StableId.
    pub document: StableId,
    /// 기존 closed command 입력. 전부 성공하거나 파일을 쓰지 않습니다.
    pub commands: Vec<CommandInput>,
}

/// 명령 적용 결과. dry-run의 source_hash는 변경하지 않은 실제 파일 hash입니다.
#[derive(Debug)]
pub struct TextSourceApplyOutcome {
    /// 적용 방식.
    pub mode: ApplyMode,
    /// 작업 시작 시 원본 hash.
    pub base_source_hash: ContentHash,
    /// 반환 시 원본 hash(no-op은 주석을 포함한 기존 bytes 유지).
    pub source_hash: ContentHash,
    /// 대상 문서.
    pub document: StableId,
    /// 적용 결과의 노드 수.
    pub node_count: usize,
    /// 성공한 명령 수.
    pub command_count: usize,
    /// 적용 전후 의미 차이.
    pub diff: DiffReport,
    /// 실제 원본 bytes를 변경했는지 여부.
    pub written: bool,
}

/// 원본을 열어 기존 command transaction으로 편집하고 조건부 원자 저장합니다.
/// Write에는 필수 저장 잠금을 요구합니다. 문서 ID + canonical 파일 경로로 같은 파일의
/// 협조적 writer를 직렬화하며, 잠금 뒤 원본 hash와 ID를 검사합니다. dry-run은 잠금·cache를
/// 쓰지 않습니다. 비협조적 외부 writer 및 게시 후 fsync 실패의 한계는 source_file과 같습니다.
pub fn apply(
    path: &Path,
    request: &TextSourceApplyRequest,
    mode: ApplyMode,
    lock: &LockPolicy,
) -> Result<TextSourceApplyOutcome, TextSourceOperationError> {
    let commands = decode_commands(&request.commands)?;
    let _guard = match (mode, lock) {
        (ApplyMode::Write, LockPolicy::Required { dir }) => {
            let identity = ProjectIdentity::new(request.document, path)
                .map_err(|error| TextSourceOperationError::Io(error.kind()))?;
            Some(SaveLock::for_identity(dir, &identity).map_err(ApplyError::Lock)?)
        }
        (ApplyMode::Write, LockPolicy::None) => {
            return Err(ApplyError::Lock(LockError::NoUserDataDir).into());
        }
        (ApplyMode::DryRun, _) => None,
    };
    let opened = text_source::open(path)?;
    if opened.source_hash != request.base_source_hash {
        return Err(TextSourceOperationError::Changed);
    }
    if opened.model.document_id() != request.document {
        return Err(TextSourceOperationError::Invalid(
            "문서 StableId 불일치".into(),
        ));
    }
    let mut before = opened
        .model
        .to_records()
        .map_err(|e| TextSourceOperationError::Invalid(e.to_string()))?;
    before.sort_canonical();
    let mut state = DocumentState::new(opened.model);
    let transaction = CommandTransaction {
        // 이 단일 실행의 ID는 history나 디스크에 기록하지 않습니다.
        id: CommandId::from_stable_id(request.document),
        origin: CommandOrigin::User,
        requests: commands
            .iter()
            .cloned()
            .map(|command| CommandRequest::new(DocumentRevision(0), command))
            .collect(),
    };
    state
        .execute_transaction(&transaction)
        .map_err(|failure| ApplyError::Rejected {
            index: failure.index,
            reason: failure.error.to_string(),
        })?;
    let model = state.into_model();
    // dry-run도 실제 저장과 같은 지원 범위 및 크기 검사를 통과해야 합니다.
    text_source::validate_model(&model)?;
    let mut after = model
        .to_records()
        .map_err(|e| ApplyError::InvalidResult(e.to_string()))?;
    after.sort_canonical();
    let diff = diff_records(&[&before], &[&after]).map_err(|e| ApplyError::Projection(e.0))?;
    let source_hash = match mode {
        ApplyMode::DryRun => opened.source_hash,
        ApplyMode::Write => text_source::save(path, &model, opened.source_hash)?,
    };
    Ok(TextSourceApplyOutcome {
        mode,
        base_source_hash: opened.source_hash,
        source_hash,
        document: request.document,
        node_count: model.node_count(),
        command_count: commands.len(),
        diff,
        written: source_hash != opened.source_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_types::ids::StableId;
    use crate::test_support::{
        file_digest, large_document, project_id, save_documents, scaffold_project, TempDir,
    };
    use stellar_storage::chunk::envelope::ChunkKindCode;
    use stellar_storage::chunk::id::ChunkId;
    use stellar_storage::chunk::kinds::{KIND_DOCUMENT_MANIFEST, KIND_VIEW_TREE};
    use stellar_storage::chunk::store::ChunkStore;
    use stellar_storage::generation::current::write_current_pointer;
    use stellar_storage::generation::loader::{load_current_generation, write_manifest_file};

    #[test]
    fn text_apply_requires_write_lock_and_dry_run_never_creates_one() {
        let dir = TempDir::new("text-source-apply-lock");
        let lock_dir = TempDir::new("text-source-apply-lock-data");
        let path = dir.path.join("view.stellar.yaml");
        let model = large_document(4);
        let root = model.to_records().unwrap().nodes[0].id;
        let hash = text_source::save_new(&path, &model).unwrap();
        let request = TextSourceApplyRequest {
            base_source_hash: hash,
            document: model.document_id(),
            commands: vec![CommandInput::Rename {
                node: root,
                name: "Edited".into(),
            }],
        };
        let locks = lock_dir.path.join("locks");
        let policy = LockPolicy::Required { dir: locks.clone() };
        let before = file_digest(&dir.path);
        let preview = apply(&path, &request, ApplyMode::DryRun, &policy).unwrap();
        assert!(!locks.exists());
        assert!(!preview.written);
        assert_eq!(preview.source_hash, hash);
        assert_eq!(preview.diff.entries.len(), 1);
        assert_eq!(file_digest(&dir.path), before);
        assert!(matches!(
            apply(&path, &request, ApplyMode::Write, &LockPolicy::None),
            Err(TextSourceOperationError::Apply(ApplyError::Lock(_)))
        ));
        let guard = SaveLock::try_acquire(&locks, request.document, &path).unwrap();
        assert!(apply(&path, &request, ApplyMode::DryRun, &policy).is_ok());
        assert!(matches!(
            apply(&path, &request, ApplyMode::Write, &policy),
            Err(TextSourceOperationError::Apply(ApplyError::Lock(
                LockError::Busy { .. }
            )))
        ));
        assert_eq!(file_digest(&dir.path), before);
        drop(guard);
        let saved = apply(&path, &request, ApplyMode::Write, &policy).unwrap();
        assert!(saved.written);
        assert_eq!(saved.diff.entries, preview.diff.entries);
        let reopened = text_source::open(&path).unwrap();
        assert_eq!(reopened.source_hash, saved.source_hash);
        let mut expected = model;
        expected.set_display_name(root, "Edited".into()).unwrap();
        assert_eq!(
            reopened.model.to_records().unwrap(),
            expected.to_records().unwrap()
        );
        assert!(matches!(
            apply(&path, &request, ApplyMode::Write, &policy),
            Err(TextSourceOperationError::Changed)
        ));
    }

    #[test]
    fn frame_geometry_commands_reopen_identically_from_text_and_generation() {
        use crate::apply::{self, ApplyRequest};
        use crate::core_types::value::{Size2, Transform2D};
        use stellar_document::records::NodePayload;

        let project = TempDir::new("frame-geometry-project");
        let source = TempDir::new("frame-geometry-source");
        let locks = TempDir::new("frame-geometry-locks");
        let model = large_document(4);
        let mut expected = model.to_records().unwrap();
        let root = expected
            .nodes
            .iter_mut()
            .find(|n| n.parent.is_none())
            .unwrap();
        let node = root.id;
        let NodePayload::Frame(frame) = &mut root.payload else {
            panic!("fixture root Frame");
        };
        let size = Size2::new(320.5, 240.25).unwrap();
        let transform = Transform2D::new(0.0, 1.0, -1.0, 0.0, 12.5, -24.25).unwrap();
        frame.size = size;
        frame.transform = transform;
        expected.sort_canonical();
        let commands = vec![
            CommandInput::SetFrameSize { node, size },
            CommandInput::SetFrameTransform { node, transform },
        ];
        scaffold_project(&project.path);
        save_documents(&project.path, &[&model]);
        let path = source.path.join("view.stellar.yaml");
        let hash = export(&project.path, &path).unwrap().source_hash;
        let source_request = TextSourceApplyRequest {
            base_source_hash: hash,
            document: model.document_id(),
            commands: commands.clone(),
        };
        let policy = LockPolicy::Required {
            dir: locks.path.clone(),
        };
        let before = file_digest(&project.path);
        let saved = apply(&path, &source_request, ApplyMode::Write, &policy).unwrap();
        assert_eq!(saved.diff.entries.len(), 2);
        assert_eq!(file_digest(&project.path), before);
        let mut text_records = text_source::open(&path)
            .unwrap()
            .model
            .to_records()
            .unwrap();
        text_records.sort_canonical();
        assert_eq!(text_records, expected);

        let request = ApplyRequest {
            base_revision: open_snapshot(&project.path).unwrap().revision(),
            document: model.document_id(),
            commands,
        };
        let preview = apply::apply(&project.path, &request, ApplyMode::DryRun, &policy).unwrap();
        assert_eq!(file_digest(&project.path), before);
        assert_eq!(preview.diff.entries, saved.diff.entries);
        apply::apply(&project.path, &request, ApplyMode::Write, &policy).unwrap();
        let mut generation_records = open_snapshot(&project.path)
            .unwrap()
            .document_model(request.document)
            .unwrap()
            .to_records()
            .unwrap();
        generation_records.sort_canonical();
        assert_eq!(generation_records, expected);
    }

    #[test]
    fn unsupported_command_result_is_rejected_even_in_dry_run() {
        use stellar_commands::command::Command;
        use stellar_commands::node_ops::InsertNode;
        use stellar_document::records::NodePayload;
        use stellar_document::shape::{RectangleParams, ShapeGeometry};

        let dir = TempDir::new("text-source-apply-unsupported");
        let lock_dir = TempDir::new("text-source-apply-unsupported-lock");
        let path = dir.path.join("view.stellar.yaml");
        let model = large_document(4);
        let hash = text_source::save_new(&path, &model).unwrap();
        let mut node = model
            .to_records()
            .unwrap()
            .nodes
            .into_iter()
            .find(|node| node.parent.is_some())
            .unwrap();
        node.id = StableId::new_v7(99, 0, 99);
        node.order_key = crate::core_types::order_key::OrderKey::from_bytes(vec![255]).unwrap();
        node.payload = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: crate::core_types::value::Size2::ZERO,
            corner_radius: stellar_document::frame::CornerRadius::ZERO,
        }));
        let command = Command::InsertNode(InsertNode::single(node));
        let request = TextSourceApplyRequest {
            base_source_hash: hash,
            document: model.document_id(),
            commands: vec![CommandInput::Canonical {
                kind: "insert-node".into(),
                bytes: stellar_commands::codec::encode_command(&command),
            }],
        };
        let before = file_digest(&dir.path);
        for mode in [ApplyMode::DryRun, ApplyMode::Write] {
            assert!(matches!(
                apply(
                    &path,
                    &request,
                    mode,
                    &LockPolicy::Required {
                        dir: lock_dir.path.join("locks"),
                    }
                ),
                Err(TextSourceOperationError::Unsupported(_))
            ));
            assert_eq!(file_digest(&dir.path), before);
        }
    }

    #[test]
    fn export_rejects_unconsumed_chunks_and_changed_pointer_metadata_without_writing() {
        let dir = TempDir::new("text-source-chunks");
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&large_document(4)]);
        let original = load_current_generation(&dir.path, project_id()).unwrap();
        let output = dir.path.join("view.stellar.yaml");
        for case in 0..6 {
            let mut manifest = original.manifest.clone();
            match case {
                0 | 1 => {
                    let mut extra = *manifest
                        .chunk_pointers
                        .iter()
                        .find(|p| p.kind == KIND_VIEW_TREE)
                        .unwrap();
                    extra.chunk_id = ChunkId::from_stable_id(StableId::new_v7(99, 0, 99));
                    if case == 0 {
                        extra.kind = ChunkKindCode(999);
                        let payload = b"unconsumed optional project data";
                        ChunkStore::new(&dir.path)
                            .write_chunk(extra.kind, payload)
                            .unwrap();
                        extra.content_hash = ContentHash::compute(payload);
                    }
                    manifest.chunk_pointers.push(extra);
                }
                2 => manifest.chunk_pointers[0].schema_version += 1,
                3 => manifest.root_document_ids.clear(),
                4 => manifest.schema_version += 1,
                5 => {
                    manifest
                        .chunk_pointers
                        .iter_mut()
                        .find(|p| p.kind == KIND_DOCUMENT_MANIFEST)
                        .unwrap()
                        .chunk_id = ChunkId::from_stable_id(StableId::new_v7(99, 0, 99))
                }
                _ => unreachable!(),
            }
            let hash = write_manifest_file(&dir.path, &manifest).unwrap();
            write_current_pointer(&dir.path, &hash).unwrap();
            let before = file_digest(&dir.path);
            assert!(export(&dir.path, &output).is_err(), "case {case}");
            assert!(!output.exists(), "case {case}");
            assert_eq!(file_digest(&dir.path), before, "case {case}");
        }
    }
}
