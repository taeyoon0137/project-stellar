//! `plugin.yaml` schema fixture (XXI.1.1).
//!
//! 저장소 fixture package의 manifest가 Rust 해석기를 통과하고, 생성 JSON Schema가 같은 필드·권한 표를 담는지 봅니다.

use std::path::Path;

use stellar_plugin_host::json::{parse, JsonLimits, JsonValue};
use stellar_plugin_host::manifest::{manifest_json_schema, PluginManifest, TOP_LEVEL_KEYS};
use stellar_plugin_host::permission::Permission;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .join("plugin.yaml");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn fixture_manifest_parses() {
    let manifest = PluginManifest::parse(&fixture("hello.stellar-plugin")).unwrap();
    assert_eq!(manifest.id.as_str(), "xyz.taeyoon.fixture.hello");
    assert_eq!(manifest.version.to_string(), "1.0.0");
    assert_eq!(manifest.entry.as_str(), "ui/index.html");
    assert_eq!(manifest.host_api, 1);
    assert_eq!(
        manifest.permissions,
        [Permission::DocumentRead, Permission::GitRead]
    );
    assert_eq!(manifest.wasm[0].path.as_str(), "wasm/add.wasm");
}

#[test]
fn schema_properties_match_parser_keys() {
    let schema = parse(&manifest_json_schema(), JsonLimits::default()).unwrap();
    let properties = schema
        .get("properties")
        .and_then(JsonValue::as_object)
        .unwrap();
    let names: Vec<&str> = properties.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, TOP_LEVEL_KEYS);
    let enum_names: Vec<&str> = schema
        .get("properties")
        .and_then(|p| p.get("permissions"))
        .and_then(|p| p.get("items"))
        .and_then(|i| i.get("enum"))
        .and_then(JsonValue::as_array)
        .unwrap()
        .iter()
        .filter_map(JsonValue::as_str)
        .collect();
    let expected: Vec<&str> = Permission::ALL.iter().map(|p| p.name()).collect();
    assert_eq!(enum_names, expected);
    assert_eq!(
        schema
            .get("additionalProperties")
            .and_then(JsonValue::as_bool),
        Some(false)
    );
}
