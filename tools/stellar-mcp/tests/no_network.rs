//! XIX.1.2 검증: 브리지는 network port를 열거나 연결하지 않습니다.
//!
//! 소스 전체에 TCP·UDP socket API와 `std::net` 사용이 없음을 확인합니다. 로컬 IPC는
//! `std::os::unix::net`(Unix domain socket)만 씁니다.

use std::path::Path;

#[test]
fn sources_use_no_network_sockets() {
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
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
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
    assert!(unix_ipc_uses > 0, "로컬 IPC는 Unix domain socket을 사용");
}

#[test]
fn manifest_has_no_dependencies() {
    // 외부 network crate를 통한 우회도 없도록 의존성이 비어 있어야 합니다.
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("\n[")
        .next()
        .unwrap();
    let entries: Vec<&str> = deps
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert!(entries.is_empty(), "의존성 없음: {entries:?}");
}
