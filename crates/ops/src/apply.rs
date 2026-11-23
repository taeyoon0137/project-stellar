//! headless 편집 적용 (XXIV.4.4).
//!
//! [`apply`]는 실행 중 앱 없이 프로젝트 파일에 CommandTransaction(VII.1.6)을 적용합니다.
//!
//! 1. snapshot을 열고(XXIV.4.2) 요청의 base revision과 현재 revision을 비교합니다. 다르면 아무것도
//!    읽거나 쓰지 않고 [`ApplyError::BaseRevisionMismatch`]입니다.
//! 2. 요청의 command 입력([`CommandInput`])을 commands crate의 **닫힌 enum** [`Command`]로만
//!    바꿉니다. 등록되지 않은 종류는 [`ApplyError::UnknownCommand`]로 거부합니다(raw patch·script 없음).
//!    컴포넌트 인스턴스 삽입([`CommandInput::InsertComponentInstance`])은 이때 프로젝트
//!    `design/components` 원본에서 정의를 찾고, 원본 파일 hash가 요청과 같을 때만 같은 transaction의
//!    InsertNode 하나가 됩니다. 다르면 [`ApplyError::ComponentSourceChanged`]입니다.
//!    registry 속성 설정([`CommandInput::SetProperty`])은 앞 command까지 적용한 문서에서 속성 패널·`stellar css`와
//!    같은 선언 붙여넣기 경로([`crate::css::paste_plan`])로 command가 됩니다. 속성은 registry 이름 또는
//!    numeric key(`0x0303`)로 받습니다(ADR-0075 개정). 속성 패널 행이 없는 CSS layout longhand(`display`,
//!    `padding-top` 등)는 그 경로가 node의 명시 layout record로 바꿉니다(ADR-0086).
//!    명시 layout longhand 두기·지우기([`CommandInput::SetLayoutProperty`]·[`CommandInput::UnsetProperty`])는 문서
//!    없이 `SetLayoutProperty`·`UnsetProperty` command 하나가 됩니다. 값은 텍스트 원본 `layout` 줄과 같은 문서
//!    parser(`LayoutLonghand::parse`)로 읽습니다.
//! 3. 대상 문서를 읽어 [`DocumentState::execute_transaction`]으로 전부 적용하거나 전부 롤백합니다.
//!    validator가 거부하면 [`ApplyError::Rejected`]입니다. 잠긴 노드를 바꾸는 결과도 같은 오류로
//!    거부합니다(D8, 앱 레이어 토글과 같은 규칙: 잠긴 조상 아래 노드는 바꾸지 않고, 잠금 해제는 그 노드의
//!    `set-locked`로만 합니다).
//! 4. 적용 전후 문서 레코드의 semantic diff(XIV.3)를 계산합니다. dry-run은 여기서 끝나며 파일을
//!    쓰지 않습니다.
//! 5. 쓰기 모드는 앱 창 저장과 같은 순서로 텍스트 원본(Git 원본)과 로컬 chunk cache를 함께 씁니다
//!    ([`crate::snapshot::save`], ADR-0113 결정 3): 결과 문서를 원본으로 인코딩하고, 바뀐 원본 파일만 저장
//!    journal 한 거래로 쓴 뒤, 모든 문서를 다시 직렬화해(VI.10.2) 같은 base generation 위의 SavePlan(V.5.1)을
//!    저장 파이프라인(V.5.2~V.5.5)으로 쓰고 stamp를 씁니다. 내용이 같은 chunk는 hash 비교로 재사용되고, 문서가 아닌
//!    chunk(project metadata·token·component·asset)는 base pointer를 그대로 씁니다. 연 뒤 다른 writer가 원본이나
//!    current pointer를 바꿨으면 쓰기 전에 거부합니다([`ApplyError::ConcurrentWrite`]). 텍스트 원본이 없는 chunk
//!    프로젝트는 generation만 씁니다([`crate::snapshot::save::SourceWrite::ChunkOnly`]).
//!
//! 6. transaction이 활성 기본값 세트(`SetStyleDefaults`, 모델 거울)를 바꿨으면 4번 뒤에 `stellar.yaml` 한 줄 쓰기를
//!    준비하고(세트 평탄화·연 시점 선택 확인, 실패하면 [`ApplyError::Rejected`]), 쓰기 모드는 generation 저장 뒤에
//!    그 한 줄을 씁니다(ADR-0087). dry-run은 준비만 합니다.
//! 7. 결과 문서 레코드와 Component Context가 적용 전과 같으면(세트만 바꾸는 `SetStyleDefaults`, 이미 같은 값 등) 5번의
//!    원본·cache 저장을 건너뜁니다(ADR-0128 결정 1). 세트를 바꿨으면 `stellar.yaml` 한 줄만 쓰고, Git diff는 그 한
//!    줄입니다(ADR-0082 결정 4). 결과 revision은 원본·cache revision에 새 세트 선택을 더한 값입니다(ADR-0128 결정 2).
//!
//! 쓰기 모드는 1번 전에 프로젝트별 교차 프로세스 저장 잠금([`crate::lock`], XXIV.4.5)을 잡고 저장을
//! 마칠 때까지 쥡니다. 잠금을 쥔 동안 같은 잠금을 쓰는 앱 창과 CLI는 들어오지 못하므로, 잠금 안에서 한
//! generation 비교와 pointer 교체 사이에 다른 저장이 끼어들 수 없습니다.

pub use stellar_commands::text_edit::ReplaceText;

use std::fmt;
use std::path::Path;

use stellar_commands::codec::{decode_command, CommandCodecError};
use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::kind::CommandKind;
use stellar_commands::node_ops::{DeleteSubtree, InsertNode, MoveNode};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::request::CommandRequest;
use stellar_commands::state::DocumentState;
use stellar_commands::style_defaults::{SetLayoutProperty, UnsetProperty};
use stellar_commands::transaction::CommandTransaction;
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::frame::keys as frame_keys;
use stellar_document::layout_record::LayoutLonghand;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_project::identity::parse_project_id;
use stellar_project::manifest_style_defaults::{prepare_selection, PreparedSelection};
use stellar_storage::hash::ContentHash;

use crate::core_types::ids::StableId;
use crate::core_types::order_key::OrderKey;
use crate::core_types::reference::ComponentRef;
use crate::core_types::revision::DocumentRevision;
use crate::core_types::value::{FiniteF32, Size2, Transform2D, Vec2};
use crate::diff::{diff_documents, DiffReport, DocumentInput};
use crate::error::{DocumentReadError, OpenError};
use crate::lock::{LockError, LockPolicy, SaveLock};
use crate::revision::ProjectRevision;
use crate::snapshot::save::{root_order, save_documents_selecting, SaveError, SourceWrite};
use crate::snapshot::{open_snapshot, probe, ProjectSnapshot};
use crate::text_source::TextSourceOperationError;

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
    /// Text node 본문의 scalar 범위를 교체합니다.
    ReplaceText(ReplaceText),
    /// Frame의 비음수 크기 변경. 기존 size 속성 command로 실행합니다.
    SetFrameSize {
        /// 대상 Frame.
        node: StableId,
        /// 유한·비음수 크기.
        size: Size2,
    },
    /// 다른 축은 유지하고 Frame의 너비만 바꿉니다.
    SetFrameWidth {
        /// 대상 Frame.
        node: StableId,
        /// 요청한 너비.
        width: FiniteF32,
    },
    /// 다른 축은 유지하고 Frame의 높이만 바꿉니다.
    SetFrameHeight {
        /// 대상 Frame.
        node: StableId,
        /// 요청한 높이.
        height: FiniteF32,
    },
    /// Frame의 local 아핀 변환 변경. 기존 transform 속성 command로 실행합니다.
    SetFrameTransform {
        /// 대상 Frame.
        node: StableId,
        /// 유한 성분의 아핀 변환.
        transform: Transform2D,
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
    /// 프로젝트 컴포넌트 원본의 정의 하나를 참조하는 인스턴스 삽입. 적용할 때 정의와 원본 hash를
    /// 확인한 뒤 InsertNode 하나가 됩니다([`apply`]).
    InsertComponentInstance(ComponentInstanceInsert),
    /// node 하나의 registry 속성을 CSS 값 텍스트로 설정합니다. `property`는 registry 이름(`opacity`,
    /// `border-top-left-radius`) 또는 numeric key(`0x0303`)입니다. 적용할 때 앞 command까지 적용한 문서에서
    /// `name: value` 선언 하나를 붙여넣기 경로로 command로 바꿉니다([`apply`]). 받지 못한 선언은 그 이유 코드로
    /// 거부합니다. 값이 이미 같으면 command가 없습니다.
    SetProperty {
        /// 대상 node.
        node: StableId,
        /// registry 이름 또는 `0x`로 시작하는 4자리 이상 16진 key.
        property: String,
        /// CSS 값 텍스트(`;`·`{`·`}` 없음).
        value: String,
    },
    /// node의 명시 layout longhand 하나를 둡니다(ADR-0086 결정 10). 세트 값과 같아도 명시로 남습니다. 속성 패널
    /// 행과 이름이 같은 longhand(`left`, Text의 `width`)도 layout record에 둡니다.
    SetLayoutProperty {
        /// 대상 node.
        node: StableId,
        /// CSS layout longhand 이름(`display`, `padding-top`, `grid-template-columns` …).
        property: String,
        /// CSS 값 텍스트.
        value: String,
    },
    /// node의 명시 layout longhand 하나를 지웁니다. 그 뒤 값은 활성 기본값 세트가 정합니다. 명시 값이 없으면
    /// 적용할 때 거부합니다.
    UnsetProperty {
        /// 대상 node.
        node: StableId,
        /// CSS layout longhand 이름.
        property: String,
    },
    /// 종류 이름 + canonical bytes(`encode_command`, VII.1.2).
    Canonical {
        /// 선언한 종류 이름([`CommandKind::name`]).
        kind: String,
        /// canonical bytes(kind `u16` + 길이 + payload).
        bytes: Vec<u8>,
    },
}

/// 프로젝트 `design/components` 원본의 정의 하나를 참조하는 인스턴스 삽입 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentInstanceInsert {
    /// 배치할 컴포넌트 정의 ID.
    pub component: StableId,
    /// 요청자가 읽은 정의 원본 파일 bytes hash(`stellar query . components`의 `source_hash`).
    pub source_hash: ContentHash,
    /// 인스턴스를 마지막 자식으로 넣을 부모 node.
    pub parent: StableId,
    /// 부모 좌표계 위치. 인스턴스의 Transform modifier 하나에 평행이동으로 저장합니다.
    pub position: Vec2,
}

/// 필드 형식으로 받는 종류 이름. 끝 11개는 Component Context 편집이며 `edit-context` canonical 입력이 됩니다
/// (`mcp_command_input::CONTEXT_FIELD_KINDS`, ADR-0114 결정 3).
pub const FIELD_KINDS: [&str; 25] = [
    "set-visible",
    "set-locked",
    "rename",
    "replace-text",
    "set-frame-size",
    "set-frame-width",
    "set-frame-height",
    "set-frame-transform",
    "delete-subtree",
    "move-node",
    "insert-component-instance",
    "set-property",
    "set-layout-property",
    "unset-property",
    "create-context-key",
    "rename-context-key",
    "set-context-key-default",
    "delete-context-key",
    "create-context-provision",
    "set-context-provision-value",
    "delete-context-provision",
    "add-context-requirement",
    "remove-context-requirement",
    "add-slot-context-key",
    "remove-slot-context-key",
];

/// layout longhand 이름을 풉니다. 모르는 이름은 `unknown-property`입니다.
fn layout_longhand(property: &str) -> Result<LayoutLonghand, CommandInputError> {
    LayoutLonghand::from_name(property).ok_or_else(|| {
        CommandInputError::Invalid(format!(
            "unknown-property: {property:?}는 layout longhand가 아닙니다"
        ))
    })
}

impl CommandInput {
    /// 단일 canonical command로 나타냅니다. 이전 size 묶음 bytes도 이 형식으로 유지합니다.
    /// 새 편집 transaction은 [`decode_commands`]를 통해 width·height로 펼칩니다.
    /// 컴포넌트 인스턴스 삽입은 프로젝트 원본이 있어야 command가 되므로 [`apply`]만 바꿉니다.
    pub fn to_command(&self) -> Result<Command, CommandInputError> {
        match self {
            CommandInput::InsertComponentInstance(_) => Err(CommandInputError::Invalid(
                "insert-component-instance는 프로젝트 컴포넌트 원본을 읽은 뒤에만 command가 됩니다"
                    .into(),
            )),
            CommandInput::SetProperty { .. } => Err(CommandInputError::Invalid(
                "set-property는 대상 문서를 읽은 뒤에만 command가 됩니다".into(),
            )),
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
            CommandInput::ReplaceText(edit) => Ok(Command::ReplaceText(edit.clone())),
            CommandInput::SetLayoutProperty {
                node,
                property,
                value,
            } => {
                let longhand = layout_longhand(property)?;
                let value = longhand.parse(value).map_err(|e| {
                    CommandInputError::Invalid(format!("{}: {property}: {}", e.code, e.message))
                })?;
                Ok(Command::SetLayoutProperty(SetLayoutProperty {
                    node: *node,
                    longhand,
                    value,
                }))
            }
            CommandInput::UnsetProperty { node, property } => {
                Ok(Command::UnsetProperty(UnsetProperty {
                    node: *node,
                    longhand: layout_longhand(property)?,
                }))
            }
            CommandInput::SetFrameSize { node, size } => Ok(Command::SetProperty(SetProperty {
                node: *node,
                key: frame_keys::SIZE,
                value: PropertyValue::Size(*size),
            })),
            CommandInput::SetFrameWidth { node, width } => Ok(Command::SetProperty(SetProperty {
                node: *node,
                key: frame_keys::WIDTH,
                value: PropertyValue::NonNegative(*width),
            })),
            CommandInput::SetFrameHeight { node, height } => {
                Ok(Command::SetProperty(SetProperty {
                    node: *node,
                    key: frame_keys::HEIGHT,
                    value: PropertyValue::NonNegative(*height),
                }))
            }
            CommandInput::SetFrameTransform { node, transform } => {
                Ok(Command::SetProperty(SetProperty {
                    node: *node,
                    key: frame_keys::TRANSFORM,
                    value: PropertyValue::Transform(*transform),
                }))
            }
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
    /// 컴포넌트 인스턴스 삽입의 정의가 프로젝트 원본에 없거나 원본 파일 hash가 요청과 다릅니다.
    /// 원본을 다시 읽고 재시도할 데이터 오류입니다.
    ComponentSourceChanged {
        /// 요청 index.
        index: usize,
        /// 요청한 정의.
        component: StableId,
    },
    /// 프로젝트 컴포넌트 원본(`design/components`)을 읽을 수 없습니다.
    ComponentSource(Box<TextSourceOperationError>),
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
            ApplyError::ComponentSourceChanged { index, component } => write!(
                f,
                "command #{index}: 컴포넌트 {component}의 원본이 없거나 source_hash가 다릅니다"
            ),
            ApplyError::ComponentSource(e) => write!(f, "컴포넌트 원본을 읽을 수 없습니다: {e}"),
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
    /// 적용 후 revision. dry-run이면 base와 같습니다. 문서가 그대로이고 세트만 바꿨으면 generation 번호는 그대로이고
    /// 세트 선택만 바뀐 revision입니다.
    pub revision: ProjectRevision,
    /// 대상 문서.
    pub document: StableId,
    /// 적용한 command 수.
    pub command_count: usize,
    /// 적용 전후 semantic diff.
    pub diff: DiffReport,
    /// 새로 기록한 chunk 수(dry-run·문서 레코드가 그대로면 0).
    pub written_chunks: usize,
    /// 원본 쓰기 결과(dry-run·문서 레코드가 그대로면 `None`, ADR-0113 결정 3·ADR-0128 결정 1).
    pub source: Option<SourceWrite>,
    /// 원본은 썼지만 로컬 cache를 맞추지 못했습니다(다음 쓰기가 맞춤).
    pub cache_stale: bool,
}

/// 요청 command 하나가 만드는 command. 컴포넌트 인스턴스 삽입만 프로젝트 원본을 읽은 뒤 정합니다.
enum Planned {
    Ready {
        index: usize,
        command: Command,
    },
    Instance {
        index: usize,
        insert: ComponentInstanceInsert,
    },
    Property {
        index: usize,
        node: StableId,
        declaration: String,
        /// numeric key로 받았으면 그 key. 이름이 node에서 가리키는 key와 같아야 합니다.
        key: Option<crate::core_types::property::PropertyKey>,
    },
}

/// `set-property`의 속성을 registry 이름으로 풉니다. numeric key는 registry 이름으로 바꾸고 그 key를 함께
/// 돌려줍니다(적용할 때 node 범위와 대조, [`check_numeric_key`]). 묶음 key는 `bundle-key-reference`로
/// 거부합니다(ADR-0075 결정 1).
fn property_name(
    property: &str,
) -> Result<(String, Option<crate::core_types::property::PropertyKey>), String> {
    use stellar_document::css::{bundle, registry};
    if let Some(hex) = property.strip_prefix("0x") {
        let key = u32::from_str_radix(hex, 16)
            .ok()
            .filter(|_| hex.len() >= 4 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(crate::core_types::property::PropertyKey)
            .ok_or_else(|| format!("{property}는 16진 property key가 아닙니다"))?;
        if let Some(longhands) = bundle::bundle_longhands(key) {
            let names: Vec<&str> = longhands
                .iter()
                .filter_map(|k| registry::name_of(*k))
                .collect();
            return Err(format!(
                "{}: {property}는 묶음 key입니다. longhand({})로 지정하세요",
                bundle::BUNDLE_KEY_REFERENCE,
                names.join(", ")
            ));
        }
        return registry::name_of(key)
            .map(|name| (name.to_owned(), Some(key)))
            .ok_or_else(|| format!("unknown-property: registry에 없는 key {property}"));
    }
    if property.is_empty()
        || !property
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!(
            "{property:?}는 registry 이름(소문자·숫자·-) 또는 0x key가 아닙니다"
        ));
    }
    Ok((property.to_owned(), None))
}

/// 이름 `name`이 `payload` node에서 가리키는 registry key. 붙여넣기 경로와 같은 순서로 node payload 범위, 공통
/// node 범위, modifier 범위에서 찾습니다(예: `width`는 Frame 0x0108·Rectangle 0x0301·Image 0x0602, `transform`은
/// Frame 0x0102·그 밖 0x1011).
fn key_on_node(
    payload: &NodePayload,
    name: &str,
) -> Option<crate::core_types::property::PropertyKey> {
    use stellar_document::css::registry::{key_of, PropertyScope};
    let scope = match payload {
        NodePayload::Frame(_) => Some(PropertyScope::Frame),
        NodePayload::Shape(_) => Some(PropertyScope::Shape),
        NodePayload::Path(_) => Some(PropertyScope::Path),
        NodePayload::Text(_) => Some(PropertyScope::Text),
        NodePayload::Image(_) => Some(PropertyScope::Image),
        NodePayload::Fragment(_) | NodePayload::ComponentInstance(_) => None,
    };
    scope
        .and_then(|scope| key_of(scope, name))
        .or_else(|| key_of(PropertyScope::Node, name))
        .or_else(|| key_of(PropertyScope::Modifier, name))
}

/// numeric key로 받은 `set-property`가 그 node에서 정확히 그 key를 가리키는지 확인합니다. 이름은 범위 안에서만
/// 유일하므로(`width`, `transform`) 다른 범위의 key(Frame node에 Shape `0x0301`)를 같은 이름의 다른 속성으로
/// 바꿔 쓰지 않고 `not-applicable`로 거부합니다.
fn check_numeric_key(
    model: &DocumentModel,
    index: usize,
    node: StableId,
    declaration: &str,
    key: crate::core_types::property::PropertyKey,
) -> Result<(), ApplyError> {
    let Some(record) = model.record(node) else {
        // 없는 node는 붙여넣기 경로가 그 이유로 거부합니다.
        return Ok(());
    };
    let name = declaration.split_once(':').map_or(declaration, |(n, _)| n);
    let target = key_on_node(&record.payload, name);
    if target == Some(key) {
        return Ok(());
    }
    Err(ApplyError::InvalidCommand {
        index,
        reason: format!(
            "not-applicable: {:?} node의 {name}는 key {} 입니다(받은 key 0x{:04x})",
            record.payload.kind(),
            target.map_or_else(|| "없음".to_owned(), |k| format!("0x{:04x}", k.0)),
            key.0
        ),
    })
}

/// `set-property` 입력을 선언 한 줄과 numeric key(받았으면)로 바꿉니다. 값은 선언 하나만 담을 수 있습니다.
fn declaration(
    property: &str,
    value: &str,
) -> Result<(String, Option<crate::core_types::property::PropertyKey>), String> {
    let (name, key) = property_name(property)?;
    if value.trim().is_empty() || value.contains([';', '{', '}']) {
        return Err(format!(
            "{name}의 값은 비어 있지 않은 CSS 값 하나여야 합니다(;·{{·}} 없음)"
        ));
    }
    Ok((format!("{name}: {value}"), key))
}

/// 프로젝트를 열기 전에 command 수와 형식을 검사합니다.
fn plan_commands(inputs: &[CommandInput]) -> Result<Vec<Planned>, ApplyError> {
    if inputs.is_empty() {
        return Err(ApplyError::EmptyTransaction);
    }
    if inputs.len() > MAX_COMMANDS {
        return Err(ApplyError::TooManyCommands {
            count: inputs.len(),
        });
    }
    let mut planned = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        match input {
            CommandInput::SetFrameSize { node, size } => {
                planned.extend(SetProperty::frame_dimensions(*node, *size).map(|set| {
                    Planned::Ready {
                        index,
                        command: Command::SetProperty(set),
                    }
                }))
            }
            CommandInput::InsertComponentInstance(insert) => planned.push(Planned::Instance {
                index,
                insert: insert.clone(),
            }),
            CommandInput::SetProperty {
                node,
                property,
                value,
            } => {
                let (declaration, key) = declaration(property, value)
                    .map_err(|reason| ApplyError::InvalidCommand { index, reason })?;
                planned.push(Planned::Property {
                    index,
                    node: *node,
                    declaration,
                    key,
                })
            }
            _ => planned.push(Planned::Ready {
                index,
                command: input.to_command().map_err(|e| match e {
                    CommandInputError::UnknownKind(kind) => {
                        ApplyError::UnknownCommand { index, kind }
                    }
                    CommandInputError::Invalid(reason) => {
                        ApplyError::InvalidCommand { index, reason }
                    }
                })?,
            }),
        }
    }
    Ok(planned)
}

/// 요청 command를 closed enum으로 바꿉니다. 컴포넌트 인스턴스 삽입은 프로젝트 원본이 필요하므로
/// 거부합니다([`apply`]가 원본을 확인해 바꿉니다).
pub fn decode_commands(inputs: &[CommandInput]) -> Result<Vec<Command>, ApplyError> {
    plan_commands(inputs)?
        .into_iter()
        .map(|planned| match planned {
            Planned::Ready { command, .. } => Ok(command),
            Planned::Instance { index, .. } => Err(ApplyError::InvalidCommand {
                index,
                reason: "insert-component-instance는 프로젝트 컴포넌트 원본이 필요합니다".into(),
            }),
            Planned::Property { index, .. } => Err(ApplyError::InvalidCommand {
                index,
                reason: "set-property는 대상 문서가 필요합니다".into(),
            }),
        })
        .collect()
}

/// 계획의 컴포넌트 인스턴스 삽입을 프로젝트 원본으로 확인해 InsertNode로 바꾸고, `set-property`를 앞 command까지
/// 적용한 문서에서 선언 붙여넣기로 command로 바꿉니다. 원본은 인스턴스 삽입이 있을 때만 한 번 읽습니다. 같은
/// 부모에 여러 개를 넣으면 요청 순서대로 뒤에 놓입니다. 결과의 두 번째 값은 command마다 요청 입력 index입니다.
fn resolve_commands(
    planned: Vec<Planned>,
    snapshot: &ProjectSnapshot,
    model: &DocumentModel,
) -> Result<(Vec<Command>, Vec<usize>), ApplyError> {
    let mut library = None;
    let mut last_keys = std::collections::BTreeMap::<StableId, OrderKey>::new();
    let mut commands = Vec::with_capacity(planned.len());
    let mut origins = Vec::with_capacity(planned.len());
    // 첫 `set-property`부터 앞 command를 적용한 사본 문서입니다.
    let mut scratch: Option<DocumentState> = None;
    for planned in planned {
        let (index, insert) = match planned {
            Planned::Ready { index, command } => {
                advance(&mut scratch, index, std::slice::from_ref(&command))?;
                commands.push(command);
                origins.push(index);
                continue;
            }
            Planned::Property {
                index,
                node,
                declaration,
                key,
            } => {
                if scratch.is_none() {
                    let mut state = DocumentState::new(copy_model(model)?);
                    // 앞 command의 거부는 그 command를 만든 입력 index로 알립니다.
                    for (command, origin) in commands.iter().zip(&origins) {
                        run(&mut state, *origin, std::slice::from_ref(command))?;
                    }
                    scratch = Some(state);
                }
                let state = scratch.as_mut().expect("방금 만듦");
                if let Some(key) = key {
                    check_numeric_key(state.model(), index, node, &declaration, key)?;
                }
                let pasted = paste_property(state.model(), index, node, &declaration)?;
                run(state, index, &pasted)?;
                origins.extend(std::iter::repeat_n(index, pasted.len()));
                commands.extend(pasted);
                continue;
            }
            Planned::Instance { index, insert } => (index, insert),
        };
        if library.is_none() {
            library = Some(
                crate::component_source::project_library(snapshot)
                    .map_err(|e| ApplyError::ComponentSource(Box::new(e)))?,
            );
        }
        let (library, files) = library.as_ref().expect("원본을 읽었음");
        let entry = library
            .get(ComponentRef::new(insert.component))
            .filter(|_| {
                files
                    .get(&insert.component)
                    .is_some_and(|(_, hash)| *hash == insert.source_hash)
            })
            .ok_or(ApplyError::ComponentSourceChanged {
                index,
                component: insert.component,
            })?;
        let order_key = match last_keys.get(&insert.parent) {
            Some(last) => OrderKey::between(Some(last), None),
            None => model.next_child_order_key(insert.parent),
        };
        last_keys.insert(insert.parent, order_key.clone());
        let ids = stellar_project::new_ids::new_ids()
            .map_err(|e| ApplyError::Save(format!("새 node ID를 만들 수 없습니다: {e}")))?;
        let command = Command::InsertNode(instance_node(
            index,
            &insert,
            entry.definition().display_name().into(),
            order_key,
            ids,
        )?);
        advance(&mut scratch, index, std::slice::from_ref(&command))?;
        commands.push(command);
        origins.push(index);
    }
    Ok((commands, origins))
}

/// 레코드를 거쳐 문서 사본을 만듭니다.
fn copy_model(model: &DocumentModel) -> Result<DocumentModel, ApplyError> {
    let records = model
        .to_records()
        .map_err(|e| ApplyError::Document(DocumentReadError::Records(e.to_string())))?;
    DocumentModel::from_records(records)
        .map(|(model, _)| model)
        .map_err(|e| ApplyError::Document(DocumentReadError::Records(format!("{e:?}"))))
}

/// 사본 문서가 있으면 command를 그 위에 적용합니다.
fn advance(
    scratch: &mut Option<DocumentState>,
    index: usize,
    commands: &[Command],
) -> Result<(), ApplyError> {
    match scratch {
        Some(state) => run(state, index, commands),
        None => Ok(()),
    }
}

/// 사본 문서에 command를 차례로 적용합니다. 실패는 그 입력의 거부입니다(본 적용과 같은 command).
fn run(state: &mut DocumentState, index: usize, commands: &[Command]) -> Result<(), ApplyError> {
    for command in commands {
        let request = CommandRequest::new(state.revision(), command.clone());
        state.execute(&request).map_err(|e| ApplyError::Rejected {
            index,
            reason: e.to_string(),
        })?;
    }
    Ok(())
}

/// 선언 하나를 `node`에 붙여넣는 command. 받지 못한 선언은 그 이유 코드로 거부합니다.
fn paste_property(
    model: &DocumentModel,
    index: usize,
    node: StableId,
    declaration: &str,
) -> Result<Vec<Command>, ApplyError> {
    let invalid = |reason: String| ApplyError::InvalidCommand { index, reason };
    let id_error = std::cell::RefCell::new(None);
    let mut new_id = || match stellar_project::new_ids::new_ids::<1>() {
        Ok([id]) => id,
        Err(error) => {
            id_error.borrow_mut().get_or_insert(error);
            StableId::NIL
        }
    };
    let plan = crate::css::paste_plan(model, &[node], declaration, &mut new_id)
        .map_err(|e| invalid(format!("{}: {}", e.code, e.message)))?;
    if let Some(error) = id_error.into_inner() {
        return Err(ApplyError::Save(format!(
            "새 modifier ID를 만들 수 없습니다: {error}"
        )));
    }
    if let Some(skipped) = plan.skipped.first() {
        return Err(invalid(format!(
            "{}: {} ({declaration})",
            skipped.code, skipped.message
        )));
    }
    Ok(plan.commands)
}

/// 정의를 참조하는 인스턴스 node와 위치 Transform modifier 하나.
fn instance_node(
    index: usize,
    insert: &ComponentInstanceInsert,
    name: String,
    order_key: OrderKey,
    [node, transform_id]: [StableId; 2],
) -> Result<InsertNode, ApplyError> {
    let invalid = |reason: String| ApplyError::InvalidCommand { index, reason };
    let mut command = InsertNode::single(NodeRecord {
        id: node,
        parent: Some(insert.parent),
        order_key,
        visible: true,
        locked: false,
        display_name: name,
        payload: NodePayload::ComponentInstance(ComponentInstancePayload::new(ComponentRef::new(
            insert.component,
        ))),
    });
    let transform = Transform2D::new(1., 0., 0., 1., insert.position.x(), insert.position.y())
        .map_err(|e| invalid(format!("위치: {e:?}")))?;
    let mut stack = ModifierStack::new(node);
    stack
        .insert(ModifierEntry {
            id: ModifierId::new(transform_id),
            order: OrderKey::between(None, None),
            kind: ModifierKind::Transform,
            enabled: true,
            payload: ModifierPayload::Transform(transform),
        })
        .map_err(|e| invalid(format!("위치 modifier: {e:?}")))?;
    command.modifier_stacks.push(stack);
    Ok(command)
}

/// SetFrameSize가 두 요청으로 펼쳐져도 오류 위치는 호출자가 보낸 입력 배열 기준입니다.
pub(crate) fn rejected_input(
    origins: &[usize],
    inputs: usize,
    failure: stellar_commands::transaction::TransactionError,
) -> ApplyError {
    let index = origins.get(failure.index).copied().unwrap_or(inputs);
    ApplyError::Rejected {
        index,
        reason: failure.error.to_string(),
    }
}

/// 적용 전 문서에서 잠긴 노드(또는 잠긴 조상 아래 노드)를 바꾼 결과의 첫 노드(D8).
///
/// 그 노드의 레코드·modifier stack 변경, 삭제, 그 아래로의 삽입·이동을 거부합니다. 예외는 조상이 잠기지 않은
/// 노드의 `locked`만 바뀐 경우(잠금·잠금 해제 명령)입니다. 판정은 앱과 같은 `EditIntent::blocked_by`이며, 잠금
/// 상태는 적용 전 레코드에서 `NodeLock::resolve`로 계산합니다(모델을 다시 만들지 않음). 적용 전 문서에 없던
/// 노드(이번 transaction이 삽입한 노드)는 잠금 대상이 아니고, 그 부모만 검사합니다.
fn locked_change(
    before: &stellar_document::records::DocumentRecords,
    after: &stellar_document::records::DocumentRecords,
) -> Option<StableId> {
    use std::collections::HashMap;
    use stellar_commands::lock::{EditIntent, NodeLock};
    let base: HashMap<_, _> = before.nodes.iter().map(|n| (n.id, n)).collect();
    let blocked = |id: StableId, intent: EditIntent| {
        NodeLock::resolve(id, base.len(), |id| {
            base.get(&id).map(|node| (node.locked, node.parent))
        })
        .is_some_and(|lock| intent.blocked_by(lock))
    };
    let next: HashMap<_, _> = after.nodes.iter().map(|n| (n.id, n)).collect();
    for old in &before.nodes {
        let Some(new) = next.get(&old.id) else {
            if blocked(old.id, EditIntent::Delete) {
                return Some(old.id);
            }
            continue;
        };
        if *new == old {
            continue;
        }
        let mut relocked = (*old).clone();
        relocked.locked = new.locked;
        let intent = if relocked == **new {
            EditIntent::ToggleLock
        } else {
            EditIntent::Edit
        };
        if blocked(old.id, intent)
            || (new.parent != old.parent
                && new.parent.is_some_and(|p| blocked(p, EditIntent::Edit)))
        {
            return Some(old.id);
        }
    }
    for new in &after.nodes {
        if !base.contains_key(&new.id) && new.parent.is_some_and(|p| blocked(p, EditIntent::Edit)) {
            return Some(new.id);
        }
    }
    let old_stacks: HashMap<_, _> = before.modifier_stacks.iter().map(|s| (s.node, s)).collect();
    let new_stacks: HashMap<_, _> = after.modifier_stacks.iter().map(|s| (s.node, s)).collect();
    old_stacks
        .keys()
        .chain(new_stacks.keys())
        .copied()
        .find(|node| {
            old_stacks.get(node) != new_stacks.get(node) && blocked(*node, EditIntent::Edit)
        })
}

/// 잠금 거부를 처음 일으킨 입력 위치. 거부 경로에서만 앞에서부터 command를 하나씩 더 적용하며 같은 검사를 다시
/// 합니다. 같은 노드나 조상을 가리키는 무관한 앞 command를 지목하지 않습니다. 찾지 못하면 입력 수입니다.
fn locked_input(
    snapshot: &crate::snapshot::ProjectSnapshot,
    document: StableId,
    before: &stellar_document::records::DocumentRecords,
    commands: &[Command],
) -> usize {
    (1..=commands.len())
        .find(|&end| {
            let Ok(model) = snapshot.document_model(document) else {
                return true;
            };
            let mut state = DocumentState::new(model);
            let transaction = CommandTransaction {
                id: command_id(document, &commands[..end]),
                origin: CommandOrigin::User,
                requests: commands[..end]
                    .iter()
                    .cloned()
                    .map(|command| CommandRequest::new(DocumentRevision(0), command))
                    .collect(),
            };
            if state.execute_transaction(&transaction).is_err() {
                return true;
            }
            state.into_model().to_records().map_or(true, |mut after| {
                after.sort_canonical();
                locked_change(before, &after).is_some()
            })
        })
        .map_or(commands.len(), |end| end - 1)
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
    let planned = plan_commands(&request.commands)?;
    let locked_root = match (mode, lock) {
        (ApplyMode::Write, LockPolicy::Required { dir }) => {
            // 잠금과 저장 모두 같은 실제 경로를 사용합니다.
            let root = std::fs::canonicalize(path)
                .map_err(|e| ApplyError::Open(OpenError::Path { kind: e.kind() }))?;
            let probe = probe(&root).map_err(ApplyError::Open)?;
            let project_id = parse_project_id(&probe.manifest.project.id).ok_or_else(|| {
                ApplyError::Open(OpenError::InvalidProjectId {
                    id: probe.manifest.project.id.clone(),
                })
            })?;
            let guard = SaveLock::try_acquire(dir, project_id, &root).map_err(ApplyError::Lock)?;
            Some((root, guard))
        }
        _ => None,
    };
    let path = locked_root
        .as_ref()
        .map_or(path, |(root, _)| root.as_path());
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
    // 결과 diff는 레코드와 Component Context를 함께 비교합니다(ADR-0114 결정 2). requirement·Slot 공개 목록은 프로젝트
    // 컴포넌트 library에 그 대상이 있을 때만 diff에 나타납니다. 원본을 읽지 못하면 diff는 문서 범위입니다(저장과 무관).
    let before_context = model.context().live(&model);
    let library = crate::component_source::project_library(&snapshot)
        .ok()
        .map(|(library, _)| library);
    let document_diff =
        |after: &stellar_document::records::DocumentRecords,
         after_context: &stellar_document::context::set::ContextSet| {
            diff_documents(
                &[DocumentInput {
                    records: &before,
                    context: &before_context,
                }],
                &[DocumentInput {
                    records: after,
                    context: after_context,
                }],
                library.as_ref(),
            )
            .map_err(|e| ApplyError::Projection(e.0))
        };
    let (commands, origins) = resolve_commands(planned, &snapshot, &model)?;
    if commands.is_empty() {
        // 모든 입력이 이미 같은 값인 `set-property`였습니다. 바꿀 것이 없어 저장하지 않습니다.
        let diff = document_diff(&before, &before_context)?;
        return Ok(ApplyOutcome {
            mode,
            base_revision,
            revision: base_revision,
            document: request.document,
            command_count: 0,
            diff,
            written_chunks: 0,
            source: None,
            cache_stale: false,
        });
    }

    // 복제는 원본을 바꾸지 않아 결과 비교로 잡히지 않으므로, 원본과 조상의 잠금을 적용 전에 봅니다(D8).
    for (index, command) in commands.iter().enumerate() {
        if let Command::DuplicateSubtree(duplicate) = command {
            let locked = stellar_commands::lock::NodeLock::of(&model, duplicate.source)
                .is_some_and(|lock| stellar_commands::lock::EditIntent::Duplicate.blocked_by(lock));
            if locked {
                return Err(ApplyError::Rejected {
                    index,
                    reason: format!("잠긴 노드 {}는 복제할 수 없습니다", duplicate.source),
                });
            }
        }
    }

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
        .map_err(|failure| rejected_input(&origins, request.commands.len(), failure))?;
    let model = state.into_model();
    let mut after = model
        .to_records()
        .map_err(|e| ApplyError::InvalidResult(e.to_string()))?;
    after.sort_canonical();
    if let Some(node) = locked_change(&before, &after) {
        return Err(ApplyError::Rejected {
            index: origins
                .get(locked_input(
                    &snapshot,
                    request.document,
                    &before,
                    &commands,
                ))
                .copied()
                .unwrap_or(request.commands.len()),
            reason: format!(
                "잠긴 노드 {node}는 바꿀 수 없습니다(잠금 해제는 그 노드의 set-locked)"
            ),
        });
    }
    let after_context = model.context().live(&model);
    let diff = document_diff(&after, &after_context)?;
    let selection = prepare_style_defaults(&snapshot, &model, &commands, &origins)?;

    if mode == ApplyMode::DryRun {
        return Ok(ApplyOutcome {
            mode,
            base_revision,
            revision: base_revision,
            document: request.document,
            command_count: request.commands.len(),
            diff,
            written_chunks: 0,
            source: None,
            cache_stale: false,
        });
    }

    // 결과 revision에 더할 세트 선택: 이번 transaction이 바꾼 선택, 아니면 연 시점 선택(ADR-0128 결정 2).
    let selected = selection
        .as_ref()
        .map_or(snapshot.style_defaults_name(), |s| s.name())
        .to_owned();
    let commit_selection = |selection: Option<PreparedSelection>, what: &str| {
        if let Some(selection) = selection {
            selection.commit(snapshot.root()).map_err(|e| {
                ApplyError::Save(format!(
                    "{what}stellar.yaml 기본값 세트를 쓰지 못했습니다: {e}"
                ))
            })?;
        }
        Ok::<(), ApplyError>(())
    };
    if before == after && before_context == after_context {
        // 문서 레코드와 Context가 그대로면 원본·cache를 쓰지 않습니다(ADR-0128 결정 1). 세트만 바꾸는 transaction의
        // 영속 결과는 `stellar.yaml` 한 줄입니다. 잠금 안에서 연 snapshot이라 base revision 비교 뒤 다른 저장이 끼지
        // 않았고, 세트 한 줄 쓰기는 준비 때 읽은 bytes hash가 그대로일 때만 씁니다.
        commit_selection(selection, "")?;
        return Ok(ApplyOutcome {
            mode,
            base_revision,
            revision: snapshot.storage_revision().with_style_defaults(&selected),
            document: request.document,
            command_count: request.commands.len(),
            diff,
            written_chunks: 0,
            source: None,
            cache_stale: false,
        });
    }

    // 원본·cache 저장(ADR-0113 결정 3): 모든 root 문서를 넘기고 바꾼 문서만 결과 모델로 바꿉니다.
    let roots = root_order(&snapshot);
    let mut models = Vec::with_capacity(roots.len());
    let mut edited = Some(model);
    for id in &roots {
        if *id == request.document {
            models.push(edited.take().ok_or(ApplyError::InvalidResult(
                "root 문서 목록에 같은 문서가 두 번 있습니다".into(),
            ))?);
        } else {
            models.push(snapshot.document_model(*id).map_err(ApplyError::Document)?);
        }
    }
    if edited.is_some() {
        return Err(ApplyError::Document(DocumentReadError::UnknownDocument {
            document: request.document,
        }));
    }
    let saved =
        save_documents_selecting(&snapshot, roots, models, &selected).map_err(|e| match e {
            SaveError::ConcurrentWrite => ApplyError::ConcurrentWrite,
            SaveError::Encode(e) => {
                ApplyError::InvalidResult(format!("텍스트 원본으로 쓸 수 없습니다: {e}"))
            }
            SaveError::Source(e) => ApplyError::Document(DocumentReadError::Records(format!(
                "텍스트 원본을 읽을 수 없습니다: {e}"
            ))),
            SaveError::Invalid(e) => ApplyError::InvalidResult(e),
            SaveError::Write(e) => ApplyError::Save(e),
        })?;
    let revision = saved.revision;
    let written_chunks = saved.written_chunks;
    commit_selection(selection, "generation은 저장했지만 ")?;
    Ok(ApplyOutcome {
        mode,
        base_revision,
        revision,
        document: request.document,
        command_count: request.commands.len(),
        diff,
        written_chunks,
        source: Some(saved.source),
        cache_stale: saved.cache_stale,
    })
}

/// transaction이 모델의 활성 기본값 세트 거울을 바꿨으면 `stellar.yaml` 한 줄 쓰기를 준비합니다(ADR-0087 결정 2).
/// 세트가 이 프로젝트에서 평탄화되지 않거나 연 뒤 선택이 바뀌었으면 마지막 `SetStyleDefaults` 입력의 거부입니다.
/// 준비는 파일을 쓰지 않습니다. 쓰기 모드는 generation 저장 뒤에 commit합니다.
fn prepare_style_defaults(
    snapshot: &ProjectSnapshot,
    model: &DocumentModel,
    commands: &[Command],
    origins: &[usize],
) -> Result<Option<PreparedSelection>, ApplyError> {
    let opened = snapshot.style_defaults_name();
    if model.style_defaults() == opened {
        return Ok(None);
    }
    let index = commands
        .iter()
        .rposition(|c| matches!(c, Command::SetStyleDefaults(_)))
        .and_then(|i| origins.get(i).copied())
        .unwrap_or(origins.len());
    prepare_selection(snapshot.root(), Some(opened), model.style_defaults())
        .map(Some)
        .map_err(|e| ApplyError::Rejected {
            index,
            reason: e.to_string(),
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
        // 잠긴 노드와 잠긴 조상 아래 노드의 이름은 바꿀 수 없습니다(D8).
        let unlocked = |mut id: StableId| loop {
            let Some(node) = records.nodes.iter().find(|n| n.id == id) else {
                return true;
            };
            if node.locked {
                return false;
            }
            match node.parent {
                Some(parent) => id = parent,
                None => return true,
            }
        };
        let targets: Vec<StableId> = records
            .nodes
            .iter()
            .filter(|n| n.parent.is_some() && unlocked(n.id))
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
    fn locked_nodes_reject_edits_but_accept_their_own_lock_change() {
        let (dir, model) = project_with(7);
        let records = model.to_records().unwrap();
        let target = some_child(&model);
        let child = |parent| {
            records
                .nodes
                .iter()
                .find(|n| n.parent == Some(parent))
                .map(|n| n.id)
        };
        let run = |commands: Vec<CommandInput>| {
            let snapshot = open_snapshot(&dir.path).unwrap();
            let request = ApplyRequest {
                base_revision: snapshot.revision(),
                document: model.document_id(),
                commands,
            };
            apply(&dir.path, &request, ApplyMode::Write, &locks(&dir))
        };
        run(vec![CommandInput::SetLocked {
            node: target,
            locked: true,
        }])
        .unwrap();
        let before = file_digest(&dir.path);
        for edit in [
            CommandInput::Rename {
                node: target,
                name: "잠김".into(),
            },
            CommandInput::SetVisible {
                node: target,
                visible: false,
            },
            CommandInput::DeleteSubtree { root: target },
        ] {
            assert!(matches!(
                run(vec![edit]),
                Err(ApplyError::Rejected { index: 0, .. })
            ));
        }
        if let Some(grandchild) = child(target) {
            assert!(matches!(
                run(vec![CommandInput::SetLocked {
                    node: grandchild,
                    locked: true,
                }]),
                Err(ApplyError::Rejected { index: 0, .. })
            ));
        }
        assert_eq!(
            file_digest(&dir.path),
            before,
            "거부는 파일을 바꾸지 않습니다"
        );
        // 거부 위치는 잠금을 처음 어긴 command입니다. 같은 노드를 가리키는 앞의 허용된 command가 아닙니다.
        assert!(matches!(
            run(vec![
                CommandInput::SetLocked {
                    node: target,
                    locked: true,
                },
                CommandInput::Rename {
                    node: target,
                    name: "잠김".into(),
                },
            ]),
            Err(ApplyError::Rejected { index: 1, .. })
        ));
        run(vec![CommandInput::SetLocked {
            node: target,
            locked: false,
        }])
        .unwrap();
        run(vec![CommandInput::Rename {
            node: target,
            name: "풀림".into(),
        }])
        .unwrap();
    }

    /// 잠금 판정 fixture: View root R 아래 잠긴 P → C → G, 잠기지 않은 U, 잠기지 않은 Q → 잠긴 L.
    struct LockTree {
        dir: ProjectDir,
        model: crate::test_support::FixtureDocument,
        root: StableId,
        p: StableId,
        c: StableId,
        g: StableId,
        u: StableId,
        q: StableId,
    }

    fn lock_tree() -> LockTree {
        use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
        use stellar_document::frame::FramePayload;
        let id = |n: u64| StableId::new_v7(1_950_000_000_000 + n, 0x0D8, n);
        let mut definition = DocumentDefinition::new(id(1), "잠금".into());
        let mut page = PageDefinition::new(id(2), "Page".into());
        page.add_view(ViewDefinition::new(id(3), "View".into(), id(10)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut model = DocumentModel::new(definition);
        for (n, parent, locked) in [
            (10, None, false),
            (11, Some(10), true),
            (12, Some(11), false),
            (13, Some(12), false),
            (14, Some(10), false),
            (15, Some(10), false),
            (16, Some(15), true),
        ] {
            let parent = parent.map(id);
            let order_key = parent.map_or(OrderKey::between(None, None), |p| {
                model.next_child_order_key(p)
            });
            model
                .insert_node(NodeRecord {
                    id: id(n),
                    parent,
                    order_key,
                    visible: true,
                    locked,
                    display_name: format!("n{n}"),
                    payload: NodePayload::Frame(FramePayload::default()),
                })
                .unwrap();
        }
        let tmp = TempDir::new("apply-lock");
        let dir = ProjectDir {
            path: tmp.path.join("project"),
            locks: tmp.path.join("user-data").join("locks"),
            _tmp: tmp,
        };
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&model]);
        LockTree {
            dir,
            model,
            root: id(10),
            p: id(11),
            c: id(12),
            g: id(13),
            u: id(14),
            q: id(15),
        }
    }

    fn canonical(command: Command) -> CommandInput {
        CommandInput::Canonical {
            kind: command.kind().name().to_string(),
            bytes: stellar_commands::codec::encode_command(&command),
        }
    }

    /// D8: 잠긴 node·잠긴 조상 아래 node의 변경·삭제, 잠긴 부모 아래 삽입·이동, 잠긴 부모 밖 이동, 같은 요청 안의
    /// 해제 뒤 편집, 잠긴 자손을 가진 삭제는 transaction 전체를 거부하고(dry-run 포함) 파일을 바꾸지 않습니다.
    /// 거부 index는 처음 어긴 command입니다. 조상이 잠기지 않은 node의 `locked` 토글만 허용합니다.
    #[test]
    fn lock_rules_reject_whole_transactions_at_the_first_violation() {
        let t = lock_tree();
        let run = |commands: Vec<CommandInput>, mode| {
            let snapshot = open_snapshot(&t.dir.path).unwrap();
            let request = ApplyRequest {
                base_revision: snapshot.revision(),
                document: t.model.document_id(),
                commands,
            };
            apply(&t.dir.path, &request, mode, &locks(&t.dir))
        };
        let order = |parent| t.model.next_child_order_key(parent).as_bytes().to_vec();
        let mut fresh = 0u64;
        let mut new_id = || {
            fresh += 1;
            StableId::new_v7(1_960_000_000_000 + fresh, 0x0D8, fresh)
        };
        let new_frame = |id, parent| NodeRecord {
            id,
            parent: Some(parent),
            order_key: t.model.next_child_order_key(parent),
            visible: true,
            locked: false,
            display_name: "new".into(),
            payload: NodePayload::Frame(stellar_document::frame::FramePayload::default()),
        };
        let rename = |node| CommandInput::Rename {
            node,
            name: "바뀜".into(),
        };
        let unlock_p = CommandInput::SetLocked {
            node: t.p,
            locked: false,
        };
        let duplicate_into_p = stellar_commands::duplicate::DuplicateSubtree::plan_with(
            &t.model,
            t.u,
            t.p,
            t.model.next_child_order_key(t.p),
            &mut new_id,
        )
        .unwrap();
        let cases: Vec<(&str, Vec<CommandInput>, usize)> = vec![
            ("잠긴 node 이름", vec![rename(t.p)], 0),
            (
                "잠긴 조상 아래 손자 이름",
                vec![rename(t.g), rename(t.u)],
                0,
            ),
            (
                "잠긴 조상 아래 손자 잠금",
                vec![CommandInput::SetLocked {
                    node: t.g,
                    locked: true,
                }],
                0,
            ),
            (
                "같은 요청 안의 해제 뒤 편집",
                vec![unlock_p.clone(), rename(t.p)],
                1,
            ),
            (
                "잠긴 자손을 가진 삭제",
                vec![rename(t.u), CommandInput::DeleteSubtree { root: t.q }],
                1,
            ),
            (
                "잠긴 부모 아래로 이동",
                vec![CommandInput::MoveNode {
                    node: t.u,
                    new_parent: t.p,
                    order_key: order(t.p),
                }],
                0,
            ),
            (
                "잠긴 부모 밖으로 이동",
                vec![CommandInput::MoveNode {
                    node: t.c,
                    new_parent: t.root,
                    order_key: order(t.root),
                }],
                0,
            ),
            (
                "잠긴 부모 아래 InsertNode",
                vec![canonical(Command::InsertNode(InsertNode::single(
                    new_frame(new_id(), t.p),
                )))],
                0,
            ),
            (
                "잠긴 부모 아래 DuplicateSubtree",
                vec![canonical(Command::DuplicateSubtree(duplicate_into_p))],
                0,
            ),
        ];
        let before = file_digest(&t.dir.path);
        for (name, commands, index) in cases {
            for mode in [ApplyMode::DryRun, ApplyMode::Write] {
                match run(commands.clone(), mode) {
                    Err(ApplyError::Rejected { index: got, reason }) => {
                        assert_eq!(got, index, "{name}: {reason}");
                        assert!(reason.contains("잠긴 노드"), "{name}: {reason}");
                    }
                    other => panic!("{name}: 거부가 아님: {other:?}"),
                }
            }
        }
        assert_eq!(
            file_digest(&t.dir.path),
            before,
            "거부는 파일을 바꾸지 않습니다"
        );
        // 조상이 잠기지 않은 node의 잠금 토글은 허용하고, 해제한 뒤에는 그 아래도 편집됩니다.
        run(vec![unlock_p], ApplyMode::Write).unwrap();
        run(vec![rename(t.p), rename(t.g)], ApplyMode::Write).unwrap();
    }

    /// D8: 잠긴 부모 아래 컴포넌트 인스턴스 삽입도 거부합니다(정의·원본 hash가 맞아도).
    #[test]
    fn component_instance_insert_under_a_locked_parent_is_rejected() {
        let (dir, model, component, hash) = project_with_component();
        let root = view_root(&model);
        let run = |commands: Vec<CommandInput>| {
            let request = ApplyRequest {
                base_revision: open_snapshot(&dir.path).unwrap().revision(),
                document: model.document_id(),
                commands,
            };
            apply(&dir.path, &request, ApplyMode::Write, &locks(&dir))
        };
        run(vec![CommandInput::SetLocked {
            node: root,
            locked: true,
        }])
        .unwrap();
        let before = file_digest(&dir.path);
        let insert = CommandInput::InsertComponentInstance(ComponentInstanceInsert {
            component,
            source_hash: hash,
            parent: root,
            position: Vec2::new(4., 8.).unwrap(),
        });
        assert!(matches!(
            run(vec![insert]),
            Err(ApplyError::Rejected { index: 0, reason }) if reason.contains("잠긴 노드")
        ));
        assert_eq!(file_digest(&dir.path), before);
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
    fn frame_geometry_commands_write_and_reopen_through_generation() {
        // set-frame-size·set-frame-transform은 dry-run과 같은 diff로 저장되고, reopen한 문서가
        // 메모리에서 기대한 Frame 크기·transform과 같습니다.
        use crate::core_types::value::{Size2, Transform2D};
        use crate::test_support::large_document;
        use stellar_document::records::NodePayload;

        let tmp = TempDir::new("apply-frame-geometry");
        let dir = ProjectDir {
            path: tmp.path.join("project"),
            locks: tmp.path.join("user-data").join("locks"),
            _tmp: tmp,
        };
        let model = large_document(4);
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&model]);
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
        let request = ApplyRequest {
            base_revision: open_snapshot(&dir.path).unwrap().revision(),
            document: model.document_id(),
            commands: vec![
                CommandInput::SetFrameSize { node, size },
                CommandInput::SetFrameTransform { node, transform },
            ],
        };
        let before = file_digest(&dir.path);
        let preview = apply(&dir.path, &request, ApplyMode::DryRun, &locks(&dir)).unwrap();
        assert_eq!(file_digest(&dir.path), before);
        assert_eq!(preview.command_count, 2);
        assert_eq!(preview.diff.entries.len(), 3);
        let saved = apply(&dir.path, &request, ApplyMode::Write, &locks(&dir)).unwrap();
        assert_eq!(saved.diff.entries, preview.diff.entries);
        let mut reopened = open_snapshot(&dir.path)
            .unwrap()
            .document_model(request.document)
            .unwrap()
            .to_records()
            .unwrap();
        reopened.sort_canonical();
        assert_eq!(reopened, expected);
    }

    #[test]
    fn set_property_accepts_registry_names_and_numeric_keys_through_the_paste_path() {
        use crate::test_support::large_document;

        let tmp = TempDir::new("apply-set-property");
        let dir = ProjectDir {
            path: tmp.path.join("project"),
            locks: tmp.path.join("user-data").join("locks"),
            _tmp: tmp,
        };
        let model = large_document(4);
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&model]);
        let records = model.to_records().unwrap();
        let node = records
            .nodes
            .iter()
            .find(|n| n.parent.is_none())
            .unwrap()
            .id;
        let revision = || open_snapshot(&dir.path).unwrap().revision();
        let request = |commands: Vec<CommandInput>| ApplyRequest {
            base_revision: revision(),
            document: model.document_id(),
            commands,
        };
        let set = |property: &str, value: &str| CommandInput::SetProperty {
            node,
            property: property.into(),
            value: value.into(),
        };
        // 이름(`width`)과 numeric key(`0x0109` = height)를 한 transaction으로 받습니다.
        let saved = apply(
            &dir.path,
            &request(vec![
                set("width", "321px"),
                set("0x0109", "123.5px"),
                set("background-color", "#ff0000"),
            ]),
            ApplyMode::Write,
            &locks(&dir),
        )
        .unwrap();
        assert_eq!(saved.command_count, 3);
        let labels: Vec<(Option<&str>, Option<&str>)> = saved
            .diff
            .entries
            .iter()
            .map(|e| (e.path.as_deref(), e.key.as_deref()))
            .collect();
        assert_eq!(
            labels,
            vec![
                (Some("background-color"), Some("0x0104")),
                (Some("width"), Some("0x0108")),
                (Some("height"), Some("0x0109")),
            ]
        );
        let reopened = open_snapshot(&dir.path)
            .unwrap()
            .document_model(model.document_id())
            .unwrap();
        let detail = crate::query::node_detail(
            &open_snapshot(&dir.path).unwrap(),
            model.document_id(),
            node,
        )
        .unwrap();
        assert!(detail
            .properties
            .iter()
            .any(|p| p.path == "width" && p.key == "0x0108"));
        let stellar_document::records::NodePayload::Frame(frame) =
            &reopened.record(node).unwrap().payload
        else {
            panic!("root Frame");
        };
        assert_eq!((frame.size.width(), frame.size.height()), (321.0, 123.5));
        // 이미 같은 값이면 command가 없고 저장하지 않습니다.
        let before = file_digest(&dir.path);
        let same = apply(
            &dir.path,
            &request(vec![set("width", "321px")]),
            ApplyMode::Write,
            &locks(&dir),
        )
        .unwrap();
        assert_eq!(same.command_count, 0);
        assert!(same.diff.entries.is_empty());
        assert_eq!(file_digest(&dir.path), before);
        // 묶음 key, 모르는 이름, 두 선언을 담은 값, 받지 않는 값은 그 입력 index의 invalid-command입니다.
        for (input, code) in [
            (set("0x0101", "1px"), "bundle-key-reference"),
            (set("0x0106", "1px"), "bundle-key-reference"),
            (set("no-such-property", "1px"), "unknown-property"),
            (set("width", "1px; height: 2px"), "CSS 값 하나"),
            (set("width", "1em"), "unsupported-unit"),
            (set("Width", "1px"), "registry 이름"),
            // 이름이 같아도 다른 범위의 key는 이 node의 속성이 아닙니다(Frame에 Shape width, Transform modifier).
            (set("0x0301", "1px"), "not-applicable"),
            (set("0x1011", "none"), "not-applicable"),
            // 값이 이미 같아 command가 없어도 범위 검사는 합니다.
            (set("0x0301", "321px"), "not-applicable"),
        ] {
            let error = apply(
                &dir.path,
                &request(vec![set("width", "10px"), input]),
                ApplyMode::DryRun,
                &locks(&dir),
            )
            .unwrap_err();
            let ApplyError::InvalidCommand { index, reason } = &error else {
                panic!("{code}: {error:?}");
            };
            assert_eq!(*index, 1, "{reason}");
            assert!(reason.contains(code), "{code}: {reason}");
        }
        assert_eq!(file_digest(&dir.path), before);
        // Frame 자기 범위의 numeric key는 받습니다(`0x0102` = Frame transform).
        let frame_transform = apply(
            &dir.path,
            &request(vec![set("0x0102", "none")]),
            ApplyMode::DryRun,
            &locks(&dir),
        )
        .unwrap();
        assert_eq!(frame_transform.command_count, 0);
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
            CommandInput::SetFrameSize {
                node,
                size: Size2::new(320.5, 240.25).unwrap(),
            },
            CommandInput::SetFrameTransform {
                node,
                transform: Transform2D::new(1.0, 0.0, 0.0, -1.0, 12.5, -24.25).unwrap(),
            },
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

    /// View root Frame 하나짜리 문서와 `design/components`의 정의 하나를 가진 프로젝트.
    fn project_with_component() -> (
        ProjectDir,
        crate::test_support::FixtureDocument,
        StableId,
        ContentHash,
    ) {
        use stellar_project::document_io::component_source::{self, ComponentSource};
        let tmp = TempDir::new("apply-instance");
        let dir = ProjectDir {
            path: tmp.path.join("project"),
            locks: tmp.path.join("user-data").join("locks"),
            _tmp: tmp,
        };
        scaffold_project(&dir.path);
        let model = crate::test_support::large_document(3);
        save_documents(&dir.path, &[&model]);
        let folder = dir.path.join("design/components");
        std::fs::create_dir(&folder).unwrap();
        let source = ComponentSource::new(
            StableId::new_v7(30, 0, 1),
            "Card".into(),
            StableId::new_v7(30, 0, 2),
        )
        .unwrap();
        let (_, hash) = component_source::create(&folder, &source).unwrap();
        (dir, model, StableId::new_v7(30, 0, 1), hash)
    }

    fn view_root(model: &crate::test_support::FixtureDocument) -> StableId {
        model.definition().pages[0].views[0].root_frame
    }

    #[test]
    fn component_instances_insert_in_one_transaction_and_check_the_source_hash() {
        let (dir, model, component, hash) = project_with_component();
        let root = view_root(&model);
        let base = open_snapshot(&dir.path).unwrap().revision();
        let insert = |x: f32, y: f32| {
            CommandInput::InsertComponentInstance(ComponentInstanceInsert {
                component,
                source_hash: hash,
                parent: root,
                position: Vec2::new(x, y).unwrap(),
            })
        };
        let request = |commands| ApplyRequest {
            base_revision: base,
            document: model.document_id(),
            commands,
        };

        // 원본이 없거나 hash가 다르면 아무것도 쓰지 않습니다.
        let before = file_digest(&dir.path);
        let mut stale = insert(0., 0.);
        if let CommandInput::InsertComponentInstance(edit) = &mut stale {
            edit.source_hash = ContentHash::compute(b"other");
        }
        let mut missing = insert(0., 0.);
        if let CommandInput::InsertComponentInstance(edit) = &mut missing {
            edit.component = StableId::new_v7(31, 0, 1);
        }
        for (commands, failed) in [(vec![insert(1., 1.), stale], 1), (vec![missing], 0)] {
            for mode in [ApplyMode::DryRun, ApplyMode::Write] {
                assert!(matches!(
                    apply(&dir.path, &request(commands.clone()), mode, &locks(&dir)),
                    Err(ApplyError::ComponentSourceChanged { index, component: c })
                        if index == failed && (c == component || failed == 0)
                ));
            }
        }
        // 뒤 command가 거부되면 앞의 인스턴스 삽입도 적용되지 않습니다.
        assert!(matches!(
            apply(
                &dir.path,
                &request(vec![
                    insert(1., 1.),
                    CommandInput::DeleteSubtree {
                        root: StableId::new_v7(1, 1, 1),
                    },
                ]),
                ApplyMode::Write,
                &locks(&dir),
            ),
            Err(ApplyError::Rejected { index: 1, .. })
        ));
        assert_eq!(file_digest(&dir.path), before);
        assert!(decode_commands(&[insert(0., 0.)]).is_err());

        // 같은 부모에 두 개와 이름 변경을 한 transaction(한 generation)으로 씁니다.
        let outcome = apply(
            &dir.path,
            &request(vec![
                insert(230.5, -10.),
                CommandInput::Rename {
                    node: root,
                    name: "Placed".into(),
                },
                insert(30., 20.),
            ]),
            ApplyMode::Write,
            &locks(&dir),
        )
        .unwrap();
        assert_eq!(outcome.revision.generation(), base.generation() + 1);
        assert_eq!(outcome.command_count, 3);
        let reopened = open_snapshot(&dir.path)
            .unwrap()
            .document_model(model.document_id())
            .unwrap();
        let placed = reopened.references().inbound(component);
        assert_eq!(placed.len(), 2);
        let records = reopened.to_records().unwrap();
        let mut found = Vec::new();
        for node in &placed {
            let record = records.nodes.iter().find(|n| n.id == *node).unwrap();
            assert_eq!(record.parent, Some(root));
            assert_eq!(record.display_name, "Card");
            assert_eq!(
                record.payload,
                NodePayload::ComponentInstance(ComponentInstancePayload::new(ComponentRef::new(
                    component
                )))
            );
            let [entry] = reopened.modifier_stack(*node).unwrap().entries() else {
                panic!("위치 modifier 하나");
            };
            let ModifierPayload::Transform(transform) = entry.payload else {
                panic!("Transform modifier");
            };
            found.push((record.order_key.clone(), transform));
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            found.iter().map(|(_, t)| *t).collect::<Vec<_>>(),
            [
                Transform2D::new(1., 0., 0., 1., 230.5, -10.).unwrap(),
                Transform2D::new(1., 0., 0., 1., 30., 20.).unwrap(),
            ],
            "요청 순서대로 마지막 자식 뒤에 놓임"
        );
        let renamed = records.nodes.iter().find(|n| n.id == root).unwrap();
        assert_eq!(renamed.display_name, "Placed");
    }

    #[test]
    fn layout_longhands_are_set_and_unset_through_apply_and_saved() {
        use crate::test_support::large_document;
        use stellar_document::layout_record::LayoutLonghand;

        let tmp = TempDir::new("apply-layout-property");
        let dir = ProjectDir {
            path: tmp.path.join("project"),
            locks: tmp.path.join("user-data").join("locks"),
            _tmp: tmp,
        };
        let model = large_document(4);
        scaffold_project(&dir.path);
        save_documents(&dir.path, &[&model]);
        let records = model.to_records().unwrap();
        let root = records
            .nodes
            .iter()
            .find(|n| n.parent.is_none())
            .unwrap()
            .id;
        let fragment = records
            .nodes
            .iter()
            .find(|n| matches!(n.payload, NodePayload::Fragment(_)))
            .unwrap()
            .id;
        let request = |commands: Vec<CommandInput>| ApplyRequest {
            base_revision: open_snapshot(&dir.path).unwrap().revision(),
            document: model.document_id(),
            commands,
        };
        let layout = |property: &str, value: &str| CommandInput::SetLayoutProperty {
            node: root,
            property: property.into(),
            value: value.into(),
        };
        let reopened = || {
            open_snapshot(&dir.path)
                .unwrap()
                .document_model(model.document_id())
                .unwrap()
        };
        // 명시 longhand 두기와, 속성 패널 행이 없는 layout longhand를 set-property로 두기를 한 transaction으로.
        let saved = apply(
            &dir.path,
            &request(vec![
                layout("display", "flex"),
                layout("grid-template-columns", "1fr 2fr"),
                CommandInput::SetProperty {
                    node: root,
                    property: "padding-top".into(),
                    value: "4px".into(),
                },
            ]),
            ApplyMode::Write,
            &locks(&dir),
        )
        .unwrap();
        assert_eq!(saved.command_count, 3);
        assert_eq!(saved.diff.entries.len(), 3, "{:?}", saved.diff.entries);
        let after = reopened();
        let record = after.layout_record(root).unwrap();
        assert_eq!(
            record.get(LayoutLonghand::Display),
            Some(&LayoutLonghand::Display.parse("flex").unwrap())
        );
        assert_eq!(
            record.get(LayoutLonghand::PaddingTop),
            Some(&LayoutLonghand::PaddingTop.parse("4px").unwrap())
        );
        // 지우기: 명시 값이 사라지고 세트 값으로 돌아갑니다.
        apply(
            &dir.path,
            &request(vec![CommandInput::UnsetProperty {
                node: root,
                property: "display".into(),
            }]),
            ApplyMode::Write,
            &locks(&dir),
        )
        .unwrap();
        let after = reopened();
        assert!(after
            .layout_record(root)
            .unwrap()
            .get(LayoutLonghand::Display)
            .is_none());
        // 모르는 longhand·받지 않는 값은 입력 오류, 명시 값 없는 지우기·Fragment의 record는 적용 거부입니다.
        let before = file_digest(&dir.path);
        for (input, invalid) in [
            (layout("no-such", "1"), true),
            (layout("display", "var(--x)"), true),
            (layout("column-gap", "1em"), true),
            (
                CommandInput::UnsetProperty {
                    node: root,
                    property: "display".into(),
                },
                false,
            ),
            (
                CommandInput::SetLayoutProperty {
                    node: fragment,
                    property: "flex-grow".into(),
                    value: "1".into(),
                },
                false,
            ),
        ] {
            let error = apply(
                &dir.path,
                &request(vec![input]),
                ApplyMode::Write,
                &locks(&dir),
            )
            .unwrap_err();
            if invalid {
                assert!(
                    matches!(error, ApplyError::InvalidCommand { index: 0, .. }),
                    "{error:?}"
                );
            } else {
                assert!(
                    matches!(error, ApplyError::Rejected { index: 0, .. }),
                    "{error:?}"
                );
            }
        }
        assert_eq!(file_digest(&dir.path), before);
    }

    fn set_style_defaults(name: &str) -> CommandInput {
        canonical(Command::SetStyleDefaults(
            stellar_commands::style_defaults::SetStyleDefaults { name: name.into() },
        ))
    }

    /// 두 digest에서 내용이 다른 파일(추가·삭제 포함) 목록.
    fn changed_files(
        before: &std::collections::BTreeMap<String, String>,
        after: &std::collections::BTreeMap<String, String>,
    ) -> Vec<String> {
        before
            .keys()
            .chain(after.keys())
            .filter(|name| before.get(*name) != after.get(*name))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[test]
    fn a_style_defaults_only_transaction_writes_one_manifest_line_and_no_generation() {
        // ADR-0128 결정 1·2: 문서 레코드가 그대로인 transaction은 generation을 쓰지 않습니다. 세트만 바꾸면 바뀌는
        // 파일은 stellar.yaml 하나, 한 줄이고, revision은 generation 번호를 둔 채 세트 선택만 바뀝니다.
        let (dir, model) = project_with(7);
        let manifest = dir.path.join("stellar.yaml");
        let opened = open_snapshot(&dir.path).unwrap();
        let base = opened.revision();
        let original = std::fs::read_to_string(&manifest).unwrap();
        let before = file_digest(&dir.path);
        let request = |base_revision, name: &str| ApplyRequest {
            base_revision,
            document: model.document_id(),
            commands: vec![set_style_defaults(name)],
        };
        let outcome = apply(
            &dir.path,
            &request(base, "react-native"),
            ApplyMode::Write,
            &locks(&dir),
        )
        .unwrap();
        assert_eq!(outcome.written_chunks, 0);
        assert!(outcome.source.is_none());
        assert!(
            outcome.diff.entries.is_empty(),
            "{:?}",
            outcome.diff.entries
        );
        assert_eq!(
            changed_files(&before, &file_digest(&dir.path)),
            ["stellar.yaml"]
        );
        let after = std::fs::read_to_string(&manifest).unwrap();
        assert_eq!(after, format!("{original}style_defaults: react-native\n"));
        // revision: generation은 그대로, 세트 선택이 바뀌어 표기가 다릅니다. 다시 열면 같은 값입니다.
        assert_eq!(outcome.revision.generation(), base.generation());
        assert_ne!(outcome.revision, base);
        let reopened = open_snapshot(&dir.path).unwrap();
        assert_eq!(reopened.revision(), outcome.revision);
        assert_eq!(reopened.storage_revision(), opened.storage_revision());
        assert_eq!(reopened.style_defaults_name(), "react-native");
        assert_eq!(
            outcome.revision,
            opened
                .storage_revision()
                .with_style_defaults("react-native")
        );
        // 옛 base로는 쓸 수 없습니다(다른 writer의 세트 변경을 base revision이 잡음).
        assert!(matches!(
            apply(&dir.path, &request(base, "css"), ApplyMode::Write, &locks(&dir)),
            Err(ApplyError::BaseRevisionMismatch { expected, found })
                if expected == base && found == outcome.revision
        ));
        // 되돌리면 stellar.yaml bytes와 revision이 처음과 같습니다.
        let back = apply(
            &dir.path,
            &request(outcome.revision, "css"),
            ApplyMode::Write,
            &locks(&dir),
        )
        .unwrap();
        assert_eq!(back.revision, base);
        assert_eq!(file_digest(&dir.path), before);
        // 없는 세트는 거부하고 아무것도 쓰지 않습니다.
        assert!(matches!(
            apply(
                &dir.path,
                &request(base, "nowhere"),
                ApplyMode::Write,
                &locks(&dir)
            ),
            Err(ApplyError::Rejected { index: 0, .. })
        ));
        assert_eq!(file_digest(&dir.path), before);
    }

    #[test]
    fn an_unchanged_result_writes_nothing() {
        // ADR-0128 결정 1: 이미 같은 값으로 바꾸는 transaction은 원본·cache를 쓰지 않고 revision도 그대로입니다.
        let (dir, model) = project_with(7);
        let base = open_snapshot(&dir.path).unwrap().revision();
        let node = some_child(&model);
        let visible = model
            .to_records()
            .unwrap()
            .nodes
            .iter()
            .find(|n| n.id == node)
            .unwrap()
            .visible;
        let before = file_digest(&dir.path);
        let outcome = apply(
            &dir.path,
            &ApplyRequest {
                base_revision: base,
                document: model.document_id(),
                commands: vec![CommandInput::SetVisible { node, visible }],
            },
            ApplyMode::Write,
            &locks(&dir),
        )
        .unwrap();
        assert_eq!(outcome.revision, base);
        assert_eq!(outcome.written_chunks, 0);
        assert!(outcome.source.is_none());
        assert_eq!(file_digest(&dir.path), before);
    }

    #[test]
    fn a_document_edit_with_a_set_change_saves_a_generation_and_the_manifest_line() {
        // 문서도 바뀌면 generation을 쓰고 그 뒤에 stellar.yaml 한 줄을 씁니다. 결과 revision은 새 generation에 새 세트
        // 선택을 더한 값이고 다시 열었을 때와 같습니다.
        let (dir, model) = project_with(7);
        let base = open_snapshot(&dir.path).unwrap().revision();
        let outcome = apply(
            &dir.path,
            &ApplyRequest {
                base_revision: base,
                document: model.document_id(),
                commands: vec![
                    CommandInput::Rename {
                        node: some_child(&model),
                        name: "renamed".into(),
                    },
                    set_style_defaults("react-native"),
                ],
            },
            ApplyMode::Write,
            &locks(&dir),
        )
        .unwrap();
        assert_eq!(outcome.revision.generation(), base.generation() + 1);
        let reopened = open_snapshot(&dir.path).unwrap();
        assert_eq!(reopened.style_defaults_name(), "react-native");
        assert_eq!(reopened.revision(), outcome.revision);
        assert_eq!(
            reopened.revision(),
            reopened
                .storage_revision()
                .with_style_defaults("react-native")
        );
        assert_ne!(reopened.revision(), reopened.storage_revision());
    }

    #[test]
    fn another_writers_set_change_is_caught_by_the_base_revision() {
        // ADR-0128 결정 2: generation을 쓰지 않는 세트 변경(다른 writer)도 base revision을 바꿉니다.
        let (dir, model) = project_with(7);
        let base = open_snapshot(&dir.path).unwrap().revision();
        stellar_project::manifest_style_defaults::set_active(&dir.path, "react-native").unwrap();
        let before = file_digest(&dir.path);
        let error = apply(
            &dir.path,
            &ApplyRequest {
                base_revision: base,
                document: model.document_id(),
                commands: vec![CommandInput::Rename {
                    node: some_child(&model),
                    name: "late".into(),
                }],
            },
            ApplyMode::Write,
            &locks(&dir),
        )
        .unwrap_err();
        assert!(
            matches!(error, ApplyError::BaseRevisionMismatch { .. }),
            "{error:?}"
        );
        assert_eq!(file_digest(&dir.path), before);
    }
}
