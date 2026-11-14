//! 실제 저장·로드와 CLI에서 공유하는 두 View fixture. 원본 문서 모델로만 생성합니다.

use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::FramePayload;
use stellar_document::image::{CropRect, FitMode, ImagePayload, SamplingMode};
use stellar_document::model::DocumentModel;
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::text::{Paragraph, TextPayload};

use crate::core_types::{
    ids::StableId, order_key::OrderKey, reference::AssetRef, text::UserText, value::Size2,
};

/// 결정론 fixture id(문서 1, 페이지 2, View 3·4, root 10·20).
pub fn id(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

/// View A: 중첩 Fragment·한글/줄바꿈/HTML 문자·숨김 Text. View B: 별도 문자열.
/// node 삽입 순서·StableId 순서와 OrderKey 순서를 의도적으로 다르게 둡니다.
pub fn document() -> DocumentModel {
    let mut definition = DocumentDefinition::new(id(1), "접근성 fixture".into());
    let mut page = PageDefinition::new(id(2), "페이지".into());
    page.add_view(ViewDefinition::new(id(3), "A".into(), id(10)))
        .unwrap();
    page.add_view(ViewDefinition::new(id(4), "B".into(), id(20)))
        .unwrap();
    definition.add_page(page).unwrap();
    let mut model = DocumentModel::new(definition);
    let text = |value: &str| {
        NodePayload::Text(
            TextPayload::new(
                UserText::new(value),
                vec![Paragraph { style: 0 }; value.matches('\n').count() + 1],
                vec![],
            )
            .unwrap(),
        )
    };
    for (n, parent, order, visible, payload) in [
        (
            10,
            None,
            1,
            true,
            NodePayload::Frame(FramePayload::default()),
        ),
        (13, Some(10), 2, true, text("두 번째")),
        (
            11,
            Some(10),
            1,
            true,
            NodePayload::Fragment(FragmentPayload),
        ),
        (
            14,
            Some(11),
            1,
            true,
            NodePayload::Fragment(FragmentPayload),
        ),
        (
            12,
            Some(14),
            1,
            true,
            text("첫 번째 <본문> & \"인용\"\n다음 줄"),
        ),
        (
            15,
            Some(10),
            3,
            false,
            NodePayload::Fragment(FragmentPayload),
        ),
        (16, Some(15), 1, true, text("숨긴 내용")),
        (
            20,
            None,
            2,
            true,
            NodePayload::Frame(FramePayload::default()),
        ),
        (21, Some(20), 1, true, text("두 번째 View 전용")),
    ] {
        model
            .insert_node(NodeRecord {
                id: id(n),
                parent: parent.map(id),
                order_key: OrderKey::from_bytes(vec![order]).unwrap(),
                visible,
                locked: false,
                display_name: "button 이름은 의미가 아님".into(),
                payload,
            })
            .unwrap();
    }
    model
}

/// View B만 미지원 Image를 가진 문서. View A 내보내기는 여전히 성공해야 합니다.
pub fn unsupported_document() -> DocumentModel {
    let mut model = document();
    let mut record = model.remove_leaf(id(21)).unwrap();
    record.payload = NodePayload::Image(ImagePayload {
        asset: AssetRef::new(id(50)),
        size: Size2::ZERO,
        fit: FitMode::Contain,
        crop: CropRect::FULL,
        sampling: SamplingMode::Linear,
        color_space_override: None,
    });
    model.insert_node(record).unwrap();
    model
}
