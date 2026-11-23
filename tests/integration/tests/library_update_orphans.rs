//! 후보 library 적용 전 external refs·instance override 검증 종단 시험 (XV.7.3, ADR-0119 결정 5).
//!
//! library DS v1의 Card는 `title`·`badge`·`tone`을 공개합니다. v2는 `badge`를 지우고 `tone`을 Number로 바꾸며
//! Notice component를 지웁니다. 사용 프로젝트 문서의 instance는 v1 기준 override를 가집니다. 제품 update 후보 준비
//! (`prepare_library_update` + 제품 `CacheUpdateSource` 연결)로 v2 후보를 만들고, 제품 후보 검증
//! (`check_update_candidate`)이 적용 전에 orphan 진단을 내는지 봅니다. 검증은 문서·lock을 바꾸지 않습니다.

mod e2e_common;
mod library_common;

use e2e_common::{trusted_git, TempDir};
use library_common::*;
use stellar_dependencies::cache_entry::LibraryEntryStore;
use stellar_dependencies::candidate_check::{CandidateIssue, OrphanCause};
use stellar_dependencies::declaration::{LogicalName, RefName, VersionSelector};
use stellar_dependencies::git_refs::GitSelectorResolver;
use stellar_dependencies::library_snapshot::SnapshotLimits;
use stellar_dependencies::library_update::{
    prepare_library_update, CacheUpdateSource, LibraryUpdateRequest,
};
use stellar_dependencies::mirror::MirrorCache;
use stellar_document::component::definition::{ComponentLibrary, ComponentPropertyId};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey};
use stellar_document::component::property::PropertyVisibility;
use stellar_document::context::fixtures::SceneBuilder;
use stellar_document::expressions::types::{ExpressionType, TypeSpec};
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_project::library_entry::ProjectLibraryValidator;
use stellar_project::library_exports::SourceExports;
use stellar_project::library_git::StorageSha256;
use stellar_project::library_open::{open_project_libraries, read_lock};
use stellar_project::library_refs::{
    check_update_candidate, document_ref_issues, DocumentRefIssue,
};
use stellar_project::library_resolve::ManifestEdit;

const SPACE: u64 = 5;
const CARD: u64 = 1;
const NOTICE: u64 = 3;
const TITLE: u64 = 10;
const BADGE: u64 = 11;
const TONE: u64 = 12;
const CARD_INSTANCE: u64 = 100;
const NOTICE_INSTANCE: u64 = 101;

fn release_library(v2: bool) -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    let mut props = vec![public_string(sid(SPACE, TITLE), "title", "제목")];
    if !v2 {
        props.push(public_string(sid(SPACE, BADGE), "badge", "new"));
    }
    props.push(PropertySpec {
        id: sid(SPACE, TONE),
        name: "tone",
        value_type: if v2 {
            TypeSpec::required(ExpressionType::Number)
        } else {
            TypeSpec::required(ExpressionType::String)
        },
        default: if v2 {
            number_value(1.0)
        } else {
            text_value("calm")
        },
        visibility: PropertyVisibility::Public,
    });
    add_component(
        &mut library,
        sid(SPACE, CARD),
        sid(SPACE, 2),
        "Card",
        &props,
    );
    if !v2 {
        add_component(
            &mut library,
            sid(SPACE, NOTICE),
            sid(SPACE, 4),
            "Notice",
            &[],
        );
    }
    library
}

fn prop(n: u64) -> ComponentPropertyId {
    ComponentPropertyId::new(sid(SPACE, n))
}

#[test]
fn removed_property_override_is_orphan_diagnostic_before_candidate_is_applied() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("library-update-orphans");
    let libraries = dir.path.join("libraries");
    let ds = library_repo(&gate, &libraries, "ds");
    let ds_manifest = manifest_text(library_id(51), "DS", &[]);
    let v1 = release(
        &gate,
        &ds,
        Release {
            manifest: &ds_manifest,
            document: None,
            components: release_library(false),
            token_sets: vec![],
        },
        "v1",
    );
    let v2 = release(
        &gate,
        &ds,
        Release {
            manifest: &ds_manifest,
            document: None,
            components: release_library(true),
            token_sets: vec![],
        },
        "v2",
    );
    let manifest = manifest_text(
        library_id(95),
        "사용 프로젝트",
        &[("ds", library_id(51), &ds.url, "v1")],
    );
    let project = using_project(&dir.path, &manifest);
    let cache = cache_root("update-orphans");
    let mut store: Store = LibraryEntryStore::new(cache.clone(), StorageSha256, Sha256Keyer);
    let work = dir.path.join("neutral");
    std::fs::create_dir_all(&work).unwrap();
    let root = declarations(&manifest);
    let graph = resolve(&cache, &work, &root);
    materialize_and_lock(&mut store, &cache, &graph, &project);
    let lock = read_lock(&project).unwrap().unwrap();
    let current = open_project_libraries(&project, &store, &SnapshotLimits::default(), 2);
    assert!(current.is_complete());

    // 사용 프로젝트 문서: Card instance(title·badge·tone override), Notice instance.
    let mut card = ComponentInstancePayload::new(stellar_core_types::reference::ComponentRef::new(
        sid(SPACE, CARD),
    ));
    card.set_property_override(OverrideKey::direct(prop(TITLE)), text_value("저장"));
    card.set_property_override(OverrideKey::direct(prop(BADGE)), text_value("hot"));
    card.set_property_override(OverrideKey::direct(prop(TONE)), text_value("loud"));
    let notice = ComponentInstancePayload::new(stellar_core_types::reference::ComponentRef::new(
        sid(SPACE, NOTICE),
    ));
    let model = SceneBuilder::new(
        sid(SPACE, 900),
        sid(SPACE, 901),
        sid(SPACE, 902),
        sid(SPACE, 903),
    )
    .instance(sid(SPACE, CARD_INSTANCE), sid(SPACE, 903), card)
    .instance(sid(SPACE, NOTICE_INSTANCE), sid(SPACE, 903), notice)
    .build();
    let before = model_snapshot_json(&model).unwrap();
    let own = ComponentLibrary::new();

    // 제품 update 후보 준비: 선택자를 다시 해석하고 후보 commit을 materialize·검증·snapshot합니다.
    let mut selectors = GitSelectorResolver::new(
        commands(),
        MirrorCache::new(cache.clone(), commands(), Sha256Keyer),
        &work,
    );
    let mut runner = commands();
    let mut mirrors = MirrorCache::new(cache.clone(), commands(), Sha256Keyer);
    let mut validator = ProjectLibraryValidator::cache_entry();
    let mut exports = SourceExports;
    let mut source = CacheUpdateSource {
        store: &mut store,
        runner: &mut runner,
        mirrors: &mut mirrors,
        validator: &mut validator,
        exports: &mut exports,
        limits: SnapshotLimits::default(),
        now_unix_ms: 3,
    };
    let candidate = prepare_library_update(
        &root,
        &lock,
        &LibraryUpdateRequest {
            dependency: LogicalName::new("ds").unwrap(),
            selector: Some(VersionSelector::Tag(RefName::new("v2").unwrap())),
        },
        &mut selectors,
        &mut source,
    )
    .expect("update 후보");
    assert_eq!(candidate.current_commit(), &v1);
    assert_eq!(candidate.candidate_commit(), &v2);

    // 적용 전 검증: 사라진 property의 override와 사라진 component instance가 orphan 진단입니다.
    let (check, ambiguous) = check_update_candidate(&model, &own, &current.instance(), &candidate);
    assert!(ambiguous.is_empty());
    let issues: Vec<_> = check
        .diagnostics()
        .iter()
        .map(|d| d.issue.clone())
        .collect();
    assert_eq!(
        issues,
        [
            CandidateIssue::ComponentRemoved {
                instance: sid(SPACE, NOTICE_INSTANCE)
            },
            CandidateIssue::OrphanOverride {
                instance: sid(SPACE, CARD_INSTANCE),
                property: sid(SPACE, BADGE),
                cause: OrphanCause::PropertyRemoved,
            },
            CandidateIssue::OrphanOverride {
                instance: sid(SPACE, CARD_INSTANCE),
                property: sid(SPACE, TONE),
                cause: OrphanCause::PropertyRetyped,
            },
        ]
    );
    assert!(check
        .diagnostics()
        .iter()
        .all(|d| d.dependency.as_str() == "ds"));
    let text = check.diagnostics()[1].to_string();
    assert!(
        text.contains("orphan") && text.contains("사라졌습니다"),
        "{text}"
    );
    assert!(!text.contains(&ds.url), "진단에 URL 원문 없음");

    // 검증은 문서·lock을 바꾸지 않습니다(override 값은 저장에 남음).
    assert_eq!(model_snapshot_json(&model).unwrap(), before);
    assert_eq!(read_lock(&project).unwrap().unwrap(), lock);

    // 현재 버전(v1)을 다시 후보로 대면 끊기는 참조가 없습니다.
    let (same, _) = check_update_candidate(
        &model,
        &own,
        &current.instance(),
        &prepare_library_update(
            &root,
            &lock,
            &LibraryUpdateRequest {
                dependency: LogicalName::new("ds").unwrap(),
                selector: None,
            },
            &mut selectors,
            &mut source,
        )
        .unwrap(),
    );
    assert!(same.is_clean(), "{:?}", same.diagnostics());

    // 문서 참조 진단(ADR-0137 결정 2): 참조를 stellar.yaml에 저장하면 v1 lock에서는 문제가 없습니다.
    let mut edit = ManifestEdit::open(&project).unwrap();
    edit.declarations_mut()[0].components = [
        stellar_core_types::reference::ComponentRef::new(sid(SPACE, CARD)),
        stellar_core_types::reference::ComponentRef::new(sid(SPACE, NOTICE)),
    ]
    .into();
    edit.write(&project, &edit.encoded().unwrap().unwrap())
        .unwrap();
    let stored = open_project_libraries(&project, &store, &SnapshotLimits::default(), 4);
    assert!(
        document_ref_issues(&model, &own, &stored.instance(), stored.loaded()).is_empty(),
        "v1에서는 모든 override가 유효"
    );

    // 승인 뒤 lock을 v2로 바꾸면 같은 override StableId가 새 snapshot에 다시 대어집니다: title은 그대로 유효하고, 사라진
    // badge·타입이 바뀐 tone만 orphan override, 사라진 Notice는 해석 실패입니다. 문서는 다시 쓰지 않습니다.
    let mut edit = ManifestEdit::open(&project).unwrap();
    edit.declarations_mut()[0].version = VersionSelector::Tag(RefName::new("v2").unwrap());
    edit.write(&project, &edit.encoded().unwrap().unwrap())
        .unwrap();
    let graph = resolve(&cache, &work, edit.declarations());
    materialize_and_lock(&mut store, &cache, &graph, &project);
    let updated = open_project_libraries(&project, &store, &SnapshotLimits::default(), 5);
    assert!(updated.is_complete());
    let issues = document_ref_issues(&model, &own, &updated.instance(), updated.loaded());
    let orphans: Vec<CandidateIssue> = issues
        .iter()
        .filter_map(|issue| match issue {
            DocumentRefIssue::OrphanOverride(d) => Some(d.issue.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        orphans,
        [
            CandidateIssue::OrphanOverride {
                instance: sid(SPACE, CARD_INSTANCE),
                property: sid(SPACE, BADGE),
                cause: OrphanCause::PropertyRemoved,
            },
            CandidateIssue::OrphanOverride {
                instance: sid(SPACE, CARD_INSTANCE),
                property: sid(SPACE, TONE),
                cause: OrphanCause::PropertyRetyped,
            },
        ],
        "title override는 v2에서도 같은 StableId로 유효"
    );
    assert!(issues.iter().any(|issue| matches!(
        issue,
        DocumentRefIssue::Unresolved { instance, .. } if *instance == sid(SPACE, NOTICE_INSTANCE)
    )));
    assert_eq!(issues.len(), 3, "{issues:?}");
    assert_eq!(model_snapshot_json(&model).unwrap(), before);
}
