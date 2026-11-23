//! Frame 위치와 transform 분리 (IX.11.9, ADR-0024 결정 8, ADR-0086 결정 3, ADR-0134 결정 4).
//!
//! - 부모가 있는 Free 자식 Frame: transform 이동 성분(tx, ty)을 layout record `left`·`top`(px)으로 옮기고
//!   `position: absolute`를 명시합니다. 옛 schema에는 layout 정의가 없어 모든 자식이 Free 자식이기 때문입니다. 선형
//!   부분(a, b, c, d)은 `matrix(a, b, c, d, 0, 0)`으로 bit 그대로 둡니다. 회전·확대로 분해하지 않습니다.
//! - 흐름 부모(활성 기본값 세트로 푼 `display`가 `flex`·`grid`인 컨테이너, `commands::arrange::align::flow_parent`)의
//!   자식: 위치 원본은 layout 결과이고, 저장 transform 이동 성분은 scene 합성에서 CSS `translate`처럼 layout 위치 뒤에
//!   더해집니다(ADR-0125 결정 3). `translate` longhand가 아직 없어 이동 성분을 transform에 그대로 두고(같은 뜻)
//!   [`FramePositionReport::flow_children`]에 남깁니다. `position: absolute`로 옮기면 흐름에서 빠져 형제 배치가 바뀝니다.
//! - Frame transform은 border box 좌상단 원점 기준이므로 `transform-origin: 0 0`과 같은 뜻입니다. `transform-origin`
//!   longhand key가 아직 없어 따로 저장하지 않고, scene 합성 origin도 `0 0`입니다(ADR-0134 결정 4).
//! - 부모가 없는 Frame(View root `ViewDefinition.root_frame`)의 이동 성분은 페이지 캔버스 배치입니다. 캔버스 배치 속성
//!   longhand가 아직 없어 그대로 두고 보고서에 남깁니다. ComponentDefinition root도 같은 규칙입니다
//!   ([`component_root_placement`]).
//! - 이미 `left`·`top`·`position`을 명시한 Frame은 건드리지 않고 보고서에 남깁니다(이미 옮긴 문서).
//! - 0x0102(`transform` 묶음)를 가리키는 저장 참조(View Variant property patch, ValueBinding target, Component variant
//!   override)는 값 변환 없이 보존하고 [`transform_bundle_references`]로 보고합니다(`TypedValue`·`ExpressionType`에
//!   Transform 타입이 없음). 문서 레코드 안의 View Variant patch 경로는 [`stored_reference_paths`]가 모읍니다. ValueBinding
//!   target·Component variant override는 문서 레코드 밖(binding·컴포넌트 원본)에 있어 호출자가 그 경로를 같은 함수에
//!   넘깁니다.

use stellar_commands::arrange::align::{flow_parent, model_style_set, FlowParent};
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::value::Transform2D;
use stellar_document::component::definition::DefinitionTree;
use stellar_document::frame::keys as frame_keys;
use stellar_document::layout_record::{LayoutLonghand, LayoutRecord, LayoutValue};
use stellar_document::model::DocumentModel;
use stellar_document::record_codec::decode_payload;
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
    /// 흐름 부모의 자식이라 이동 성분을 `translate`와 같은 뜻으로 transform에 남긴 Frame(이동 성분이 0이 아닌 것).
    pub flow_children: Vec<StableId>,
}

/// 0x0102(`transform` 묶음 key)를 가리키는 저장 참조(값 변환 없이 보존).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformBundleReference {
    /// 참조 경로(그대로).
    pub path: PropertyPath,
}

/// ComponentDefinition root의 캔버스 배치(이동 성분). 인스턴스 배치는 이 값을 쓰지 않습니다(`placed_free_frame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasPlacement {
    /// root Frame.
    pub node: StableId,
    /// 캔버스 x(px).
    pub x: f32,
    /// 캔버스 y(px).
    pub y: f32,
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
    // 흐름 부모 판정은 편집 경로와 같은 함수(`flow_parent`)로, 변환 전 레코드에서 합니다.
    let (model, _) =
        DocumentModel::from_records(records.clone()).map_err(|e| FramePositionError {
            node: StableId::NIL,
            reason: format!("문서를 읽지 못했습니다: {e:?}"),
        })?;
    let set = model_style_set(&model);
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
        if matches!(
            flow_parent(&model, &set, node.id),
            FlowParent::Flex { .. } | FlowParent::Grid
        ) {
            if (tx, ty) != (0.0, 0.0) {
                report.flow_children.push(node.id);
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

/// 저장 참조 경로 중 0x0102(`transform` 묶음 key)를 가리키는 것(입력 순서). 경로는 바꾸지 않습니다.
pub fn transform_bundle_references<'a>(
    paths: impl IntoIterator<Item = &'a PropertyPath>,
) -> Vec<TransformBundleReference> {
    paths
        .into_iter()
        .filter(|path| path.keys().contains(&frame_keys::TRANSFORM))
        .map(|path| TransformBundleReference { path: path.clone() })
        .collect()
}

/// 문서 레코드가 담은 저장 참조 경로(View Variant property patch, View 순서·Variant 순서).
pub fn stored_reference_paths(records: &DocumentRecords) -> Vec<PropertyPath> {
    records
        .view_variants
        .iter()
        .flat_map(|model| model.variants())
        .flat_map(|variant| {
            variant
                .overrides
                .property_patches()
                .map(|(path, _)| path.clone())
        })
        .collect()
}

/// ComponentDefinition root Frame의 캔버스 배치. 이동 성분이 0이거나 root가 Frame이 아니면 `None`입니다.
pub fn component_root_placement(tree: &DefinitionTree) -> Option<CanvasPlacement> {
    let root = tree.node(tree.tree_root()?)?;
    let NodePayload::Frame(frame) = decode_payload(root.kind, &root.payload).ok()? else {
        return None;
    };
    let (_, _, _, _, x, y) = frame.transform.components();
    ((x, y) != (0.0, 0.0)).then_some(CanvasPlacement {
        node: root.id,
        x,
        y,
    })
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

    #[test]
    fn flow_parent_children_keep_translation_as_translate_and_are_reported() {
        // 흐름 부모(display: flex) 자식의 이동 성분은 scene에서 translate처럼 layout 위치 뒤에 더해집니다. absolute로
        // 옮기면 흐름에서 빠지므로 transform에 그대로 둡니다(ADR-0134 결정 4).
        let mut before = records();
        let mut flex = LayoutRecord::new(sid(2));
        flex.set(LayoutLonghand::Display, LayoutValue::Keyword("flex"))
            .unwrap();
        before.layout_records.push(flex);
        before.nodes.push(frame(
            4,
            Some(2),
            Transform2D::new(1.0, 0.0, 0.0, 1.0, 7.0, 0.0).unwrap(),
        ));
        before.sort_canonical();
        let (after, report) = split_frame_position(before.clone()).unwrap();
        validate_document(&after).unwrap();
        assert_eq!(report.moved, [sid(2)]);
        assert_eq!(report.flow_children, [sid(4)]);
        let find = |records: &DocumentRecords, id| {
            records.nodes.iter().find(|n| n.id == id).unwrap().clone()
        };
        assert_eq!(find(&after, sid(4)), find(&before, sid(4)));
        assert_eq!(find(&after, sid(3)), find(&before, sid(3)));
        assert!(!after
            .layout_records
            .iter()
            .any(|r| r.node == sid(4) || r.node == sid(3)));
    }

    #[test]
    fn transform_bundle_references_are_preserved_and_listed() {
        use stellar_core_types::typed_value::TypedValue;
        use stellar_document::component::variant_override::VariantOverrideSet;
        use stellar_document::view_variant::commands::{ViewVariantCommand, ViewVariantModel};
        use stellar_document::view_variant::condition::VariantCondition;
        use stellar_document::view_variant::resolver::{ViewVariant, ViewVariantId};
        let mut before = records();
        let mut overrides = VariantOverrideSet::new();
        let transform = PropertyPath::with_keys(sid(2), vec![frame_keys::TRANSFORM]);
        let visible = PropertyPath::with_keys(
            sid(2),
            vec![stellar_document::payload_metadata::node_keys::VISIBLE],
        );
        overrides.set_property(transform.clone(), TypedValue::Boolean(true));
        overrides.set_property(visible, TypedValue::Boolean(false));
        let mut model = ViewVariantModel::new(sid(102));
        ViewVariantCommand::CreateVariant(ViewVariant::new(
            ViewVariantId::new(sid(200)),
            VariantCondition::new(),
            overrides,
        ))
        .apply(&mut model)
        .unwrap();
        before.view_variants.push(model);
        let (after, _) = split_frame_position(before.clone()).unwrap();
        // 참조 bytes는 그대로입니다(값 변환 없음).
        assert_eq!(
            after
                .view_variants
                .iter()
                .map(|m| m.encode())
                .collect::<Vec<_>>(),
            before
                .view_variants
                .iter()
                .map(|m| m.encode())
                .collect::<Vec<_>>()
        );
        let paths = stored_reference_paths(&after);
        assert_eq!(paths.len(), 2);
        assert_eq!(
            transform_bundle_references(&paths),
            [TransformBundleReference { path: transform }]
        );
    }

    #[test]
    fn component_definition_root_translation_is_a_canvas_placement() {
        use stellar_core_types::reference::ComponentRef;
        use stellar_document::component::definition::DefinitionNode;
        use stellar_document::node_kind::NodeKind;
        use stellar_document::record_codec::encode_payload;
        let mut tree = DefinitionTree::new(ComponentRef::new(sid(500)));
        let payload = NodePayload::Frame(FramePayload {
            transform: Transform2D::new(1.0, 0.0, 0.0, 1.0, 120.0, -8.0).unwrap(),
            ..FramePayload::default()
        });
        tree.insert(DefinitionNode {
            id: sid(501),
            kind: NodeKind::Frame,
            parent: None,
            order_key: OrderKey::from_bytes(vec![1]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload: encode_payload(&payload),
        })
        .unwrap();
        assert_eq!(
            component_root_placement(&tree),
            Some(CanvasPlacement {
                node: sid(501),
                x: 120.0,
                y: -8.0
            })
        );
    }

    /// `stellar_layout::transform::css_node_transform`(origin 0)과 같은 합성. project는 layout에 의존하지 않아
    /// 같은 식(translate ∘ matrix)을 test에서만 씁니다.
    fn stellar_layout_compose(position: (f32, f32), linear: Transform2D) -> Transform2D {
        linear
            .then(&Transform2D::new(1.0, 0.0, 0.0, 1.0, position.0, position.1).unwrap())
            .unwrap()
    }
}
