//! base revision과 precondition을 포함한 command 요청 (VII.1.3).
//!
//! 모든 command는 [`CommandRequest`]로 감싸 실행합니다. 요청은
//!
//! - **base revision**: 요청을 만든 시점의 문서 [`DocumentRevision`]. 현재 revision과 다르면
//!   [`CommandError::RevisionMismatch`]로 거부합니다(낡은 UI 상태·agent proposal 차단).
//! - **precondition 목록**: 적용 직전에 성립해야 하는 사실(node 존재·부재·종류·부모). 하나라도
//!   거짓이면 [`CommandError::PreconditionFailed`]로 거부합니다.
//!
//! 을 함께 가집니다. 검사는 문서를 바꾸지 않습니다.
//!
//! 직렬화: base revision `u64` LE + precondition 수 `u32` + precondition* + command
//! ([`codec`](crate::codec) 형식). precondition은 tag `u8` + 필드이며 tag는 저장 계약입니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_document::model::DocumentModel;
use stellar_document::node_kind::NodeKind;

use crate::codec::{decode_command_from, encode_command_into, CommandCodecError, Reader, Writer};
use crate::command::{Command, CommandError};
use crate::state::DocumentState;

/// 적용 직전에 성립해야 하는 사실.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precondition {
    /// node가 있어야 합니다.
    NodeExists(StableId),
    /// node가 없어야 합니다.
    NodeAbsent(StableId),
    /// node가 있고 종류가 `kind`여야 합니다.
    NodeKindIs {
        /// 대상 node.
        node: StableId,
        /// 기대 종류.
        kind: NodeKind,
    },
    /// node가 있고 부모가 `parent`여야 합니다(`None`이면 View root).
    ParentIs {
        /// 대상 node.
        node: StableId,
        /// 기대 부모.
        parent: Option<StableId>,
    },
}

impl fmt::Display for Precondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Precondition::NodeExists(id) => write!(f, "node {id} 존재"),
            Precondition::NodeAbsent(id) => write!(f, "node {id} 부재"),
            Precondition::NodeKindIs { node, kind } => {
                write!(f, "node {node} 종류 = {}", kind.name())
            }
            Precondition::ParentIs { node, parent } => match parent {
                Some(parent) => write!(f, "node {node} 부모 = {parent}"),
                None => write!(f, "node {node}는 View root"),
            },
        }
    }
}

const TAG_NODE_EXISTS: u8 = 1;
const TAG_NODE_ABSENT: u8 = 2;
const TAG_NODE_KIND_IS: u8 = 3;
const TAG_PARENT_IS: u8 = 4;

impl Precondition {
    /// 현재 모델에서 성립하는지.
    pub fn holds(&self, model: &DocumentModel) -> bool {
        match *self {
            Precondition::NodeExists(id) => model.contains(id),
            Precondition::NodeAbsent(id) => !model.contains(id),
            Precondition::NodeKindIs { node, kind } => {
                model.payload(node).is_some_and(|p| p.kind() == kind)
            }
            Precondition::ParentIs { node, parent } => {
                let Some(fields) = model
                    .runtime_id(node)
                    .and_then(|handle| model.columns().get(handle))
                else {
                    return false;
                };
                fields.parent.and_then(|p| model.stable_id(p)) == parent
            }
        }
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        match *self {
            Precondition::NodeExists(id) => {
                out.u8(TAG_NODE_EXISTS);
                out.id(id);
            }
            Precondition::NodeAbsent(id) => {
                out.u8(TAG_NODE_ABSENT);
                out.id(id);
            }
            Precondition::NodeKindIs { node, kind } => {
                out.u8(TAG_NODE_KIND_IS);
                out.id(node);
                out.u16(kind.as_u16());
            }
            Precondition::ParentIs { node, parent } => {
                out.u8(TAG_PARENT_IS);
                out.id(node);
                out.opt_id(parent);
            }
        }
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<Precondition, CommandCodecError> {
        match input.u8()? {
            TAG_NODE_EXISTS => Ok(Precondition::NodeExists(input.id()?)),
            TAG_NODE_ABSENT => Ok(Precondition::NodeAbsent(input.id()?)),
            TAG_NODE_KIND_IS => {
                let node = input.id()?;
                let kind =
                    NodeKind::from_u16(input.u16()?).ok_or(CommandCodecError::Malformed {
                        reason: "알 수 없는 node kind",
                    })?;
                Ok(Precondition::NodeKindIs { node, kind })
            }
            TAG_PARENT_IS => Ok(Precondition::ParentIs {
                node: input.id()?,
                parent: input.opt_id()?,
            }),
            _ => Err(CommandCodecError::Malformed {
                reason: "알 수 없는 precondition tag",
            }),
        }
    }
}

/// 실행 요청: base revision + precondition + command.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandRequest {
    /// 요청을 만든 시점의 문서 revision.
    pub base_revision: DocumentRevision,
    /// 적용 직전에 성립해야 하는 사실.
    pub preconditions: Vec<Precondition>,
    /// 실행할 command.
    pub command: Command,
}

impl CommandRequest {
    /// precondition 없는 요청.
    pub fn new(base_revision: DocumentRevision, command: Command) -> CommandRequest {
        CommandRequest {
            base_revision,
            preconditions: Vec::new(),
            command,
        }
    }

    /// precondition을 덧붙입니다(빌더).
    pub fn with_precondition(mut self, precondition: Precondition) -> CommandRequest {
        self.preconditions.push(precondition);
        self
    }

    /// 상태의 현재 revision을 base로 하는 요청.
    pub fn at(state: &DocumentState, command: Command) -> CommandRequest {
        CommandRequest::new(state.revision(), command)
    }

    /// base revision이 `revision`인지와 precondition을 검사합니다(문서 무변경).
    pub fn check(
        &self,
        model: &DocumentModel,
        revision: DocumentRevision,
    ) -> Result<(), CommandError> {
        if self.base_revision != revision {
            return Err(CommandError::RevisionMismatch {
                expected: self.base_revision,
                actual: revision,
            });
        }
        self.check_preconditions(model)
    }

    /// precondition만 검사합니다.
    pub fn check_preconditions(&self, model: &DocumentModel) -> Result<(), CommandError> {
        for (index, precondition) in self.preconditions.iter().enumerate() {
            if !precondition.holds(model) {
                return Err(CommandError::PreconditionFailed {
                    index,
                    precondition: *precondition,
                });
            }
        }
        Ok(())
    }

    /// 상태 기준 검사.
    pub fn check_state(&self, state: &DocumentState) -> Result<(), CommandError> {
        self.check(state.model(), state.revision())
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Writer::new();
        self.encode_into(&mut out);
        out.into_bytes()
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.u64(self.base_revision.0);
        out.len(self.preconditions.len());
        for precondition in &self.preconditions {
            precondition.encode_into(out);
        }
        encode_command_into(&self.command, out);
    }

    /// [`CommandRequest::encode`]의 역.
    pub fn decode(bytes: &[u8]) -> Result<CommandRequest, CommandCodecError> {
        let mut input = Reader::new(bytes);
        let request = CommandRequest::decode_from(&mut input)?;
        input.expect_end()?;
        Ok(request)
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<CommandRequest, CommandCodecError> {
        let base_revision = DocumentRevision(input.u64()?);
        let count = input.len()?;
        let mut preconditions = Vec::with_capacity(count);
        for _ in 0..count {
            preconditions.push(Precondition::decode_from(input)?);
        }
        let command = decode_command_from(input)?;
        Ok(CommandRequest {
            base_revision,
            preconditions,
            command,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_fixtures::{sid, small_model};
    use crate::command::TestProbe;

    fn probe(node: u64) -> Command {
        Command::TestProbe(TestProbe {
            node: sid(node),
            name: "x".into(),
            fail_after_apply: false,
        })
    }

    #[test]
    fn revision_mismatch_fails() {
        // 검증 기준(VII.1.3): revision mismatch fixture 실패.
        let state = DocumentState::with_revision(small_model(), DocumentRevision(5));
        let stale = CommandRequest::new(DocumentRevision(4), probe(11));
        assert_eq!(
            stale.check_state(&state),
            Err(CommandError::RevisionMismatch {
                expected: DocumentRevision(4),
                actual: DocumentRevision(5),
            })
        );
        let ahead = CommandRequest::new(DocumentRevision(6), probe(11));
        assert!(matches!(
            ahead.check_state(&state),
            Err(CommandError::RevisionMismatch { .. })
        ));
        let current = CommandRequest::at(&state, probe(11));
        assert_eq!(current.base_revision, DocumentRevision(5));
        assert_eq!(current.check_state(&state), Ok(()));
    }

    #[test]
    fn preconditions_are_checked_in_order() {
        let state = DocumentState::new(small_model());
        let ok = CommandRequest::at(&state, probe(11))
            .with_precondition(Precondition::NodeExists(sid(11)))
            .with_precondition(Precondition::NodeAbsent(sid(99)))
            .with_precondition(Precondition::NodeKindIs {
                node: sid(13),
                kind: NodeKind::Frame,
            })
            .with_precondition(Precondition::ParentIs {
                node: sid(13),
                parent: Some(sid(11)),
            })
            .with_precondition(Precondition::ParentIs {
                node: sid(10),
                parent: None,
            });
        assert_eq!(ok.check_state(&state), Ok(()));

        let failing = [
            Precondition::NodeExists(sid(99)),
            Precondition::NodeAbsent(sid(11)),
            Precondition::NodeKindIs {
                node: sid(11),
                kind: NodeKind::Text,
            },
            Precondition::ParentIs {
                node: sid(13),
                parent: Some(sid(10)),
            },
            Precondition::ParentIs {
                node: sid(99),
                parent: None,
            },
        ];
        for precondition in failing {
            let request = CommandRequest::at(&state, probe(11))
                .with_precondition(Precondition::NodeExists(sid(10)))
                .with_precondition(precondition);
            assert_eq!(
                request.check_state(&state),
                Err(CommandError::PreconditionFailed {
                    index: 1,
                    precondition
                }),
                "{precondition}"
            );
        }
    }

    #[test]
    fn request_round_trips() {
        let request = CommandRequest::new(DocumentRevision(0x0102_0304_0506_0708), probe(12))
            .with_precondition(Precondition::NodeExists(sid(12)))
            .with_precondition(Precondition::NodeAbsent(sid(40)))
            .with_precondition(Precondition::NodeKindIs {
                node: sid(12),
                kind: NodeKind::Image,
            })
            .with_precondition(Precondition::ParentIs {
                node: sid(12),
                parent: None,
            })
            .with_precondition(Precondition::ParentIs {
                node: sid(12),
                parent: Some(sid(10)),
            });
        let bytes = request.encode();
        assert_eq!(&bytes[..8], &0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(CommandRequest::decode(&bytes).unwrap(), request);
        let mut bad = bytes.clone();
        bad[12] = 0x77; // 첫 precondition tag.
        assert_eq!(
            CommandRequest::decode(&bad),
            Err(CommandCodecError::Malformed {
                reason: "알 수 없는 precondition tag"
            })
        );
        let mut trailing = bytes;
        trailing.push(0);
        assert!(CommandRequest::decode(&trailing).is_err());
    }
}
