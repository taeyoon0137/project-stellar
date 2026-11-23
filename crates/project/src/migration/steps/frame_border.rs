//! Frame 테두리 4변 longhand와 padding box 보정 (IX.11.10, ADR-0075 개정 결정 10, ADR-0024 결정 14).
//!
//! 제품 schema 1의 Frame 테두리는 묶음 값 `Border{color, thickness}`(key 0x0105) 하나이고, 레이아웃 공간을 차지하지
//! 않았습니다(scene이 border box 안쪽에 그림, 자식 좌표는 border box 원점 기준). schema 2부터 테두리는 CSS longhand
//! 열두 개(`stellar_document::css::border`)이고, CSS처럼 border box 안쪽이 padding box입니다. 그래서 테두리가 있는
//! 부모의 자식 화면 위치와 부모의 내용 기반 크기가 바뀌지 않도록 이 step이 layout record를 보정합니다.
//!
//! - **값**: 옛 묶음 값은 값 변환 없이 longhand로 읽힙니다(`BorderSides::from_payload`: 있으면 네 변 같은 두께·`solid`·
//!   색, 없으면 `medium none currentcolor`). payload bytes는 바꾸지 않습니다.
//! - **Free 자식**(`position: absolute`): containing block이 부모 padding box가 되므로 지정된 길이 inset에서 그 변의
//!   computed border 두께를 뺍니다(`left`·`top`·`right`·`bottom`). IX.11.6 이전 schema에는 layout record가 없어 모든
//!   자식이 Free 자식이고, IX.11.9 step([`super::frame_position`])이 위치를 `left`·`top`으로 옮긴 뒤 이 step을
//!   적용합니다. 백분율 inset·크기는 기준(padding box)이 줄어 길이 보정으로 보존할 수 없으므로 보고서에 남깁니다.
//! - **flow 부모**(명시 `display: flex|grid`): 각 변 `padding-*`(길이)에서 그 변 border 두께를 빼 flow 자식 위치와
//!   부모의 내용 기반 크기를 보존합니다. padding이 두께보다 작은 변(미설정 포함)은 `0px`로 두고 보존하지 못한 차이를
//!   노드·변별로 보고합니다. 백분율 padding은 보고합니다.
//! - **위치가 payload에 있는 자식**: layout record에 inset이 없는 자식(IX.11.9가 옮기지 않는 Frame이 아닌 node의 bounds
//!   원점 등)은 레이아웃 입력이 아니므로(ADR-0086 결정 3) 보정하지 않고 보고합니다.
//! - **0x0105 참조**: schema 2에서 0x0105는 묶음 key입니다. 저장 참조(ValueBinding target, Variant property patch)는
//!   IX.11.9·IX.11.22와 같은 규칙으로 값 변환 없이 보존하고 보고합니다([`border_bundle_references`]).
//!
//! 이 step은 schema 1 → 2에서 한 번만 실행되는 변환입니다(같은 입력에 두 번 적용하면 두께를 두 번 뺍니다).
//! [`super::super::PRODUCT_STEPS`]에는 IX.11.9와 같은 시점(scene·layout이 border longhand와 layout 결과로 그리게 된 뒤)에
//! 1→2 묶음 step으로 등록합니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_document::css::border::{self as border_css, BorderSide, BorderSides};
use stellar_document::layout_record::{LayoutLonghand, LayoutRecord, LayoutValue};
use stellar_document::records::{DocumentRecords, NodePayload};

/// padding이 border 두께보다 작아 보존하지 못한 변.
#[derive(Debug, Clone, PartialEq)]
pub struct PaddingShortfall {
    /// flow 부모 Frame.
    pub node: StableId,
    /// 변.
    pub side: BorderSide,
    /// 그 변 border 두께(px).
    pub border: f32,
    /// 원래 padding(px, 미설정은 0).
    pub padding: f32,
    /// 보존하지 못한 차이(px) = border − padding.
    pub lost: f32,
}

/// 길이 보정으로 보존할 수 없는 값.
#[derive(Debug, Clone, PartialEq)]
pub struct Unpreserved {
    /// 값을 가진 node.
    pub node: StableId,
    /// longhand.
    pub longhand: LayoutLonghand,
    /// 이유.
    pub reason: &'static str,
}

/// 저장 참조가 schema 2의 묶음 key 0x0105를 가리킵니다(값 변환 없이 보존).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorderBundleReference {
    /// 참조 경로(그대로).
    pub path: PropertyPath,
    /// 대신 써야 할 longhand key.
    pub longhands: [PropertyKey; 12],
}

/// 변환 보고서.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameBorderReport {
    /// computed 두께가 0보다 큰 테두리를 가진 Frame.
    pub bordered: Vec<StableId>,
    /// inset을 보정한 Free 자식.
    pub insets_adjusted: Vec<StableId>,
    /// padding을 보정한 flow 부모.
    pub padding_adjusted: Vec<StableId>,
    /// 보존하지 못한 padding 차이(노드·변별).
    pub padding_shortfall: Vec<PaddingShortfall>,
    /// 백분율 inset·크기·padding처럼 보정하지 못한 값.
    pub unpreserved: Vec<Unpreserved>,
    /// 위치가 payload에 있어 보정하지 않은 자식.
    pub payload_positioned: Vec<StableId>,
}

/// 변환 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameBorderError {
    /// 대상 node.
    pub node: StableId,
    /// 원인.
    pub reason: String,
}

const INSETS: [(LayoutLonghand, BorderSide); 4] = [
    (LayoutLonghand::Top, BorderSide::Top),
    (LayoutLonghand::Right, BorderSide::Right),
    (LayoutLonghand::Bottom, BorderSide::Bottom),
    (LayoutLonghand::Left, BorderSide::Left),
];

const PADDINGS: [(LayoutLonghand, BorderSide); 4] = [
    (LayoutLonghand::PaddingTop, BorderSide::Top),
    (LayoutLonghand::PaddingRight, BorderSide::Right),
    (LayoutLonghand::PaddingBottom, BorderSide::Bottom),
    (LayoutLonghand::PaddingLeft, BorderSide::Left),
];

fn side_index(side: BorderSide) -> usize {
    BorderSide::ALL.iter().position(|s| *s == side).expect("변")
}

fn is_flow_container(record: Option<&LayoutRecord>) -> bool {
    matches!(
        record.and_then(|r| r.get(LayoutLonghand::Display)),
        Some(LayoutValue::Keyword("flex" | "grid"))
    )
}

fn is_free(record: Option<&LayoutRecord>) -> bool {
    matches!(
        record.and_then(|r| r.get(LayoutLonghand::Position)),
        Some(LayoutValue::Keyword("absolute"))
    )
}

fn has_inset(record: Option<&LayoutRecord>) -> bool {
    record.is_some_and(|r| INSETS.iter().any(|(l, _)| r.get(*l).is_some()))
}

/// 레코드의 테두리를 CSS longhand 의미로 옮기고 padding box 보정을 적용합니다. IX.11.9 step 뒤에 적용합니다.
pub fn split_frame_border(
    mut records: DocumentRecords,
) -> Result<(DocumentRecords, FrameBorderReport), FrameBorderError> {
    let mut report = FrameBorderReport::default();
    // 테두리가 있는 Frame → 변별 computed 두께.
    let mut widths: BTreeMap<StableId, [f32; 4]> = BTreeMap::new();
    for node in &records.nodes {
        if let NodePayload::Frame(frame) = &node.payload {
            let computed = BorderSides::from_payload(frame.border).computed_widths();
            if computed.iter().any(|w| *w > 0.0) {
                widths.insert(node.id, computed);
                report.bordered.push(node.id);
            }
        }
    }
    let mut layout: BTreeMap<StableId, LayoutRecord> = records
        .layout_records
        .drain(..)
        .map(|r| (r.node, r))
        .collect();
    let error = |node: StableId, reason: String| FrameBorderError { node, reason };

    // flow 부모의 padding 보정.
    for (&parent, w) in &widths {
        if !is_flow_container(layout.get(&parent)) {
            continue;
        }
        let record = layout
            .entry(parent)
            .or_insert_with(|| LayoutRecord::new(parent));
        let mut changed = false;
        for (longhand, side) in PADDINGS {
            let border = w[side_index(side)];
            if border <= 0.0 {
                continue;
            }
            let padding = match record.get(longhand) {
                None => 0.0,
                Some(LayoutValue::Length(value)) => *value,
                Some(_) => {
                    report.unpreserved.push(Unpreserved {
                        node: parent,
                        longhand,
                        reason: "백분율 padding은 border 두께를 길이로 뺄 수 없습니다",
                    });
                    continue;
                }
            };
            let next = padding - border;
            if next < 0.0 {
                report.padding_shortfall.push(PaddingShortfall {
                    node: parent,
                    side,
                    border,
                    padding,
                    lost: border - padding,
                });
                if record.get(longhand).is_some() {
                    record
                        .set(longhand, LayoutValue::Length(0.0))
                        .map_err(|e| error(parent, e.to_string()))?;
                    changed = true;
                }
            } else {
                record
                    .set(longhand, LayoutValue::Length(next + 0.0))
                    .map_err(|e| error(parent, e.to_string()))?;
                changed = true;
            }
        }
        if changed {
            report.padding_adjusted.push(parent);
        }
    }

    // Free 자식의 inset 보정. 부모 순서와 무관하게 자식마다 한 번입니다.
    for node in &records.nodes {
        let Some(parent) = node.parent else {
            continue;
        };
        let Some(w) = widths.get(&parent) else {
            continue;
        };
        let flow_parent = is_flow_container(layout.get(&parent));
        let record = layout.get(&node.id);
        let free = is_free(record) || (!flow_parent && has_inset(record));
        if !free {
            if !flow_parent && !matches!(node.payload, NodePayload::Fragment(_)) {
                // flow가 아닌 부모의 위치 longhand 없는 자식: 위치가 payload에 있습니다.
                report.payload_positioned.push(node.id);
            }
            continue;
        }
        let record = layout
            .get_mut(&node.id)
            .expect("Free 자식은 layout record가 있습니다");
        let mut changed = false;
        for (longhand, side) in INSETS {
            let border = w[side_index(side)];
            match record.get(longhand) {
                Some(LayoutValue::Length(value)) if border > 0.0 => {
                    let next = *value - border + 0.0;
                    record
                        .set(longhand, LayoutValue::Length(next))
                        .map_err(|e| error(node.id, e.to_string()))?;
                    changed = true;
                }
                Some(LayoutValue::Percentage(_)) => report.unpreserved.push(Unpreserved {
                    node: node.id,
                    longhand,
                    reason: "백분율 inset의 기준(padding box)이 border 두께만큼 줄어듭니다",
                }),
                _ => {}
            }
        }
        for longhand in [
            LayoutLonghand::Width,
            LayoutLonghand::Height,
            LayoutLonghand::MinWidth,
            LayoutLonghand::MinHeight,
            LayoutLonghand::MaxWidth,
            LayoutLonghand::MaxHeight,
        ] {
            if matches!(record.get(longhand), Some(LayoutValue::Percentage(_))) {
                report.unpreserved.push(Unpreserved {
                    node: node.id,
                    longhand,
                    reason: "백분율 크기의 기준(padding box)이 border 두께만큼 줄어듭니다",
                });
            }
        }
        if changed {
            report.insets_adjusted.push(node.id);
        }
    }

    records.layout_records = layout.into_values().filter(|r| !r.is_empty()).collect();
    records.sort_canonical();
    Ok((records, report))
}

/// 저장 참조 경로 중 schema 2의 묶음 key 0x0105를 가리키는 것. 경로는 바꾸지 않습니다(값 변환 없이 보존).
pub fn border_bundle_references<'a>(
    paths: impl IntoIterator<Item = &'a PropertyPath>,
) -> Vec<BorderBundleReference> {
    paths
        .into_iter()
        .filter(|path| path.keys().contains(&stellar_document::frame::keys::BORDER))
        .map(|path| BorderBundleReference {
            path: path.clone(),
            longhands: border_css::keys::LONGHANDS,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::value::{Size2, Transform2D};
    use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
    use stellar_document::frame::{Border, FramePayload};
    use stellar_document::records::NodeRecord;
    use stellar_document::shape::{RectangleParams, ShapeGeometry};
    use stellar_document::validate::validate_document;

    use crate::migration::steps::frame_position::split_frame_position;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn frame(id: u64, parent: Option<u64>, border: Option<f32>, tx: f32, ty: f32) -> NodeRecord {
        let color = Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap();
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![id as u8]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload: NodePayload::Frame(FramePayload {
                size: Size2::new(100.0, 80.0).unwrap(),
                transform: Transform2D::new(1.0, 0.0, 0.0, 1.0, tx, ty).unwrap(),
                border: border.map(|t| Border::new(color, t).unwrap()),
                ..FramePayload::default()
            }),
        }
    }

    fn rect(id: u64, parent: u64) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: Some(sid(parent)),
            order_key: OrderKey::from_bytes(vec![id as u8]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload: NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(4.0, 4.0).unwrap(),
                corner_radius: Default::default(),
            })),
        }
    }

    fn document(nodes: Vec<NodeRecord>) -> DocumentRecords {
        let mut definition = DocumentDefinition::new(sid(100), String::new());
        let mut page = PageDefinition::new(sid(101), String::new());
        page.add_view(ViewDefinition::new(sid(102), String::new(), sid(1)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut records = DocumentRecords::new(definition);
        records.nodes = nodes;
        records
    }

    fn layout(records: &DocumentRecords, id: u64) -> Option<&LayoutRecord> {
        records.layout_records.iter().find(|r| r.node == sid(id))
    }

    fn length(records: &DocumentRecords, id: u64, longhand: LayoutLonghand) -> Option<f32> {
        match layout(records, id)?.get(longhand)? {
            LayoutValue::Length(v) => Some(*v),
            _ => None,
        }
    }

    /// schema 1 화면 위치: 부모 border box 원점 + 자식 이동 성분(테두리는 공간을 차지하지 않음).
    /// schema 2 화면 위치: 부모 border box 원점 + 그 변 border 두께(padding box 원점) + `left`·`top`.
    fn screen_after(records: &DocumentRecords, child: u64, border: [f32; 4]) -> (f32, f32) {
        (
            border[3] + length(records, child, LayoutLonghand::Left).unwrap(),
            border[0] + length(records, child, LayoutLonghand::Top).unwrap(),
        )
    }

    #[test]
    fn free_children_of_a_bordered_parent_keep_their_screen_position() {
        // 검증 기준(IX.11.10): border가 있는 부모의 Free 자식 화면 위치 보존 fixture(IX.11.9 뒤 적용).
        let before = document(vec![
            frame(1, None, None, 0.0, 0.0),
            frame(2, Some(1), Some(4.0), 10.0, 20.0),
            frame(3, Some(2), None, 12.5, -3.0),
            frame(4, Some(2), Some(1.0), 0.0, 0.0),
            rect(5, 2),
        ]);
        let (positioned, _) = split_frame_position(before).unwrap();
        let (after, report) = split_frame_border(positioned.clone()).unwrap();
        validate_document(&after).unwrap();
        assert_eq!(report.bordered, [sid(2), sid(4)]);
        assert_eq!(report.insets_adjusted, [sid(3), sid(4)]);
        // 부모 2의 border box 원점 기준 화면 위치가 schema 1과 같습니다.
        assert_eq!(screen_after(&after, 3, [4.0; 4]), (12.5, -3.0));
        assert_eq!(screen_after(&after, 4, [4.0; 4]), (0.0, 0.0));
        // 테두리 없는 부모(1)의 자식 2는 그대로입니다.
        assert_eq!(
            length(&after, 2, LayoutLonghand::Left),
            length(&positioned, 2, LayoutLonghand::Left)
        );
        // 위치가 payload에 있는 Rectangle은 보정하지 않고 보고합니다.
        assert_eq!(report.payload_positioned, [sid(5)]);
        // payload(테두리 묶음 값)는 bytes 그대로입니다.
        for (a, b) in positioned.nodes.iter().zip(&after.nodes) {
            assert_eq!(a.payload, b.payload);
        }
    }

    #[test]
    fn right_bottom_and_percentages_of_intermediate_schema_free_children() {
        // 중간 schema(layout record 있음): 지정된 right·bottom도 보정하고 백분율은 보고합니다.
        let mut records = document(vec![
            frame(1, None, None, 0.0, 0.0),
            frame(2, Some(1), Some(2.0), 0.0, 0.0),
            frame(3, Some(2), None, 0.0, 0.0),
        ]);
        let mut child = LayoutRecord::new(sid(3));
        for (l, v) in [
            (LayoutLonghand::Position, LayoutValue::Keyword("absolute")),
            (LayoutLonghand::Right, LayoutValue::Length(10.0)),
            (LayoutLonghand::Bottom, LayoutValue::Length(1.0)),
            (LayoutLonghand::Left, LayoutValue::Percentage(50.0)),
            (LayoutLonghand::Width, LayoutValue::Percentage(25.0)),
        ] {
            child.set(l, v).unwrap();
        }
        records.layout_records.push(child);
        let (after, report) = split_frame_border(records).unwrap();
        assert_eq!(length(&after, 3, LayoutLonghand::Right), Some(8.0));
        assert_eq!(length(&after, 3, LayoutLonghand::Bottom), Some(-1.0));
        let unpreserved: Vec<LayoutLonghand> =
            report.unpreserved.iter().map(|u| u.longhand).collect();
        assert_eq!(
            unpreserved,
            [LayoutLonghand::Left, LayoutLonghand::Width],
            "백분율 inset·크기는 보고서에 남습니다"
        );
    }

    #[test]
    fn flow_parent_padding_absorbs_the_border_and_reports_shortfall() {
        // 검증 기준(IX.11.10): border가 있는 flex 부모의 flow 자식 위치와 내용 기반 크기 보존, padding이 border
        // 두께보다 작은 변의 보고서 기록.
        let mut records = document(vec![
            frame(1, None, None, 0.0, 0.0),
            frame(2, Some(1), Some(3.0), 0.0, 0.0),
            frame(3, Some(2), None, 0.0, 0.0),
        ]);
        let mut parent = LayoutRecord::new(sid(2));
        for (l, v) in [
            (LayoutLonghand::Display, LayoutValue::Keyword("flex")),
            (LayoutLonghand::PaddingTop, LayoutValue::Length(8.0)),
            (LayoutLonghand::PaddingRight, LayoutValue::Length(3.0)),
            (LayoutLonghand::PaddingBottom, LayoutValue::Length(1.0)),
        ] {
            parent.set(l, v).unwrap();
        }
        records.layout_records.push(parent);
        let (after, report) = split_frame_border(records.clone()).unwrap();
        assert_eq!(report.padding_adjusted, [sid(2)]);
        // content box 원점(border + padding)과 내용 기반 크기(content + padding + border)는 padding이 두께 이상인 변에서
        // schema 1(padding만)과 같습니다.
        assert_eq!(length(&after, 2, LayoutLonghand::PaddingTop), Some(5.0));
        assert_eq!(3.0 + 5.0, 8.0);
        assert_eq!(length(&after, 2, LayoutLonghand::PaddingRight), Some(0.0));
        // padding 1px < border 3px인 아래쪽, 미설정(0)인 왼쪽은 보존하지 못한 차이를 보고합니다.
        assert_eq!(length(&after, 2, LayoutLonghand::PaddingBottom), Some(0.0));
        assert_eq!(
            layout(&after, 2).unwrap().get(LayoutLonghand::PaddingLeft),
            None
        );
        let shortfall: Vec<(BorderSide, f32)> = report
            .padding_shortfall
            .iter()
            .map(|s| (s.side, s.lost))
            .collect();
        assert_eq!(
            shortfall,
            [(BorderSide::Bottom, 2.0), (BorderSide::Left, 3.0)]
        );
        // flow 자식(position 미설정)은 inset을 보정하지 않습니다.
        assert!(report.insets_adjusted.is_empty());
        assert!(report.payload_positioned.is_empty());
    }

    #[test]
    fn border_bundle_references_are_reported_and_left_unchanged() {
        // 검증 기준(IX.11.10): 참조 bytes 보존 fixture.
        let node = sid(9);
        let bundle = PropertyPath::with_keys(node, vec![stellar_document::frame::keys::BORDER]);
        let longhand = PropertyPath::with_keys(node, vec![border_css::keys::BORDER_TOP_WIDTH]);
        let before = (bundle.clone(), longhand.clone());
        let found = border_bundle_references([&bundle, &longhand]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, bundle);
        assert_eq!(found[0].longhands, border_css::keys::LONGHANDS);
        assert_eq!((bundle, longhand), before);
    }
}
