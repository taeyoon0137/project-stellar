//! headless 편집 적용 (XXIV.4.4).
//!
//! [`apply`]는 실행 중 앱 없이 프로젝트 파일에 CommandTransaction(VII.1.6)을 적용합니다.
//!
//! 1. snapshot을 열고(XXIV.4.2) 요청의 base revision과 현재 revision을 비교합니다. 다르면 아무것도
//!    읽거나 쓰지 않고 [`ApplyError::BaseRevisionMismatch`]입니다.
//! 2. 요청의 command 입력([`CommandInput`])을 commands crate의 **닫힌 enum** [`Command`]로만
//!    바꿉니다. 등록되지 않은 종류는 [`ApplyError::UnknownCommand`]로 거부합니다(raw patch·script 없음).
//! 3. 대상 문서를 읽어 [`DocumentState::execute_transaction`]으로 전부 적용하거나 전부 롤백합니다.
//!    validator가 거부하면 [`ApplyError::Rejected`]입니다.
//! 4. 적용 전후 문서 레코드의 semantic diff(XIV.3)를 계산합니다. dry-run은 여기서 끝나며 파일을
//!    쓰지 않습니다.
//! 5. 쓰기 모드는 모든 문서를 다시 직렬화해(VI.10.2) 같은 base generation 위의 SavePlan(V.5.1)을
//!    만들고, 저장 파이프라인(V.5.2~V.5.5)으로 chunk·manifest를 쓴 뒤 current pointer를 원자적으로
//!    교체합니다(세대 전환 저장). 내용이 같은 chunk는 hash 비교로 재사용되고, 문서가 아닌 chunk
//!    (project metadata·token·component·asset)는 base pointer를 그대로 씁니다. 계획과 교체 사이에
//!    다른 writer가 current pointer를 바꿨으면 optimistic check(V.5.2)가 쓰기 전에 거부합니다
//!    ([`ApplyError::ConcurrentWrite`]).
//!
//! 쓰기 모드는 1번 전에 프로젝트별 교차 프로세스 저장 잠금([`crate::lock`], XXIV.4.5)을 잡고 저장을
//! 마칠 때까지 쥡니다. 잠금을 쥔 동안 다른 headless writer는 들어오지 못하므로, 잠금 안에서 한
//! generation 비교와 pointer 교체 사이에 다른 headless 저장이 끼어들 수 없습니다.

use std::fmt;
use std::path::Path;

use stellar_commands::codec::{decode_command, CommandCodecError};
use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::kind::CommandKind;
use stellar_commands::node_ops::{DeleteSubtree, MoveNode};
use stellar_commands::property::SetProperty;
use stellar_commands::request::CommandRequest;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::identity::parse_project_id;
use stellar_storage::chunk::kinds::{KIND_DOCUMENT_MANIFEST, KIND_VIEW_TREE};
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::generation::loader::LoadedGeneration;
use stellar_storage::hash::ContentHash;
use stellar_storage::save::cancel::{run_save, CancelToken, SaveRunError};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::executor::SaveExecError;
use stellar_storage::save::optimistic::BaseCheckError;
use stellar_storage::save::plan::build_save_plan;

use crate::core_types::ids::StableId;
use crate::core_types::order_key::OrderKey;
use crate::core_types::revision::DocumentRevision;
use crate::diff::{diff_records, DiffReport};
use crate::error::{DocumentReadError, OpenError};
use crate::lock::{LockError, LockPolicy, SaveLock};
use crate::revision::ProjectRevision;
use crate::snapshot::{open_snapshot, probe};

/// 요청 하나에 담을 수 있는 command 수 상한.
pub const MAX_COMMANDS: usize = 10_000;

/// 형식과 무관한 command 입력.
///
/// 자주 쓰는 편집은 필드로 받고, 그 밖의 모든 closed enum 종류는 commands crate canonical
/// bytes(VII.1.2)로 받습니다. 어느 형식이든 [`Command`] closed enum으로만 바뀝니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandInput {
    /// 표시 여부 변경(`SetProperty` specialization, VII.2.5).
    SetVisible {
        /// 대상 node.
        node: StableId,
        /// 새 값.
        visible: bool,
    },
    /// 잠금 변경(VII.2.5).
    SetLocked {
        /// 대상 node.
        node: StableId,
        /// 새 값.
        locked: bool,
    },
    /// 표시 이름 변경(VII.2.5).
    Rename {
        /// 대상 node.
        node: StableId,
        /// 새 이름.
        name: String,
    },
    /// subtree 삭제(VII.2.1).
    DeleteSubtree {
        /// subtree root.
        root: StableId,
    },
    /// node 이동·재정렬(VII.2.3).
    MoveNode {
        /// 옮길 node.
        node: StableId,
        /// 새 부모.
        new_parent: StableId,
        /// 새 OrderKey bytes.
        order_key: Vec<u8>,
    },
    /// 종류 이름 + canonical bytes(`encode_command`, VII.1.2).
    Canonical {
        /// 선언한 종류 이름([`CommandKind::name`]).
        kind: String,
        /// canonical bytes(kind `u16` + 길이 + payload).
        bytes: Vec<u8>,
    },
}

/// 필드 형식으로 받는 종류 이름.
pub const FIELD_KINDS: [&str; 5] = [
    "set-visible",
    "set-locked",
    "rename",
    "delete-subtree",
    "move-node",
];

impl CommandInput {
    /// closed enum [`Command`]로 바꿉니다.
    pub fn to_command(&self) -> Result<Command, CommandInputError> {
        match self {
            CommandInput::SetVisible { node, visible } => Ok(Command::SetProperty(
                SetProperty::visibility(*node, *visible),
            )),
            CommandInput::SetLocked { node, locked } => {
                Ok(Command::SetProperty(SetProperty::locked(*node, *locked)))
            }
            CommandInput::Rename { node, name } => Ok(Command::SetProperty(SetProperty::rename(
                *node,
                name.clone(),
            ))),
            CommandInput::DeleteSubtree { root } => {
                Ok(Command::DeleteSubtree(DeleteSubtree { root: *root }))
            }
            CommandInput::MoveNode {
                node,
                new_parent,
                order_key,
            } => {
                let order_key = OrderKey::from_bytes(order_key.clone())
                    .map_err(|e| CommandInputError::Invalid(format!("OrderKey: {e:?}")))?;
                Ok(Command::MoveNode(MoveNode {
                    node: *node,
                    new_parent: *new_parent,
                    order_key,
                }))
            }
            CommandInput::Canonical { kind, bytes } => {
                if !CommandKind::ALL.iter().any(|k| k.name() == kind) {
                    return Err(CommandInputError::UnknownKind(kind.clone()));
                }
                let command = decode_command(bytes).map_err(|e| match e {
                    CommandCodecError::UnknownKind { value } => {
                        CommandInputError::UnknownKind(format!("{value:#06x}"))
                    }
                    CommandCodecError::Malformed { reason } => {
                        CommandInputError::Invalid(reason.to_string())
                    }
                })?;
                if command.kind().name() != kind {
                    return Err(CommandInputError::Invalid(format!(
                        "선언한 종류 {kind}와 canonical bytes의 종류 {}가 다릅니다",
                        command.kind().name()
                    )));
                }
                Ok(command)
            }
        }
    }
}

/// command 입력 변환 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandInputError {
    /// closed enum registry에 없는 종류.
    UnknownKind(String),
    /// 형식이 틀렸습니다.
    Invalid(String),
}

impl fmt::Display for CommandInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandInputError::UnknownKind(kind) => write!(f, "알 수 없는 command 종류: {kind}"),
            CommandInputError::Invalid(reason) => write!(f, "command 형식 오류: {reason}"),
        }
    }
}

/// 적용 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyRequest {
    /// 요청을 만든 시점의 프로젝트 revision.
    pub base_revision: ProjectRevision,
    /// 대상 문서.
    pub document: StableId,
    /// 순서대로 적용할 command(전부 성공하거나 전부 미적용).
    pub commands: Vec<CommandInput>,
}

/// 적용 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// semantic diff만 계산하고 파일을 쓰지 않습니다.
    DryRun,
    /// 세대 전환 저장으로 기록합니다.
    Write,
}

/// 적용 실패. 어느 경우든 프로젝트의 current generation은 바뀌지 않습니다.
#[derive(Debug)]
pub enum ApplyError {
    /// 프로젝트를 열 수 없습니다.
    Open(OpenError),
    /// 요청 base revision이 현재 revision과 다릅니다.
    BaseRevisionMismatch {
        /// 요청 base revision.
        expected: ProjectRevision,
        /// 현재 revision.
        found: ProjectRevision,
    },
    /// command가 없습니다.
    EmptyTransaction,
    /// command 수가 [`MAX_COMMANDS`]를 넘습니다.
    TooManyCommands {
        /// 요청 수.
        count: usize,
    },
    /// closed enum에 없는 command 종류입니다.
    UnknownCommand {
        /// 요청 index.
        index: usize,
        /// 종류.
        kind: String,
    },
    /// command 형식이 틀렸습니다.
    InvalidCommand {
        /// 요청 index.
        index: usize,
        /// 원인.
        reason: String,
    },
    /// 대상 문서를 읽을 수 없습니다.
    Document(DocumentReadError),
    /// command validator가 거부했습니다(전부 롤백).
    Rejected {
        /// 실패한 요청 index.
        index: usize,
        /// 원인.
        reason: String,
    },
    /// 결과 문서가 저장 불변식을 위반합니다(VI.10.1, 쓰지 않음).
    InvalidResult(String),
    /// semantic diff를 계산할 수 없습니다.
    Projection(String),
    /// 계획 뒤 다른 writer가 current generation을 바꿨습니다(쓰기 시작 전 거부).
    ConcurrentWrite,
    /// 저장 잠금을 잡지 못했습니다(사용 중이면 일시적 실패, XXIV.4.5).
    Lock(LockError),
    /// 저장 실패(쓰다 만 파일은 current에 연결되지 않음).
    Save(String),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::Open(e) => write!(f, "{e}"),
            ApplyError::BaseRevisionMismatch { expected, found } => write!(
                f,
                "base revision이 현재 revision과 다릅니다(요청 {expected}, 현재 {found})"
            ),
            ApplyError::EmptyTransaction => f.write_str("적용할 command가 없습니다"),
            ApplyError::TooManyCommands { count } => {
                write!(f, "command {count}개는 상한 {MAX_COMMANDS}개를 넘습니다")
            }
            ApplyError::UnknownCommand { index, kind } => {
                write!(f, "command #{index}: 알 수 없는 command 종류 {kind:?}")
            }
            ApplyError::InvalidCommand { index, reason } => {
                write!(f, "command #{index}: 형식 오류: {reason}")
            }
            ApplyError::Document(e) => write!(f, "{e}"),
            ApplyError::Rejected { index, reason } => {
                write!(f, "command #{index}가 거부되었습니다: {reason}")
            }
            ApplyError::InvalidResult(e) => write!(f, "적용 결과가 문서 불변식을 위반합니다: {e}"),
            ApplyError::Projection(e) => write!(f, "{e}"),
            ApplyError::ConcurrentWrite => f.write_str(
                "계획 뒤 다른 writer가 current generation을 바꿨습니다(아무것도 쓰지 않음)",
            ),
            ApplyError::Lock(e) => write!(f, "{e}"),
            ApplyError::Save(e) => write!(f, "저장 실패: {e}"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// 적용 결과.
#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    /// 적용 방식.
    pub mode: ApplyMode,
    /// 적용 전 revision.
    pub base_revision: ProjectRevision,
    /// 적용 후 revision. dry-run이면 base와 같습니다.
    pub revision: ProjectRevision,
    /// 대상 문서.
    pub document: StableId,
    /// 적용한 command 수.
    pub command_count: usize,
    /// 적용 전후 semantic diff.
    pub diff: DiffReport,
    /// 새로 기록한 chunk 수(dry-run이면 0).
    pub written_chunks: usize,
}

/// 요청 command를 closed enum으로 바꿉니다.
pub fn decode_commands(inputs: &[CommandInput]) -> Result<Vec<Command>, ApplyError> {
    if inputs.is_empty() {
        return Err(ApplyError::EmptyTransaction);
    }
    if inputs.len() > MAX_COMMANDS {
        return Err(ApplyError::TooManyCommands {
            count: inputs.len(),
        });
    }
    inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            input.to_command().map_err(|e| match e {
                CommandInputError::UnknownKind(kind) => ApplyError::UnknownCommand { index, kind },
                CommandInputError::Invalid(reason) => ApplyError::InvalidCommand { index, reason },
            })
        })
        .collect()
}

/// command id: 요청 내용 canonical bytes의 content hash 앞 16바이트(결정론적, 저장하지 않음).
fn command_id(document: StableId, commands: &[Command]) -> CommandId {
    let mut bytes = document.to_bytes().to_vec();
    for command in commands {
        bytes.extend_from_slice(&stellar_commands::codec::encode_command(command));
    }
    let digest = ContentHash::compute(&bytes);
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&digest.digest()[..16]);
    CommandId::from_stable_id(StableId::from_bytes(raw))
}

/// 요청을 적용합니다(모듈 문서 순서).
///
/// 쓰기 모드에서 `lock`이 [`LockPolicy::Required`]이면 프로젝트별 저장 잠금(XXIV.4.5)을 기다리지
/// 않고 잡은 **뒤에** generation을 열어 base revision을 비교하고, 저장을 마칠 때까지 잠금을 쥡니다.
/// 잠금이 사용 중이면 아무것도 읽거나 쓰지 않고 [`ApplyError::Lock`]입니다. dry-run은 쓰지 않으므로
/// 잠금을 잡지 않습니다.
pub fn apply(
    path: &Path,
    request: &ApplyRequest,
    mode: ApplyMode,
    lock: &LockPolicy,
) -> Result<ApplyOutcome, ApplyError> {
    let commands = decode_commands(&request.commands)?;
    let _guard = match (mode, lock) {
        (ApplyMode::Write, LockPolicy::Required { dir }) => {
            let probe = probe(path).map_err(ApplyError::Open)?;
            let project_id = parse_project_id(&probe.manifest.project.id).ok_or_else(|| {
                ApplyError::Open(OpenError::InvalidProjectId {
                    id: probe.manifest.project.id.clone(),
                })
            })?;
            Some(SaveLock::try_acquire(dir, project_id, &probe.root).map_err(ApplyError::Lock)?)
        }
        _ => None,
    };
    let snapshot = open_snapshot(path).map_err(ApplyError::Open)?;
    let base_revision = snapshot.revision();
    if base_revision != request.base_revision {
        return Err(ApplyError::BaseRevisionMismatch {
            expected: request.base_revision,
            found: base_revision,
        });
    }
    let model = snapshot
        .document_model(request.document)
        .map_err(ApplyError::Document)?;
    let mut before = model
        .to_records()
        .map_err(|e| ApplyError::Document(DocumentReadError::Records(e.to_string())))?;
    before.sort_canonical();

    let mut state = DocumentState::new(model);
    let base = DocumentRevision(0);
    let transaction = CommandTransaction {
        id: command_id(request.document, &commands),
        origin: CommandOrigin::User,
        requests: commands
            .iter()
            .cloned()
            .map(|command| CommandRequest::new(base, command))
            .collect(),
    };
    state
        .execute_transaction(&transaction)
        .map_err(|failure| ApplyError::Rejected {
            index: failure.index,
            reason: failure.error.to_string(),
        })?;
    let model = state.into_model();
    let mut after = model
        .to_records()
        .map_err(|e| ApplyError::InvalidResult(e.to_string()))?;
    after.sort_canonical();
    let diff = diff_records(&[&before], &[&after]).map_err(|e| ApplyError::Projection(e.0))?;

    if mode == ApplyMode::DryRun {
        return Ok(ApplyOutcome {
            mode,
            base_revision,
            revision: base_revision,
            document: request.document,
            command_count: commands.len(),
            diff,
            written_chunks: 0,
        });
    }

    let generation =
        snapshot
            .generation()
            .ok_or(ApplyError::Document(DocumentReadError::UnknownDocument {
                document: request.document,
            }))?;
    let manifest = generation.manifest();
    let manifest_hash = base_revision
        .manifest_hash()
        .ok_or(ApplyError::ConcurrentWrite)?;

    // 문서가 아닌 chunk는 base pointer 재사용, 모든 문서는 다시 직렬화(같은 내용은 재사용됨).
    let mut states: Vec<LogicalChunkState> = manifest
        .chunk_pointers
        .iter()
        .filter(|p| p.kind != KIND_DOCUMENT_MANIFEST && p.kind != KIND_VIEW_TREE)
        .map(|p| LogicalChunkState::Clean {
            chunk_id: p.chunk_id,
        })
        .collect();
    for definition in snapshot.documents() {
        let serialized = if definition.id == request.document {
            serialize_document(&model)
        } else {
            let other = snapshot
                .document_model(definition.id)
                .map_err(ApplyError::Document)?;
            serialize_document(&other)
        }
        .map_err(|e| ApplyError::InvalidResult(e.to_string()))?;
        states.extend(serialized.to_dirty_states());
    }
    let base = LoadedGeneration {
        manifest_hash,
        manifest: manifest.clone(),
    };
    let plan = build_save_plan(
        Some(&base),
        snapshot.project_id(),
        manifest.schema_version,
        &states,
        manifest.root_document_ids.clone(),
    )
    .map_err(|e| ApplyError::Save(e.to_string()))?;
    let written_chunks = plan.dirty_chunks.len();
    let mut memory = SavedGenerationState::new(Some((manifest.generation, manifest_hash)));
    run_save(snapshot.root(), &plan, &mut memory, &CancelToken::new()).map_err(|e| match e {
        SaveRunError::Exec(SaveExecError::BaseCheck(BaseCheckError::StaleBase { .. })) => {
            ApplyError::ConcurrentWrite
        }
        other => ApplyError::Save(other.to_string()),
    })?;
    let revision = match memory.saved() {
        Some((generation, manifest_hash)) => ProjectRevision::Saved {
            generation,
            manifest_hash,
        },
        None => return Err(ApplyError::Save("저장 결과 generation이 없습니다".into())),
    };
    Ok(ApplyOutcome {
        mode,
        base_revision,
        revision,
        document: request.document,
        command_count: commands.len(),
        diff,
        written_chunks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::open_snapshot;
    use crate::test_support::{
        file_digest, random_valid_document, save_documents, scaffold_project, TempDir,
    };
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;

    /// 프로젝트 root(`path`)와 프로젝트 밖 잠금 디렉터리(OS 사용자 데이터 대역)를 가진 fixture.
    struct ProjectDir {
        _tmp: TempDir,
        path: std::path::PathBuf,
        locks: std::path::PathBuf,
    }

    fn locks(dir: &ProjectDir) -> LockPolicy {
        LockPolicy::Required {
            dir: dir.locks.clone(),
        }
    }

    fn project_with(seed: u64) -> (ProjectDir, crate::test_support::FixtureDocument) {
        let tmp = TempDir::new("apply");
        let dir = ProjectDir {
            path: tmp.path.join("project"),
            locks: tmp.path.join("user-data").join("locks"),
            _tmp: tmp,
        };
        scaffold_project(&dir.path);
        let model = random_valid_document(seed);
        let other = random_valid_document(seed + 1000);
        save_documents(&dir.path, &[&model, &other]);
        (dir, model)
    }

    #[test]
    fn held_lock_makes_write_fail_without_touching_files_and_dry_run_skips_the_lock() {
        // 검증 기준(XXIV.4.5): 잠금 경합 시 일시적 실패, 파일 변경 0.
        let (dir, model) = project_with(7);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let request = ApplyRequest {
            base_revision: snapshot.revision(),
            document: model.document_id(),
            commands: vec![CommandInput::SetVisible {
                node: some_child(&model),
                visible: false,
            }],
        };
        let held = SaveLock::try_acquire(&dir.locks, snapshot.project_id(), &dir.path).unwrap();
        let before = file_digest(&dir.path);
        assert!(matches!(
            apply(&dir.path, &request, ApplyMode::Write, &locks(&dir)),
            Err(ApplyError::Lock(LockError::Busy { .. }))
        ));
        assert_eq!(file_digest(&dir.path), before);
        // dry-run은 쓰지 않으므로 잠금이 잡혀 있어도 동작합니다.
        apply(&dir.path, &request, ApplyMode::DryRun, &locks(&dir)).unwrap();
        drop(held);
        let outcome = apply(&dir.path, &request, ApplyMode::Write, &locks(&dir)).unwrap();
        assert_eq!(outcome.revision.generation(), 2);
        // 잠금 파일은 프로젝트 밖에만 있습니다.
        assert!(!file_digest(&dir.path).keys().any(|k| k.ends_with(".lock")));
    }

    #[test]
    fn concurrent_writers_lose_no_update() {
        // 검증 기준(XXIV.4.5): 동시 쓰기 fixture 손실 0. 여러 thread가 서로 다른 node 이름을 바꾸며
        // 잠금 사용 중·base 불일치(잠금 밖에서 읽은 base가 낡음)면 최신 revision으로 다시 시도합니다.
        // 모든 변경이 남아야 하고, 계획 뒤 교체(ConcurrentWrite)는 한 번도 없어야 합니다.
        let (dir, model) = project_with(7);
        let records = model.to_records().unwrap();
        let targets: Vec<StableId> = records
            .nodes
            .iter()
            .filter(|n| n.parent.is_some())
            .map(|n| n.id)
            .take(6)
            .collect();
        assert!(targets.len() >= 4, "{}", targets.len());
        let document = model.document_id();
        std::thread::scope(|scope| {
            for (i, node) in targets.iter().copied().enumerate() {
                let dir = &dir;
                scope.spawn(move || {
                    for _ in 0..10_000 {
                        let base = open_snapshot(&dir.path).unwrap().revision();
                        let request = ApplyRequest {
                            base_revision: base,
                            document,
                            commands: vec![CommandInput::Rename {
                                node,
                                name: format!("writer-{i}"),
                            }],
                        };
                        match apply(&dir.path, &request, ApplyMode::Write, &locks(dir)) {
                            Ok(_) => return,
                            Err(ApplyError::Lock(LockError::Busy { .. }))
                            | Err(ApplyError::BaseRevisionMismatch { .. }) => {
                                std::thread::yield_now();
                            }
                            // 잠금이 비교·교체를 감싸므로 V.5.2 optimistic check가 잡는 계획 뒤
                            // 교체는 headless writer끼리 생기면 안 됩니다(재시도하지 않고 실패).
                            Err(ApplyError::ConcurrentWrite) => {
                                panic!("writer {i}: 잠금 아래에서 계획 뒤 교체가 일어났습니다")
                            }
                            Err(e) => panic!("{e}"),
                        }
                    }
                    panic!("writer {i}가 끝나지 못했습니다");
                });
            }
        });
        let snapshot = open_snapshot(&dir.path).unwrap();
        assert_eq!(
            snapshot.revision().generation(),
            1 + targets.len() as u64,
            "writer마다 generation 하나"
        );
        let records = snapshot.document_records(document).unwrap();
        for (i, node) in targets.iter().enumerate() {
            let record = records.nodes.iter().find(|n| n.id == *node).unwrap();
            assert_eq!(record.display_name, format!("writer-{i}"), "손실 0");
        }
    }

    #[test]
    fn lock_policy_none_never_creates_the_lock_dir() {
        // merge driver 모드(LockPolicy::None)는 잠금을 쓰지 않습니다.
        let (dir, model) = project_with(7);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let request = ApplyRequest {
            base_revision: snapshot.revision(),
            document: model.document_id(),
            commands: vec![CommandInput::SetLocked {
                node: some_child(&model),
                locked: true,
            }],
        };
        apply(&dir.path, &request, ApplyMode::Write, &LockPolicy::None).unwrap();
        assert!(!dir.locks.exists());
    }

    fn some_child(model: &crate::test_support::FixtureDocument) -> StableId {
        model
            .to_records()
            .unwrap()
            .nodes
            .iter()
            .find(|n| n.parent.is_some())
            .map(|n| n.id)
            .expect("부모가 있는 node")
    }

    #[test]
    fn dry_run_reports_diff_without_changing_revision_or_files() {
        let (dir, model) = project_with(7);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let target = some_child(&model);
        let before = file_digest(&dir.path);
        let request = ApplyRequest {
            base_revision: snapshot.revision(),
            document: model.document_id(),
            commands: vec![CommandInput::Rename {
                node: target,
                name: "renamed".into(),
            }],
        };
        let outcome = apply(&dir.path, &request, ApplyMode::DryRun, &locks(&dir)).unwrap();
        assert_eq!(outcome.revision, outcome.base_revision);
        assert_eq!(outcome.written_chunks, 0);
        assert_eq!(outcome.diff.entries.len(), 1);
        assert_eq!(outcome.diff.entries[0].object, target);
        assert_eq!(file_digest(&dir.path), before);
        assert_eq!(
            open_snapshot(&dir.path).unwrap().revision(),
            snapshot.revision()
        );
    }

    #[test]
    fn write_switches_generation_and_reopen_matches_the_applied_model() {
        let (dir, model) = project_with(7);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let target = some_child(&model);
        let request = ApplyRequest {
            base_revision: snapshot.revision(),
            document: model.document_id(),
            commands: vec![
                CommandInput::SetVisible {
                    node: target,
                    visible: false,
                },
                CommandInput::SetLocked {
                    node: target,
                    locked: true,
                },
            ],
        };
        let outcome = apply(&dir.path, &request, ApplyMode::Write, &locks(&dir)).unwrap();
        assert_eq!(outcome.revision.generation(), 2);
        assert_ne!(outcome.revision, snapshot.revision());
        assert!(outcome.written_chunks >= 1);

        // 같은 command를 메모리에서 적용한 결과와 reopen 결과의 semantic snapshot이 같습니다.
        let mut expected = DocumentState::new(random_valid_document(7));
        let commands = decode_commands(&request.commands).unwrap();
        let transaction = CommandTransaction {
            id: command_id(model.document_id(), &commands),
            origin: CommandOrigin::User,
            requests: commands
                .into_iter()
                .map(|c| CommandRequest::new(DocumentRevision(0), c))
                .collect(),
        };
        expected.execute_transaction(&transaction).unwrap();
        let reopened = open_snapshot(&dir.path).unwrap();
        assert_eq!(reopened.revision(), outcome.revision);
        let loaded = reopened.document_model(model.document_id()).unwrap();
        assert_eq!(
            model_snapshot_json(&loaded).unwrap(),
            model_snapshot_json(expected.model()).unwrap()
        );
        // 다른 문서는 그대로입니다.
        let other = random_valid_document(7 + 1000);
        assert_eq!(
            model_snapshot_json(&reopened.document_model(other.document_id()).unwrap()).unwrap(),
            model_snapshot_json(&other).unwrap()
        );
    }

    #[test]
    fn base_revision_mismatch_unknown_and_rejected_commands_change_nothing() {
        let (dir, model) = project_with(7);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let target = some_child(&model);
        let before = file_digest(&dir.path);
        let ok = CommandInput::SetVisible {
            node: target,
            visible: false,
        };
        let stale = ApplyRequest {
            base_revision: ProjectRevision::Unsaved,
            document: model.document_id(),
            commands: vec![ok.clone()],
        };
        assert!(matches!(
            apply(&dir.path, &stale, ApplyMode::Write, &locks(&dir)),
            Err(ApplyError::BaseRevisionMismatch { .. })
        ));
        let unknown = ApplyRequest {
            base_revision: snapshot.revision(),
            document: model.document_id(),
            commands: vec![
                ok.clone(),
                CommandInput::Canonical {
                    kind: "run-script".into(),
                    bytes: vec![0xff, 0xff, 0, 0, 0, 0],
                },
            ],
        };
        assert!(matches!(
            apply(&dir.path, &unknown, ApplyMode::Write, &locks(&dir)),
            Err(ApplyError::UnknownCommand { index: 1, .. })
        ));
        // 등록된 이름이라도 bytes의 kind가 registry 밖이면 거부합니다.
        let forged = ApplyRequest {
            base_revision: snapshot.revision(),
            document: model.document_id(),
            commands: vec![CommandInput::Canonical {
                kind: "set-property".into(),
                bytes: vec![0xf0, 0xff, 0, 0, 0, 0],
            }],
        };
        assert!(matches!(
            apply(&dir.path, &forged, ApplyMode::Write, &locks(&dir)),
            Err(ApplyError::UnknownCommand { index: 0, .. })
        ));
        // 두 번째 command가 거부되면 첫 번째도 적용되지 않습니다.
        let rejected = ApplyRequest {
            base_revision: snapshot.revision(),
            document: model.document_id(),
            commands: vec![
                ok,
                CommandInput::DeleteSubtree {
                    root: StableId::new_v7(1, 1, 1),
                },
            ],
        };
        assert!(matches!(
            apply(&dir.path, &rejected, ApplyMode::Write, &locks(&dir)),
            Err(ApplyError::Rejected { index: 1, .. })
        ));
        let empty = ApplyRequest {
            base_revision: snapshot.revision(),
            document: model.document_id(),
            commands: Vec::new(),
        };
        assert!(matches!(
            apply(&dir.path, &empty, ApplyMode::Write, &locks(&dir)),
            Err(ApplyError::EmptyTransaction)
        ));
        assert_eq!(file_digest(&dir.path), before);
        assert_eq!(
            open_snapshot(&dir.path).unwrap().revision(),
            snapshot.revision()
        );
    }

    #[test]
    fn canonical_form_round_trips_every_field_form() {
        let node = StableId::new_v7(3, 4, 5);
        let inputs = [
            CommandInput::SetVisible {
                node,
                visible: true,
            },
            CommandInput::DeleteSubtree { root: node },
        ];
        for input in inputs {
            let command = input.to_command().unwrap();
            let canonical = CommandInput::Canonical {
                kind: command.kind().name().to_string(),
                bytes: stellar_commands::codec::encode_command(&command),
            };
            assert_eq!(canonical.to_command().unwrap(), command);
            // 종류 이름을 속이면 거부합니다.
            let lie = CommandInput::Canonical {
                kind: "move-node".into(),
                bytes: stellar_commands::codec::encode_command(&command),
            };
            assert!(matches!(
                lie.to_command(),
                Err(CommandInputError::Invalid(_))
            ));
        }
        assert!(matches!(
            CommandInput::MoveNode {
                node,
                new_parent: node,
                order_key: vec![0x80, 0],
            }
            .to_command(),
            Err(CommandInputError::Invalid(_))
        ));
    }
}
