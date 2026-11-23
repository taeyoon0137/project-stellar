//! 직렬화 가능한 closed enum command (VII.1.2).
//!
//! 편집 command는 `Command` trait 객체가 아니라 **닫힌 enum** [`Command`]입니다. 모든
//! variant는 numeric [`CommandKind`]를 가지며 [`codec`](crate::codec)으로 canonical bytes와
//! 왕복합니다. registry에 없는 종류는 디코딩 단계에서 거부되므로 임의 script·raw memory
//! patch가 command 경로로 들어올 수 없습니다(agent proposal 포함, 기술 아키텍처 §31.4).
//!
//! 인터페이스(모든 variant 공통):
//!
//! - [`Command::validate`]: 현재 문서에 적용 가능한지 **변경 없이** 검사합니다.
//! - [`Command::invert`]: 적용 **전** 문서를 보고, 적용 결과를 정확히 되돌리는 command를
//!   만듭니다(변경 없음). undo payload가 됩니다.
//! - [`Command::apply`]: [`EditContext`]의 primitive 변경만으로 문서를 바꿉니다.
//!
//! 실행기는 `validate → invert → apply` 순서로 부르며, apply 실패 시 되돌리기는 transaction
//! guard(VII.1.5)의 책임입니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_document::model::{DocumentModel, ModelError};

use crate::convert::{ConvertFragmentToFrame, ConvertShapeToPath, RestoreNodeRecords};
use crate::duplicate::DuplicateSubtree;
use crate::edit::EditContext;
use crate::kind::CommandKind;
use crate::modifier_ops::{InsertModifier, MoveModifier, RemoveModifier, SetModifierProperty};
use crate::node_ops::{DeleteSubtree, InsertNode, MoveNode};
use crate::property::{PropertyValueType, SetProperty};

/// command 검증·적용 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandError {
    /// 대상 node가 없습니다.
    UnknownNode {
        /// 요청 id.
        id: StableId,
    },
    /// 문서 모델이 변경을 거부했습니다.
    Model(ModelError),
    /// revision 카운터가 한계에 도달했습니다.
    RevisionOverflow,
    /// 요청이 없는 transaction입니다(VII.1.6).
    EmptyTransaction,
    /// 요청 형식이 잘못되었습니다(빈 삽입, 범위 밖 stack 등).
    InvalidRequest {
        /// 원인.
        reason: &'static str,
    },
    /// 영속 id로 NIL을 쓸 수 없습니다.
    NilId,
    /// id가 문서나 요청 안에서 이미 쓰입니다.
    DuplicateId {
        /// 중복 id.
        id: StableId,
    },
    /// 부모가 없거나, 자식을 가질 수 없는 종류이거나, 허용되지 않는 위치입니다.
    InvalidParent {
        /// 대상 node.
        node: StableId,
        /// 요청한 부모(`None`이면 부모 없음).
        parent: Option<StableId>,
    },
    /// 같은 부모의 형제가 이미 이 OrderKey를 씁니다.
    DuplicateOrderKey {
        /// 부모.
        parent: StableId,
        /// 대상 node.
        node: StableId,
    },
    /// 최대 node 깊이를 넘습니다.
    DepthExceeded {
        /// 대상 node.
        node: StableId,
        /// 결과 깊이.
        depth: u32,
    },
    /// 참조 대상이 기대 종류가 아닙니다.
    ReferenceTypeMismatch {
        /// 참조를 가진 node.
        owner: StableId,
        /// 참조 대상.
        target: StableId,
    },
    /// modifier 규칙 위반.
    Modifier {
        /// 소유 node.
        owner: StableId,
        /// 원인.
        error: stellar_document::modifier::ModifierError,
    },
    /// modifier 참조 cycle.
    ModifierCycle(stellar_document::modifier::ModifierCycle),
    /// subtree 밖 node가 삭제 대상 subtree 안 node를 참조합니다(삭제 정책 오류, VII.2.2).
    ExternalReference {
        /// 참조되는 subtree 안 node.
        target: StableId,
        /// 참조하는 subtree 밖 node.
        from: StableId,
    },
    /// tree 이동 규칙 위반(VI.2.3: 자기 부모·자손 부모·다른 View 이동).
    Reparent {
        /// 대상 node.
        node: StableId,
        /// 원인.
        error: stellar_document::tree_ops::ReparentError,
    },
    /// metadata registry에 없는(그 node 종류가 가지지 않는) 속성입니다(VII.2.4).
    UnknownProperty {
        /// 대상 node(modifier 속성이면 modifier id).
        node: StableId,
        /// 요청 key.
        key: stellar_core_types::property::PropertyKey,
    },
    /// 값 타입이 registry 타입과 다릅니다(VII.2.4).
    PropertyTypeMismatch {
        /// 요청 key.
        key: stellar_core_types::property::PropertyKey,
        /// registry 타입.
        expected: PropertyValueType,
        /// 요청 값 타입.
        found: PropertyValueType,
    },
    /// 이 command로 설정할 수 없는 속성입니다(부모·순서는 이동 command 전용).
    PropertyNotSettable {
        /// 요청 key.
        key: stellar_core_types::property::PropertyKey,
    },
    /// 변환 대상 종류가 아닙니다(VII.2.7).
    ConversionKind {
        /// 대상 node.
        node: StableId,
        /// 변환이 요구하는 종류.
        expected: stellar_document::node_kind::NodeKind,
        /// 실제 종류.
        found: stellar_document::node_kind::NodeKind,
    },
    /// 변환이 자식의 시각 위치를 보존할 수 없습니다(위치 필드 없는 자식, VII.2.7).
    ConversionNotRepresentable {
        /// 변환 대상.
        node: StableId,
        /// 옮길 수 없는 자손.
        child: StableId,
    },
    /// View root는 이 command로 삭제·이동할 수 없습니다.
    ViewRoot {
        /// 대상 node.
        node: StableId,
    },
    /// View root는 Frame이어야 합니다(레코드 복원이 root 종류를 바꾸려 함, VI.10.1 root 규칙).
    RootKind {
        /// 대상 root node.
        node: StableId,
        /// 요청한 종류.
        found: stellar_document::node_kind::NodeKind,
    },
    /// 적용 결과 문서가 전체 validator(VI.10.1)를 통과하지 못합니다(새로 생길 위반 목록).
    InvalidResult(Box<stellar_document::validate::DocumentValidationError>),
    /// 적용 실패 후 되돌리기마저 실패했습니다(내부 불일치 신호, VII.1.5).
    RollbackFailed {
        /// 원래 실패 원인.
        original: Box<CommandError>,
        /// 되돌리기 실패 정보.
        rollback: Box<crate::edit::RollbackError>,
    },
    /// command가 기준으로 삼은 revision과 현재 문서 revision이 다릅니다(VII.1.3).
    RevisionMismatch {
        /// command의 base revision.
        expected: DocumentRevision,
        /// 현재 문서 revision.
        actual: DocumentRevision,
    },
    /// precondition이 성립하지 않습니다(VII.1.3).
    PreconditionFailed {
        /// precondition 목록 안 위치.
        index: usize,
        /// 실패한 precondition.
        precondition: crate::request::Precondition,
    },
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::UnknownNode { id } => write!(f, "node {id}가 없습니다"),
            CommandError::Model(error) => write!(f, "문서 모델 오류: {error}"),
            CommandError::RevisionOverflow => f.write_str("문서 revision이 한계에 도달했습니다"),
            CommandError::EmptyTransaction => f.write_str("요청이 없는 transaction입니다"),
            CommandError::InvalidRequest { reason } => write!(f, "잘못된 요청: {reason}"),
            CommandError::NilId => f.write_str("NIL은 영속 id로 쓸 수 없습니다"),
            CommandError::DuplicateId { id } => write!(f, "id {id}가 이미 쓰입니다"),
            CommandError::InvalidParent { node, parent } => match parent {
                Some(parent) => write!(f, "node {node}의 부모로 {parent}를 쓸 수 없습니다"),
                None => write!(f, "node {node}에 부모가 필요합니다"),
            },
            CommandError::DuplicateOrderKey { parent, node } => {
                write!(
                    f,
                    "부모 {parent} 아래 node {node}의 OrderKey가 형제와 같습니다"
                )
            }
            CommandError::DepthExceeded { node, depth } => {
                write!(f, "node {node}의 깊이 {depth}가 최대를 넘습니다")
            }
            CommandError::ReferenceTypeMismatch { owner, target } => {
                write!(
                    f,
                    "node {owner}의 참조 대상 {target}의 종류가 맞지 않습니다"
                )
            }
            CommandError::Modifier { owner, error } => {
                write!(f, "node {owner}의 modifier: {error}")
            }
            CommandError::ModifierCycle(cycle) => write!(f, "{cycle}"),
            CommandError::ExternalReference { target, from } => write!(
                f,
                "subtree 밖 node {from}가 삭제 대상 {target}를 참조합니다(참조를 먼저 정리해야 합니다)"
            ),
            CommandError::Reparent { node, error } => write!(f, "node {node} 이동 거부: {error}"),
            CommandError::UnknownProperty { node, key } => {
                write!(f, "{node}에 속성 {:#06x}가 없습니다", key.0)
            }
            CommandError::PropertyTypeMismatch {
                key,
                expected,
                found,
            } => write!(
                f,
                "속성 {:#06x}의 값 타입은 {expected}여야 하지만 {found}입니다",
                key.0
            ),
            CommandError::PropertyNotSettable { key } => {
                write!(f, "속성 {:#06x}는 이 command로 설정할 수 없습니다", key.0)
            }
            CommandError::ConversionKind {
                node,
                expected,
                found,
            } => write!(
                f,
                "node {node}는 {}이 아니라 {}라 변환할 수 없습니다",
                expected.name(),
                found.name()
            ),
            CommandError::ConversionNotRepresentable { node, child } => write!(
                f,
                "node {node} 변환은 자손 {child}의 위치를 보존할 수 없습니다"
            ),
            CommandError::ViewRoot { node } => {
                write!(f, "View root {node}는 삭제·이동할 수 없습니다")
            }
            CommandError::RootKind { node, found } => write!(
                f,
                "View root {node}는 Frame이어야 합니다({}로 바꿀 수 없음)",
                found.name()
            ),
            CommandError::InvalidResult(error) => {
                write!(f, "적용 결과 문서가 불변식을 위반합니다: {error}")
            }
            CommandError::RollbackFailed { original, rollback } => {
                write!(f, "{original} (되돌리기 실패: {rollback})")
            }
            CommandError::RevisionMismatch { expected, actual } => write!(
                f,
                "base revision {}이 현재 문서 revision {}과 다릅니다",
                expected.0, actual.0
            ),
            CommandError::PreconditionFailed {
                index,
                precondition,
            } => write!(f, "precondition #{index} 실패: {precondition}"),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<ModelError> for CommandError {
    fn from(error: ModelError) -> CommandError {
        CommandError::Model(error)
    }
}

/// 문서 편집 command(closed enum).
///
/// variant는 문서 command 작업(VII.2.x)이 [`CommandKind`] 번호와 함께 등록합니다.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// node(subtree) 삽입 (VII.2.1).
    InsertNode(InsertNode),
    /// node subtree 삭제 (VII.2.1·VII.2.2).
    DeleteSubtree(DeleteSubtree),
    /// node 이동·재정렬 (VII.2.3).
    MoveNode(MoveNode),
    /// node 속성 변경 (VII.2.4).
    SetProperty(SetProperty),
    /// modifier 삽입 (VII.2.6).
    InsertModifier(InsertModifier),
    /// modifier 제거 (VII.2.6).
    RemoveModifier(RemoveModifier),
    /// modifier 순서 변경 (VII.2.6).
    MoveModifier(MoveModifier),
    /// modifier 속성 변경 (VII.2.6).
    SetModifierProperty(SetModifierProperty),
    /// Fragment → Frame 변환 (VII.2.7).
    ConvertFragmentToFrame(ConvertFragmentToFrame),
    /// Shape → Path 변환 (VII.2.7).
    ConvertShapeToPath(ConvertShapeToPath),
    /// node 레코드 복원(변환의 역, VII.2.7).
    RestoreNodeRecords(RestoreNodeRecords),
    /// subtree 복제 (VII.2.8).
    DuplicateSubtree(DuplicateSubtree),
    /// Text node의 부분 문자열을 편집합니다.
    ReplaceText(crate::text_edit::ReplaceText),
    /// Text 범위 문자 style 설정 (XII.10.32).
    SetTextRangeStyle(crate::text_edit::SetTextRangeStyle),
    /// 페이지 추가 (UX.12.1).
    AddPage(crate::page_view::AddPage),
    /// 페이지 삭제(AddPage의 역).
    RemovePage(crate::page_view::RemovePage),
    /// 페이지 이름 변경 (UX.12.1).
    RenamePage(crate::page_view::RenamePage),
    /// View 추가 (UX.12.1).
    AddView(crate::page_view::AddView),
    /// View 삭제(AddView의 역).
    RemoveView(crate::page_view::RemoveView),
    /// View 이름 변경 (UX.12.1).
    RenameView(crate::page_view::RenameView),
    /// 외부 원본 반영 (UX.4.1).
    ApplyExternalSnapshot(crate::snapshot_apply::ApplyExternalSnapshot),
    /// Path point 편집 (XII.10.28).
    EditPathPoint(crate::path_edit::EditPathPoint),
    /// 인스턴스 교체 (XII.10.35).
    SwapInstance(crate::node_ops::SwapInstance),
    /// 활성 기본값 세트 변경 (SD.4).
    SetStyleDefaults(crate::style_defaults::SetStyleDefaults),
    /// 명시 layout longhand 지우기 (SD.4).
    UnsetProperty(crate::style_defaults::UnsetProperty),
    /// 명시 layout longhand 두기 (SD.4).
    SetLayoutProperty(crate::style_defaults::SetLayoutProperty),
    /// Component Context 편집(XXX.7.8, ADR-0099).
    EditContext(Box<crate::context::ContextCommand>),
    /// 인스턴스 Detach 실행(XII.10.34, ADR-0129).
    DetachInstance(Box<crate::detach::InstanceRestructure>),
    /// Slot content 넣기·꺼내기(XII.10.34, ADR-0129).
    PlaceSlotContent(Box<crate::detach::InstanceRestructure>),
    /// 테스트 전용 fault-injection probe. 프로덕션 빌드에는 없습니다.
    #[cfg(test)]
    TestProbe(TestProbe),
}

impl Command {
    /// 표시 여부 변경. 별도 종류가 아닌 [`SetProperty`] specialization입니다(VII.2.5).
    pub fn set_visibility(node: StableId, visible: bool) -> Command {
        Command::SetProperty(SetProperty::visibility(node, visible))
    }

    /// 잠금 변경. [`SetProperty`] specialization입니다(VII.2.5).
    pub fn set_locked(node: StableId, locked: bool) -> Command {
        Command::SetProperty(SetProperty::locked(node, locked))
    }

    /// 표시 이름 변경. [`SetProperty`] specialization입니다(VII.2.5).
    pub fn rename_node(node: StableId, name: impl Into<String>) -> Command {
        Command::SetProperty(SetProperty::rename(node, name))
    }

    /// command 종류.
    pub fn kind(&self) -> CommandKind {
        match self {
            Command::InsertNode(_) => CommandKind::InsertNode,
            Command::DeleteSubtree(_) => CommandKind::DeleteSubtree,
            Command::MoveNode(_) => CommandKind::MoveNode,
            Command::SetProperty(_) => CommandKind::SetProperty,
            Command::InsertModifier(_) => CommandKind::InsertModifier,
            Command::RemoveModifier(_) => CommandKind::RemoveModifier,
            Command::MoveModifier(_) => CommandKind::MoveModifier,
            Command::SetModifierProperty(_) => CommandKind::SetModifierProperty,
            Command::ConvertFragmentToFrame(_) => CommandKind::ConvertFragmentToFrame,
            Command::ConvertShapeToPath(_) => CommandKind::ConvertShapeToPath,
            Command::RestoreNodeRecords(_) => CommandKind::RestoreNodeRecords,
            Command::ReplaceText(_) => CommandKind::ReplaceText,
            Command::DuplicateSubtree(_) => CommandKind::DuplicateSubtree,
            Command::SetTextRangeStyle(_) => CommandKind::SetTextRangeStyle,
            Command::AddPage(_) => CommandKind::AddPage,
            Command::RemovePage(_) => CommandKind::RemovePage,
            Command::RenamePage(_) => CommandKind::RenamePage,
            Command::AddView(_) => CommandKind::AddView,
            Command::RemoveView(_) => CommandKind::RemoveView,
            Command::RenameView(_) => CommandKind::RenameView,
            Command::ApplyExternalSnapshot(_) => CommandKind::ApplyExternalSnapshot,
            Command::EditPathPoint(_) => CommandKind::EditPathPoint,
            Command::SwapInstance(_) => CommandKind::SwapInstance,
            Command::SetStyleDefaults(_) => CommandKind::SetStyleDefaults,
            Command::UnsetProperty(_) => CommandKind::UnsetProperty,
            Command::SetLayoutProperty(_) => CommandKind::SetLayoutProperty,
            Command::EditContext(_) => CommandKind::EditContext,
            Command::DetachInstance(_) => CommandKind::DetachInstance,
            Command::PlaceSlotContent(_) => CommandKind::PlaceSlotContent,
            #[cfg(test)]
            Command::TestProbe(_) => CommandKind::TestProbe,
        }
    }

    /// 현재 문서에 적용 가능한지 검사합니다. 문서를 바꾸지 않습니다.
    pub fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        match self {
            Command::InsertNode(command) => command.validate(model),
            Command::DeleteSubtree(command) => command.validate(model),
            Command::MoveNode(command) => command.validate(model),
            Command::SetProperty(command) => command.validate(model),
            Command::InsertModifier(command) => command.validate(model),
            Command::RemoveModifier(command) => command.validate(model),
            Command::MoveModifier(command) => command.validate(model),
            Command::SetModifierProperty(command) => command.validate(model),
            Command::ConvertFragmentToFrame(command) => command.validate(model),
            Command::ConvertShapeToPath(command) => command.validate(model),
            Command::RestoreNodeRecords(command) => command.validate(model),
            Command::ReplaceText(command) => command.validate(model),
            Command::DuplicateSubtree(command) => command.validate(model),
            Command::SetTextRangeStyle(command) => command.validate(model),
            Command::AddPage(command) => command.validate(model),
            Command::RemovePage(command) => command.validate(model),
            Command::RenamePage(command) => command.validate(model),
            Command::AddView(command) => command.validate(model),
            Command::RemoveView(command) => command.validate(model),
            Command::RenameView(command) => command.validate(model),
            Command::ApplyExternalSnapshot(command) => command.validate(model),
            Command::EditPathPoint(command) => command.validate(model),
            Command::SwapInstance(command) => command.validate(model),
            Command::SetStyleDefaults(command) => command.validate(model),
            Command::UnsetProperty(command) => command.validate(model),
            Command::SetLayoutProperty(command) => command.validate(model),
            Command::EditContext(command) => command.validate(model),
            Command::DetachInstance(command) | Command::PlaceSlotContent(command) => {
                command.validate(model)
            }
            #[cfg(test)]
            Command::TestProbe(probe) => probe.validate(model),
        }
    }

    /// 적용 전 문서 `model`을 기준으로, 이 command의 효과를 되돌리는 command를 만듭니다.
    pub fn invert(&self, model: &DocumentModel) -> Result<Command, CommandError> {
        match self {
            Command::InsertNode(command) => command.invert().map(Command::DeleteSubtree),
            Command::DeleteSubtree(command) => command.invert(model).map(Command::InsertNode),
            Command::MoveNode(command) => command.invert(model).map(Command::MoveNode),
            Command::SetProperty(command) => command.invert(model).map(Command::SetProperty),
            Command::InsertModifier(command) => Ok(Command::RemoveModifier(command.invert())),
            Command::RemoveModifier(command) => command.invert(model).map(Command::InsertModifier),
            Command::MoveModifier(command) => command.invert(model).map(Command::MoveModifier),
            Command::SetModifierProperty(command) => {
                command.invert(model).map(Command::SetModifierProperty)
            }
            Command::ConvertFragmentToFrame(command) => {
                command.invert(model).map(Command::RestoreNodeRecords)
            }
            Command::ConvertShapeToPath(command) => {
                command.invert(model).map(Command::RestoreNodeRecords)
            }
            Command::RestoreNodeRecords(command) => {
                command.invert(model).map(Command::RestoreNodeRecords)
            }
            Command::ReplaceText(command) => command.invert(model).map(Command::RestoreNodeRecords),
            Command::DuplicateSubtree(command) => command.invert().map(Command::DeleteSubtree),
            Command::SetTextRangeStyle(command) => {
                command.invert(model).map(Command::RestoreNodeRecords)
            }
            Command::AddPage(command) => Ok(Command::RemovePage(command.invert())),
            Command::RemovePage(command) => command.invert(model).map(Command::AddPage),
            Command::RenamePage(command) => command.invert(model).map(Command::RenamePage),
            Command::AddView(command) => Ok(Command::RemoveView(command.invert())),
            Command::RemoveView(command) => command.invert(model).map(Command::AddView),
            Command::RenameView(command) => command.invert(model).map(Command::RenameView),
            Command::ApplyExternalSnapshot(command) => {
                Ok(Command::ApplyExternalSnapshot(command.invert()))
            }
            Command::EditPathPoint(command) => {
                command.invert(model).map(Command::RestoreNodeRecords)
            }
            Command::SwapInstance(command) => command.invert(model).map(Command::SwapInstance),
            Command::SetStyleDefaults(command) => {
                command.invert(model).map(Command::SetStyleDefaults)
            }
            Command::UnsetProperty(command) => command.invert(model),
            Command::SetLayoutProperty(command) => command.invert(model),
            Command::EditContext(command) => command
                .invert(model)
                .map(|inverse| Command::EditContext(Box::new(inverse))),
            Command::DetachInstance(command) => command
                .invert(model)
                .map(|inverse| Command::DetachInstance(Box::new(inverse))),
            Command::PlaceSlotContent(command) => command
                .invert(model)
                .map(|inverse| Command::PlaceSlotContent(Box::new(inverse))),
            #[cfg(test)]
            Command::TestProbe(probe) => probe.invert(model).map(Command::TestProbe),
        }
    }

    /// context의 primitive 변경으로 command를 적용합니다.
    pub fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        match self {
            Command::InsertNode(command) => command.apply(ctx),
            Command::DeleteSubtree(command) => command.apply(ctx),
            Command::MoveNode(command) => command.apply(ctx),
            Command::SetProperty(command) => command.apply(ctx),
            Command::InsertModifier(command) => command.apply(ctx),
            Command::RemoveModifier(command) => command.apply(ctx),
            Command::MoveModifier(command) => command.apply(ctx),
            Command::SetModifierProperty(command) => command.apply(ctx),
            Command::ConvertFragmentToFrame(command) => command.apply(ctx),
            Command::ConvertShapeToPath(command) => command.apply(ctx),
            Command::RestoreNodeRecords(command) => command.apply(ctx),
            Command::ReplaceText(command) => command.apply(ctx),
            Command::DuplicateSubtree(command) => command.apply(ctx),
            Command::SetTextRangeStyle(command) => command.apply(ctx),
            Command::AddPage(command) => command.apply(ctx),
            Command::RemovePage(command) => command.apply(ctx),
            Command::RenamePage(command) => command.apply(ctx),
            Command::AddView(command) => command.apply(ctx),
            Command::RemoveView(command) => command.apply(ctx),
            Command::RenameView(command) => command.apply(ctx),
            Command::ApplyExternalSnapshot(command) => command.apply(ctx),
            Command::EditPathPoint(command) => command.apply(ctx),
            Command::SwapInstance(command) => command.apply(ctx),
            Command::SetStyleDefaults(command) => command.apply(ctx),
            Command::UnsetProperty(command) => command.apply(ctx),
            Command::SetLayoutProperty(command) => command.apply(ctx),
            Command::EditContext(command) => command.apply(ctx),
            Command::DetachInstance(command) | Command::PlaceSlotContent(command) => {
                command.apply(ctx)
            }
            #[cfg(test)]
            Command::TestProbe(probe) => probe.apply(ctx),
        }
    }
}

/// 테스트 전용 probe: node 표시 이름을 바꾸고, 선택적으로 **변경 후** 실패합니다.
///
/// command 계층(transaction guard·transaction 원자성)의 fault injection용 test double입니다.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub struct TestProbe {
    /// 대상 node.
    pub node: StableId,
    /// 새 표시 이름.
    pub name: String,
    /// 이름을 바꾼 **뒤** 오류를 반환할지(중간 실패 주입).
    pub fail_after_apply: bool,
}

#[cfg(test)]
impl TestProbe {
    /// 주입된 실패의 오류 값.
    pub fn injected_fault(node: StableId) -> CommandError {
        CommandError::Model(ModelError::Inconsistent {
            id: node,
            reason: "test probe fault injection",
        })
    }

    fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        if model.contains(self.node) {
            Ok(())
        } else {
            Err(CommandError::UnknownNode { id: self.node })
        }
    }

    fn invert(&self, model: &DocumentModel) -> Result<TestProbe, CommandError> {
        let name = model
            .record(self.node)
            .map(|record| record.display_name)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        Ok(TestProbe {
            node: self.node,
            name,
            fail_after_apply: false,
        })
    }

    fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        ctx.set_display_name(self.node, self.name.clone())?;
        ctx.changes().node_property(
            self.node,
            crate::change::node_keys::DISPLAY_NAME,
            crate::change::node_field_invalidation(crate::change::node_keys::DISPLAY_NAME)
                .unwrap_or_default(),
        );
        if self.fail_after_apply {
            return Err(TestProbe::injected_fault(self.node));
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    //! command 계층 unit test 공용 fixture.

    use stellar_core_types::ids::StableId;
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
    use stellar_document::frame::FramePayload;
    use stellar_document::model::DocumentModel;
    use stellar_document::records::{NodePayload, NodeRecord};

    /// 결정론적 id.
    pub fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    /// 한 View(root = sid(10))를 가진 빈 문서 정의.
    pub fn definition() -> DocumentDefinition {
        let mut definition = DocumentDefinition::new(sid(1), String::new());
        let mut page = PageDefinition::new(sid(2), String::new());
        page.add_view(ViewDefinition::new(sid(3), String::new(), sid(10)))
            .expect("view");
        definition.add_page(page).expect("page");
        definition
    }

    /// Frame node 레코드.
    pub fn frame(id: u64, parent: Option<u64>, key: u8) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![key]).expect("key"),
            visible: true,
            locked: false,
            display_name: format!("n{id}"),
            payload: NodePayload::Frame(FramePayload::default()),
        }
    }

    /// root(10) 아래 자식 11·12, 11 아래 13을 가진 모델.
    pub fn small_model() -> DocumentModel {
        let mut model = DocumentModel::new(definition());
        model.insert_node(frame(10, None, 0x10)).expect("root");
        model.insert_node(frame(11, Some(10), 0x10)).expect("11");
        model.insert_node(frame(12, Some(10), 0x20)).expect("12");
        model.insert_node(frame(13, Some(11), 0x10)).expect("13");
        model
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::*;
    use super::*;

    #[test]
    fn validate_invert_apply_interface_round_trips() {
        let mut model = small_model();
        let command = Command::TestProbe(TestProbe {
            node: sid(11),
            name: "새 이름".into(),
            fail_after_apply: false,
        });
        assert_eq!(command.kind(), CommandKind::TestProbe);
        command.validate(&model).unwrap();
        let inverse = command.invert(&model).unwrap();
        command.apply(&mut EditContext::new(&mut model)).unwrap();
        assert_eq!(model.record(sid(11)).unwrap().display_name, "새 이름");
        inverse.validate(&model).unwrap();
        inverse.apply(&mut EditContext::new(&mut model)).unwrap();
        assert_eq!(model.record(sid(11)).unwrap().display_name, "n11");
    }

    #[test]
    fn validate_rejects_missing_target_without_change() {
        let model = small_model();
        let command = Command::TestProbe(TestProbe {
            node: sid(99),
            name: String::new(),
            fail_after_apply: false,
        });
        assert_eq!(
            command.validate(&model),
            Err(CommandError::UnknownNode { id: sid(99) })
        );
        assert_eq!(
            command.invert(&model),
            Err(CommandError::UnknownNode { id: sid(99) })
        );
    }
}
