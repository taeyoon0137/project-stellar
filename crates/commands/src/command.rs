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
use stellar_document::model::{DocumentModel, ModelError};

use crate::edit::EditContext;
use crate::kind::CommandKind;

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
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::UnknownNode { id } => write!(f, "node {id}가 없습니다"),
            CommandError::Model(error) => write!(f, "문서 모델 오류: {error}"),
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
    /// 테스트 전용 fault-injection probe. 프로덕션 빌드에는 없습니다.
    #[cfg(test)]
    TestProbe(TestProbe),
}

impl Command {
    /// command 종류.
    pub fn kind(&self) -> CommandKind {
        match *self {
            #[cfg(test)]
            Command::TestProbe(_) => CommandKind::TestProbe,
        }
    }

    /// 현재 문서에 적용 가능한지 검사합니다. 문서를 바꾸지 않습니다.
    pub fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let _ = model;
        match *self {
            #[cfg(test)]
            Command::TestProbe(ref probe) => probe.validate(model),
        }
    }

    /// 적용 전 문서 `model`을 기준으로, 이 command의 효과를 되돌리는 command를 만듭니다.
    pub fn invert(&self, model: &DocumentModel) -> Result<Command, CommandError> {
        let _ = model;
        match *self {
            #[cfg(test)]
            Command::TestProbe(ref probe) => probe.invert(model).map(Command::TestProbe),
        }
    }

    /// context의 primitive 변경으로 command를 적용합니다.
    pub fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let _ = &ctx;
        match *self {
            #[cfg(test)]
            Command::TestProbe(ref probe) => probe.apply(ctx),
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
