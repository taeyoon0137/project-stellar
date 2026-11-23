//! 캔버스 배치 명령의 문서 쪽 계획 (XII.10.21, XII.10.23, ADR-0105).
//!
//! - [`place`]: 부모 공간 affine 변경과 크기 longhand를 기존 command로 바꾸는 한 곳. 크기 조절·회전·뒤집기·비례
//!   확대·정렬·분배·키보드 이동이 모두 이 module을 지나므로 저장 위치 규칙(IX.11.9 전후)은 여기에만 있습니다.
//! - [`align`]: 정렬 6종·분배 2종·행열 정리의 순수 계산과 흐름 자식 정렬(`align-self`·`justify-self`).
//! - [`order`]: Bring Forward·Send Backward·Bring to Front·Send to Back(OrderKey를 바꾸는 MoveNode).
//! - [`group`]: Group·Ungroup·Frame Selection.
//! - [`plan`]: 상자 원천([`plan::Geometry`])을 받아 정렬·분배·행열 정리·뒤집기·비례 확대 계획을 만드는 한 곳(runtime은
//!   scene, CLI·MCP는 문서 값).
//!
//! 계산은 셸·scene 없이 문서 모델과 호출자가 준 상자만 읽습니다. 화면 상자(Text layout 결과 등)가 필요한 값은
//! 호출자(runtime `canvas::transform`, 이후 CLI·MCP)가 넘깁니다.

pub mod align;
pub mod place;
pub mod plan;

#[cfg(test)]
pub(crate) mod test_fixtures {
    //! 배치 계획 test 문서.
    use stellar_core_types::ids::StableId;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};
    use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
    use stellar_document::fragment::FragmentPayload;
    use stellar_document::frame::FramePayload;
    use stellar_document::model::DocumentModel;
    use stellar_document::modifier::{
        ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
    };
    use stellar_document::records::{NodePayload, NodeRecord};
    use stellar_document::shape::{
        EllipseParams, LineCap, LineParams, RectangleParams, ShapeGeometry,
    };
    use stellar_document::text::TextPayload;

    use crate::command::Command;
    use crate::id::{CommandId, CommandOrigin};
    use crate::state::DocumentState;
    use crate::transaction::CommandTransaction;

    pub fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    /// 결정적 새 id(1000번부터).
    pub fn ids() -> impl FnMut() -> StableId {
        let mut next = 1000;
        move || {
            next += 1;
            sid(next)
        }
    }

    pub fn record(id: u64, parent: Option<u64>, key: u8, payload: NodePayload) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![key]).expect("key"),
            visible: true,
            locked: false,
            display_name: format!("n{id}"),
            payload,
        }
    }

    pub fn frame(w: f32, h: f32, x: f32, y: f32) -> NodePayload {
        NodePayload::Frame(FramePayload {
            size: Size2::new(w, h).unwrap(),
            transform: Transform2D::new(1., 0., 0., 1., x, y).unwrap(),
            ..FramePayload::default()
        })
    }

    pub fn rectangle(w: f32, h: f32) -> NodePayload {
        NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(w, h).unwrap(),
            corner_radius: Default::default(),
        }))
    }

    pub fn position(node: u64, order: u8, x: f32, y: f32) -> ModifierEntry {
        ModifierEntry {
            id: ModifierId::new(sid(500 + node)),
            order: OrderKey::from_bytes(vec![order]).unwrap(),
            kind: ModifierKind::Transform,
            enabled: true,
            payload: ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., x, y).unwrap()),
        }
    }

    pub fn with_stack(model: &mut DocumentModel, node: u64, entries: Vec<ModifierEntry>) {
        let mut stack = ModifierStack::new(sid(node));
        for entry in entries {
            stack.insert(entry).unwrap();
        }
        model.set_modifier_stack(stack).unwrap();
    }

    /// View 3(root 10, 400×300) 아래:
    /// - 11 Frame 100×50, translate(10, 20)
    /// - 12 Fragment
    /// - 13 Rectangle 40×20, 위치 modifier translate(30, 40)
    /// - 14 Ellipse bounds (5, 5, 20, 10), modifier 없음
    /// - 15 Line (0, 0)-(10, 0)
    /// - 16 Text(빈 본문)
    pub fn shapes() -> DocumentModel {
        let mut definition = DocumentDefinition::new(sid(1), "문서".to_string());
        let mut page = PageDefinition::new(sid(2), "페이지".to_string());
        page.add_view(ViewDefinition::new(sid(3), "화면".to_string(), sid(10)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut model = DocumentModel::new(definition);
        model
            .insert_node(record(10, None, 0x10, frame(400., 300., 0., 0.)))
            .unwrap();
        model
            .insert_node(record(11, Some(10), 0x10, frame(100., 50., 10., 20.)))
            .unwrap();
        model
            .insert_node(record(
                12,
                Some(10),
                0x20,
                NodePayload::Fragment(FragmentPayload),
            ))
            .unwrap();
        model
            .insert_node(record(13, Some(10), 0x30, rectangle(40., 20.)))
            .unwrap();
        with_stack(&mut model, 13, vec![position(13, 0x10, 30., 40.)]);
        let ellipse = EllipseParams::new(Rect::new(
            Vec2::new(5., 5.).unwrap(),
            Size2::new(20., 10.).unwrap(),
        ))
        .unwrap();
        model
            .insert_node(record(
                14,
                Some(10),
                0x40,
                NodePayload::Shape(ShapeGeometry::Ellipse(ellipse)),
            ))
            .unwrap();
        model
            .insert_node(record(
                15,
                Some(10),
                0x50,
                NodePayload::Shape(ShapeGeometry::Line(LineParams {
                    start: Vec2::new(0., 0.).unwrap(),
                    end: Vec2::new(10., 0.).unwrap(),
                    cap: LineCap::Butt,
                })),
            ))
            .unwrap();
        model
            .insert_node(record(
                16,
                Some(10),
                0x60,
                NodePayload::Text(TextPayload::empty(0)),
            ))
            .unwrap();
        model
    }

    /// 모델 사본(`DocumentModel`은 Clone이 아님).
    pub fn copy(model: &DocumentModel) -> DocumentModel {
        DocumentModel::from_records(model.to_records().unwrap())
            .unwrap()
            .0
    }

    /// command들을 transaction 하나로 적용하고 (적용 뒤 상태, 역 transaction)을 돌려줍니다.
    pub fn apply(
        model: DocumentModel,
        commands: Vec<Command>,
    ) -> (DocumentState, CommandTransaction) {
        let mut state = DocumentState::new(model);
        let revision = state.revision();
        let tx = commands.into_iter().fold(
            CommandTransaction::new(CommandId::from_bytes([7; 16]), CommandOrigin::User),
            |tx, command| tx.with_command(revision, command),
        );
        let applied = state.execute_transaction(&tx).expect("transaction 적용");
        (state, applied.inverse)
    }

    /// 역 transaction을 적용합니다(undo 1회).
    pub fn undo(state: &mut DocumentState, inverse: &CommandTransaction) {
        let tx = inverse.rebased(state.revision());
        state.execute_transaction(&tx).expect("undo");
    }
}
