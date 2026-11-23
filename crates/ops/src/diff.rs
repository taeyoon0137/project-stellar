//! 문서 레코드의 semantic diff 요약 (XXIV.4.4, XIV.3).
//!
//! 두 문서 레코드 집합을 semantic snapshot(XIV.3.1)으로 투영하고 diff(XIV.3.3~XIV.3.8)를 계산해,
//! 출력 형식과 무관한 [`DiffEntry`] 목록으로 바꿉니다. 순서는 semantic-diff의 canonical 순서
//! (XIV.3.8: 문서 → 객체 경로 → 속성 경로 → 변경 종류)를 그대로 따릅니다.
//!
//! 문서 하나의 투영 입력은 영속 레코드와 레코드 밖 Component Context([`DocumentInput`], ADR-0099·ADR-0114 결정 2)입니다.
//!
//! - **문서 범위**([`snapshot_of`]): 컴포넌트 library를 투영하지 않는 snapshot(창 reconciliation, Git stage, 병합 결과
//!   저장, 텍스트 원본 merge driver). Context key·provision 객체와 requirement·Slot 공개 목록 전부를 넣습니다. 목록은
//!   Component·Slot 객체가 없으므로 문서의 Document 객체에 대상 id map key로 붙습니다(ADR-0123 결정 1). 그래서 이
//!   범위도 "문서 → snapshot → 문서" 왕복이 무손실이고, 목록 변경도 병합·stage 단위입니다.
//! - **프로젝트 범위**([`add_contexts`]에 library): library 투영 뒤에 Context를 넣습니다. requirement·Slot 공개 목록은
//!   그 컴포넌트·Slot이 library에 있을 때만 투영합니다(없는 대상의 항목은 validator의 orphan 진단 대상이고 diff에는
//!   나타나지 않음, [`projectable_context`]). `stellar diff`·compare·HEAD 비교·`stellar apply` dry-run이 씁니다.
//! - 역투영([`document_from_snapshot`])은 레코드와 Context를 함께 되돌립니다(병합 결과 반영).

use std::collections::BTreeMap;
use std::fmt;

use stellar_document::component::definition::ComponentLibrary;
use stellar_document::context::set::ContextSet;
use stellar_document::model::DocumentModel;
use stellar_document::records::DocumentRecords;
use stellar_semantic_diff::build::context::context_from_snapshot;
use stellar_semantic_diff::build::SnapshotBuilder;
use stellar_semantic_diff::change::{Position, SemanticChange};
use stellar_semantic_diff::diff::{diff, SemanticDiff};
use stellar_semantic_diff::snapshot::{ObjectKind, SemanticSnapshot};

use crate::core_types::ids::StableId;
use crate::query::{property_key_label, property_path_label, OpsValue};

/// semantic snapshot 구성 실패(레코드 불변식 위반).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionError(pub String);

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "semantic snapshot을 만들 수 없습니다: {}", self.0)
    }
}

impl std::error::Error for ProjectionError {}

/// 위치(부모와 OrderKey 16진수).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPosition {
    /// 부모(`None`이면 root).
    pub parent: Option<StableId>,
    /// OrderKey bytes 16진수(`None`이면 순서 없음).
    pub order_key_hex: Option<String>,
}

impl From<&Position> for DiffPosition {
    fn from(position: &Position) -> DiffPosition {
        DiffPosition {
            parent: position.parent,
            order_key_hex: position
                .order_key
                .as_ref()
                .map(|key| key.as_bytes().iter().map(|b| format!("{b:02x}")).collect()),
        }
    }
}

/// semantic 변경 한 건(표시용).
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    /// 변경 종류 이름(XIV.3.2, 예: `set-property`).
    pub kind: &'static str,
    /// 변경 종류 code.
    pub code: u8,
    /// 소유 문서(프로젝트 범위면 `None`).
    pub document: Option<StableId>,
    /// 정의나 그 tree의 속성이 속한 컴포넌트.
    pub component: Option<StableId>,
    /// 대상 객체.
    pub object: StableId,
    /// 대상 객체 종류(추가·삭제 변경에만, 예: `node:frame`).
    pub object_kind: Option<String>,
    /// 속성 경로([`property_path_label`] registry 이름, 속성 변경에만).
    pub path: Option<String>,
    /// numeric 속성 경로([`property_key_label`], 보조, 속성 변경에만).
    pub key: Option<String>,
    /// 이전 값(속성 변경, 없으면 추가).
    pub old: Option<OpsValue>,
    /// 이후 값(속성 변경, 없으면 삭제).
    pub new: Option<OpsValue>,
    /// 이전 위치(위치 변경에만).
    pub from: Option<DiffPosition>,
    /// 이후 위치(위치 변경에만).
    pub to: Option<DiffPosition>,
}

/// 객체 종류 표시 이름.
pub fn object_kind_name(kind: ObjectKind) -> String {
    match kind {
        ObjectKind::Project => "project".to_string(),
        ObjectKind::Document => "document".to_string(),
        ObjectKind::Page => "page".to_string(),
        ObjectKind::View => "view".to_string(),
        ObjectKind::Node(kind) => format!("node:{}", kind.name()),
        ObjectKind::Modifier(kind) => format!("modifier:{}", kind.name()),
        ObjectKind::Component => "component".to_string(),
        ObjectKind::ComponentSlot => "component-slot".to_string(),
        ObjectKind::Token => "token".to_string(),
        ObjectKind::Asset => "asset".to_string(),
        ObjectKind::ExpressionNode => "expression-node".to_string(),
        ObjectKind::Variant => "variant".to_string(),
        ObjectKind::ContextKey => "context-key".to_string(),
        ObjectKind::ContextProvision => "context-provision".to_string(),
    }
}

impl From<&SemanticChange> for DiffEntry {
    fn from(change: &SemanticChange) -> DiffEntry {
        let kind = change.kind();
        let mut entry = DiffEntry {
            kind: kind.name(),
            code: kind.code(),
            document: change.document(),
            component: change.provenance().definition_component(),
            object: change.object_id(),
            object_kind: None,
            path: None,
            key: None,
            old: None,
            new: None,
            from: None,
            to: None,
        };
        match change {
            SemanticChange::AddObject(c)
            | SemanticChange::RemoveObject(c)
            | SemanticChange::AddToken(c)
            | SemanticChange::RemoveToken(c) => {
                entry.object_kind = Some(object_kind_name(c.object.kind()));
            }
            SemanticChange::MoveObject(c) | SemanticChange::ReorderObject(c) => {
                entry.from = Some(DiffPosition::from(&c.from));
                entry.to = Some(DiffPosition::from(&c.to));
            }
            SemanticChange::SetProperty(c)
            | SemanticChange::ChangeToken(c)
            | SemanticChange::ComponentInterfaceChange(c)
            | SemanticChange::AssetRefChange(c) => {
                entry.path = Some(property_path_label(&c.path));
                entry.key = Some(property_key_label(&c.path));
                entry.old = c.old.as_ref().map(OpsValue::from);
                entry.new = c.new.as_ref().map(OpsValue::from);
            }
        }
        entry
    }
}

/// 문서 하나의 투영 입력: 영속 레코드와 레코드 밖 Context(ADR-0099).
#[derive(Debug, Clone, Copy)]
pub struct DocumentInput<'a> {
    /// 영속 레코드(canonical 정렬).
    pub records: &'a DocumentRecords,
    /// 영속 Context(`ContextSet::live` 결과).
    pub context: &'a ContextSet,
}

/// 문서 하나의 소유 입력(레코드 + Context).
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSource {
    /// 영속 레코드(canonical 정렬).
    pub records: DocumentRecords,
    /// 영속 Context.
    pub context: ContextSet,
}

impl DocumentSource {
    /// 문서 모델의 영속 레코드(canonical 정렬)와 영속 Context.
    pub fn of_model(model: &DocumentModel) -> Result<DocumentSource, ProjectionError> {
        let mut records = model
            .to_records()
            .map_err(|e| ProjectionError(e.to_string()))?;
        records.sort_canonical();
        Ok(DocumentSource {
            records,
            context: model.context().live(model),
        })
    }

    /// 투영 입력.
    pub fn input(&self) -> DocumentInput<'_> {
        DocumentInput {
            records: &self.records,
            context: &self.context,
        }
    }

    /// 레코드와 Context를 담은 문서 모델.
    pub fn into_model(self) -> Result<DocumentModel, ProjectionError> {
        let (mut model, _) = DocumentModel::from_records(self.records)
            .map_err(|e| ProjectionError(e.to_string()))?;
        model.replace_context(self.context);
        Ok(model)
    }
}

static EMPTY_CONTEXT: std::sync::OnceLock<ContextSet> = std::sync::OnceLock::new();

/// Context가 없는 문서의 투영 입력(Context를 모르는 호출자, 예: 레코드만 비교하는 agent 경로).
pub fn records_only(records: &DocumentRecords) -> DocumentInput<'_> {
    DocumentInput {
        records,
        context: EMPTY_CONTEXT.get_or_init(ContextSet::new),
    }
}

/// snapshot에 넣을 수 있는 Context: key·provision 전부와, `library`에 있는 컴포넌트·Slot의 requirement·공개 목록.
/// `library`가 없으면(문서 범위) 전부입니다(목록은 Document 객체에 붙음, ADR-0123 결정 1).
pub fn projectable_context(set: &ContextSet, library: Option<&ComponentLibrary>) -> ContextSet {
    match library {
        Some(library) => {
            let mut out = declarations(set);
            add_library_members(&mut out, set, library);
            out
        }
        None => set.clone(),
    }
}

/// key·provision만 담은 사본.
fn declarations(set: &ContextSet) -> ContextSet {
    let mut out = ContextSet::new();
    for key in set.keys() {
        let _ = out.insert_key(key.clone());
    }
    for provision in set.provisions() {
        let _ = out.insert_provision(provision.clone());
    }
    out
}

/// `set`의 requirement·Slot 공개 목록 가운데 `library`에 그 컴포넌트·Slot이 있는 항목을 `out`에 더합니다.
fn add_library_members(out: &mut ContextSet, set: &ContextSet, library: &ComponentLibrary) {
    for (component, keys) in set.all_requirements() {
        if library
            .get(crate::core_types::reference::ComponentRef::new(component))
            .is_some()
        {
            for key in keys {
                out.add_requirement(component, *key);
            }
        }
    }
    for (slot, keys) in set.all_slot_keys() {
        let present = library
            .iter()
            .any(|entry| entry.slots().any(|s| s.id.stable_id() == slot));
        if present {
            for key in keys {
                out.add_slot_key(slot, *key);
            }
        }
    }
}

/// 이미 문서(와 프로젝트 범위면 library)를 넣은 builder에 문서마다 Context key·provision을 넣고 requirement·Slot 공개
/// 목록을 넣습니다([`projectable_context`]의 범위). 문서 범위면 목록은 그 문서의 Document 객체에 붙습니다.
///
/// requirement·공개 목록은 프로젝트 library의 Component·Slot 객체 하나에 붙는 속성이라 문서별로 따로 붙일 수 없습니다.
/// 여러 문서가 같은 컴포넌트·Slot에 항목을 두면 합집합 하나로 붙입니다(문서마다 붙이면 같은 속성이 두 번 들어가
/// 투영 전체가 실패합니다).
pub fn add_contexts<'a>(
    builder: &mut SnapshotBuilder,
    documents: impl IntoIterator<Item = DocumentInput<'a>>,
    library: Option<&ComponentLibrary>,
) -> Result<(), ProjectionError> {
    let mut members = ContextSet::new();
    let mut anchor = None;
    for document in documents {
        let id = document.records.definition.id;
        if let Some(library) = library {
            add_library_members(&mut members, document.context, library);
            anchor.get_or_insert(id);
        }
        let set = match library {
            Some(_) => declarations(document.context),
            None => document.context.clone(),
        };
        if set.is_empty() {
            continue;
        }
        builder
            .add_context(id, &set)
            .map_err(|e| ProjectionError(format!("문서 {id}의 Context: {e}")))?;
    }
    if let Some(anchor) = anchor.filter(|_| !members.is_empty()) {
        // 목록 항목만 담은 set이라 문서 id는 객체 출처로 쓰이지 않습니다.
        builder.add_context(anchor, &members).map_err(|e| {
            ProjectionError(format!(
                "프로젝트 컴포넌트의 Context requirement·Slot 공개 목록: {e}"
            ))
        })?;
    }
    Ok(())
}

/// 문서들의 semantic snapshot(문서 범위: 레코드와 Context 전부, 모듈 문서).
pub fn snapshot_of(documents: &[DocumentInput<'_>]) -> Result<SemanticSnapshot, ProjectionError> {
    let mut builder = SnapshotBuilder::new();
    for document in documents {
        builder
            .add_document(document.records)
            .map_err(|e| ProjectionError(e.to_string()))?;
    }
    add_contexts(&mut builder, documents.iter().copied(), None)?;
    Ok(builder.finish())
}

/// 문서들과 프로젝트 컴포넌트 library의 semantic snapshot(프로젝트 범위, 모듈 문서).
pub fn project_snapshot_of(
    documents: &[DocumentInput<'_>],
    library: &ComponentLibrary,
) -> Result<SemanticSnapshot, ProjectionError> {
    let mut builder = SnapshotBuilder::new();
    for document in documents {
        builder
            .add_document(document.records)
            .map_err(|e| ProjectionError(e.to_string()))?;
    }
    builder
        .add_project_component_library(library)
        .map_err(|e| ProjectionError(e.to_string()))?;
    add_contexts(&mut builder, documents.iter().copied(), Some(library))?;
    Ok(builder.finish())
}

/// snapshot에서 문서 `id`의 레코드와 Context를 되돌립니다(병합 결과 반영, ADR-0114 결정 2).
pub fn document_from_snapshot(
    snapshot: &SemanticSnapshot,
    id: StableId,
) -> Result<DocumentSource, ProjectionError> {
    let records = stellar_semantic_diff::records::document_records(snapshot, id)
        .map_err(|e| ProjectionError(e.to_string()))?
        .records;
    let context = context_from_snapshot(snapshot, id)
        .map_err(|object| ProjectionError(format!("Context 객체 {object}를 되돌릴 수 없습니다")))?;
    Ok(DocumentSource { records, context })
}

/// semantic diff(XIV.3)와 표시용 목록.
#[derive(Debug, Clone, Default)]
pub struct DiffReport {
    /// canonical 순서의 변경.
    pub diff: SemanticDiff,
    /// 표시용 목록(같은 순서).
    pub entries: Vec<DiffEntry>,
}

impl DiffReport {
    /// 변경 종류 이름별 개수(이름 순).
    pub fn counts(&self) -> BTreeMap<&'static str, usize> {
        let mut out = BTreeMap::new();
        for entry in &self.entries {
            *out.entry(entry.kind).or_default() += 1;
        }
        out
    }

    /// 변경이 없는지.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 두 snapshot의 semantic diff.
pub fn diff_snapshots(base: &SemanticSnapshot, target: &SemanticSnapshot) -> DiffReport {
    let diff = diff(base, target);
    let entries = diff.changes().iter().map(DiffEntry::from).collect();
    DiffReport { diff, entries }
}

/// 두 문서 집합(레코드 + Context)의 semantic diff. `library`가 있으면 프로젝트 범위(requirement·Slot 공개 목록
/// 포함), 없으면 문서 범위입니다. library 객체는 양쪽에 같으므로 diff에 나타나지 않습니다.
pub fn diff_documents(
    base: &[DocumentInput<'_>],
    target: &[DocumentInput<'_>],
    library: Option<&ComponentLibrary>,
) -> Result<DiffReport, ProjectionError> {
    let snapshot = |documents| match library {
        Some(library) => project_snapshot_of(documents, library),
        None => snapshot_of(documents),
    };
    Ok(diff_snapshots(&snapshot(base)?, &snapshot(target)?))
}

/// 두 문서 레코드 집합의 semantic diff. Context는 비교하지 않습니다(Context를 다루는 호출자는 [`diff_documents`]).
pub fn diff_records(
    base: &[&DocumentRecords],
    target: &[&DocumentRecords],
) -> Result<DiffReport, ProjectionError> {
    let base: Vec<DocumentInput<'_>> = base.iter().map(|r| records_only(r)).collect();
    let target: Vec<DocumentInput<'_>> = target.iter().map(|r| records_only(r)).collect();
    diff_documents(&base, &target, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::random_valid_document;

    #[test]
    fn identical_records_have_no_changes_and_visibility_change_is_one_property_change() {
        let model = random_valid_document(5);
        let records = model.to_records().unwrap();
        assert!(diff_records(&[&records], &[&records]).unwrap().is_empty());

        let mut changed = records.clone();
        let node = changed.nodes.first_mut().unwrap();
        node.visible = !node.visible;
        let target = node.id;
        let report = diff_records(&[&records], &[&changed]).unwrap();
        assert_eq!(report.entries.len(), 1);
        let entry = &report.entries[0];
        assert_eq!(entry.kind, "set-property");
        assert_eq!(entry.object, target);
        assert_eq!(entry.path.as_deref(), Some("0xe001"));
        assert_eq!(entry.document, Some(model.document_id()));
        assert_eq!(report.counts().get("set-property"), Some(&1));
    }

    #[test]
    fn requirements_of_one_library_component_from_two_documents_project_as_one_union() {
        // 프로젝트 범위: 두 문서가 같은 library 컴포넌트·Slot에 requirement·공개 목록을 두어도 투영이 실패하지 않고
        // 그 Component·Slot 객체에 합집합 하나가 붙습니다.
        use stellar_document::context::fixtures::world::{BODY, CARD, D, E, N};
        use stellar_document::context::fixtures::{sample_world, sid};
        use stellar_semantic_diff::build::context::{
            COMPONENT_CONTEXT_REQUIREMENTS, SLOT_CONTEXT_KEYS,
        };
        use stellar_semantic_diff::value::SemanticValue;
        let (model, library) = sample_world();
        let records = model.to_records().unwrap();
        let first = model.context().live(&model);
        assert!(first.requirements(sid(CARD)).contains(&sid(N)));
        let mut other_records = records.clone();
        other_records.definition.id = sid(9_999);
        let mut other = ContextSet::new();
        other.add_requirement(sid(CARD), sid(D));
        other.add_slot_key(sid(BODY), sid(N));
        let mut builder = SnapshotBuilder::new();
        builder.add_project_component_library(&library).unwrap();
        add_contexts(
            &mut builder,
            [
                DocumentInput {
                    records: &records,
                    context: &first,
                },
                DocumentInput {
                    records: &other_records,
                    context: &other,
                },
            ],
            Some(&library),
        )
        .expect("같은 컴포넌트의 requirement를 두 문서가 둠");
        let snapshot = builder.finish();
        let ids = |object: StableId, key| {
            let object = snapshot.get(object).unwrap();
            match object.property(&object.path(&[key])) {
                Some(SemanticValue::Ids(ids)) => ids.clone(),
                other => panic!("{other:?}"),
            }
        };
        let mut want = vec![sid(N), sid(D)];
        want.sort();
        assert_eq!(ids(sid(CARD), COMPONENT_CONTEXT_REQUIREMENTS), want);
        let mut want = vec![sid(D), sid(E), sid(N)];
        want.sort();
        assert_eq!(ids(sid(BODY), SLOT_CONTEXT_KEYS), want);
    }
}
