//! Path point 편집 command (XII.10.28, ADR-0079).
//!
//! [`EditPathPoint`]는 Path node 하나에 [`PathEdit`] 연산 하나를 적용합니다. 연산은 closed enum이며 point·
//! handle을 StableId로 가리킵니다. 편집하지 않은 point·handle의 id와 handle mode는 그대로입니다(point 삽입
//! 뒤에도 기존 point id 유지). 연산 의미는 문서 모델의 [`IdentifiedPath::edited`]가 정합니다.
//!
//! 역은 원래 node 레코드를 되돌리는 [`RestoreNodeRecords`]입니다. 지운 point·handle의 id와 좌표, 합쳐진
//! segment 모양을 정확히 되살리기 위해서입니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Vec2};
use stellar_document::model::DocumentModel;
use stellar_document::path::{
    HandleMode, IdentifiedPath, PathEdit, PathError, PathHandleId, PathPointId,
};
use stellar_document::records::NodePayload;

use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::convert::RestoreNodeRecords;
use crate::edit::EditContext;

/// Path node 하나의 point 편집.
#[derive(Debug, Clone, PartialEq)]
pub struct EditPathPoint {
    /// 대상 Path node.
    pub node: StableId,
    /// 편집 연산.
    pub edit: PathEdit,
}

fn path_error(error: PathError) -> CommandError {
    match error {
        PathError::UnknownElement { id } => CommandError::UnknownNode { id },
        PathError::DuplicateId { id } => CommandError::DuplicateId { id },
        PathError::InvalidEdit { reason } => CommandError::InvalidRequest { reason },
        PathError::NonFinite => CommandError::InvalidRequest {
            reason: "path 편집 결과 좌표가 유한하지 않습니다",
        },
        _ => CommandError::InvalidRequest {
            reason: "path 편집 결과가 유효하지 않습니다",
        },
    }
}

impl EditPathPoint {
    fn source<'a>(&self, model: &'a DocumentModel) -> Result<&'a IdentifiedPath, CommandError> {
        match model.payload(self.node) {
            Some(NodePayload::Path(path)) => Ok(path),
            None => Err(CommandError::UnknownNode { id: self.node }),
            Some(_) => Err(CommandError::InvalidRequest {
                reason: "point 편집은 Path node에서만 할 수 있습니다",
            }),
        }
    }

    /// 새로 만드는 point·handle id.
    fn new_ids(&self) -> Vec<StableId> {
        match self.edit {
            PathEdit::InsertPoint { point, handles, .. } => {
                let mut ids = vec![point.0];
                if let Some((left, right)) = handles {
                    ids.extend([left.0, right.0]);
                }
                ids
            }
            _ => Vec::new(),
        }
    }

    fn result(&self, model: &DocumentModel) -> Result<IdentifiedPath, CommandError> {
        let path = self.source(model)?;
        for id in self.new_ids() {
            if id == StableId::NIL {
                return Err(CommandError::NilId);
            }
            if model.contains_persistent_id(id) {
                return Err(CommandError::DuplicateId { id });
            }
        }
        path.edited(&self.edit).map_err(path_error)
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        self.result(model).map(|_| ())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<RestoreNodeRecords, CommandError> {
        let record = model
            .record(self.node)
            .ok_or(CommandError::UnknownNode { id: self.node })?;
        Ok(RestoreNodeRecords {
            records: vec![record],
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let path = self.result(ctx.model())?;
        ctx.set_payload(self.node, NodePayload::Path(path))?;
        ctx.changes().structure(self.node);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.node);
        let vec2 = |out: &mut Writer, v: Vec2| {
            out.f32(v.x());
            out.f32(v.y());
        };
        match self.edit {
            PathEdit::InsertPoint {
                segment,
                t,
                point,
                handles,
            } => {
                out.u8(TAG_INSERT);
                out.id(segment.0);
                out.f32(t.get());
                out.id(point.0);
                match handles {
                    None => out.u8(0),
                    Some((left, right)) => {
                        out.u8(1);
                        out.id(left.0);
                        out.id(right.0);
                    }
                }
            }
            PathEdit::MovePoint { point, to } => {
                out.u8(TAG_MOVE_POINT);
                out.id(point.0);
                vec2(out, to);
            }
            PathEdit::DeletePoint { point } => {
                out.u8(TAG_DELETE);
                out.id(point.0);
            }
            PathEdit::MoveHandle { handle, to } => {
                out.u8(TAG_MOVE_HANDLE);
                out.id(handle.0);
                vec2(out, to);
            }
            PathEdit::SetHandleMode { point, mode } => {
                out.u8(TAG_HANDLE_MODE);
                out.id(point.0);
                out.u8(mode as u8);
            }
            PathEdit::CloseSubpath { start } => {
                out.u8(TAG_CLOSE);
                out.id(start.0);
            }
            PathEdit::OpenSubpath { start } => {
                out.u8(TAG_OPEN);
                out.id(start.0);
            }
        }
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<EditPathPoint, CommandCodecError> {
        let node = input.id()?;
        let vec2 = |input: &mut Reader<'_>| -> Result<Vec2, CommandCodecError> {
            Vec2::new(input.f32()?, input.f32()?).map_err(|_| CommandCodecError::Malformed {
                reason: "유한하지 않은 좌표",
            })
        };
        let edit = match input.u8()? {
            TAG_INSERT => PathEdit::InsertPoint {
                segment: PathPointId(input.id()?),
                t: FiniteF32::new(input.f32()?).map_err(|_| CommandCodecError::Malformed {
                    reason: "유한하지 않은 t",
                })?,
                point: PathPointId(input.id()?),
                handles: if input.bool()? {
                    Some((PathHandleId(input.id()?), PathHandleId(input.id()?)))
                } else {
                    None
                },
            },
            TAG_MOVE_POINT => PathEdit::MovePoint {
                point: PathPointId(input.id()?),
                to: vec2(input)?,
            },
            TAG_DELETE => PathEdit::DeletePoint {
                point: PathPointId(input.id()?),
            },
            TAG_MOVE_HANDLE => PathEdit::MoveHandle {
                handle: PathHandleId(input.id()?),
                to: vec2(input)?,
            },
            TAG_HANDLE_MODE => PathEdit::SetHandleMode {
                point: PathPointId(input.id()?),
                mode: HandleMode::from_tag(input.u8()?).ok_or(CommandCodecError::Malformed {
                    reason: "알 수 없는 handle mode",
                })?,
            },
            TAG_CLOSE => PathEdit::CloseSubpath {
                start: PathPointId(input.id()?),
            },
            TAG_OPEN => PathEdit::OpenSubpath {
                start: PathPointId(input.id()?),
            },
            _ => {
                return Err(CommandCodecError::Malformed {
                    reason: "알 수 없는 path 편집 tag",
                })
            }
        };
        Ok(EditPathPoint { node, edit })
    }
}

// 연산 tag. command 직렬화 계약이라 바꾸거나 재사용하지 않습니다.
const TAG_INSERT: u8 = 1;
const TAG_MOVE_POINT: u8 = 2;
const TAG_DELETE: u8 = 3;
const TAG_MOVE_HANDLE: u8 = 4;
const TAG_HANDLE_MODE: u8 = 5;
const TAG_CLOSE: u8 = 6;
const TAG_OPEN: u8 = 7;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{definition, frame, sid};
    use crate::command::Command;
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::path::{PathCommand, PathData, WindingRule};
    use stellar_document::records::NodeRecord;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).unwrap()
    }

    fn p(n: u64) -> PathPointId {
        PathPointId(sid(n))
    }

    fn h(n: u64) -> PathHandleId {
        PathHandleId(sid(n))
    }

    /// MoveTo(0,0) p100 → LineTo(100,0) p101 → CubicTo(h103,h104 → (100,100)) p102 → CubicTo(h106,h107 →
    /// (0,100)) p105, 열림.
    fn path() -> IdentifiedPath {
        let data = PathData::new(
            WindingRule::NonZero,
            vec![
                PathCommand::MoveTo(v(0., 0.)),
                PathCommand::LineTo(v(100., 0.)),
                PathCommand::CubicTo {
                    c1: v(150., 0.),
                    c2: v(150., 100.),
                    to: v(100., 100.),
                },
                PathCommand::CubicTo {
                    c1: v(50., 120.),
                    c2: v(20., 120.),
                    to: v(0., 100.),
                },
            ],
        )
        .unwrap();
        let mut n = 99;
        IdentifiedPath::assign(data, &mut || {
            n += 1;
            sid(n)
        })
    }

    fn state() -> DocumentState {
        let mut model = DocumentModel::new(definition());
        model.insert_node(frame(10, None, 0x10)).unwrap();
        model
            .insert_node(NodeRecord {
                id: sid(20),
                parent: Some(sid(10)),
                order_key: OrderKey::from_bytes(vec![0x10]).unwrap(),
                visible: true,
                locked: false,
                display_name: "path".into(),
                payload: NodePayload::Path(path()),
            })
            .unwrap();
        DocumentState::new(model)
    }

    fn current(state: &DocumentState) -> IdentifiedPath {
        match state.model().payload(sid(20)) {
            Some(NodePayload::Path(path)) => path.clone(),
            other => panic!("{other:?}"),
        }
    }

    /// apply → 기대 path 확인 → inverse로 원래 문서 → codec 왕복.
    fn fixture(edit: PathEdit, check: impl FnOnce(&IdentifiedPath)) {
        let mut state = state();
        let before = model_snapshot_json(state.model()).unwrap();
        let command = Command::EditPathPoint(EditPathPoint {
            node: sid(20),
            edit,
        });
        let outcome = state
            .execute(&CommandRequest::at(&state, command.clone()))
            .unwrap();
        check(&current(&state));
        state
            .execute(&CommandRequest::at(&state, outcome.inverse))
            .unwrap();
        assert_eq!(
            model_snapshot_json(state.model()).unwrap(),
            before,
            "{edit:?} 역"
        );
        // semantic snapshot은 handle mode를 담지 않으므로 path payload 전체(mode 포함)도 비교합니다.
        assert_eq!(current(&state), path(), "{edit:?} 역 payload");
        let bytes = encode_command(&command);
        assert_eq!(&bytes[..2], &0x0080u16.to_le_bytes());
        assert_eq!(decode_command(&bytes).unwrap(), command);
        for cut in 0..bytes.len() {
            assert!(decode_command(&bytes[..cut]).is_err(), "절단 {cut}");
        }
    }

    #[test]
    fn insert_point_keeps_existing_ids_and_curve_shape() {
        let original = path();
        let before_ids = original.point_ids();
        fixture(
            PathEdit::InsertPoint {
                segment: p(101),
                t: FiniteF32::new(0.25).unwrap(),
                point: p(200),
                handles: None,
            },
            |path| {
                assert_eq!(path.data().commands()[1], PathCommand::LineTo(v(25., 0.)));
                let ids = path.point_ids();
                assert_eq!(ids[1], p(200));
                assert_eq!(
                    ids.iter()
                        .filter(|id| **id != p(200))
                        .copied()
                        .collect::<Vec<_>>(),
                    before_ids,
                    "기존 point id 유지"
                );
            },
        );
        fixture(
            PathEdit::InsertPoint {
                segment: p(102),
                t: FiniteF32::new(0.5).unwrap(),
                point: p(201),
                handles: Some((h(202), h(203))),
            },
            |path| {
                let commands = path.data().commands();
                // (100,0) → c1(150,0) c2(150,100) → (100,100)의 t=0.5 분할점은 (137.5, 50).
                assert_eq!(
                    commands[2],
                    PathCommand::CubicTo {
                        c1: v(125., 0.),
                        c2: v(137.5, 25.),
                        to: v(137.5, 50.)
                    }
                );
                assert_eq!(
                    commands[3],
                    PathCommand::CubicTo {
                        c1: v(137.5, 75.),
                        c2: v(125., 100.),
                        to: v(100., 100.)
                    }
                );
                assert_eq!(path.ids()[2].point, Some(p(201)));
                assert_eq!(path.ids()[2].handles, Some((h(103), h(202))));
                assert_eq!(path.ids()[3].point, Some(p(102)));
                assert_eq!(path.ids()[3].handles, Some((h(203), h(104))));
            },
        );
    }

    #[test]
    fn move_and_delete_points() {
        fixture(
            PathEdit::MovePoint {
                point: p(102),
                to: v(110., 90.),
            },
            |path| {
                let commands = path.data().commands();
                // anchor와 두 handle(들어오는 c2, 나가는 다음 c1)이 함께 (+10, -10) 이동합니다.
                assert_eq!(
                    commands[2],
                    PathCommand::CubicTo {
                        c1: v(150., 0.),
                        c2: v(160., 90.),
                        to: v(110., 90.)
                    }
                );
                assert_eq!(
                    commands[3],
                    PathCommand::CubicTo {
                        c1: v(60., 110.),
                        c2: v(20., 120.),
                        to: v(0., 100.)
                    }
                );
            },
        );
        // 두 곡선 사이 point 삭제: 바깥 handle로 한 곡선.
        fixture(PathEdit::DeletePoint { point: p(102) }, |path| {
            assert_eq!(path.data().commands().len(), 3);
            assert_eq!(
                path.data().commands()[2],
                PathCommand::CubicTo {
                    c1: v(150., 0.),
                    c2: v(20., 120.),
                    to: v(0., 100.)
                }
            );
            assert_eq!(path.ids()[2].handles, Some((h(103), h(107))));
            assert_eq!(path.ids()[2].point, Some(p(105)));
        });
        // 직선과 곡선 사이: 직선으로 합칩니다.
        fixture(PathEdit::DeletePoint { point: p(101) }, |path| {
            assert_eq!(
                path.data().commands()[1],
                PathCommand::LineTo(v(100., 100.))
            );
            assert_eq!(path.ids()[1].point, Some(p(102)));
        });
        // 시작 point: 다음 point가 시작이 됩니다.
        fixture(PathEdit::DeletePoint { point: p(100) }, |path| {
            assert_eq!(path.data().commands()[0], PathCommand::MoveTo(v(100., 0.)));
            assert_eq!(path.ids()[0].point, Some(p(101)));
        });
    }

    #[test]
    fn handles_follow_the_symmetry_mode() {
        // Mirrored 전환: 들어오는 handle(150,100) 기준으로 나가는 handle이 (50,100)으로 점대칭.
        fixture(
            PathEdit::SetHandleMode {
                point: p(102),
                mode: HandleMode::Mirrored,
            },
            |path| {
                assert_eq!(path.handle_mode(p(102)), HandleMode::Mirrored);
                let PathCommand::CubicTo { c1, .. } = path.data().commands()[3] else {
                    panic!()
                };
                assert_eq!(c1, v(50., 100.));
                // 저장 형식 왕복(handle mode 절).
                assert_eq!(&IdentifiedPath::decode(&path.encode()).unwrap(), path);
            },
        );
        // Mirrored point의 handle 이동은 반대 handle을 따라 움직입니다.
        let mut state = state();
        for edit in [
            PathEdit::SetHandleMode {
                point: p(102),
                mode: HandleMode::Mirrored,
            },
            PathEdit::MoveHandle {
                handle: h(104),
                to: v(100., 140.),
            },
        ] {
            let command = Command::EditPathPoint(EditPathPoint {
                node: sid(20),
                edit,
            });
            state.execute(&CommandRequest::at(&state, command)).unwrap();
        }
        let PathCommand::CubicTo { c1, .. } = current(&state).data().commands()[3] else {
            panic!()
        };
        assert_eq!(c1, v(100., 60.));
        // Aligned: 방향만 반대로, 길이(10)는 유지합니다.
        let mut state = self::state();
        for edit in [
            PathEdit::MoveHandle {
                handle: h(106),
                to: v(100., 110.),
            },
            PathEdit::SetHandleMode {
                point: p(102),
                mode: HandleMode::Aligned,
            },
        ] {
            let command = Command::EditPathPoint(EditPathPoint {
                node: sid(20),
                edit,
            });
            state.execute(&CommandRequest::at(&state, command)).unwrap();
        }
        let commands = current(&state).data().commands().to_vec();
        let PathCommand::CubicTo { c2: incoming, .. } = commands[2] else {
            panic!()
        };
        let PathCommand::CubicTo { c1: outgoing, .. } = commands[3] else {
            panic!()
        };
        // 기준(들어오는 handle) 방향 (1,0)의 반대, 원래 길이 10.
        assert_eq!(incoming, v(150., 100.));
        assert_eq!(outgoing, v(90., 100.));
        // 기본 mode로 돌리면 저장 bytes에서 mode 절이 사라집니다.
        fixture(
            PathEdit::SetHandleMode {
                point: p(102),
                mode: HandleMode::Independent,
            },
            |path| assert_eq!(path.encode(), self::path().encode()),
        );
    }

    #[test]
    fn undo_restores_handle_modes_of_deleted_and_changed_points() {
        // mode가 있는 point를 지우거나 mode를 바꾼 뒤 Undo하면 mode까지 원래대로입니다.
        let mut state = state();
        let set = Command::EditPathPoint(EditPathPoint {
            node: sid(20),
            edit: PathEdit::SetHandleMode {
                point: p(102),
                mode: HandleMode::Aligned,
            },
        });
        state.execute(&CommandRequest::at(&state, set)).unwrap();
        let moded = current(&state);
        assert_eq!(moded.handle_mode(p(102)), HandleMode::Aligned);
        for edit in [
            PathEdit::DeletePoint { point: p(102) },
            PathEdit::SetHandleMode {
                point: p(102),
                mode: HandleMode::Mirrored,
            },
            PathEdit::SetHandleMode {
                point: p(102),
                mode: HandleMode::Independent,
            },
        ] {
            let command = Command::EditPathPoint(EditPathPoint {
                node: sid(20),
                edit,
            });
            let outcome = state.execute(&CommandRequest::at(&state, command)).unwrap();
            assert_ne!(current(&state), moded, "{edit:?}");
            state
                .execute(&CommandRequest::at(&state, outcome.inverse))
                .unwrap();
            assert_eq!(current(&state), moded, "{edit:?} 역");
        }
    }

    #[test]
    fn close_and_open_subpaths() {
        fixture(PathEdit::CloseSubpath { start: p(100) }, |path| {
            assert_eq!(path.data().commands().last(), Some(&PathCommand::Close));
            assert!(path.data().subpaths()[0].closed);
        });
        let mut state = state();
        let close = Command::EditPathPoint(EditPathPoint {
            node: sid(20),
            edit: PathEdit::CloseSubpath { start: p(100) },
        });
        state
            .execute(&CommandRequest::at(&state, close.clone()))
            .unwrap();
        assert!(matches!(
            state.execute(&CommandRequest::at(&state, close)),
            Err(CommandError::InvalidRequest { .. })
        ));
        let open = Command::EditPathPoint(EditPathPoint {
            node: sid(20),
            edit: PathEdit::OpenSubpath { start: p(100) },
        });
        state.execute(&CommandRequest::at(&state, open)).unwrap();
        assert_eq!(current(&state), path());
    }

    #[test]
    fn invalid_edits_are_rejected_without_change() {
        let mut state = state();
        let before = model_snapshot_json(state.model()).unwrap();
        for edit in [
            PathEdit::DeletePoint { point: p(999) },
            PathEdit::MoveHandle {
                handle: h(999),
                to: v(0., 0.),
            },
            // MoveTo로 끝나는 segment는 없습니다.
            PathEdit::InsertPoint {
                segment: p(100),
                t: FiniteF32::new(0.5).unwrap(),
                point: p(300),
                handles: None,
            },
            // 곡선 분할에 handle id 없음, t 범위 밖, 이미 쓰는 id.
            PathEdit::InsertPoint {
                segment: p(102),
                t: FiniteF32::new(0.5).unwrap(),
                point: p(300),
                handles: None,
            },
            PathEdit::InsertPoint {
                segment: p(101),
                t: FiniteF32::new(1.0).unwrap(),
                point: p(300),
                handles: None,
            },
            PathEdit::InsertPoint {
                segment: p(101),
                t: FiniteF32::new(0.5).unwrap(),
                point: p(102),
                handles: None,
            },
            PathEdit::InsertPoint {
                segment: p(101),
                t: FiniteF32::new(0.5).unwrap(),
                point: PathPointId(sid(10)),
                handles: None,
            },
            PathEdit::OpenSubpath { start: p(100) },
            PathEdit::CloseSubpath { start: p(101) },
        ] {
            let command = Command::EditPathPoint(EditPathPoint {
                node: sid(20),
                edit,
            });
            assert!(
                state.execute(&CommandRequest::at(&state, command)).is_err(),
                "{edit:?}"
            );
        }
        let not_path = Command::EditPathPoint(EditPathPoint {
            node: sid(10),
            edit: PathEdit::DeletePoint { point: p(100) },
        });
        assert!(state
            .execute(&CommandRequest::at(&state, not_path))
            .is_err());
        assert_eq!(model_snapshot_json(state.model()).unwrap(), before);
    }
}
