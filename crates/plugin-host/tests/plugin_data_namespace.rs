//! 플러그인 data namespace (XXI.6.2): 플러그인 데이터는 `plugins/<plugin-id>/`에만 있고, 지워도 프로젝트 core가 열립니다.

mod support;

use std::collections::BTreeMap;

use stellar_ops::snapshot::open_snapshot;
use stellar_ops::test_support::file_digest;
use stellar_ops::validate::validate_project;
use stellar_plugin_host::json::JsonValue;
use stellar_plugin_host::permission::Permission;
use support::{json, Fixture, PLUGIN_ID};

fn core_files(digest: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    digest
        .iter()
        .filter(|(path, _)| !path.starts_with("plugins/"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[test]
fn plugin_data_lives_only_in_its_namespace_and_core_opens_without_it() {
    let fx = Fixture::new("plugin-data", 3);
    let before = open_snapshot(&fx.project).unwrap().revision();
    let core_before = core_files(&file_digest(&fx.project));
    let dispatcher = fx.dispatcher(&[Permission::File, Permission::DocumentRead]);

    for (path, text) in [("presence.json", "{}"), ("threads/1.json", "[]")] {
        dispatcher
            .call(
                "file.write",
                &json(&format!(r#"{{"path":"{path}","text":"{text}"}}"#)),
            )
            .unwrap();
    }
    let list = dispatcher.call("file.list", &json("{}")).unwrap();
    let paths: Vec<&str> = list
        .get("paths")
        .and_then(JsonValue::as_array)
        .unwrap()
        .iter()
        .filter_map(JsonValue::as_str)
        .collect();
    assert_eq!(paths, ["presence.json", "threads/1.json"]);

    // 모든 플러그인 파일은 plugins/<id>/ 아래에 있고 core 파일은 그대로입니다.
    let after = file_digest(&fx.project);
    let plugin_files: Vec<&String> = after.keys().filter(|p| p.starts_with("plugins/")).collect();
    assert_eq!(plugin_files.len(), 2);
    assert!(plugin_files
        .iter()
        .all(|p| p.starts_with(&format!("plugins/{PLUGIN_ID}/"))));
    assert_eq!(core_files(&after), core_before, "core metadata 0");
    assert_eq!(open_snapshot(&fx.project).unwrap().revision(), before);
    assert!(validate_project(&fx.project).unwrap().is_valid());

    // 플러그인 제거(데이터 삭제) 뒤에도 프로젝트 core가 그대로 열립니다.
    std::fs::remove_dir_all(fx.project.join("plugins")).unwrap();
    let reopened = open_snapshot(&fx.project).unwrap();
    assert_eq!(reopened.revision(), before);
    assert!(validate_project(&fx.project).unwrap().is_valid());
    assert!(dispatcher
        .call("document.read_snapshot", &json("{}"))
        .is_ok());
    let empty = dispatcher.call("file.list", &json("{}")).unwrap();
    assert_eq!(write_paths(&empty), "[]");
}

fn write_paths(value: &JsonValue) -> String {
    stellar_plugin_host::json::write(value.get("paths").unwrap())
}

#[test]
fn plugin_data_paths_do_not_escape_or_follow_symlinks() {
    let fx = Fixture::new("plugin-data-escape", 1);
    let dispatcher = fx.dispatcher(&[Permission::File]);
    let code =
        |method: &str, params: &str| dispatcher.call(method, &json(params)).unwrap_err().code;
    for path in ["../../stellar.yaml", "/etc/hosts", "a/../../b", "a\\b"] {
        let params = format!(
            r#"{{"path":{},"text":"x"}}"#,
            stellar_plugin_host::json::write(&JsonValue::string(path))
        );
        assert_eq!(code("file.write", &params), "invalid-params", "{path}");
    }
    // namespace 안 symlink는 따라가지 않습니다.
    dispatcher
        .call("file.write", &json(r#"{"path":"ok.txt","text":"x"}"#))
        .unwrap();
    let dir = fx.project.join("plugins").join(PLUGIN_ID);
    std::os::unix::fs::symlink(fx.project.join("stellar.yaml"), dir.join("link.txt")).unwrap();
    assert_eq!(
        code("file.read", r#"{"path":"link.txt"}"#),
        "permission-denied"
    );
    assert_eq!(
        code("file.write", r#"{"path":"link.txt","text":"overwrite"}"#),
        "permission-denied"
    );
    std::os::unix::fs::symlink(&fx.project, dir.join("up")).unwrap();
    assert_eq!(
        code("file.write", r#"{"path":"up/stellar.yaml","text":"x"}"#),
        "permission-denied"
    );
    let manifest = std::fs::read_to_string(fx.project.join("stellar.yaml")).unwrap();
    assert!(!manifest.contains("overwrite"));
    // 목록은 symlink를 건너뜁니다.
    let list = dispatcher.call("file.list", &json("{}")).unwrap();
    assert_eq!(write_paths(&list), r#"["ok.txt"]"#);
}
