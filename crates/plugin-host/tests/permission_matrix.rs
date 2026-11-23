//! permission matrix (XXI.4.5): 권한별 method 허용·거부, 승인 저장·재열기, package 변경 뒤 저장 승인 무효.

mod support;

use std::collections::BTreeSet;

use stellar_ops::test_support::select_trusted_project;
use stellar_plugin_host::generated::host_api::Method;
use stellar_plugin_host::grants::{GrantScope, GrantStore};
use stellar_plugin_host::host::Dispatcher;
use stellar_plugin_host::json::JsonValue;
use stellar_plugin_host::permission::Permission;
use support::{json, Fixture};

/// 권한 검사를 지나는 최소 매개변수(뒤 단계 오류는 상관없음).
fn params(method: Method) -> JsonValue {
    let id = "0195c530-3acd-7bc1-8414-c213f61cb002";
    json(&match method {
        Method::QueryTreePage => format!(r#"{{"document":"{id}"}}"#),
        Method::QueryNode => format!(r#"{{"document":"{id}","node":"{id}"}}"#),
        Method::CommandApply => {
            format!(r#"{{"base_revision":"g0","document":"{id}","commands":[],"dry_run":true}}"#)
        }
        Method::FileRead => r#"{"path":"a.txt"}"#.to_owned(),
        Method::FileWrite => r#"{"path":"a.txt","text":"x"}"#.to_owned(),
        Method::NetworkFetch => r#"{"url":"https://example.com/"}"#.to_owned(),
        _ => "{}".to_owned(),
    })
}

fn denied(dispatcher: &Dispatcher, method: Method) -> bool {
    matches!(
        dispatcher.call(method.name(), &params(method)),
        Err(e) if e.code == "permission-denied"
    )
}

#[test]
fn permission_matrix() {
    let fx = Fixture::new("plugin-matrix", 2);
    let mut options: Vec<Option<Permission>> = vec![None];
    options.extend(Permission::ALL.into_iter().map(Some));
    for granted in options {
        let set: Vec<Permission> = granted.into_iter().collect();
        let dispatcher = fx.dispatcher(&set);
        for method in Method::ALL {
            let expect_denied = match method.permission() {
                None => false,
                Some(required) => Some(required) != granted,
            };
            assert_eq!(
                denied(&dispatcher, method),
                expect_denied,
                "granted {granted:?}, method {}",
                method.name()
            );
        }
    }
}

#[test]
fn grants_persist_per_plugin_and_session_grants_do_not() {
    let fx = Fixture::new("plugin-grants", 1);
    let path = fx.temp.path.join("app-data/plugins/grants.json");
    let plugin = fx.plugin();
    let mut store = GrantStore::open(&path).unwrap();
    assert!(store.effective(&plugin).is_empty());
    store
        .grant(&plugin, Permission::DocumentRead, GrantScope::Persistent)
        .unwrap();
    store
        .grant(&plugin, Permission::Network, GrantScope::Session)
        .unwrap();
    assert_eq!(
        store.effective(&plugin),
        BTreeSet::from([Permission::DocumentRead, Permission::Network])
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"document.read\"") && !text.contains("network"));

    // 다시 열면 저장 승인만 남습니다.
    let reopened = GrantStore::open(&path).unwrap();
    assert_eq!(
        reopened.effective(&plugin),
        BTreeSet::from([Permission::DocumentRead])
    );

    // 승인으로 만든 dispatcher가 그 권한만 씁니다.
    let dispatcher = Dispatcher::new(
        select_trusted_project(&fx.project, fx.data.clone()).unwrap(),
        plugin.clone(),
        &reopened.effective(&plugin),
        fx.config(),
    );
    assert!(dispatcher
        .call("document.read_snapshot", &json("{}"))
        .is_ok());
    assert!(denied(&dispatcher, Method::FileList));

    // 철회는 파일에서도 지웁니다.
    let mut store = reopened;
    store
        .revoke(&plugin.manifest().id, Permission::DocumentRead)
        .unwrap();
    assert!(GrantStore::open(&path)
        .unwrap()
        .effective(&plugin)
        .is_empty());
}

#[test]
fn undeclared_permissions_cannot_be_granted_and_updates_need_reapproval() {
    let fx = Fixture::new("plugin-grants-update", 1);
    let path = fx.temp.path.join("grants.json");
    // 일부 권한만 선언한 package.
    std::fs::write(
        fx.package.join("plugin.yaml"),
        format!(
            "manifest_version: 1\nid: {}\nversion: 1.0.0\nname: Some\nentry: ui/index.html\nhost_api: 1\npermissions:\n  - document.read\n",
            support::PLUGIN_ID
        ),
    )
    .unwrap();
    let plugin = fx.plugin();
    let mut store = GrantStore::open(&path).unwrap();
    assert_eq!(
        store
            .grant(&plugin, Permission::DocumentApply, GrantScope::Persistent)
            .unwrap_err()
            .code(),
        "permission-not-declared"
    );
    store
        .grant(&plugin, Permission::DocumentRead, GrantScope::Persistent)
        .unwrap();
    // 선언하지 않은 권한을 dispatcher에 넘겨도 유효 권한이 되지 않습니다.
    let dispatcher = fx.dispatcher(&Permission::ALL);
    assert_eq!(
        dispatcher.permissions(),
        &BTreeSet::from([Permission::DocumentRead])
    );

    // package 내용이 바뀌면(digest 변경) 저장 승인을 쓰지 않습니다.
    std::fs::write(fx.package.join("ui/index.html"), "<!doctype html><p>v2").unwrap();
    let updated = fx.plugin();
    assert_ne!(updated.package().digest(), plugin.package().digest());
    let store = GrantStore::open(&path).unwrap();
    assert!(store.effective(&updated).is_empty());
    assert_eq!(
        store.effective(&plugin),
        BTreeSet::from([Permission::DocumentRead])
    );
}

#[test]
fn corrupt_grant_files_are_rejected() {
    let fx = Fixture::new("plugin-grants-corrupt", 1);
    let path = fx.temp.path.join("grants.json");
    for text in [
        "not json",
        r#"{"format":2,"plugins":[]}"#,
        r#"{"format":1,"plugins":[{"id":"a.b","digest":"sha256:00","permissions":["root"]}]}"#,
        r#"{"format":1,"plugins":[{"id":"a.b","digest":"md5:00","permissions":[]}]}"#,
        r#"{"format":1,"plugins":[],"extra":1}"#,
    ] {
        std::fs::write(&path, text).unwrap();
        assert_eq!(
            GrantStore::open(&path).unwrap_err().code(),
            "grants-corrupt",
            "{text}"
        );
    }
}
