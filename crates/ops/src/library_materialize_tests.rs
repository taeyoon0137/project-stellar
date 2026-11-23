//! 제품 명령 종단 시험: 실제 Git library 저장소, 시험 cache 위치(ADR-0137).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use stellar_commands::command::Command;
use stellar_commands::swap::SwapInstance;
use stellar_dependencies::cache_entry::{EntryReuse, MaterializeError};
use stellar_dependencies::declaration::{LogicalName, RefName, RepositoryUrl, VersionSelector};
use stellar_dependencies::read_only::LibraryEditRejection;
use stellar_document::component::definition::{ComponentLibrary, ComponentPropertyId};
use stellar_document::component::instance::OverrideKey;
use stellar_project::library_guard::LibraryGuardRejection;
use stellar_project::library_open::read_lock;
use stellar_project::library_refs::{classify_component, ComponentOrigin};

use super::*;
use crate::core_types::typed_value::TypedValue;
use crate::library::{check_commands, open_libraries};
use crate::library_fixture::*;
use crate::test_support::TempDir;

const SPACE: u64 = 7;
const CARD: u64 = 1;
const BADGE: u64 = 2;
const CHIP: u64 = 20;
const CARD_INSTANCE: u64 = 100;
const BADGE_INSTANCE: u64 = 101;
const GHOST: u64 = 99;
const GHOST_INSTANCE: u64 = 102;

fn name(text: &str) -> LogicalName {
    LogicalName::new(text).unwrap()
}

fn tag(text: &str) -> VersionSelector {
    VersionSelector::Tag(RefName::new(text).unwrap())
}

fn component(n: u64) -> ComponentRef {
    ComponentRef::new(sid(SPACE, n))
}

fn defs(ids: &[u64]) -> ComponentLibrary {
    let names = ["", "Card", "Badge", "Tag"];
    let owned: Vec<(StableId, StableId, String)> = ids
        .iter()
        .map(|n| {
            let label = names
                .get(*n as usize)
                .map_or_else(|| format!("C{n}"), |s| (*s).to_string());
            (sid(SPACE, *n), sid(SPACE, 500 + n), label)
        })
        .collect();
    let borrowed: Vec<(StableId, StableId, &str)> =
        owned.iter().map(|(a, b, c)| (*a, *b, c.as_str())).collect();
    components(&borrowed)
}

struct Env {
    _dir: TempDir,
    base: PathBuf,
    project: PathBuf,
    app: PathBuf,
}

impl Env {
    fn new(tag: &str, instances: &[(u64, u64)]) -> Option<Env> {
        if !has_git() {
            eprintln!("시스템 Git 없음: 건너뜀");
            return None;
        }
        let dir = TempDir::new(tag);
        let base = dir.path.clone();
        let instances: Vec<_> = instances
            .iter()
            .map(|(id, c)| (sid(SPACE, *id), instance_of(sid(SPACE, *c))))
            .collect();
        let project = using_project(&base, document(SPACE, &instances));
        let app = base.join("app-data");
        Some(Env {
            _dir: dir,
            base,
            project,
            app,
        })
    }

    fn tools(&self) -> LibraryTools {
        LibraryTools::in_app_data(&self.app).unwrap()
    }

    fn repo(&self, repo: &str, id: u64) -> LibraryRepo {
        LibraryRepo::init(&self.base.join("libraries"), repo, id)
    }

    fn files(&self) -> (Vec<u8>, Option<Vec<u8>>) {
        (
            std::fs::read(self.project.join("stellar.yaml")).unwrap(),
            std::fs::read(self.project.join("stellar.lock.yaml")).ok(),
        )
    }
}

fn add_request(dependency: &str, repo: &LibraryRepo, version: &str, stored: &[u64]) -> AddRequest {
    AddRequest {
        name: name(dependency),
        repository: RepositoryUrl::new(&repo.url).unwrap(),
        version: tag(version),
        components: stored.iter().map(|n| component(*n)).collect(),
    }
}

fn locked_commit(project: &Path, dependency: &str) -> String {
    let lock = read_lock(project).unwrap().expect("lock");
    let path = DependencyPath::from_names(vec![name(dependency)]);
    lock.entries()
        .iter()
        .find(|e| e.dependency_path == path)
        .expect("root entry")
        .exact_commit
        .to_string()
}

fn stored(project: &Path, dependency: &str) -> BTreeSet<ComponentRef> {
    ManifestEdit::open(project)
        .unwrap()
        .declarations()
        .iter()
        .find(|d| d.name.as_str() == dependency)
        .expect("선언")
        .components
        .clone()
}

/// 문서 참조 진단 code(instance 순).
fn ref_codes(env: &Env, cache: &LibraryCache) -> Vec<(StableId, String)> {
    let snapshot = crate::snapshot::open_snapshot(&env.project).unwrap();
    let document = sid(SPACE, 900);
    let model = snapshot.document_model(document).unwrap();
    open_libraries(&env.project, Ok(cache), 9)
        .document_contribution(document, &model, &ComponentLibrary::new())
        .diagnostics
        .into_iter()
        .map(|d| (d.key.source.unwrap(), d.key.code))
        .collect()
}

#[test]
fn add_stores_declaration_and_lock_and_reuses_lock_commits_offline() {
    let Some(env) = Env::new("lib-add", &[(CARD_INSTANCE, CARD)]) else {
        return;
    };
    let ds = env.repo("ds", 61);
    let v1 = ds.release(SPACE, defs(&[CARD, BADGE]), "v1");
    let mut tools = env.tools();

    let report = add(
        &env.project,
        &mut tools,
        &add_request("ds", &ds, "v1", &[CARD]),
        1,
    )
    .unwrap();
    assert!(report.manifest_written && report.lock_written);
    assert_eq!(report.nodes.len(), 1);
    assert_eq!(report.nodes[0].reuse, EntryReuse::Created);
    assert_eq!(report.pinned, 0, "lock이 없으면 Git으로 해석");
    let edit = ManifestEdit::open(&env.project).unwrap();
    let declaration = &edit.declarations()[0];
    assert_eq!(
        declaration.library_id.to_string(),
        ds.id,
        "library id는 해석한 commit의 project.id"
    );
    assert_eq!(declaration.version, tag("v1"));
    assert_eq!(stored(&env.project, "ds"), [component(CARD)].into());
    assert_eq!(locked_commit(&env.project, "ds"), v1);

    // 같은 이름은 거부하고 파일을 바꾸지 않습니다.
    let before = env.files();
    assert!(matches!(
        add(
            &env.project,
            &mut tools,
            &add_request("ds", &ds, "v1", &[]),
            2
        ),
        Err(LibraryCommandError::DuplicateDependency { .. })
    ));
    // export되지 않는 component를 저장하려 하면 거부합니다(파일 불변).
    let chips = env.repo("chips", 62);
    chips.release(SPACE + 1, defs(&[CHIP]), "v1");
    match add(
        &env.project,
        &mut tools,
        &add_request("chips", &chips, "v1", &[CARD]),
        3,
    ) {
        Err(LibraryCommandError::UnknownComponents { components, .. }) => {
            assert_eq!(components, [sid(SPACE, CARD)]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(env.files(), before, "실패한 add는 파일을 바꾸지 않음");

    // ds 저장소를 지웁니다(offline). lock이 있고 선택자가 그대로인 ds는 Git 없이 lock commit·cache를 씁니다(XV.4.3).
    std::fs::remove_dir_all(&ds.dir).unwrap();
    let report = add(
        &env.project,
        &mut tools,
        &add_request("chips", &chips, "v1", &[]),
        4,
    )
    .unwrap();
    assert_eq!(report.pinned, 1, "ds는 lock commit");
    assert!(
        report.manifests_from_cache >= 1,
        "ds 매니페스트는 cache entry"
    );
    let ds_node = report
        .nodes
        .iter()
        .find(|n| n.path.to_string() == "ds")
        .unwrap();
    assert_eq!(
        ds_node.reuse,
        EntryReuse::CommitHit,
        "ds entry는 Git 없이 재사용"
    );
    assert_eq!(locked_commit(&env.project, "ds"), v1, "ds lock commit 유지");
    assert_eq!(read_lock(&env.project).unwrap().unwrap().entries().len(), 2);

    // fetch: cache에 있으면 Git 없이 끝납니다.
    let fetched = fetch(&env.project, &mut tools, 5).unwrap();
    assert!(fetched
        .nodes
        .iter()
        .all(|n| n.reuse == EntryReuse::CommitHit));
    // 새 기기(빈 cache)에서 ds 저장소가 없으면 fetch는 실패하고 lock은 그대로입니다.
    let before = env.files();
    let mut fresh = LibraryTools::in_app_data(&env.base.join("fresh-app")).unwrap();
    match fetch(&env.project, &mut fresh, 6) {
        Err(LibraryCommandError::Materialize { path, error }) => {
            assert_eq!(path.to_string(), "ds");
            assert!(matches!(error, MaterializeError::Mirror(_)), "{error:?}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(env.files(), before);
}

#[test]
fn stored_refs_decide_external_components_and_document_diagnostics() {
    let Some(env) = Env::new(
        "lib-refs",
        &[
            (CARD_INSTANCE, CARD),
            (BADGE_INSTANCE, BADGE),
            (GHOST_INSTANCE, GHOST),
        ],
    ) else {
        return;
    };
    let ds = env.repo("ds", 81);
    ds.release(SPACE, defs(&[CARD, BADGE]), "v1");
    let mut tools = env.tools();
    add(
        &env.project,
        &mut tools,
        &add_request("ds", &ds, "v1", &[]),
        1,
    )
    .unwrap();
    let cache = tools.cache();
    let own = ComponentLibrary::new();

    // 저장된 참조가 없으면 호환 읽기(Undeclared)이고 진단이 저장을 알립니다.
    let instance = open_libraries(&env.project, Ok(cache), 2).instance();
    assert!(matches!(
        classify_component(&own, &instance, component(CARD)),
        ComponentOrigin::Undeclared(_)
    ));
    assert_eq!(
        ref_codes(&env, cache),
        [
            (
                sid(SPACE, CARD_INSTANCE),
                "library-ref/undeclared".to_string()
            ),
            (
                sid(SPACE, BADGE_INSTANCE),
                "library-ref/undeclared".to_string()
            ),
        ],
        "Ghost는 어디에도 없어 미해결(library 진단 아님)"
    );

    // import로 Card 참조를 저장합니다. export되지 않는 component는 거부합니다.
    let card: BTreeSet<ComponentRef> = [component(CARD)].into();
    let imported = import(&env.project, cache, &name("ds"), &card, 3).unwrap();
    assert!(imported.manifest_written);
    assert_eq!(imported.components, [sid(SPACE, CARD)]);
    assert!(matches!(
        import(
            &env.project,
            cache,
            &name("ds"),
            &[component(GHOST)].into(),
            3
        ),
        Err(LibraryCommandError::UnknownComponents { .. })
    ));
    let instance = open_libraries(&env.project, Ok(cache), 4).instance();
    match classify_component(&own, &instance, component(CARD)) {
        ComponentOrigin::External(reference) => assert_eq!(reference.dependency().as_str(), "ds"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        ref_codes(&env, cache),
        [(
            sid(SPACE, BADGE_INSTANCE),
            "library-ref/undeclared".to_string()
        )]
    );

    // 사전 검사: 저장된 참조의 library가 있으면 없는 property override는 정책 거부입니다.
    let model = crate::snapshot::open_snapshot(&env.project)
        .unwrap()
        .document_model(sid(SPACE, 900))
        .unwrap();
    let mut payload = instance_of(sid(SPACE, CARD));
    payload.set_property_override(
        OverrideKey::direct(ComponentPropertyId::new(sid(SPACE, 77))),
        TypedValue::String("x".into()),
    );
    let swap = [Command::SwapInstance(SwapInstance {
        node: sid(SPACE, CARD_INSTANCE),
        payload,
    })];
    assert!(matches!(
        check_commands(&own, &instance, &model, &swap),
        Err((
            0,
            LibraryGuardRejection::Library(LibraryEditRejection::UnknownProperty { .. })
        ))
    ));

    // offline 기기(빈 cache): 저장된 Card 참조는 사용할 수 없는 library 진단이고, override 설정은 확인할 수 없어 거부합니다
    // (fail closed). 저장되지 않은 Badge는 snapshot이 없어 미해결이라 진단하지 않습니다.
    let offline = LibraryCache::create_in_app_data(&env.base.join("offline-app")).unwrap();
    assert_eq!(
        ref_codes(&env, &offline),
        [(
            sid(SPACE, CARD_INSTANCE),
            "library-resource-unavailable".to_string()
        )]
    );
    let offline_instance = open_libraries(&env.project, Ok(&offline), 5).instance();
    assert!(matches!(
        check_commands(&own, &offline_instance, &model, &swap),
        Err((
            0,
            LibraryGuardRejection::Library(LibraryEditRejection::LibraryUnavailable { .. })
        ))
    ));

    // lock이 없으면 저장된 참조는 아직 받지 않은 의존성입니다.
    let lock = std::fs::read(env.project.join("stellar.lock.yaml")).unwrap();
    std::fs::remove_file(env.project.join("stellar.lock.yaml")).unwrap();
    assert_eq!(
        ref_codes(&env, cache),
        [(
            sid(SPACE, CARD_INSTANCE),
            "library-ref/not-locked".to_string()
        )]
    );
    std::fs::write(env.project.join("stellar.lock.yaml"), lock).unwrap();

    // 저장된 참조가 library에 없는 component면 해석 실패 진단입니다(손으로 고친 stellar.yaml). 저장하지 않은 Ghost는 어디에도
    // 없어 미해결이라 library 진단이 아닙니다(문서 validator 몫).
    let mut edit = ManifestEdit::open(&env.project).unwrap();
    edit.declarations_mut()[0]
        .components
        .insert(component(GHOST));
    let bytes = edit.encoded().unwrap().unwrap();
    edit.write(&env.project, &bytes).unwrap();
    assert_eq!(
        ref_codes(&env, cache),
        [
            (
                sid(SPACE, BADGE_INSTANCE),
                "library-ref/undeclared".to_string()
            ),
            (
                sid(SPACE, GHOST_INSTANCE),
                "library-ref/unresolved".to_string()
            ),
        ]
    );

    // remove는 저장된 참조를 지웁니다.
    let removed = remove_imports(&env.project, &name("ds"), &card).unwrap();
    assert_eq!(removed.components, [sid(SPACE, GHOST)]);
}
