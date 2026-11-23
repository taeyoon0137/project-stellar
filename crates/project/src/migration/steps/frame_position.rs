//! Frame 위치와 transform 분리 (IX.11.9, ADR-0024 결정 8, ADR-0086 결정 3).
//!
//! - 부모가 있는 Frame: transform 이동 성분(tx, ty)을 layout record `left`·`top`(px)으로 옮기고 `position: absolute`를
//!   명시합니다. 옛 schema에는 layout 정의가 없어 모든 자식이 Free 자식이기 때문입니다. 선형 부분(a, b, c, d)은
//!   `matrix(a, b, c, d, 0, 0)`으로 bit 그대로 둡니다. 회전·확대로 분해하지 않습니다.
//! - Frame transform은 border box 좌상단 원점 기준이므로 `transform-origin: 0 0`과 같은 뜻입니다. 이 의미는 모델에
//!   고정되어 있어 따로 저장하지 않습니다(새 노드의 `50% 50%` 기본 origin은 origin longhand가 생길 때 정함).
//! - 부모가 없는 Frame(View root)의 이동 성분은 페이지 캔버스 배치입니다. 캔버스 배치 속성 저장 위치가 아직 없어
//!   그대로 두고 보고서에 남깁니다.
//! - 이미 `left`·`top`·`position`을 명시한 Frame은 건드리지 않고 보고서에 남깁니다(이미 옮긴 문서).

use stellar_core_types::ids::StableId;
use stellar_core_types::value::Transform2D;
use stellar_document::layout_record::{LayoutLonghand, LayoutRecord, LayoutValue};
use stellar_document::records::{DocumentRecords, NodePayload};

/// 변환 보고서.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FramePositionReport {
    /// 위치를 옮긴 Frame.
    pub moved: Vec<StableId>,
    /// 캔버스 배치로 남긴 root Frame(이동 성분이 0이 아닌 것).
    pub canvas_roots: Vec<StableId>,
    /// 이미 위치 longhand가 있어 건너뛴 Frame.
    pub skipped: Vec<StableId>,
}

/// 변환 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramePositionError {
    /// 대상 Frame.
    pub node: StableId,
    /// 원인.
    pub reason: String,
}

/// 레코드의 Frame 위치를 layout record로 옮깁니다.
pub fn split_frame_position(
    mut records: DocumentRecords,
) -> Result<(DocumentRecords, FramePositionReport), FramePositionError> {
    let mut report = FramePositionReport::default();
    for node in &mut records.nodes {
        let NodePayload::Frame(frame) = &mut node.payload else {
            continue;
        };
        let (a, b, c, d, tx, ty) = frame.transform.components();
        if node.parent.is_none() {
            if (tx, ty) != (0.0, 0.0) {
                report.canvas_roots.push(node.id);
            }
            continue;
        }
        let index = records
            .layout_records
            .iter()
            .position(|r| r.node == node.id);
        let mut layout = index
            .map(|i| records.layout_records[i].clone())
            .unwrap_or_else(|| LayoutRecord::new(node.id));
        if [
            LayoutLonghand::Left,
            LayoutLonghand::Top,
            LayoutLonghand::Position,
        ]
        .iter()
        .any(|l| layout.get(*l).is_some())
        {
            report.skipped.push(node.id);
            continue;
        }
        let error = |reason: String| FramePositionError {
            node: node.id,
            reason,
        };
        for (longhand, value) in [
            (LayoutLonghand::Position, LayoutValue::Keyword("absolute")),
            (LayoutLonghand::Left, LayoutValue::Length(tx + 0.0)),
            (LayoutLonghand::Top, LayoutValue::Length(ty + 0.0)),
        ] {
            layout
                .set(longhand, value)
                .map_err(|e| error(e.to_string()))?;
        }
        frame.transform =
            Transform2D::new(a, b, c, d, 0.0, 0.0).map_err(|e| error(e.to_string()))?;
        match index {
            Some(i) => records.layout_records[i] = layout,
            None => records.layout_records.push(layout),
        }
        report.moved.push(node.id);
    }
    records.sort_canonical();
    Ok((records, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
    use stellar_document::frame::FramePayload;
    use stellar_document::records::NodeRecord;
    use stellar_document::validate::validate_document;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn frame(id: u64, parent: Option<u64>, transform: Transform2D) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![id as u8]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload: NodePayload::Frame(FramePayload {
                transform,
                ..FramePayload::default()
            }),
        }
    }

    fn records() -> DocumentRecords {
        let mut definition = DocumentDefinition::new(sid(100), String::new());
        let mut page = PageDefinition::new(sid(101), String::new());
        page.add_view(ViewDefinition::new(sid(102), String::new(), sid(1)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut records = DocumentRecords::new(definition);
        records.nodes = vec![
            frame(
                1,
                None,
                Transform2D::new(1.0, 0.0, 0.0, 1.0, 40.0, 30.0).unwrap(),
            ),
            frame(
                2,
                Some(1),
                Transform2D::new(0.6, 0.8, -0.8, 0.6, 12.5, -4.0).unwrap(),
            ),
            frame(3, Some(2), Transform2D::identity()),
        ];
        records
    }

    #[test]
    fn translation_moves_to_left_top_and_linear_part_stays_bit_identical() {
        let before = records();
        let (after, report) = split_frame_position(before.clone()).unwrap();
        validate_document(&after).unwrap();
        assert_eq!(report.moved, [sid(2), sid(3)]);
        assert_eq!(report.canvas_roots, [sid(1)]);
        let payload = |records: &DocumentRecords, id| match &records
            .nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .payload
        {
            NodePayload::Frame(f) => f.transform,
            _ => unreachable!(),
        };
        let (a, b, c, d, _, _) = payload(&before, sid(2)).components();
        let (a2, b2, c2, d2, tx, ty) = payload(&after, sid(2)).components();
        assert_eq!(
            [a, b, c, d].map(f32::to_bits),
            [a2, b2, c2, d2].map(f32::to_bits)
        );
        assert_eq!((tx, ty), (0.0, 0.0));
        let layout = after
            .layout_records
            .iter()
            .find(|r| r.node == sid(2))
            .unwrap();
        assert_eq!(
            layout.get(LayoutLonghand::Left),
            Some(&LayoutValue::Length(12.5))
        );
        assert_eq!(
            layout.get(LayoutLonghand::Top),
            Some(&LayoutValue::Length(-4.0))
        );
        assert_eq!(
            layout.get(LayoutLonghand::Position),
            Some(&LayoutValue::Keyword("absolute"))
        );
        // CSS 순서 합성(translate(left, top) ∘ matrix)이 옛 transform과 같습니다.
        let composed = stellar_layout_compose((12.5, -4.0), payload(&after, sid(2)));
        assert_eq!(composed, payload(&before, sid(2)));
        // root 이동 성분은 그대로입니다.
        assert_eq!(payload(&after, sid(1)), payload(&before, sid(1)));
        // 두 번 돌려도 같습니다(이미 옮긴 Frame은 건너뜀).
        let (again, report) = split_frame_position(after.clone()).unwrap();
        assert_eq!(again, after);
        assert_eq!(report.skipped, [sid(2), sid(3)]);
    }

    /// `stellar_layout::transform::css_node_transform`(origin 0)과 같은 합성. project는 layout에 의존하지 않아
    /// 같은 식(translate ∘ matrix)을 test에서만 씁니다.
    fn stellar_layout_compose(position: (f32, f32), linear: Transform2D) -> Transform2D {
        linear
            .then(&Transform2D::new(1.0, 0.0, 0.0, 1.0, position.0, position.1).unwrap())
            .unwrap()
    }
}
