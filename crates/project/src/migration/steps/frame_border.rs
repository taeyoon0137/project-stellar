//! Frame 테두리 4변 longhand와 padding box 보정 (IX.11.10, ADR-0075 개정 결정 10, ADR-0024 결정 14).
//!
//! 제품 schema 1의 Frame 테두리는 묶음 값 `Border{color, thickness}`(key 0x0105) 하나이고, 레이아웃 공간을 차지하지
//! 않았습니다(scene이 border box 안쪽에 그림, 자식 좌표는 border box 원점 기준). schema 2부터 테두리는 CSS longhand
//! 열두 개(`stellar_document::css::border`)이고, CSS처럼 border box 안쪽이 padding box입니다. 그래서 테두리가 있는
//! 부모의 자식 화면 위치와 부모의 내용 기반 크기가 바뀌지 않도록 이 step이 layout record를 보정합니다.
//!
//! - **값**: 옛 묶음 값은 값 변환 없이 longhand로 읽힙니다(`BorderSides::from_payload`: 있으면 네 변 같은 두께·`solid`·
//!   색, 없으면 `medium none currentcolor`). 테두리 값 bytes는 바꾸지 않습니다(바뀌는 payload 필드는 아래 content-box
//!   크기 보정뿐입니다).
//! - **해석 기준**: `display`·`position`·`box-sizing`·`padding-*`·inset은 명시 값이 없으면 문서의 활성 기본값 세트
//!   (ADR-0082) 값입니다(`style_defaults::resolve`). `css` 세트의 Frame은 `display: block`, `react-native` 세트는
//!   `display: flex`이고 둘 다 flow 컨테이너입니다(block도 in-flow 자식을 content box에 놓음, ADR-0076).
//! - **Free 자식**(`position: absolute`): containing block이 부모 padding box가 되므로 길이 inset에서 그 변의 computed
//!   border 두께를 뺍니다(`left`·`top`·`right`·`bottom`). IX.11.6 이전 schema에는 layout record가 없어 모든 Frame
//!   자식이 Free 자식이고, IX.11.9 step([`super::frame_position`])이 위치를 `left`·`top`으로 옮긴 뒤 이 step을
//!   적용합니다. 백분율 inset·크기는 기준(padding box)이 줄어 길이 보정으로 보존할 수 없으므로 보고서에 남깁니다.
//! - **테두리가 있는 Frame 자신**: 각 변 `padding-*`(길이)이 그 변 두께를 흡수합니다(padding − 두께, 0 아래로 가지
//!   않음). 그래서 in-flow 자식 위치(content box 원점)와 내용 기반 크기가 보존됩니다. padding이 두께보다 작은 변은
//!   in-flow 자식이 있으면 보존하지 못한 차이를 노드·변별로 보고합니다. 백분율 padding은 보고합니다.
//!   `box-sizing: content-box`(`css` 세트 기본)이면 border box = `width` + padding + border이므로, padding이 흡수하지
//!   못한 두께만큼 payload 크기(`width`·`height`, ADR-0086 결정 2)를 줄여 schema 1 border box 크기를 지킵니다.
//!   `border-box`이면 크기는 그대로입니다.
//! - **위치가 payload에 있는 자식**: layout record가 없는 Frame이 아닌 in-flow 자식(IX.11.9가 옮기지 않는 node의
//!   bounds 원점 등)은 위치가 레이아웃 입력이 아니므로(ADR-0086 결정 3) 보정하지 않고 보고합니다.
//! - **0x0105 참조**: schema 2에서 0x0105는 묶음 key입니다. 저장 참조(ValueBinding target, Variant property patch)는
//!   IX.11.9·IX.11.22와 같은 규칙으로 값 변환 없이 보존하고 보고합니다([`border_bundle_references`]).
//!
//! 이 step은 schema 1 → 2에서 한 번만 실행되는 변환입니다(같은 입력에 두 번 적용하면 두께를 두 번 뺍니다).
//! [`super::super::PRODUCT_STEPS`]에는 IX.11.9와 같은 시점(scene·layout이 border longhand와 layout 결과로 그리게 된 뒤)에
//! 1→2 묶음 step으로 등록합니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::value::Size2;
use stellar_document::css::border::{self as border_css, BorderSide, BorderSides};
use stellar_document::css::style_defaults::{self, FlatSet, ResolvedLayout};
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
    /// content-box라 padding이 흡수하지 못한 두께만큼 payload 크기(`width`·`height`)를 줄인 Frame.
    pub size_adjusted: Vec<StableId>,
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

fn resolved(
    set: &FlatSet,
    payload: &NodePayload,
    layout: &BTreeMap<StableId, LayoutRecord>,
    id: StableId,
) -> Option<ResolvedLayout> {
    style_defaults::resolve(set, payload, layout.get(&id))
}

fn keyword(resolved: &ResolvedLayout, longhand: LayoutLonghand) -> Option<&'static str> {
    match resolved.get(longhand).value {
        LayoutValue::Keyword(k) => Some(k),
        _ => None,
    }
}

/// 레코드의 테두리를 CSS longhand 의미로 옮기고 padding box 보정을 적용합니다. IX.11.9 step 뒤에 적용합니다.
///
/// `set`은 문서의 활성 기본값 세트(ADR-0082, ADR-0086 결정 7)입니다. `display`·`position`·`box-sizing`·`padding-*`·
/// inset은 명시 값이 없으면 이 세트 값으로 풉니다(`style_defaults::resolve`). 그래서 `css` 세트의 `display: block`
/// 부모와 `react-native` 세트의 `display: flex` 부모도 flow 부모입니다.
pub fn split_frame_border(
    mut records: DocumentRecords,
    set: &FlatSet,
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

    // 자식 분류(보정 전 값 기준): Free 자식(`position: absolute`)과 in-flow 자식.
    let mut free_children: Vec<usize> = Vec::new();
    let mut has_in_flow: BTreeMap<StableId, bool> = BTreeMap::new();
    for (index, node) in records.nodes.iter().enumerate() {
        let Some(parent) = node.parent else {
            continue;
        };
        if !widths.contains_key(&parent) {
            continue;
        }
        // Fragment는 상자가 없습니다.
        let Some(child) = resolved(set, &node.payload, &layout, node.id) else {
            continue;
        };
        if keyword(&child, LayoutLonghand::Position) == Some("absolute") {
            free_children.push(index);
            continue;
        }
        has_in_flow.insert(parent, true);
        if !layout.contains_key(&node.id) && !matches!(node.payload, NodePayload::Frame(_)) {
            // layout record가 없는 Frame이 아닌 자식: schema 1 위치가 payload에 있습니다(ADR-0086 결정 3).
            report.payload_positioned.push(node.id);
        }
    }

    // 테두리가 있는 Frame 자신: padding이 두께를 흡수하고, content-box면 흡수하지 못한 두께만큼 크기를 줄입니다.
    let mut sizes: BTreeMap<StableId, (f32, f32)> = BTreeMap::new();
    for node in &records.nodes {
        let Some(w) = widths.get(&node.id) else {
            continue;
        };
        let own = resolved(set, &node.payload, &layout, node.id).expect("Frame은 상자가 있습니다");
        let content_box = keyword(&own, LayoutLonghand::BoxSizing) == Some("content-box");
        let in_flow = has_in_flow.get(&node.id).copied().unwrap_or(false);
        let mut residual = [0.0_f32; 4];
        let mut changed = false;
        for (longhand, side) in PADDINGS {
            let border = w[side_index(side)];
            if border <= 0.0 {
                continue;
            }
            let padding = match own.get(longhand).value {
                LayoutValue::Length(value) => value,
                _ => {
                    report.unpreserved.push(Unpreserved {
                        node: node.id,
                        longhand,
                        reason: "백분율 padding은 border 두께를 길이로 뺄 수 없습니다",
                    });
                    residual[side_index(side)] = border;
                    continue;
                }
            };
            let absorbed = padding.min(border).max(0.0);
            residual[side_index(side)] = border - absorbed;
            if absorbed > 0.0 {
                layout
                    .entry(node.id)
                    .or_insert_with(|| LayoutRecord::new(node.id))
                    .set(longhand, LayoutValue::Length(padding - absorbed + 0.0))
                    .map_err(|e| error(node.id, e.to_string()))?;
                changed = true;
            }
            if padding < border && in_flow {
                // content box 원점이 이 차이만큼 옮겨져 in-flow 자식 위치를 보존하지 못합니다.
                report.padding_shortfall.push(PaddingShortfall {
                    node: node.id,
                    side,
                    border,
                    padding,
                    lost: border - padding,
                });
            }
        }
        if changed {
            report.padding_adjusted.push(node.id);
        }
        if content_box {
            // content-box의 border box = width + padding + border. schema 1(border가 공간을 차지하지 않음)의
            // border box를 지키려면 padding이 흡수하지 못한 두께만큼 width·height(payload 크기)를 줄입니다.
            let NodePayload::Frame(frame) = &node.payload else {
                continue;
            };
            let horizontal =
                residual[side_index(BorderSide::Left)] + residual[side_index(BorderSide::Right)];
            let vertical =
                residual[side_index(BorderSide::Top)] + residual[side_index(BorderSide::Bottom)];
            if horizontal > 0.0 || vertical > 0.0 {
                let width = frame.size.width() - horizontal;
                let height = frame.size.height() - vertical;
                for (value, longhand) in [
                    (width, LayoutLonghand::Width),
                    (height, LayoutLonghand::Height),
                ] {
                    if value < 0.0 {
                        report.unpreserved.push(Unpreserved {
                            node: node.id,
                            longhand,
                            reason: "content-box 크기가 border 두께보다 작아 border box 크기를 보존할 수 없습니다",
                        });
                    }
                }
                sizes.insert(node.id, (width.max(0.0) + 0.0, height.max(0.0) + 0.0));
            }
        }
    }
    for node in &mut records.nodes {
        if let (Some((width, height)), NodePayload::Frame(frame)) =
            (sizes.get(&node.id), &mut node.payload)
        {
            frame.size = Size2::new(*width, *height).map_err(|e| error(node.id, e.to_string()))?;
            report.size_adjusted.push(node.id);
        }
    }

    // Free 자식의 inset 보정: containing block이 부모 padding box(border box 안쪽)가 됩니다.
    for index in free_children {
        let node = &records.nodes[index];
        let parent = node.parent.expect("자식");
        let w = widths[&parent];
        let child = resolved(set, &node.payload, &layout, node.id).expect("상자가 있는 자식");
        let mut changed = false;
        for (longhand, side) in INSETS {
            let border = w[side_index(side)];
            match child.get(longhand).value {
                LayoutValue::Length(value) if border > 0.0 => {
                    layout
                        .entry(node.id)
                        .or_insert_with(|| LayoutRecord::new(node.id))
                        .set(longhand, LayoutValue::Length(value - border + 0.0))
                        .map_err(|e| error(node.id, e.to_string()))?;
                    changed = true;
                }
                LayoutValue::Percentage(_) => report.unpreserved.push(Unpreserved {
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
            if matches!(child.get(longhand).value, LayoutValue::Percentage(_)) {
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

    fn css() -> FlatSet {
        style_defaults::builtin("css").unwrap()
    }

    fn react_native() -> FlatSet {
        style_defaults::builtin("react-native").unwrap()
    }

    fn size(records: &DocumentRecords, id: u64) -> (f32, f32) {
        let node = records.nodes.iter().find(|n| n.id == sid(id)).unwrap();
        match &node.payload {
            NodePayload::Frame(frame) => (frame.size.width(), frame.size.height()),
            _ => panic!("Frame"),
        }
    }

    /// schema 2 border box 크기: content-box면 width + padding + border, border-box면 width(ADR-0086 결정 8).
    fn border_box(
        records: &DocumentRecords,
        id: u64,
        border: [f32; 4],
        content_box: bool,
    ) -> (f32, f32) {
        let (w, h) = size(records, id);
        if !content_box {
            return (w, h);
        }
        let pad = |l| length(records, id, l).unwrap_or(0.0);
        (
            w + pad(LayoutLonghand::PaddingLeft)
                + pad(LayoutLonghand::PaddingRight)
                + border[1]
                + border[3],
            h + pad(LayoutLonghand::PaddingTop)
                + pad(LayoutLonghand::PaddingBottom)
                + border[0]
                + border[2],
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
        let (after, report) = split_frame_border(positioned.clone(), &css()).unwrap();
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
        // 위치가 payload에 있는 Rectangle은 보정하지 않고 보고합니다. 그 자식은 in-flow라 padding 0인 부모 2의 차이도
        // 보고합니다.
        assert_eq!(report.payload_positioned, [sid(5)]);
        assert_eq!(report.padding_shortfall.len(), 4);
        assert!(report
            .padding_shortfall
            .iter()
            .all(|s| s.node == sid(2) && s.lost == 4.0));
        // css 세트(content-box): border box 크기(schema 1 = payload 크기)를 지키도록 payload 크기를 줄입니다.
        assert_eq!(report.size_adjusted, [sid(2), sid(4)]);
        assert_eq!(size(&after, 2), (92.0, 72.0));
        assert_eq!(border_box(&after, 2, [4.0; 4], true), (100.0, 80.0));
        assert_eq!(border_box(&after, 4, [1.0; 4], true), (100.0, 80.0));
        // 테두리 값(묶음 값)과 다른 payload 필드는 그대로입니다.
        for (a, b) in positioned.nodes.iter().zip(&after.nodes) {
            match (&a.payload, &b.payload) {
                (NodePayload::Frame(a), NodePayload::Frame(b)) => {
                    assert_eq!(a.border, b.border);
                    assert_eq!(a.transform, b.transform);
                }
                (a, b) => assert_eq!(a, b),
            }
        }
        // border-box 세트(react-native)는 크기를 바꾸지 않고 inset만 보정합니다.
        let (rn, rn_report) = split_frame_border(positioned.clone(), &react_native()).unwrap();
        assert!(rn_report.size_adjusted.is_empty());
        assert_eq!(size(&rn, 2), (100.0, 80.0));
        assert_eq!(screen_after(&rn, 3, [4.0; 4]), (12.5, -3.0));
    }

    #[test]
    fn right_bottom_and_percentages_of_intermediate_schema_free_children() {
        // 중간 schema(layout record 있음): 지정된 right·bottom도 보정하고 백분율은 보고합니다. Frame의 width·height는
        // payload 크기라 record에 둘 수 없습니다(ADR-0086 결정 2).
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
            (LayoutLonghand::MaxWidth, LayoutValue::Percentage(25.0)),
        ] {
            child.set(l, v).unwrap();
        }
        records.layout_records.push(child);
        let (after, report) = split_frame_border(records, &css()).unwrap();
        validate_document(&after).unwrap();
        assert_eq!(length(&after, 3, LayoutLonghand::Right), Some(8.0));
        assert_eq!(length(&after, 3, LayoutLonghand::Bottom), Some(-1.0));
        let unpreserved: Vec<LayoutLonghand> =
            report.unpreserved.iter().map(|u| u.longhand).collect();
        assert_eq!(
            unpreserved,
            [LayoutLonghand::Left, LayoutLonghand::MaxWidth],
            "백분율 inset·크기는 보고서에 남습니다"
        );
        // in-flow 자식이 없으므로 padding 차이는 잃는 것이 없어 보고하지 않습니다.
        assert!(report.padding_shortfall.is_empty());
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
        let (after, report) = split_frame_border(records.clone(), &css()).unwrap();
        validate_document(&after).unwrap();
        assert_eq!(report.padding_adjusted, [sid(2)]);
        // content box 원점(border + padding)과 내용 기반 크기(content + padding + border)는 padding이 두께 이상인 변에서
        // schema 1(padding만)과 같습니다.
        assert_eq!(length(&after, 2, LayoutLonghand::PaddingTop), Some(5.0));
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
        // content-box: padding이 흡수하지 못한 두께(가로 3, 세로 2)만큼 크기를 줄여 border box를 지킵니다.
        // schema 1 border box = 100 + 0 + 3, 80 + 8 + 1.
        assert_eq!(size(&after, 2), (97.0, 78.0));
        assert_eq!(border_box(&after, 2, [3.0; 4], true), (103.0, 89.0));
        // flow 자식(position 미설정)은 inset을 보정하지 않습니다.
        assert!(report.insets_adjusted.is_empty());
        assert!(report.payload_positioned.is_empty());
    }

    #[test]
    fn default_set_display_makes_block_and_react_native_parents_flow_parents() {
        // 명시 display가 없는 부모도 기본값 세트 값으로 풉니다: css는 block, react-native는 flex(둘 다 flow).
        let mut records = document(vec![
            frame(1, None, None, 0.0, 0.0),
            frame(2, Some(1), Some(2.0), 0.0, 0.0),
            frame(3, Some(2), None, 0.0, 0.0),
        ]);
        let mut parent = LayoutRecord::new(sid(2));
        parent
            .set(LayoutLonghand::PaddingLeft, LayoutValue::Length(6.0))
            .unwrap();
        records.layout_records.push(parent);
        for set in [css(), react_native()] {
            let (after, report) = split_frame_border(records.clone(), &set).unwrap();
            assert_eq!(report.padding_adjusted, [sid(2)], "{}", set.name());
            assert_eq!(length(&after, 2, LayoutLonghand::PaddingLeft), Some(4.0));
            let sides: Vec<BorderSide> = report.padding_shortfall.iter().map(|s| s.side).collect();
            assert_eq!(
                sides,
                [BorderSide::Top, BorderSide::Right, BorderSide::Bottom]
            );
        }
        // react-native 세트(border-box)는 크기를 바꾸지 않습니다.
        let (rn, report) = split_frame_border(records, &react_native()).unwrap();
        assert!(report.size_adjusted.is_empty());
        assert_eq!(size(&rn, 2), (100.0, 80.0));
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
