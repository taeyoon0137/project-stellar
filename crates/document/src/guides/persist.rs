//! 가이드·레이아웃 그리드와 View 변수·Variant의 영속 연결 (XII.10.19·XII.10.20·XVI.11.1, ADR-0126).
//!
//! - **문서 레코드**: [`DocumentRecords::node_guides`](crate::records::DocumentRecords::node_guides)(Frame마다
//!   [`NodeGuides`] 하나, 빈 기록은 두지 않음)와 [`DocumentRecords::view_variants`](crate::records::DocumentRecords::view_variants)
//!   (View마다 [`ViewVariantModel`] 하나, 빈 model은 두지 않음).
//! - **subtree chunk 확장 절**: subtree payload 끝(layout 절 자리 뒤)에 tag 오름차순으로 둡니다. 절 하나는 tag `u8` +
//!   record 수 `u32`(1 이상) + (`u32` 길이 + record bytes)*이고, tag는 core-types `reserved::guides`의 번호입니다.
//!   가이드 절은 subtree 안 Frame의 기록(소유 node StableId 순), View 절은 View root subtree에만 그 View의
//!   model(View StableId 순)입니다. 확장 절이 있는데 layout record가 없으면 layout 절 자리에 수 `0`을 씁니다. 확장
//!   절이 없는 subtree의 bytes는 이전 형식과 같습니다.
//! - **validator**: [`check_records`]가 소유자 존재·종류(Frame), 중복, 빈 기록을 봅니다. View model의 override가
//!   가리키는 scene node는 검사하지 않습니다(orphan은 resolver가 적용하지 않고 진단합니다, IX.7.8).

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reserved::guides::{
    SUBTREE_SECTION_NODE_GUIDES, SUBTREE_SECTION_VIEW_VARIANTS,
};

use super::NodeGuides;
use crate::node_kind::NodeKind;
use crate::view_variables::is_empty_model;
use crate::view_variant::commands::ViewVariantModel;

/// 가이드·View model record 제약 위반.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuidesViolation {
    /// 가이드 기록의 소유 node가 없습니다.
    OwnerMissing {
        /// 소유 node.
        owner: StableId,
    },
    /// 가이드 기록의 소유 node가 Frame이 아닙니다.
    OwnerNotFrame {
        /// 소유 node.
        owner: StableId,
        /// 실제 종류.
        kind: NodeKind,
    },
    /// 한 Frame에 가이드 기록이 둘 이상입니다.
    DuplicateOwner {
        /// 소유 node.
        owner: StableId,
    },
    /// 가이드도 그리드도 없는 기록입니다(기록 없음으로 저장해야 함).
    EmptyGuides {
        /// 소유 node.
        owner: StableId,
    },
    /// View model의 View가 문서 정의에 없습니다.
    ViewMissing {
        /// View.
        view: StableId,
    },
    /// 한 View에 model이 둘 이상입니다.
    DuplicateView {
        /// View.
        view: StableId,
    },
    /// 변수도 Variant도 없는 View model입니다.
    EmptyView {
        /// View.
        view: StableId,
    },
}

impl GuidesViolation {
    /// 위반 대상(가이드 기록은 소유 node, View model은 View).
    pub fn subject(&self) -> StableId {
        match self {
            GuidesViolation::OwnerMissing { owner }
            | GuidesViolation::OwnerNotFrame { owner, .. }
            | GuidesViolation::DuplicateOwner { owner }
            | GuidesViolation::EmptyGuides { owner } => *owner,
            GuidesViolation::ViewMissing { view }
            | GuidesViolation::DuplicateView { view }
            | GuidesViolation::EmptyView { view } => *view,
        }
    }
}

impl fmt::Display for GuidesViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuidesViolation::OwnerMissing { owner } => {
                write!(f, "가이드 기록의 소유 node {owner}가 없습니다")
            }
            GuidesViolation::OwnerNotFrame { owner, kind } => write!(
                f,
                "가이드 기록의 소유 node {owner}가 Frame이 아닙니다({})",
                kind.name()
            ),
            GuidesViolation::DuplicateOwner { owner } => {
                write!(f, "node {owner}의 가이드 기록이 둘 이상입니다")
            }
            GuidesViolation::EmptyGuides { owner } => {
                write!(f, "node {owner}의 가이드 기록이 비어 있습니다")
            }
            GuidesViolation::ViewMissing { view } => {
                write!(f, "View 변수 model의 View {view}가 없습니다")
            }
            GuidesViolation::DuplicateView { view } => {
                write!(f, "View {view}의 변수 model이 둘 이상입니다")
            }
            GuidesViolation::EmptyView { view } => {
                write!(f, "View {view}의 변수 model이 비어 있습니다")
            }
        }
    }
}

/// 가이드 기록과 View model 목록을 검사합니다. `kind_of`는 node 종류, `views`는 문서 정의의 View id입니다.
pub fn check_records(
    guides: &[NodeGuides],
    variants: &[ViewVariantModel],
    kind_of: impl Fn(StableId) -> Option<NodeKind>,
    views: &BTreeSet<StableId>,
) -> Vec<GuidesViolation> {
    let mut violations = Vec::new();
    let mut owners = BTreeSet::new();
    for record in guides {
        let owner = record.owner();
        if !owners.insert(owner) {
            violations.push(GuidesViolation::DuplicateOwner { owner });
            continue;
        }
        if record.is_empty() {
            violations.push(GuidesViolation::EmptyGuides { owner });
        }
        match kind_of(owner) {
            None => violations.push(GuidesViolation::OwnerMissing { owner }),
            Some(NodeKind::Frame) => {}
            Some(kind) => violations.push(GuidesViolation::OwnerNotFrame { owner, kind }),
        }
    }
    let mut seen = BTreeSet::new();
    for model in variants {
        let view = model.variables().view();
        if !seen.insert(view) {
            violations.push(GuidesViolation::DuplicateView { view });
            continue;
        }
        if is_empty_model(model) {
            violations.push(GuidesViolation::EmptyView { view });
        }
        if !views.contains(&view) {
            violations.push(GuidesViolation::ViewMissing { view });
        }
    }
    violations
}

/// subtree 확장 절 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionError {
    /// 절 형식이 맞지 않습니다(모르는 tag, 순서, 빈 절, 남는 bytes 포함).
    Malformed(&'static str),
    /// 가이드 record bytes 오류.
    Guides(super::GuideError),
    /// View model bytes 오류.
    ViewVariants(String),
    /// 가이드 기록의 소유 node가 그 subtree에 없습니다.
    ForeignGuides {
        /// 소유 node.
        owner: StableId,
    },
}

impl fmt::Display for SectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SectionError::Malformed(reason) => write!(f, "subtree 확장 절 형식 위반: {reason}"),
            SectionError::Guides(e) => write!(f, "가이드 record: {e}"),
            SectionError::ViewVariants(e) => write!(f, "View 변수 model: {e}"),
            SectionError::ForeignGuides { owner } => {
                write!(f, "subtree 밖 node {owner}의 가이드 기록입니다")
            }
        }
    }
}

impl std::error::Error for SectionError {}

/// subtree 하나의 확장 절 내용.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubtreeSections {
    /// subtree 안 Frame의 가이드 기록(소유 node StableId 순).
    pub node_guides: Vec<NodeGuides>,
    /// View root subtree의 View model(View StableId 순).
    pub view_variants: Vec<ViewVariantModel>,
}

impl SubtreeSections {
    /// 절이 하나도 없으면 참.
    pub fn is_empty(&self) -> bool {
        self.node_guides.is_empty() && self.view_variants.is_empty()
    }
}

fn put_u32(buf: &mut Vec<u8>, value: usize) {
    buf.extend_from_slice(&(value as u32).to_le_bytes());
}

fn put_section(buf: &mut Vec<u8>, tag: u8, records: impl ExactSizeIterator<Item = Vec<u8>>) {
    if records.len() == 0 {
        return;
    }
    buf.push(tag);
    put_u32(buf, records.len());
    for bytes in records {
        put_u32(buf, bytes.len());
        buf.extend_from_slice(&bytes);
    }
}

/// 확장 절 bytes(절이 없으면 빈 bytes). 호출자는 목록을 canonical 순서로 넘깁니다.
pub fn encode_sections(guides: &[NodeGuides], variants: &[ViewVariantModel]) -> Vec<u8> {
    let mut buf = Vec::new();
    put_section(
        &mut buf,
        SUBTREE_SECTION_NODE_GUIDES,
        guides.iter().map(NodeGuides::encode),
    );
    put_section(
        &mut buf,
        SUBTREE_SECTION_VIEW_VARIANTS,
        variants.iter().map(ViewVariantModel::encode),
    );
    buf
}

struct Input<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Input<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], SectionError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(SectionError::Malformed("입력 부족"))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<usize, SectionError> {
        let raw = self.take(4)?;
        Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize)
    }
}

/// [`encode_sections`]의 역. `members`는 subtree node입니다. View model의 View가 문서에 있는지는 문서 validator가
/// 확인합니다(subtree 하나만으로는 정의를 모름).
pub fn decode_sections(
    bytes: &[u8],
    members: &BTreeSet<StableId>,
) -> Result<SubtreeSections, SectionError> {
    let mut input = Input { bytes, at: 0 };
    let mut out = SubtreeSections::default();
    let mut last_tag: Option<u8> = None;
    while input.at < bytes.len() {
        let tag = input.take(1)?[0];
        if last_tag.is_some_and(|last| last >= tag) {
            return Err(SectionError::Malformed(
                "확장 절은 tag 오름차순으로 한 번씩 씁니다",
            ));
        }
        last_tag = Some(tag);
        let count = input.u32()?;
        if count == 0 {
            return Err(SectionError::Malformed("빈 확장 절은 쓰지 않습니다"));
        }
        for _ in 0..count {
            let len = input.u32()?;
            let record = input.take(len)?;
            match tag {
                SUBTREE_SECTION_NODE_GUIDES => {
                    let guides = NodeGuides::decode(record).map_err(SectionError::Guides)?;
                    if !members.contains(&guides.owner()) {
                        return Err(SectionError::ForeignGuides {
                            owner: guides.owner(),
                        });
                    }
                    if out
                        .node_guides
                        .last()
                        .is_some_and(|last| last.owner() >= guides.owner())
                    {
                        return Err(SectionError::Malformed(
                            "가이드 기록은 소유 node 순서로 한 번씩 씁니다",
                        ));
                    }
                    out.node_guides.push(guides);
                }
                SUBTREE_SECTION_VIEW_VARIANTS => {
                    let model = ViewVariantModel::decode(record)
                        .map_err(|e| SectionError::ViewVariants(e.to_string()))?;
                    if model.encode() != record {
                        return Err(SectionError::Malformed("정규형이 아닌 View 변수 model"));
                    }
                    let view = model.variables().view();
                    if out
                        .view_variants
                        .last()
                        .is_some_and(|last| last.variables().view() >= view)
                    {
                        return Err(SectionError::Malformed(
                            "View 변수 model은 View 순서로 한 번씩 씁니다",
                        ));
                    }
                    out.view_variants.push(model);
                }
                _ => return Err(SectionError::Malformed("알 수 없는 확장 절 tag")),
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guides::{GuideAxis, GuideEdit, LayoutGrid, RulerGuide};
    use crate::view_variant::commands::ViewVariantCommand;
    use crate::view_variant::variable::fixtures::device_variable;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0x0a, n)
    }

    fn guides(owner: u64) -> NodeGuides {
        let mut record = NodeGuides::new(sid(owner));
        record
            .apply(&GuideEdit::Add(
                RulerGuide::new(sid(owner + 100), GuideAxis::X, 120.0).unwrap(),
            ))
            .unwrap();
        record
            .apply(&GuideEdit::SetGrids(vec![LayoutGrid::columns(
                12, 20.0, 32.0,
            )
            .unwrap()]))
            .unwrap();
        record
    }

    fn view_model(view: u64) -> ViewVariantModel {
        let mut model = ViewVariantModel::new(sid(view));
        ViewVariantCommand::CreateVariable(device_variable())
            .apply(&mut model)
            .unwrap();
        model
    }

    #[test]
    fn sections_round_trip_and_reject_foreign_or_unordered_records() {
        let members: BTreeSet<StableId> = [sid(1), sid(2)].into();
        let sections = SubtreeSections {
            node_guides: vec![guides(1), guides(2)],
            view_variants: vec![view_model(50)],
        };
        let bytes = encode_sections(&sections.node_guides, &sections.view_variants);
        assert_eq!(decode_sections(&bytes, &members).unwrap(), sections);
        assert!(encode_sections(&[], &[]).is_empty());

        // subtree 밖 owner.
        assert!(matches!(
            decode_sections(&bytes, &[sid(1)].into()),
            Err(SectionError::ForeignGuides { .. })
        ));
        // owner 역순, 남는 bytes, 모르는 tag, 빈 절.
        let reversed = encode_sections(&[guides(2), guides(1)], &[]);
        assert!(decode_sections(&reversed, &members).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_sections(&trailing, &members).is_err());
        assert!(decode_sections(&[0x7f, 1, 0, 0, 0, 0, 0, 0, 0], &members).is_err());
        assert!(decode_sections(&[SUBTREE_SECTION_NODE_GUIDES, 0, 0, 0, 0], &members).is_err());
    }

    #[test]
    fn check_records_reports_missing_owner_kind_duplicate_and_empty() {
        let kind_of = |id: StableId| match id {
            id if id == sid(1) => Some(NodeKind::Frame),
            id if id == sid(2) => Some(NodeKind::Text),
            _ => None,
        };
        let views: BTreeSet<StableId> = [sid(50)].into();
        let violations = check_records(
            &[
                guides(1),
                guides(1),
                guides(2),
                guides(3),
                NodeGuides::new(sid(1)),
            ],
            &[
                view_model(50),
                view_model(51),
                ViewVariantModel::new(sid(50)),
            ],
            kind_of,
            &views,
        );
        assert_eq!(
            violations,
            vec![
                GuidesViolation::DuplicateOwner { owner: sid(1) },
                GuidesViolation::OwnerNotFrame {
                    owner: sid(2),
                    kind: NodeKind::Text
                },
                GuidesViolation::OwnerMissing { owner: sid(3) },
                GuidesViolation::DuplicateOwner { owner: sid(1) },
                GuidesViolation::ViewMissing { view: sid(51) },
                GuidesViolation::DuplicateView { view: sid(50) },
            ]
        );
        assert!(check_records(&[guides(1)], &[view_model(50)], kind_of, &views).is_empty());
    }

    fn document() -> crate::model::DocumentModel {
        use crate::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
        use crate::frame::FramePayload;
        use crate::records::{NodePayload, NodeRecord};
        use stellar_core_types::order_key::OrderKey;
        let mut definition = DocumentDefinition::new(sid(90), "doc".into());
        let mut page = PageDefinition::new(sid(91), "page".into());
        page.add_view(ViewDefinition::new(sid(50), "view".into(), sid(1)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut model = crate::model::DocumentModel::new(definition);
        let frame = |id: u64, parent: Option<u64>| NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: format!("f{id}"),
            payload: NodePayload::Frame(FramePayload::default()),
        };
        model.insert_node(frame(1, None)).unwrap();
        model.insert_node(frame(2, Some(1))).unwrap();
        model
    }

    #[test]
    fn model_records_and_subtree_chunks_carry_guides_and_view_variants() {
        // 검증 기준(XII.10.19·XII.10.20·XVI.11.1): 문서 → 레코드 → subtree chunk bytes → 레코드 → 문서 왕복이 같습니다.
        let mut model = document();
        assert_eq!(model.set_node_guides(guides(2)).unwrap(), None);
        assert_eq!(model.set_node_guides(guides(1)).unwrap(), None);
        assert_eq!(model.set_view_variants(view_model(50)).unwrap(), None);
        // Frame이 아닌 소유자·없는 View는 거부합니다.
        assert!(model.set_node_guides(guides(7)).is_err());
        assert!(model.set_view_variants(view_model(51)).is_err());

        let records = model.to_records().unwrap();
        assert_eq!(records.node_guides, vec![guides(1), guides(2)]);
        assert_eq!(records.view_variants, vec![view_model(50)]);
        crate::validate::validate_document(&records).unwrap();
        // 확장 절을 모르는 분할은 버리지 않고 거부합니다.
        assert!(records.view_subtrees().is_err());
        let parts = records.view_subtrees_with_sections().unwrap();
        let decoded: Vec<_> = parts
            .iter()
            .map(|(_, subtree, sections)| {
                let bytes = subtree.encode_with(sections);
                assert!(crate::record_codec::SubtreeRecords::decode(&bytes).is_err());
                crate::record_codec::SubtreeRecords::decode_with_sections(&bytes).unwrap()
            })
            .collect();
        let back = crate::records::DocumentRecords::from_subtrees_with_sections(
            records.definition.clone(),
            decoded,
        );
        assert_eq!(back, records);
        let (reopened, _) = crate::model::DocumentModel::from_records(back).unwrap();
        assert_eq!(reopened.node_guides(sid(1)), Some(&guides(1)));
        assert_eq!(reopened.view_variants(sid(50)), Some(&view_model(50)));

        // 빈 기록은 기록을 지우고, 지운 node의 기록은 영속 레코드에 나오지 않습니다.
        assert_eq!(
            model.set_node_guides(NodeGuides::new(sid(2))).unwrap(),
            Some(guides(2))
        );
        assert_eq!(model.node_guides(sid(2)), None);
        let records = model.to_records().unwrap();
        assert_eq!(records.node_guides, vec![guides(1)]);
        // 가이드도 layout 절도 없는 subtree는 이전 형식 bytes와 같습니다.
        let mut plain = records.clone();
        plain.node_guides.clear();
        plain.view_variants.clear();
        let (_, subtree) = plain.view_subtrees().unwrap().remove(0);
        assert_eq!(
            subtree.encode_with(&SubtreeSections::default()),
            subtree.encode()
        );
    }

    #[test]
    fn validator_rejects_guides_on_non_frame_and_variants_of_missing_views() {
        let mut records = document().to_records().unwrap();
        records.node_guides = vec![guides(3)];
        records.view_variants = vec![view_model(51)];
        let error = crate::validate::validate_document(&records).unwrap_err();
        let found: Vec<String> = error.violations.iter().map(|v| v.to_string()).collect();
        assert_eq!(found.len(), 2, "{found:?}");
    }
}
