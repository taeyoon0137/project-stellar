//! malicious package fixture 거부 (XXI.1.2).
//!
//! path traversal, symlink escape, duplicate id, unsupported version을 적재 전에 거부하는지 봅니다. symlink는 Git
//! 원본에 두기 어려우므로 임시 디렉터리에 실행 중에 만듭니다.

use std::fs;
use std::path::{Path, PathBuf};

use stellar_ops::test_support::TempDir;
use stellar_plugin_host::package::{scan_plugins, PluginPackage};
use stellar_plugin_host::HOST_API_VERSION;

const MANIFEST: &str = "\
manifest_version: 1
id: com.example.good
version: 1.0.0
name: Good
entry: ui/index.html
host_api: 1
permissions: []
";

fn write_package(parent: &Path, name: &str, manifest: &str) -> PathBuf {
    let root = parent.join(name);
    fs::create_dir_all(root.join("ui")).unwrap();
    fs::write(root.join("plugin.yaml"), manifest).unwrap();
    fs::write(root.join("ui/index.html"), "<!doctype html>").unwrap();
    root
}

fn code(root: &Path) -> &'static str {
    PluginPackage::open(root, HOST_API_VERSION)
        .unwrap_err()
        .code()
}

#[test]
fn valid_package_opens_with_stable_digest() {
    let dir = TempDir::new("plugin-valid");
    let root = write_package(&dir.path, "good.stellar-plugin", MANIFEST);
    let first = PluginPackage::open(&root, HOST_API_VERSION).unwrap();
    let again = PluginPackage::open(&root, HOST_API_VERSION).unwrap();
    assert_eq!(first.digest(), again.digest());
    assert_eq!(first.files().len(), 2);
    fs::write(root.join("ui/index.html"), "<!doctype html><p>").unwrap();
    let changed = PluginPackage::open(&root, HOST_API_VERSION).unwrap();
    assert_ne!(first.digest(), changed.digest());
    assert!(changed.resolve("ui/index.html").is_ok());
    assert_eq!(
        changed
            .resolve("../good.stellar-plugin/plugin.yaml")
            .unwrap_err()
            .code(),
        "package-invalid-name"
    );
    assert_eq!(
        changed.resolve("ui/missing.html").unwrap_err().code(),
        "package-missing-file"
    );
}

#[test]
fn repository_fixture_opens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.stellar-plugin");
    let package = PluginPackage::open(&root, HOST_API_VERSION).unwrap();
    assert_eq!(package.manifest().id.as_str(), "xyz.taeyoon.fixture.hello");
}

#[test]
fn rejects_path_traversal_in_manifest_fields() {
    let dir = TempDir::new("plugin-traversal");
    for (i, entry) in [
        "../outside.html",
        "/etc/index.html",
        "ui/../../x.html",
        "ui\\..\\x.html",
    ]
    .iter()
    .enumerate()
    {
        let root = write_package(
            &dir.path,
            &format!("t{i}.stellar-plugin"),
            &MANIFEST.replace("ui/index.html", entry),
        );
        let code = code(&root);
        assert!(code.starts_with("path-"), "{entry}: {code}");
    }
    let root = write_package(
        &dir.path,
        "w.stellar-plugin",
        &format!("{MANIFEST}wasm:\n  - path: ../../evil.wasm\n"),
    );
    assert_eq!(code(&root), "path-traversal");
}

#[test]
fn rejects_symlinks_inside_and_escaping() {
    let dir = TempDir::new("plugin-symlink");
    let outside = dir.path.join("secret.html");
    fs::write(&outside, "secret").unwrap();

    let escape = write_package(&dir.path, "escape.stellar-plugin", MANIFEST);
    fs::remove_file(escape.join("ui/index.html")).unwrap();
    std::os::unix::fs::symlink(&outside, escape.join("ui/index.html")).unwrap();
    assert_eq!(code(&escape), "symlink-escape");

    let dir_escape = write_package(&dir.path, "direscape.stellar-plugin", MANIFEST);
    std::os::unix::fs::symlink(&dir.path, dir_escape.join("up")).unwrap();
    assert_eq!(code(&dir_escape), "symlink-escape");

    let broken = write_package(&dir.path, "broken.stellar-plugin", MANIFEST);
    std::os::unix::fs::symlink(dir.path.join("nothing"), broken.join("ui/gone")).unwrap();
    assert_eq!(code(&broken), "symlink-escape");

    let inside = write_package(&dir.path, "inside.stellar-plugin", MANIFEST);
    std::os::unix::fs::symlink(inside.join("ui/index.html"), inside.join("ui/alias.html")).unwrap();
    assert_eq!(code(&inside), "package-symlink");

    let real = write_package(&dir.path, "real.stellar-plugin", MANIFEST);
    let linked_root = dir.path.join("linked.stellar-plugin");
    std::os::unix::fs::symlink(&real, &linked_root).unwrap();
    assert_eq!(code(&linked_root), "package-not-directory");
}

#[test]
fn rejects_unsupported_versions_and_bad_files() {
    let dir = TempDir::new("plugin-version");
    let host = write_package(
        &dir.path,
        "host.stellar-plugin",
        &MANIFEST.replace("host_api: 1", "host_api: 2"),
    );
    assert_eq!(code(&host), "unsupported-host-api");
    let format = write_package(
        &dir.path,
        "format.stellar-plugin",
        &MANIFEST.replace("manifest_version: 1", "manifest_version: 9"),
    );
    assert_eq!(code(&format), "unsupported-manifest-version");

    let missing = write_package(
        &dir.path,
        "missing.stellar-plugin",
        &MANIFEST.replace("ui/index.html", "ui/other.html"),
    );
    assert_eq!(code(&missing), "package-missing-file");

    let wasm = write_package(
        &dir.path,
        "wasm.stellar-plugin",
        &format!("{MANIFEST}wasm:\n  - path: calc.wasm\n"),
    );
    fs::write(wasm.join("calc.wasm"), b"not wasm").unwrap();
    assert_eq!(code(&wasm), "package-invalid-wasm");

    let no_manifest = dir.path.join("empty.stellar-plugin");
    fs::create_dir_all(&no_manifest).unwrap();
    assert_eq!(code(&no_manifest), "manifest-missing");

    let bad_name = write_package(&dir.path, "plain-dir", MANIFEST);
    assert_eq!(code(&bad_name), "package-bad-name");

    let odd = write_package(&dir.path, "odd.stellar-plugin", MANIFEST);
    fs::write(odd.join("ui/a:b.html"), "x").unwrap();
    assert_eq!(code(&odd), "package-invalid-name");
}

#[test]
fn duplicate_ids_reject_every_copy() {
    let dir = TempDir::new("plugin-duplicate");
    write_package(&dir.path, "a.stellar-plugin", MANIFEST);
    write_package(&dir.path, "b.stellar-plugin", MANIFEST);
    write_package(
        &dir.path,
        "c.stellar-plugin",
        &MANIFEST.replace("com.example.good", "com.example.other"),
    );
    fs::create_dir_all(dir.path.join("ignored-dir")).unwrap();
    let entries = scan_plugins(&dir.path, HOST_API_VERSION).unwrap();
    let summary: Vec<(&str, Result<&str, &str>)> = entries
        .iter()
        .map(|e| {
            (
                e.name.as_str(),
                e.result
                    .as_ref()
                    .map(|p| p.manifest().id.as_str())
                    .map_err(|e| e.code()),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("a.stellar-plugin", Err("duplicate-id")),
            ("b.stellar-plugin", Err("duplicate-id")),
            ("c.stellar-plugin", Ok("com.example.other")),
        ]
    );
}
