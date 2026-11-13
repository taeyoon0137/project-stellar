//! XIX.1.2 검증: 브리지는 network port를 열거나 연결하지 않습니다.
//!
//! 브리지와 브리지가 path로 의존하는 내부 crate 전체(전이 포함)를 대상으로 다음을 확인합니다.
//!
//! - 소스에 TCP·UDP socket API와 `std::net` 사용이 없습니다. 로컬 IPC는
//!   `std::os::unix::net`(Unix domain socket)만 씁니다.
//! - manifest의 의존성은 모두 workspace 내부 `crates/` path 의존입니다. 외부(crates.io·git)
//!   crate를 통한 network 우회가 없습니다. 어떤 내부 crate에 의존할 수 있는지(허용 edge)는
//!   crate graph 정책(`tools/xtask/src/crate_graph.rs`)이 정하므로 이 검사는 목록을 두지 않습니다.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 검사하는 의존성 section. `[target.<cfg>.dependencies]` 형태도 같은 이름으로 끝납니다.
const DEPENDENCY_SECTIONS: [&str; 3] = ["dependencies", "build-dependencies", "dev-dependencies"];

/// manifest의 의존성 한 줄.
struct Dependency {
    name: String,
    value: String,
}

/// section header(`[a.b]`)가 의존성 section인지 확인합니다.
fn is_dependency_section(header: &str) -> bool {
    let name = header.trim_start_matches('[').trim_end_matches(']');
    DEPENDENCY_SECTIONS
        .iter()
        .any(|s| name == *s || name.ends_with(&format!(".{s}")))
}

/// manifest의 의존성 줄을 모읍니다. `[dependencies.foo]` 같은 table 형식은 path 검사를 우회할 수
/// 있으므로 허용하지 않습니다.
fn dependencies(manifest_path: &Path) -> Vec<Dependency> {
    let text = std::fs::read_to_string(manifest_path).unwrap();
    let mut deps = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            let name = trimmed.trim_start_matches('[').trim_end_matches(']');
            assert!(
                !DEPENDENCY_SECTIONS
                    .iter()
                    .any(|s| name.starts_with(&format!("{s}."))),
                "{}: table 형식 의존성 금지({trimmed})",
                manifest_path.display()
            );
            inside = is_dependency_section(trimmed);
            continue;
        }
        if !inside || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (name, value) = trimmed
            .split_once('=')
            .unwrap_or_else(|| panic!("{}: 의존성 형식: {trimmed}", manifest_path.display()));
        deps.push(Dependency {
            name: name.trim().to_string(),
            value: value.trim().to_string(),
        });
    }
    deps
}

/// `path = "..."` 값을 꺼냅니다.
fn path_of(value: &str) -> Option<&str> {
    let rest = &value[value.find("path = \"")? + "path = \"".len()..];
    Some(&rest[..rest.find('"')?])
}

/// 브리지 manifest에서 시작해 path 의존을 따라가며 crate 디렉터리를 모읍니다. 외부 의존이 있으면
/// 실패합니다.
fn dependency_closure() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = root.join("../..").canonicalize().unwrap();
    let crates_dir = workspace.join("crates");
    let mut seen = BTreeSet::new();
    let mut stack = vec![root.canonicalize().unwrap()];
    while let Some(dir) = stack.pop() {
        if !seen.insert(dir.clone()) {
            continue;
        }
        for dep in dependencies(&dir.join("Cargo.toml")) {
            assert!(
                dep.name.starts_with("stellar-"),
                "{}: 외부 crate 의존 금지: {}",
                dir.display(),
                dep.name
            );
            let path = path_of(&dep.value).unwrap_or_else(|| {
                panic!(
                    "{}: {}는 path 의존이어야 함: {}",
                    dir.display(),
                    dep.name,
                    dep.value
                )
            });
            let target = dir.join(path).canonicalize().unwrap();
            assert!(
                target.starts_with(&crates_dir),
                "{}: {}는 crates/ 아래여야 함",
                dir.display(),
                dep.name
            );
            stack.push(target);
        }
    }
    seen.into_iter().collect()
}

#[test]
fn bridge_and_internal_dependencies_use_no_network_sockets() {
    // 이 파일 자체가 걸리지 않도록 패턴을 조립합니다.
    let forbidden: Vec<String> = [
        ["Tcp", "Listener"],
        ["Tcp", "Stream"],
        ["Udp", "Socket"],
        ["ToSocket", "Addrs"],
        ["std::", "net::"],
        ["SocketAddr", ""],
    ]
    .iter()
    .map(|parts| parts.concat())
    .collect();
    let mut unix_ipc_uses = 0;
    for crate_dir in dependency_closure() {
        let mut stack = vec![crate_dir.join("src")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for pattern in &forbidden {
                    assert!(
                        !text.contains(pattern.as_str()),
                        "{}: network API 금지({pattern})",
                        path.display()
                    );
                }
                unix_ipc_uses += text.matches("std::os::unix::net::").count();
            }
        }
    }
    assert!(unix_ipc_uses > 0, "로컬 IPC는 Unix domain socket을 사용");
}

#[test]
fn manifests_have_only_internal_path_dependencies() {
    // 전이 closure 계산이 외부·비 path 의존을 만나면 실패합니다. 브리지 자신은 항상 포함됩니다.
    let closure = dependency_closure();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .unwrap();
    assert!(closure.contains(&root), "{closure:?}");
}

#[test]
fn dependency_parser_rejects_external_and_table_forms() {
    let tmp = std::env::temp_dir().join(format!("smcp-manifest-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let manifest = tmp.join("Cargo.toml");

    std::fs::write(
        &manifest,
        "[package]\nname = \"x\"\n[target.'cfg(unix)'.dependencies]\nserde = \"1\"\n\
         [dev-dependencies]\nstellar-a = { path = \"../a\" }\n",
    )
    .unwrap();
    let deps = dependencies(&manifest);
    let names: Vec<&str> = deps.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["serde", "stellar-a"], "target·dev section도 읽음");
    assert_eq!(path_of(&deps[0].value), None, "버전 의존은 path가 없음");
    assert_eq!(path_of(&deps[1].value), Some("../a"));

    std::fs::write(&manifest, "[dependencies.stellar-a]\npath = \"../a\"\n").unwrap();
    let table = std::panic::catch_unwind(|| dependencies(&manifest));
    assert!(table.is_err(), "table 형식 거부");
    let _ = std::fs::remove_dir_all(&tmp);
}
