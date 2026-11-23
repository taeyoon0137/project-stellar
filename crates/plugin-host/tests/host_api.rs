//! Host API inventory (XXI.4.2): namespace 분리, 권한 묶음, 결과 형태가 IDL과 같음.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use stellar_plugin_host::generated::host_api::{Method, Namespace};
use stellar_plugin_host::idl::{conforms, Idl, TypeRef};
use stellar_plugin_host::json::JsonValue;
use stellar_plugin_host::permission::Permission;
use support::{json, Fixture};

#[test]
fn each_namespace_has_one_capability() {
    let mut by_namespace: BTreeMap<Namespace, BTreeSet<Option<Permission>>> = BTreeMap::new();
    for method in Method::ALL {
        by_namespace
            .entry(method.namespace())
            .or_default()
            .insert(method.permission());
    }
    let expected = [
        (Namespace::Host, None),
        (Namespace::Document, Some(Permission::DocumentRead)),
        (Namespace::Query, Some(Permission::DocumentRead)),
        (Namespace::Command, Some(Permission::DocumentApply)),
        (Namespace::Git, Some(Permission::GitRead)),
        (Namespace::File, Some(Permission::File)),
        (Namespace::Network, Some(Permission::Network)),
    ];
    assert_eq!(by_namespace.len(), expected.len());
    for (namespace, permission) in expected {
        assert_eq!(
            by_namespace[&namespace],
            BTreeSet::from([permission]),
            "{}",
            namespace.name()
        );
    }
    // 읽기 snapshot, projection 조회, 적용, Git, file, network가 서로 다른 namespace입니다.
    let names: BTreeSet<&str> = Namespace::ALL.iter().map(|n| n.name()).collect();
    for required in ["document", "query", "command", "git", "file", "network"] {
        assert!(names.contains(required));
    }
}

fn result_conforms(idl: &Idl, method: Method, value: &JsonValue) {
    conforms(
        idl,
        value,
        &TypeRef::Record(method.result_type().to_owned()),
        method.name(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn every_method_result_matches_idl() {
    let fx = Fixture::new("plugin-host-api", 6);
    let git = fx.git_init();
    let dispatcher = fx.dispatcher(&Permission::ALL);
    let idl = Idl::source();

    let snapshot = dispatcher
        .call("document.read_snapshot", &json("{}"))
        .unwrap();
    result_conforms(&idl, Method::DocumentReadSnapshot, &snapshot);
    let revision = snapshot
        .get("revision")
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_owned();
    let document = snapshot
        .get("documents")
        .and_then(JsonValue::as_array)
        .and_then(|d| d[0].get("id"))
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_owned();

    let page = dispatcher
        .call(
            "query.tree_page",
            &json(&format!(r#"{{"document":"{document}","limit":2}}"#)),
        )
        .unwrap();
    result_conforms(&idl, Method::QueryTreePage, &page);
    let nodes = page.get("nodes").and_then(JsonValue::as_array).unwrap();
    assert_eq!(nodes.len(), 2);
    let node = nodes[1]
        .get("id")
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_owned();
    let cursor = page.get("next_cursor").and_then(JsonValue::as_str).unwrap();
    let next = dispatcher
        .call(
            "query.tree_page",
            &json(&format!(
                r#"{{"document":"{document}","cursor":"{cursor}"}}"#
            )),
        )
        .unwrap();
    result_conforms(&idl, Method::QueryTreePage, &next);

    let detail = dispatcher
        .call(
            "query.node",
            &json(&format!(r#"{{"document":"{document}","node":"{node}"}}"#)),
        )
        .unwrap();
    result_conforms(&idl, Method::QueryNode, &detail);
    for (method, name) in [
        (Method::QueryTokens, "query.tokens"),
        (Method::QueryComponents, "query.components"),
    ] {
        result_conforms(&idl, method, &dispatcher.call(name, &json("{}")).unwrap());
    }

    let applied = dispatcher
        .call(
            "command.apply",
            &json(&format!(
                r#"{{"base_revision":"{revision}","document":"{document}","dry_run":true,"commands":[{{"kind":"set-visible","node":"{node}","visible":false}}]}}"#
            )),
        )
        .unwrap();
    result_conforms(&idl, Method::CommandApply, &applied);
    assert_eq!(
        applied.get("mode").and_then(JsonValue::as_str),
        Some("dry-run")
    );

    if git {
        let status = dispatcher.call("git.status", &json("{}")).unwrap();
        result_conforms(&idl, Method::GitStatus, &status);
    }

    let written = dispatcher
        .call("file.write", &json(r#"{"path":"notes/a.txt","text":"hi"}"#))
        .unwrap();
    result_conforms(&idl, Method::FileWrite, &written);
    let read = dispatcher
        .call("file.read", &json(r#"{"path":"notes/a.txt"}"#))
        .unwrap();
    result_conforms(&idl, Method::FileRead, &read);
    assert_eq!(read.get("text").and_then(JsonValue::as_str), Some("hi"));
    let list = dispatcher.call("file.list", &json("{}")).unwrap();
    result_conforms(&idl, Method::FileList, &list);

    let fetched = dispatcher
        .call("network.fetch", &json(r#"{"url":"https://example.com/"}"#))
        .unwrap();
    result_conforms(&idl, Method::NetworkFetch, &fetched);

    let info = dispatcher.call("host.info", &json("{}")).unwrap();
    result_conforms(&idl, Method::HostInfo, &info);
}

#[test]
fn dispatcher_rejects_unknown_methods_params_and_missing_permissions() {
    let fx = Fixture::new("plugin-host-errors", 2);
    let none = fx.dispatcher(&[]);
    let code = |d: &stellar_plugin_host::host::Dispatcher, m: &str, p: &str| {
        d.call(m, &json(p)).unwrap_err().code
    };
    assert_eq!(code(&none, "document.delete_all", "{}"), "unknown-method");
    assert_eq!(code(&none, "pointer.move", "{}"), "unknown-method");
    assert_eq!(
        code(&none, "document.read_snapshot", "{}"),
        "permission-denied"
    );
    assert_eq!(
        code(&none, "document.read_snapshot", r#"{"x":1}"#),
        "invalid-params"
    );
    assert_eq!(
        code(&none, "query.node", r#"{"document":"nope","node":"x"}"#),
        "invalid-params"
    );
    assert_eq!(code(&none, "query.tree_page", "{}"), "invalid-params");
    assert_eq!(code(&none, "query.tree_page", "[]"), "invalid-params");
    // 권한 없는 host.info는 항상 됩니다.
    assert!(none.call("host.info", &json("{}")).is_ok());

    let reader = fx.dispatcher(&[Permission::DocumentRead]);
    assert!(reader.call("document.read_snapshot", &json("{}")).is_ok());
    assert_eq!(
        code(
            &reader,
            "command.apply",
            r#"{"base_revision":"g0","document":"0195c530-3acd-7bc1-8414-c213f61cb002","commands":[]}"#
        ),
        "permission-denied"
    );
    let all = fx.dispatcher(&Permission::ALL);
    assert_eq!(
        code(&all, "network.fetch", r#"{"url":"http://example.com/"}"#),
        "invalid-params"
    );
    assert_eq!(
        code(&all, "file.read", r#"{"path":"../x"}"#),
        "invalid-params"
    );
    assert_eq!(
        code(&all, "file.read", r#"{"path":"missing.txt"}"#),
        "not-found"
    );
}

#[test]
fn revoked_project_trust_stops_project_methods() {
    let fx = Fixture::new("plugin-host-trust", 2);
    let dispatcher = fx.dispatcher(&Permission::ALL);
    assert!(dispatcher
        .call("document.read_snapshot", &json("{}"))
        .is_ok());
    stellar_ops::test_support::FileTrustStore::open(&fx.data)
        .unwrap()
        .revoke(
            &stellar_ops::test_support::ProjectIdentity::new(
                stellar_ops::test_support::project_id(),
                &fx.project,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        dispatcher
            .call("document.read_snapshot", &json("{}"))
            .unwrap_err()
            .code,
        "not-trusted"
    );
    assert!(dispatcher.call("host.info", &json("{}")).is_ok());
}
