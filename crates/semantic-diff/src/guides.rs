//! Frame 가이드·레이아웃 그리드와 View 변수·Variant의 semantic snapshot 투영 (ADR-0117 결정 2.4·ADR-0126).
//!
//! | 기록 | 객체·속성 |
//! |---|---|
//! | 가이드 하나 | 소유 Frame `Node` 객체의 `[0xED01] + 가이드 id` 속성. 값은 텍스트 원본 한 줄(`RulerGuide::to_line`) |
//! | 레이아웃 그리드 목록 | 소유 Frame `Node` 객체의 `[0xED02]` 속성 하나. 값은 그리드 줄을 `\n`으로 이은 텍스트(순서 있는 값) |
//! | View 변수 하나 | `View` 객체의 `[0xED10] + 변수 id` 속성. 값은 변수 canonical bytes |
//! | Variant 하나 | `Variant` 객체(parent = View). override는 [`SnapshotBuilder::add_variant`](crate::build::SnapshotBuilder::add_variant)와 같은 patch별 속성, 조건은 `[0xED11]` 속성(조건 canonical bytes) |
//!
//! 가이드는 가이드 id별 속성이라 서로 다른 가이드의 동시 편집은 다른 경로이고(충돌 없음), 같은 가이드의 양쪽 이동이나
//! 한쪽 이동·한쪽 삭제는 같은 경로의 충돌입니다. 그리드 목록은 값 하나라 양쪽이 바꾸면 값 충돌입니다(ADR-0117 결정 2.4).
//! 기록이 없는 Frame·View는 속성이 없으므로 가이드가 없는 문서의 snapshot은 그대로입니다. [`restore`]가 같은 속성에서
//! 기록을 다시 만듭니다(merge 결과 반영).

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PathSegment, PropertyKey, PropertyPath};
use stellar_core_types::reserved::guides::{
    SNAPSHOT_NODE_GUIDE, SNAPSHOT_NODE_LAYOUT_GRIDS, SNAPSHOT_VARIANT_CONDITION,
    SNAPSHOT_VIEW_VARIABLE,
};
use stellar_document::component::variant_override::VariantOverrideSet;
use stellar_document::guides::{GuideLine, NodeGuides};
use stellar_document::records::DocumentRecords;
use stellar_document::view_variables::{model_from_parts, variable_bytes, variable_from_bytes};
use stellar_document::view_variant::condition::VariantCondition;
use stellar_document::view_variant::resolver::{ViewVariant, ViewVariantId};

use crate::build::{variant_object, VariantEntry};
use crate::keys;
use crate::records::{ProjectionError, ProjectionFailure};
use crate::snapshot::{ObjectKind, Origin, SemanticObject, SemanticSnapshot, SnapshotError};
use crate::value::SemanticValue;

/// 가이드 하나(경로 tail = 가이드 id).
pub const NODE_GUIDE: PropertyKey = PropertyKey(SNAPSHOT_NODE_GUIDE);
/// 레이아웃 그리드 목록.
pub const NODE_LAYOUT_GRIDS: PropertyKey = PropertyKey(SNAPSHOT_NODE_LAYOUT_GRIDS);
/// View 변수 하나(경로 tail = 변수 id).
pub const VIEW_VARIABLE: PropertyKey = PropertyKey(SNAPSHOT_VIEW_VARIABLE);
/// Variant 조건.
pub const VARIANT_CONDITION: PropertyKey = PropertyKey(SNAPSHOT_VARIANT_CONDITION);

/// 변경 경로의 Semantic Diff 범주(ADR-0117 결정 2.4): 가이드·레이아웃 그리드는 `guides`, View 변수·Variant 조건은
/// `view-variables`, 그 밖은 `None`.
pub fn category(path: &PropertyPath) -> Option<&'static str> {
    match path.keys().first()? {
        key if *key == NODE_GUIDE || *key == NODE_LAYOUT_GRIDS => Some("guides"),
        key if *key == VIEW_VARIABLE || *key == VARIANT_CONDITION => Some("view-variables"),
        _ => None,
    }
}

/// 문서 레코드의 가이드·View 변수를 `objects`(문서 정의·node 객체)에 투영하고 Variant 객체를 더합니다.
pub(crate) fn project(
    records: &DocumentRecords,
    objects: &mut Vec<SemanticObject>,
) -> Result<(), SnapshotError> {
    if records.node_guides.is_empty() && records.view_variants.is_empty() {
        return Ok(());
    }
    let index: BTreeMap<StableId, usize> = objects
        .iter()
        .enumerate()
        .map(|(at, object)| (object.id(), at))
        .collect();
    for record in &records.node_guides {
        let at = *index
            .get(&record.owner())
            .filter(|at| matches!(objects[**at].kind(), ObjectKind::Node(_)))
            .ok_or(SnapshotError::GuidesOwnerMissing {
                owner: record.owner(),
            })?;
        let object = &mut objects[at];
        for guide in record.guides() {
            let path = object
                .path(&[NODE_GUIDE])
                .with_key_segment(&guide.id.to_string());
            object.insert_property(path, SemanticValue::Text(guide.to_line()))?;
        }
        if !record.grids().is_empty() {
            let lines: Vec<String> = record.grids().iter().map(|g| g.to_line()).collect();
            object.insert_property(
                object.path(&[NODE_LAYOUT_GRIDS]),
                SemanticValue::Text(lines.join("\n")),
            )?;
        }
    }
    let document = records.definition.id;
    for model in &records.view_variants {
        let view = model.variables().view();
        let at = *index
            .get(&view)
            .filter(|at| objects[**at].kind() == ObjectKind::View)
            .ok_or(SnapshotError::GuidesOwnerMissing { owner: view })?;
        let object = &mut objects[at];
        for variable in model.variables().iter() {
            let path = object
                .path(&[VIEW_VARIABLE])
                .with_key_segment(&variable.id().stable_id().to_string());
            object.insert_property(path, SemanticValue::Bytes(variable_bytes(variable)))?;
        }
        for variant in model.variants() {
            let mut object = variant_object(&VariantEntry {
                id: variant.id.stable_id(),
                document,
                owner: view,
                overrides: variant.overrides.clone(),
            })?;
            object.insert_property(
                object.path(&[VARIANT_CONDITION]),
                SemanticValue::Bytes(variant.condition.encode()),
            )?;
            objects.push(object);
        }
    }
    Ok(())
}

fn fail(object: StableId, key: PropertyKey, detail: impl ToString) -> ProjectionError {
    ProjectionError {
        object,
        reason: ProjectionFailure::InvalidValue {
            key,
            detail: detail.to_string(),
        },
    }
}

fn tail_id(
    object: StableId,
    key: PropertyKey,
    path: &PropertyPath,
) -> Result<StableId, ProjectionError> {
    match path.tail() {
        Some(PathSegment::Key(text)) => text
            .parse()
            .map_err(|_| fail(object, key, "경로 tail이 StableId가 아닙니다")),
        _ => Err(fail(object, key, "경로 tail이 없습니다")),
    }
}

/// node 객체의 가이드 속성에서 기록을 만듭니다. 값은 정규 줄이어야 합니다.
pub(crate) fn node_guides(object: &SemanticObject) -> Result<Option<NodeGuides>, ProjectionError> {
    let id = object.id();
    let mut lines: Vec<String> = Vec::new();
    let mut grids: Option<&str> = None;
    for (path, value) in object.properties() {
        let key = match path.keys() {
            [key] if *key == NODE_GUIDE || *key == NODE_LAYOUT_GRIDS => *key,
            _ => continue,
        };
        let SemanticValue::Text(text) = value else {
            return Err(fail(id, key, "텍스트 값이 아닙니다"));
        };
        if key == NODE_GUIDE {
            let guide = tail_id(id, key, path)?;
            match GuideLine::parse(text).map_err(|e| fail(id, key, e))? {
                GuideLine::Guide(parsed) if parsed.id == guide && parsed.to_line() == *text => {}
                _ => return Err(fail(id, key, "가이드 줄이 정규형이 아니거나 id가 다릅니다")),
            }
            lines.push(text.clone());
        } else {
            if path.tail().is_some() {
                return Err(fail(id, key, "그리드 목록 경로에 tail이 있습니다"));
            }
            grids = Some(text);
        }
    }
    if lines.is_empty() && grids.is_none() {
        return Ok(None);
    }
    let all = lines
        .iter()
        .map(String::as_str)
        .chain(grids.into_iter().flat_map(|g| g.split('\n')));
    let record = NodeGuides::from_lines(id, all).map_err(|e| fail(id, NODE_LAYOUT_GRIDS, e))?;
    if let Some(grids) = grids {
        let canonical: Vec<String> = record.grids().iter().map(|g| g.to_line()).collect();
        if canonical.is_empty() || canonical.join("\n") != grids {
            return Err(fail(id, NODE_LAYOUT_GRIDS, "그리드 줄이 정규형이 아닙니다"));
        }
    }
    Ok(Some(record))
}

/// `PropertyPath::to_canonical_bytes`의 역.
fn path_from_bytes(bytes: &[u8]) -> Option<PropertyPath> {
    let root: [u8; 16] = bytes.get(..16)?.try_into().ok()?;
    let mut at = 16;
    let u32_at = |at: &mut usize| -> Option<u32> {
        let raw: [u8; 4] = bytes.get(*at..*at + 4)?.try_into().ok()?;
        *at += 4;
        Some(u32::from_be_bytes(raw))
    };
    let count = u32_at(&mut at)? as usize;
    let mut keys_ = Vec::with_capacity(count.min(64));
    for _ in 0..count {
        keys_.push(PropertyKey(u32_at(&mut at)?));
    }
    let mut path = PropertyPath::with_keys(StableId::from_bytes(root), keys_);
    match *bytes.get(at)? {
        0 => at += 1,
        1 => {
            at += 1;
            path = path.with_index(u32_at(&mut at)?);
        }
        2 => {
            at += 1;
            let len = u32_at(&mut at)? as usize;
            let text = std::str::from_utf8(bytes.get(at..at.checked_add(len)?)?).ok()?;
            path = path.with_key_segment(text);
            at += len;
        }
        _ => return None,
    }
    (at == bytes.len() && path.to_canonical_bytes() == bytes).then_some(path)
}

fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Variant 객체에서 Variant를 만듭니다([`variant_object`]와 조건 속성의 역).
fn variant(object: &SemanticObject) -> Result<ViewVariant, ProjectionError> {
    let id = object.id();
    let mut overrides = VariantOverrideSet::new();
    let mut condition = None;
    for (path, value) in object.properties() {
        let [key] = path.keys() else {
            return Err(fail(id, PropertyKey(0), "Variant 속성 경로"));
        };
        let key = *key;
        let segment = match path.tail() {
            Some(PathSegment::Key(text)) => Some(text.as_str()),
            None => None,
            Some(PathSegment::Index(_)) => return Err(fail(id, key, "index tail")),
        };
        match (key, segment, value) {
            (k, None, SemanticValue::Bytes(bytes)) if k == VARIANT_CONDITION => {
                let parsed = VariantCondition::decode(bytes).map_err(|e| fail(id, key, e))?;
                if parsed.encode() != *bytes {
                    return Err(fail(id, key, "정규형이 아닌 조건"));
                }
                condition = Some(parsed);
            }
            (k, Some(segment), SemanticValue::Typed(typed))
                if k == keys::VARIANT_PROPERTY_PATCH =>
            {
                let target = hex_bytes(segment)
                    .and_then(|b| path_from_bytes(&b))
                    .ok_or_else(|| fail(id, key, "patch 경로"))?;
                overrides.set_property(target, typed.clone());
            }
            (k, Some(_), SemanticValue::Bytes(bytes))
                if k == keys::VARIANT_STRUCTURE_PATCH || k == keys::VARIANT_SLOT_CHANGE =>
            {
                let single = VariantOverrideSet::decode(bytes).map_err(|e| fail(id, key, e))?;
                for (node, patch) in single.structure_patches() {
                    overrides.set_structure(node, patch.clone());
                }
                for (instance, slot, change) in single.slot_changes() {
                    overrides.set_slot(instance, slot, change.clone());
                }
            }
            _ => return Err(fail(id, key, "알 수 없는 Variant 속성")),
        }
    }
    let condition = condition.ok_or_else(|| fail(id, VARIANT_CONDITION, "조건이 없습니다"))?;
    Ok(ViewVariant::new(
        ViewVariantId::new(id),
        condition,
        overrides,
    ))
}

/// snapshot에서 문서 `document`의 가이드 기록과 View model을 `records`에 되살립니다. node 레코드와 정의는 이미 있어야
/// 합니다.
pub(crate) fn restore(
    snapshot: &SemanticSnapshot,
    document: StableId,
    records: &mut DocumentRecords,
) -> Result<(), ProjectionError> {
    let origin = Origin::Document(document);
    let mut variables: BTreeMap<StableId, Vec<_>> = BTreeMap::new();
    let mut variants: BTreeMap<StableId, Vec<ViewVariant>> = BTreeMap::new();
    for object in snapshot.objects().filter(|o| o.origin() == origin) {
        match object.kind() {
            ObjectKind::Node(_) => {
                if let Some(record) = node_guides(object)? {
                    records.node_guides.push(record);
                }
            }
            ObjectKind::View => {
                for (path, value) in object.properties() {
                    if path.keys() != [VIEW_VARIABLE] {
                        continue;
                    }
                    let variable_id = tail_id(object.id(), VIEW_VARIABLE, path)?;
                    let SemanticValue::Bytes(bytes) = value else {
                        return Err(fail(object.id(), VIEW_VARIABLE, "bytes 값이 아닙니다"));
                    };
                    let variable = variable_from_bytes(bytes)
                        .map_err(|e| fail(object.id(), VIEW_VARIABLE, e))?;
                    if variable.id().stable_id() != variable_id {
                        return Err(fail(
                            object.id(),
                            VIEW_VARIABLE,
                            "변수 id가 경로와 다릅니다",
                        ));
                    }
                    variables.entry(object.id()).or_default().push(variable);
                }
            }
            ObjectKind::Variant => {
                let view = object.parent().ok_or(ProjectionError {
                    object: object.id(),
                    reason: ProjectionFailure::MissingOwner,
                })?;
                if !object
                    .properties()
                    .keys()
                    .any(|p| p.keys() == [VARIANT_CONDITION])
                {
                    // 조건이 없는 Variant 객체는 문서 레코드 밖 입력(`add_variant`)입니다.
                    continue;
                }
                variants.entry(view).or_default().push(variant(object)?);
            }
            _ => {}
        }
    }
    let views: Vec<StableId> = variables.keys().chain(variants.keys()).copied().collect();
    let mut done = std::collections::BTreeSet::new();
    for view in views {
        if !done.insert(view) {
            continue;
        }
        let model = model_from_parts(
            view,
            variables.remove(&view).unwrap_or_default(),
            variants.remove(&view).unwrap_or_default(),
        )
        .map_err(|e| fail(view, VIEW_VARIABLE, e))?;
        records.view_variants.push(model);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::SnapshotBuilder;
    use crate::records::document_records;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
    use stellar_document::expressions::types::{ExpressionType, TypeSpec};
    use stellar_document::frame::FramePayload;
    use stellar_document::guides::{GuideAxis, GuideEdit, LayoutGrid, RulerGuide};
    use stellar_document::records::{NodePayload, NodeRecord};
    use stellar_document::view_variant::commands::{ViewVariantCommand, ViewVariantModel};
    use stellar_document::view_variant::variable::{VariableValue, ViewVariable, ViewVariableId};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0x0e, n)
    }

    fn records() -> DocumentRecords {
        let mut definition = DocumentDefinition::new(sid(1), "doc".into());
        let mut page = PageDefinition::new(sid(2), "page".into());
        page.add_view(ViewDefinition::new(sid(3), "view".into(), sid(10)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut records = DocumentRecords::new(definition);
        records.nodes.push(NodeRecord {
            id: sid(10),
            parent: None,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: "root".into(),
            payload: NodePayload::Frame(FramePayload::default()),
        });
        records
    }

    pub(crate) fn guides(positions: &[(u64, f32)], grids: Vec<LayoutGrid>) -> NodeGuides {
        let mut record = NodeGuides::new(sid(10));
        for (id, position) in positions {
            record
                .apply(&GuideEdit::Add(
                    RulerGuide::new(sid(*id), GuideAxis::X, *position).unwrap(),
                ))
                .unwrap();
        }
        record.apply(&GuideEdit::SetGrids(grids)).unwrap();
        record
    }

    fn variants() -> ViewVariantModel {
        let mut model = ViewVariantModel::new(sid(3));
        let variable = ViewVariable::new(
            ViewVariableId::new(sid(200)),
            "dark".into(),
            TypeSpec::required(ExpressionType::Boolean),
            VariableValue::Value(TypedValue::Boolean(false)),
            [],
        )
        .unwrap();
        let mut overrides = VariantOverrideSet::new();
        overrides.set_property(
            PropertyPath::with_keys(sid(10), vec![PropertyKey(0x0110)]).with_key_segment("x"),
            TypedValue::Boolean(true),
        );
        for command in [
            ViewVariantCommand::CreateVariable(variable),
            ViewVariantCommand::CreateVariant(ViewVariant::new(
                ViewVariantId::new(sid(300)),
                VariantCondition::new().equals(
                    ViewVariableId::new(sid(200)),
                    VariableValue::Value(TypedValue::Boolean(true)),
                ),
                overrides,
            )),
        ] {
            command.apply(&mut model).unwrap();
        }
        model
    }

    pub(crate) fn snapshot(records: &DocumentRecords) -> SemanticSnapshot {
        let mut builder = SnapshotBuilder::new();
        builder.add_document(records).unwrap();
        builder.finish()
    }

    #[test]
    fn guides_and_view_variants_project_per_item_and_round_trip() {
        let mut working = records();
        working.node_guides = vec![guides(
            &[(100, 120.0), (101, 8.0)],
            vec![LayoutGrid::columns(12, 20.0, 32.0).unwrap()],
        )];
        working.view_variants = vec![variants()];
        let projected = snapshot(&working);
        let root = projected.get(sid(10)).unwrap();
        assert_eq!(
            root.property(
                &root
                    .path(&[NODE_GUIDE])
                    .with_key_segment(&sid(100).to_string())
            ),
            Some(&SemanticValue::Text(format!("guide x 120px {}", sid(100))))
        );
        assert!(root.property(&root.path(&[NODE_LAYOUT_GRIDS])).is_some());
        assert_eq!(projected.get(sid(300)).unwrap().kind(), ObjectKind::Variant);
        let back = document_records(&projected, sid(1)).unwrap().records;
        assert_eq!(back, working);
        // 가이드가 없는 문서의 snapshot에는 가이드 속성이 없습니다(가이드 2개 + 그리드 목록 1개만 늘어남).
        let plain = snapshot(&records());
        assert_eq!(
            plain.get(sid(10)).unwrap().properties().len() + 3,
            root.properties().len()
        );
        assert!(plain.get(sid(300)).is_none());
    }

    #[test]
    fn moving_one_guide_touches_only_its_path() {
        // 검증 기준(ADR-0117 결정 2.4): 서로 다른 가이드 편집은 서로 다른 경로만 바꿉니다(merge 충돌 근거 없음).
        let grid = LayoutGrid::columns(4, 0.0, 0.0).unwrap();
        let base = {
            let mut r = records();
            r.node_guides = vec![guides(&[(100, 1.0), (101, 2.0)], vec![grid])];
            snapshot(&r)
        };
        let ours = {
            let mut r = records();
            r.node_guides = vec![guides(&[(100, 5.0), (101, 2.0)], vec![grid])];
            snapshot(&r)
        };
        let changed: Vec<_> = base
            .get(sid(10))
            .unwrap()
            .properties()
            .iter()
            .filter(|(path, value)| ours.get(sid(10)).unwrap().property(path) != Some(value))
            .map(|(path, _)| path.clone())
            .collect();
        assert_eq!(
            changed,
            vec![PropertyPath::with_keys(sid(10), vec![NODE_GUIDE])
                .with_key_segment(&sid(100).to_string())]
        );
        assert_eq!(category(&changed[0]), Some("guides"));
        assert_eq!(
            category(&PropertyPath::with_keys(sid(3), vec![VIEW_VARIABLE])),
            Some("view-variables")
        );
        assert_eq!(
            category(&PropertyPath::with_keys(sid(3), vec![keys::DISPLAY_NAME])),
            None
        );
    }

    #[test]
    fn non_canonical_guide_values_are_rejected() {
        let mut working = records();
        working.node_guides = vec![guides(&[(100, 1.0)], Vec::new())];
        let mut projected = snapshot(&working);
        let mut root = projected.get(sid(10)).unwrap().clone();
        let path = root
            .path(&[NODE_GUIDE])
            .with_key_segment(&sid(100).to_string());
        root.set_property(
            path.clone(),
            SemanticValue::Text(format!("guide x 1.0px {}", sid(100))),
        )
        .unwrap();
        projected.remove(sid(10));
        projected.insert(root.clone()).unwrap();
        assert!(document_records(&projected, sid(1)).is_err());
        root.set_property(
            path,
            SemanticValue::Text(format!("guide x 1px {}", sid(101))),
        )
        .unwrap();
        projected.remove(sid(10));
        projected.insert(root).unwrap();
        assert!(document_records(&projected, sid(1)).is_err());
    }

    #[test]
    fn canonical_path_bytes_round_trip() {
        for path in [
            PropertyPath::with_keys(sid(5), vec![PropertyKey(1), PropertyKey(0xE001)]),
            PropertyPath::new(sid(5)).with_index(3),
            PropertyPath::with_keys(sid(5), vec![PropertyKey(7)]).with_key_segment("가"),
        ] {
            assert_eq!(path_from_bytes(&path.to_canonical_bytes()), Some(path));
        }
        assert_eq!(path_from_bytes(&[0; 3]), None);
    }
}
