//! 열린 창의 lock·cache 변경 재로드 시험(실제 Git library, 시험 cache 위치, ADR-0137 결정 5).

use std::collections::BTreeSet;

use stellar_core_types::reference::ComponentRef;
use stellar_ops::library_fixture::*;
use stellar_ops::library_materialize::{
    add, fetch, import, update, AddRequest, DependencyName as LogicalName, LibraryTools, RefName,
    Repository as RepositoryUrl, Selector as VersionSelector, UpdateRequest,
};
use stellar_ops::test_support::TempDir;

use super::*;

const SPACE: u64 = 8;

fn tag(text: &str) -> VersionSelector {
    VersionSelector::Tag(RefName::new(text).unwrap())
}

fn locked_commit(libraries: &WindowLibraries) -> String {
    libraries
        .instance()
        .snapshots()
        .next()
        .expect("snapshot")
        .commit()
        .to_string()
}

#[test]
fn open_window_reloads_when_lock_manifest_or_cache_changes() {
    if !has_git() {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    }
    let dir = TempDir::new("runtime-library-reload");
    let card = sid(SPACE, 1);
    let badge = sid(SPACE, 2);
    let project = using_project(
        &dir.path,
        document(SPACE, &[(sid(SPACE, 100), instance_of(card))]),
    );
    let repo = LibraryRepo::init(&dir.path.join("libraries"), "ds", 91);
    let v1 = repo.release(SPACE, components(&[(card, sid(SPACE, 501), "Card")]), "v1");
    let v2 = repo.release(
        SPACE,
        components(&[
            (card, sid(SPACE, 501), "Card"),
            (badge, sid(SPACE, 502), "Badge"),
        ]),
        "v2",
    );
    let app = dir.path.join("app-data");
    let mut tools = LibraryTools::in_app_data(&app).unwrap();
    let name = LogicalName::new("ds").unwrap();
    add(
        &project,
        &mut tools,
        &AddRequest {
            name: name.clone(),
            repository: RepositoryUrl::new(&repo.url).unwrap(),
            version: tag("v1"),
            components: [ComponentRef::new(card)].into(),
        },
        1,
    )
    .unwrap();

    let window = WindowLibraries::open_in_app_data(&project, &app);
    assert_eq!(locked_commit(&window), v1);
    assert!(window.state().is_complete());
    // 바뀐 것이 없으면(다른 사용이 cache index 최근 사용 시각만 바꿔도) 다시 로드하지 않습니다.
    assert!(!window.refresh());
    let _ = WindowLibraries::open_in_app_data(&project, &app);
    assert!(!window.refresh());
    assert_eq!(window.reloads(), 0);

    // 다른 프로세스가 update하면 다음 사용 때 새 snapshot입니다.
    update(
        &project,
        &mut tools,
        &UpdateRequest {
            name: name.clone(),
            version: Some(tag("v2")),
            allow_orphans: false,
            dry_run: false,
        },
        2,
    )
    .unwrap();
    assert_eq!(locked_commit(&window), v2, "lock 변경을 보고 다시 로드");
    assert_eq!(window.reloads(), 1);

    // 저장된 참조(stellar.yaml)가 바뀌어도 다시 로드합니다.
    let badge_only: BTreeSet<ComponentRef> = [ComponentRef::new(badge)].into();
    import(&project, tools.cache(), &name, &badge_only, 3).unwrap();
    assert_eq!(
        window
            .instance()
            .imports()
            .get(ComponentRef::new(badge))
            .len(),
        1,
        "새 저장 참조"
    );
    assert_eq!(window.reloads(), 2);

    // 받지 못한 기기: 처음에는 사용할 수 없고, 같은 기기에서 fetch하면 cache 변경을 보고 다시 로드합니다.
    let fresh = dir.path.join("fresh-app");
    let offline = WindowLibraries::open_in_app_data(&project, &fresh);
    assert!(!offline.state().is_complete());
    assert_eq!(offline.instance().snapshots().count(), 0);
    let mut fresh_tools = LibraryTools::in_app_data(&fresh).unwrap();
    fetch(&project, &mut fresh_tools, 4).unwrap();
    assert!(offline.refresh());
    assert!(offline.state().is_complete());
    assert_eq!(locked_commit(&offline), v2);

    // 위치를 모르는 library는 다시 로드하지 않습니다.
    assert!(!WindowLibraries::none().refresh());
    let fixed = WindowLibraries::fixed(window.state().clone());
    assert!(!fixed.refresh());
}
