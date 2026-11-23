//! XIX.9.3 검증: headless 읽기 backend의 CLI 동등성 tool.
//!
//! - untrusted fixture 전 tool 거부, 다른 경로 요청 거부, headless tools/list의 쓰기 tool 0,
//! - 실행 전후 프로젝트 파일(작업 트리·`.git`·신뢰 저장소·HOME) hash 동일.

mod support;

use support::*;

/// headless가 광고하는 tool 이름.
fn listed(client: &mut Client) -> Vec<String> {
    let reply = client.call("tools/list", empty());
    array(reply.get("result").unwrap().get("tools").unwrap())
        .iter()
        .map(|t| text(t.get("name").unwrap()).to_string())
        .collect()
}

/// 각 tool의 정상 인자(문서·node·View는 fixture에서 읽음).
fn valid_arguments(name: &str, ids: &Ids) -> J {
    let s = |v: &str| J::string(v);
    match name {
        "document.semantic_diff" | "git.status" => J::object(vec![("limit", J::int(5))]),
        "document.validate" => J::object(vec![("document", s(&ids.document))]),
        "document.propose_commands" => J::object(vec![
            ("base_revision", s(&ids.revision)),
            ("document", s(&ids.document)),
            (
                "commands",
                J::Array(vec![J::object(vec![
                    ("kind", s("rename")),
                    ("node", s(&ids.root)),
                    ("name", s("Renamed")),
                ])]),
            ),
        ]),
        "document.source_span" => J::object(vec![("node", s(&ids.root))]),
        "document.export_web_frame" => {
            J::object(vec![("document", s(&ids.document)), ("view", s(&ids.view))])
        }
        "document.copy_css" => {
            J::object(vec![("document", s(&ids.document)), ("node", s(&ids.root))])
        }
        "document.propose_css_paste" => J::object(vec![
            ("base_revision", s(&ids.revision)),
            ("document", s(&ids.document)),
            ("nodes", J::Array(vec![s(&ids.root)])),
            ("declarations", s("width: 320px; height: 200px;")),
        ]),
        "git.semantic_conflicts" => empty(),
        other => panic!("인자를 모르는 tool: {other}"),
    }
}

struct Ids {
    revision: String,
    document: String,
    view: String,
    root: String,
}

fn ids(client: &mut Client) -> Ids {
    let structure = resource_body(&client.read(STRUCTURE));
    let document = &array(structure.get("documents").unwrap())[0];
    let view = &array(
        array(document.get("pages").unwrap())[0]
            .get("views")
            .unwrap(),
    )[0];
    Ids {
        revision: text(structure.get("revision").unwrap().get("token").unwrap()).to_string(),
        document: text(document.get("id").unwrap()).to_string(),
        view: text(view.get("id").unwrap()).to_string(),
        root: text(view.get("root_frame").unwrap()).to_string(),
    }
}

#[test]
fn every_listed_tool_runs_read_only_through_ops() {
    let fx = Fixture::new("mcp-tools");
    committed_project(&fx, 6);
    fx.grant();
    let before = file_digest(&fx.temp.path);
    let mut client = fx.launch();
    client.initialize();
    let ids = ids(&mut client);
    let names = listed(&mut client);
    assert_eq!(names.len(), 9);
    for name in &names {
        let reply = client.tool(name, valid_arguments(name, &ids));
        if name == "document.source_span" {
            // 이 fixture에는 다중 파일 텍스트 원본이 없습니다.
            assert_eq!(failure_kind(&reply), "unavailable", "{reply:?}");
            continue;
        }
        let body = tool_body(&reply);
        match name.as_str() {
            "document.propose_commands" => {
                assert_eq!(body.get("mode").map(text), Some("dry-run"));
                assert_eq!(body.get("command_count"), Some(&J::int(1)));
                let changes = body.get("changes").unwrap();
                assert_eq!(changes.get("count"), Some(&J::int(1)));
                let entry = &array(changes.get("entries").unwrap())[0];
                assert_eq!(entry.get("object").map(text), Some(ids.root.as_str()));
                assert_eq!(
                    entry.get("new").unwrap().get("value").map(text),
                    Some("Renamed")
                );
            }
            "document.copy_css" => {
                assert_eq!(body.get("node").map(text), Some(ids.root.as_str()));
                assert!(body.get("text").is_some());
            }
            "document.propose_css_paste" => {
                assert_eq!(body.get("mode").map(text), Some("dry-run"));
                let applied: Vec<&str> = array(body.get("applied").unwrap())
                    .iter()
                    .map(|a| text(a.get("name").unwrap()))
                    .collect();
                assert_eq!(applied, ["width", "height"]);
            }
            "document.export_web_frame" => {
                assert_eq!(body.get("view").map(text), Some(ids.view.as_str()));
                assert!(body.get("exported").is_some());
            }
            "git.status" => {
                assert_eq!(
                    body.get("head").unwrap().get("branch").map(text),
                    Some("main")
                );
                assert_eq!(body.get("counts").unwrap().get("staged"), Some(&J::int(0)));
                // 깨끗한 작업 트리: raw 항목 0, 잘림 없음, semantic 상태를 읽었으므로 진단 없음.
                assert_eq!(body.get("raw_total"), Some(&J::int(0)));
                assert_eq!(body.get("raw_truncated"), Some(&J::Bool(false)));
                assert!(body.get("semantic").is_some_and(|s| *s != J::Null));
                assert_eq!(body.get("diagnostics"), Some(&J::Array(Vec::new())));
            }
            "git.semantic_conflicts" => {
                assert_eq!(body.get("operation").map(text), Some("none"));
                assert_eq!(body.get("units"), Some(&J::Array(Vec::new())));
            }
            _ => {}
        }
    }
    // ADR-0089 결정 8: 기존 export tool의 대상 확장(`stellar export-styles --target react-native`와 같은 ops 함수).
    let mut arguments = match valid_arguments("document.export_web_frame", &ids) {
        J::Object(fields) => fields,
        _ => unreachable!(),
    };
    arguments.push(("target".to_string(), J::string("react-native")));
    let body = tool_body(&client.tool("document.export_web_frame", J::Object(arguments)));
    assert!(body.get("format").is_some());
    assert_eq!(body.get("style_defaults").map(text), Some("css"));
    assert!(body.get("styles").is_some_and(|s| !array(s).is_empty()));
    assert!(body.get("react_native").is_some());
    let mut arguments = match valid_arguments("document.export_web_frame", &ids) {
        J::Object(fields) => fields,
        _ => unreachable!(),
    };
    arguments.push(("target".to_string(), J::string("android")));
    let reply = client.tool("document.export_web_frame", J::Object(arguments));
    assert_eq!(failure_kind(&reply), "invalid-arguments");
    for uri in [
        "stellar://tokens",
        "stellar://target-profiles",
        "stellar://git/status",
    ] {
        let body = resource_body(&client.read(uri));
        assert!(body.get("revision").is_some(), "{uri}");
    }
    let git = resource_body(&client.read("stellar://git/status"));
    assert_eq!(git.get("branch").map(text), Some("main"));
    assert_eq!(git.get("clean"), Some(&J::Bool(true)));
    // staged·unstaged는 semantic 변경 수입니다(XIX.2.1 정의).
    assert_eq!(git.get("staged"), Some(&J::int(0)));
    assert_eq!(git.get("unstaged"), Some(&J::int(0)));
    client.finish();
    // 검증 기준: 실행 전후 프로젝트 파일 hash 동일(dry-run·Git 읽기 포함).
    assert_eq!(file_digest(&fx.temp.path), before);
}

#[test]
fn write_tools_are_unknown_and_path_arguments_are_rejected() {
    let fx = Fixture::new("mcp-tools-scope");
    committed_project(&fx, 3);
    fx.grant();
    let before = file_digest(&fx.temp.path);
    let mut client = fx.launch();
    client.initialize();
    let ids = ids(&mut client);
    // 쓰기 tool은 headless 목록에 없고, 이름으로 불러도 모르는 tool입니다.
    for write in [
        "document.apply_commands",
        "document.paste_css",
        "document.render_preview",
        "git.stage",
        "git.unstage",
        "git.commit",
        "git.push",
        "terminal.run",
    ] {
        let reply = client.tool(write, empty());
        assert_eq!(failure_kind(&reply), "unknown-tool", "{write}");
    }
    // 검증 기준: 다른 경로 요청 거부. 어느 tool에도 경로·프로젝트를 고르는 인자가 없습니다.
    let other = fx.temp.path.join("other");
    let other = other.to_str().unwrap();
    for name in listed(&mut client) {
        for key in ["project", "path", "root", "design", "base_path"] {
            let mut arguments = match valid_arguments(&name, &ids) {
                J::Object(fields) => fields,
                _ => unreachable!(),
            };
            arguments.push((key.to_string(), J::string(other)));
            let reply = client.tool(&name, J::Object(arguments));
            assert_eq!(failure_kind(&reply), "invalid-arguments", "{name} {key}");
        }
    }
    // 다른 프로젝트 문서 ID는 승인 범위 밖입니다.
    let reply = client.tool(
        "document.copy_css",
        J::object(vec![
            (
                "document",
                J::string("0195c530-3acd-7bc1-8414-c213f61cb999"),
            ),
            ("node", J::string(&ids.root)),
        ]),
    );
    assert_eq!(failure_kind(&reply), "out-of-scope");
    client.finish();
    assert_eq!(file_digest(&fx.temp.path), before);
}

#[test]
fn untrusted_project_rejects_every_tool_and_never_starts() {
    let fx = Fixture::new("mcp-tools-untrusted");
    committed_project(&fx, 3);
    // 신뢰 등록이 없으면 세션을 시작하지 않습니다(77).
    let status = fx
        .command(
            BIN,
            &["--headless", "--project", fx.project.to_str().unwrap()],
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(77));
    // 세션 중 신뢰를 철회하면 모든 tool과 resource를 거부합니다.
    fx.grant();
    let mut client = fx.launch();
    client.initialize();
    let ids = ids(&mut client);
    let names = listed(&mut client);
    fx.revoke();
    let before = file_digest(&fx.temp.path);
    for name in &names {
        let reply = client.tool(name, valid_arguments(name, &ids));
        assert_eq!(failure_kind(&reply), "not-trusted", "{name}: {reply:?}");
    }
    for uri in [STRUCTURE, "stellar://tokens", "stellar://git/status"] {
        let reply = client.read(uri);
        assert!(reply.get("error").is_some(), "{uri}");
    }
    assert!(client.call("tools/list", empty()).get("error").is_some());
    client.finish();
    assert_eq!(file_digest(&fx.temp.path), before);
}

#[test]
fn stale_base_revision_is_rejected_before_dry_run() {
    let fx = Fixture::new("mcp-tools-stale");
    committed_project(&fx, 3);
    fx.grant();
    let mut client = fx.launch();
    client.initialize();
    let mut ids = ids(&mut client);
    ids.revision = "g0".into();
    for name in ["document.propose_commands", "document.propose_css_paste"] {
        let reply = client.tool(name, valid_arguments(name, &ids));
        assert_eq!(failure_kind(&reply), "stale", "{name}");
    }
    // closed enum 밖 command 종류는 인자 오류입니다(XIX.5.1).
    let ids = support_ids_with_kind(&mut client);
    assert_eq!(failure_kind(&ids), "invalid-arguments");
    client.finish();
}

fn support_ids_with_kind(client: &mut Client) -> J {
    let ids = ids(client);
    client.tool(
        "document.propose_commands",
        J::object(vec![
            ("base_revision", J::string(&ids.revision)),
            ("document", J::string(&ids.document)),
            (
                "commands",
                J::Array(vec![J::object(vec![("kind", J::string("raw-patch"))])]),
            ),
        ]),
    )
}

#[test]
fn git_status_resource_counts_semantic_changes_like_the_tool() {
    // XIX.2.1 정의: `stellar://git/status`의 staged·unstaged는 semantic 변경 수입니다.
    // 같은 저장 파일 안의 이름 변경 세 개는 경로 하나지만 semantic 변경 세 개입니다.
    let fx = Fixture::new("mcp-tools-git-summary");
    let mut document = committed_project(&fx, 3);
    for (index, node) in document.to_records().unwrap().nodes.iter().enumerate() {
        document
            .set_display_name(node.id, format!("Renamed {index}"))
            .unwrap();
    }
    save_documents(&fx.project, &[&document]);
    fx.grant();
    let mut client = fx.launch();
    client.initialize();
    let tool = tool_body(&client.tool("git.status", J::object(vec![("limit", J::int(5))])));
    let semantic = tool.get("semantic").unwrap();
    let unstaged = semantic.get("unstaged_total").unwrap().clone();
    assert_ne!(unstaged, J::int(0), "{tool:?}");
    let paths = tool.get("counts").unwrap().get("unstaged").unwrap().clone();
    let summary = resource_body(&client.read("stellar://git/status"));
    assert_eq!(summary.get("unstaged"), Some(&unstaged), "{summary:?}");
    assert_eq!(
        summary.get("staged"),
        semantic.get("staged_total"),
        "{summary:?}"
    );
    assert_eq!(summary.get("clean"), Some(&J::Bool(false)));
    // 경로 수와 semantic 수가 다른 fixture인지 기록합니다(같으면 이 test가 두 정의를 구분하지 못함).
    assert_ne!(Some(&paths), summary.get("unstaged"), "{tool:?}");
    client.finish();
}
