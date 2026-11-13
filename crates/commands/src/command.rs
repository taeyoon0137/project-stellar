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

use crate::edit::EditContext;
use crate::kind::CommandKind;
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
    /// View root는 이 command로 삭제·이동할 수 없습니다.
    ViewRoot {
        /// 대상 node.
        node: StableId,
    },
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
            CommandError::ViewRoot { node } => {
                write!(f, "View root {node}는 삭제·이동할 수 없습니다")
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
