//! XIX.9.3 정상 stdio CLI 경로. 프로젝트·신뢰 저장소는 모두 임시 fixture입니다.
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use stellar_mcp::catalog::{paging, resources};
use stellar_mcp::json::{self, JsonLimits, JsonValue as J};
use stellar_mcp::server::MAX_MESSAGE_BYTES;
use stellar_mcp::test_support::{
    file_digest, large_document, project_id, random_valid_document, save_documents,
    scaffold_project, FileTrustStore, ProjectIdentity, TempDir, PROJECT_ID_TEXT,
};

const BIN: &str = env!("CARGO_BIN_EXE_stellar-mcp");
const STRUCTURE: &str = "stellar://project/structure";
const OTHER: &str = "0195c530-3acd-7bc1-8414-c213f61cb999";

struct Fixture {
    temp: TempDir,
    project: PathBuf,
    home: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new("mcp-read");
        let project = temp.path.join("project");
        let home = temp.path.join("home");
        let base = if cfg!(target_os = "macos") {
            home.join("Library/Application Support")
        } else if cfg!(windows) {
            home.join("AppData/Roaming")
        } else {
            home.join(".local/share")
        };
        let data = base.join("xyz.taeyoon.stellar");
        scaffold_project(&project);
        std::fs::create_dir_all(&home).unwrap();
        Self {
            temp,
            project,
            home,
            data,
        }
    }
    fn identity(&self) -> ProjectIdentity {
        ProjectIdentity::new(project_id(), &self.project).unwrap()
    }
    fn grant(&self) {
        FileTrustStore::open(&self.data)
            .unwrap()
            .grant(&self.identity())
            .unwrap();
    }
    fn command(&self, args: &[&str]) -> Command {
        self.command_with_binary(BIN, args)
    }
    fn command_with_binary(&self, binary: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Command {
        let mut command = Command::new(binary);
        command
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("APPDATA", self.home.join("AppData/Roaming"))
            .current_dir(&self.home);
        command
    }
    fn launch(&self) -> Client {
        Client::new(self.command(&["--headless", "--project", PROJECT_ID_TEXT]))
    }
}
struct Client {
    child: Child,
    input: Option<ChildStdin>,
    replies: Receiver<String>,
    id: i64,
}
impl Client {
    fn new(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (send, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if send.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            replies,
            id: 0,
        }
    }
    fn send(&mut self, value: J) {
        writeln!(self.input.as_mut().unwrap(), "{}", json::write(&value)).unwrap();
    }
    fn call(&mut self, method: &str, params: J) -> J {
        self.id += 1;
        self.send(J::object(vec![
            ("jsonrpc", J::string("2.0")),
            ("id", J::int(self.id)),
            ("method", J::string(method)),
            ("params", params),
        ]));
        let line = self
            .replies
            .recv_timeout(Duration::from_secs(15))
            .expect("stdio 응답 timeout");
        assert!(line.len() <= MAX_MESSAGE_BYTES, "stdout frame bound");
        let result = json::parse(
            &line,
            JsonLimits {
                max_bytes: MAX_MESSAGE_BYTES,
                max_depth: 64,
                max_nodes: 65_536,
            },
        )
        .unwrap();
        assert_eq!(result.get("id"), Some(&J::int(self.id)));
        result
    }
    fn initialize(&mut self) {
        let reply = self.call(
            "initialize",
            J::object(vec![
                ("protocolVersion", J::string("2025-11-25")),
                ("capabilities", empty()),
                (
                    "clientInfo",
                    J::object(vec![
                        ("name", J::string("fixture")),
                        ("version", J::string("1")),
                    ]),
                ),
            ]),
        );
        assert_eq!(
            reply.get("result").unwrap().get("capabilities"),
            Some(&J::object(vec![("resources", empty())]))
        );
        assert_error(&self.call("resources/list", empty()), -32002);
        self.send(J::object(vec![
            ("jsonrpc", J::string("2.0")),
            ("method", J::string("notifications/initialized")),
        ]));
    }
    fn read(&mut self, uri: &str) -> J {
        self.call("resources/read", J::object(vec![("uri", J::string(uri))]))
    }
    fn finish(&mut self) {
        self.input.take();
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(0));
                break;
            }
            assert!(Instant::now() < end, "EOF 종료 timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn empty() -> J {
    J::object(vec![])
}
fn array(value: &J) -> &[J] {
    match value {
        J::Array(items) => items,
        _ => panic!("array expected: {value:?}"),
    }
}
fn text(value: &J) -> &str {
    value.as_str().unwrap()
}
fn assert_error(reply: &J, code: i64) {
    assert!(reply.get("result").is_none());
    assert_eq!(
        reply
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(J::as_i64),
        Some(code),
        "{reply:?}"
    );
}
fn content(reply: &J, uri: &str) -> J {
    let contents = array(reply.get("result").unwrap().get("contents").unwrap());
    assert_eq!(contents.len(), 1);
    assert_eq!(contents[0].get("uri").map(text), Some(uri));
    assert_eq!(
        contents[0].get("mimeType").map(text),
        Some(resources::MIME_JSON)
    );
    let body = text(contents[0].get("text").unwrap());
    assert!(body.len() <= paging::MAX_PAGE_BYTES);
    json::parse(body, JsonLimits::default()).unwrap()
}

#[test]
fn approved_snapshot_lists_only_implemented_resources_and_pages_without_writes() {
    let fx = Fixture::new();
    let model = large_document(2101);
    save_documents(&fx.project, &[&model]);
    fx.grant();
    let before = file_digest(&fx.temp.path);
    let mut client = fx.launch();
    assert_error(&client.read(STRUCTURE), -32002);
    client.initialize();
    let list = client.call("resources/list", empty());
    let resources = array(list.get("result").unwrap().get("resources").unwrap());
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].get("uri").map(text), Some(STRUCTURE));
    assert_eq!(
        resources[0].get("name").map(text),
        Some("project-structure")
    );
    let templates = client.call("resources/templates/list", empty());
    let templates = array(
        templates
            .get("result")
            .unwrap()
            .get("resourceTemplates")
            .unwrap(),
    );
    assert_eq!(templates.len(), 1);
    assert_eq!(
        templates[0].get("uriTemplate").map(text),
        Some(resources::resource("document").unwrap().uri)
    );
    assert_eq!(
        client
            .call("tools/list", empty())
            .get("result")
            .unwrap()
            .get("tools"),
        Some(&J::Array(vec![]))
    );
    let structure = content(&client.read(STRUCTURE), STRUCTURE);
    let documents = array(structure.get("documents").unwrap());
    assert_eq!(documents.len(), 1);
    let document = text(documents[0].get("id").unwrap());
    assert_eq!(document, model.document_id().to_string());
    assert_eq!(
        structure.get("revision").unwrap().get("generation"),
        Some(&J::int(1))
    );
    assert!(!json::write(&structure).contains(fx.project.to_str().unwrap()));
    let mut uri = format!("stellar://documents/{document}?limit=2000");
    let mut seen = 0;
    loop {
        let body = content(&client.read(&uri), &uri);
        assert_eq!(body.get("revision"), structure.get("revision"));
        assert_eq!(body.get("offset").and_then(J::as_i64), Some(seen));
        let items = array(body.get("items").unwrap());
        assert!(items.len() <= paging::MAX_PAGE_ITEMS);
        seen += items.len() as i64;
        match body.get("next_cursor").unwrap().as_str() {
            Some(cursor) => {
                uri = format!("stellar://documents/{document}?cursor={cursor}&limit=2000")
            }
            None => break,
        }
    }
    assert_eq!(seen, 2101);
    client.finish();
    assert_eq!(
        file_digest(&fx.temp.path),
        before,
        "프로젝트·trust·HOME 무변경"
    );
}

#[test]
fn scope_revision_limits_and_write_requests_return_normal_errors() {
    let fx = Fixture::new();
    let model = large_document(5);
    save_documents(&fx.project, &[&model]);
    fx.grant();
    let before = file_digest(&fx.temp.path);
    let mut client = fx.launch();
    client.initialize();
    let document = model.document_id();
    let uri = format!("stellar://documents/{document}?limit=2");
    let first = content(&client.read(&uri), &uri);
    let cursor = text(first.get("next_cursor").unwrap());
    for uri in [
        format!("stellar://projects/{OTHER}/structure"),
        format!("stellar://documents/{OTHER}"),
        format!("stellar://documents/{document}?limit=2001"),
        format!("stellar://documents/{document}?limit=0"),
        format!("stellar://documents/{document}?project={OTHER}"),
        format!("stellar://documents/{document}?cursor=p1.2.{OTHER}.g0"),
        "stellar://project/structure?limit=1".into(),
        "stellar://components".into(),
    ] {
        assert_error(&client.read(&uri), -32602);
    }
    assert_error(
        &client.call(
            "resources/read",
            J::object(vec![
                ("uri", J::string(STRUCTURE)),
                ("project", J::string(OTHER)),
            ]),
        ),
        -32602,
    );
    for tool in [
        "document.validate",
        "document.apply_commands",
        "terminal.run",
        "git.commit",
        "git.push",
    ] {
        assert_error(
            &client.call(
                "tools/call",
                J::object(vec![("name", J::string(tool)), ("arguments", empty())]),
            ),
            -32601,
        );
    }
    assert_eq!(file_digest(&fx.temp.path), before);
    // 외부 정상 저장으로 generation을 변경하면 이전 page cursor는 거부합니다.
    save_documents(&fx.project, &[&large_document(6)]);
    let after_save = file_digest(&fx.temp.path);
    let stale = client.read(&format!("stellar://documents/{document}?cursor={cursor}"));
    assert_error(&stale, -32602);
    assert_eq!(
        stale.get("error").unwrap().get("message").map(text),
        Some(paging::PageError::StaleCursor.to_string().as_str())
    );
    let fresh = content(&client.read(STRUCTURE), STRUCTURE);
    assert_eq!(
        fresh.get("revision").unwrap().get("generation"),
        Some(&J::int(2))
    );
    assert_eq!(file_digest(&fx.temp.path), after_save);
    FileTrustStore::open(&fx.data)
        .unwrap()
        .revoke(&fx.identity())
        .unwrap();
    let after_revoke = file_digest(&fx.temp.path);
    assert_error(&client.read(STRUCTURE), -32000);
    assert_error(&client.call("resources/list", empty()), -32000);
    client.finish();
    assert_eq!(file_digest(&fx.temp.path), after_revoke);
}

#[test]
fn missing_ambiguous_or_changed_authority_never_starts_a_read_session() {
    let fx = Fixture::new();
    let denied = |args: &[&str], code| {
        let before = file_digest(&fx.temp.path);
        let output = fx.command(args).stdin(Stdio::null()).output().unwrap();
        assert_eq!(output.status.code(), Some(code), "{:?}", output.stderr);
        assert!(output.stdout.is_empty());
        assert_eq!(file_digest(&fx.temp.path), before);
    };
    denied(&["--headless", "--project", PROJECT_ID_TEXT], 77);
    denied(
        &["--headless", "--project", fx.project.to_str().unwrap()],
        64,
    );
    denied(
        &["--headless", "--project", PROJECT_ID_TEXT, "--attach"],
        64,
    );
    fx.grant();
    denied(&["--headless", "--project", OTHER], 77);
    let copy = fx.temp.path.join("copy");
    scaffold_project(&copy);
    let copy_id = ProjectIdentity::new(project_id(), &copy).unwrap();
    let mut store = FileTrustStore::open(&fx.data).unwrap();
    store.grant(&copy_id).unwrap();
    denied(&["--headless", "--project", PROJECT_ID_TEXT], 77);
    store.revoke(&copy_id).unwrap();
    let manifest = fx.project.join("stellar.yaml");
    let old = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, old.replace(PROJECT_ID_TEXT, OTHER)).unwrap();
    denied(&["--headless", "--project", PROJECT_ID_TEXT], 77);
    std::fs::write(&manifest, old).unwrap();
    std::fs::write(store.file_path(), "invalid store").unwrap();
    denied(&["--headless", "--project", PROJECT_ID_TEXT], 77);
}

#[test]
fn byte_budget_splits_real_snapshot_items_and_oversized_item_is_an_error() {
    let fx = Fixture::new();
    let mut model = large_document(8);
    let nodes: Vec<_> = model
        .to_records()
        .unwrap()
        .nodes
        .iter()
        .map(|n| n.id)
        .collect();
    for id in &nodes {
        model
            .set_display_name(*id, "문서 이름".repeat(12_000))
            .unwrap();
    }
    save_documents(&fx.project, &[&model]);
    fx.grant();
    let before = file_digest(&fx.temp.path);
    let mut client = fx.launch();
    client.initialize();
    let document = model.document_id();
    let mut uri = format!("stellar://documents/{document}?limit=8");
    let mut seen = 0;
    let mut pages = 0;
    loop {
        let body = content(&client.read(&uri), &uri);
        let items = array(body.get("items").unwrap());
        assert!(items.len() < 8);
        seen += items.len();
        pages += 1;
        match body.get("next_cursor").unwrap().as_str() {
            Some(cursor) => uri = format!("stellar://documents/{document}?cursor={cursor}&limit=8"),
            None => break,
        }
    }
    assert_eq!(seen, 8);
    assert!(pages > 1);
    assert_eq!(file_digest(&fx.temp.path), before);
    model
        .set_display_name(nodes[0], "이름".repeat(100_000))
        .unwrap();
    save_documents(&fx.project, &[&model]);
    let after_save = file_digest(&fx.temp.path);
    // 한 item이 상한보다 크면 잘린 내용을 성공으로 응답하지 않습니다.
    assert_error(
        &client.read(&format!("stellar://documents/{document}")),
        -32602,
    );
    client.finish();
    assert_eq!(file_digest(&fx.temp.path), after_save);
}

#[test]
fn document_switch_save_and_reapproval_use_the_current_source() {
    let fx = Fixture::new();
    let a = large_document(12);
    let b = random_valid_document(91);
    assert!(b.node_count() > 2);
    save_documents(&fx.project, &[&a, &b]);
    fx.grant();
    let mut client = fx.launch();
    client.initialize();
    let uri = |document| format!("stellar://documents/{document}?limit=2");
    let before = file_digest(&fx.temp.path);
    for model in [&a, &b, &a] {
        let uri = uri(model.document_id());
        let page = content(&client.read(&uri), &uri);
        assert_eq!(
            page.get("document").map(text),
            Some(model.document_id().to_string().as_str())
        );
        assert_eq!(
            page.get("total").and_then(J::as_i64),
            Some(model.node_count() as i64)
        );
    }
    assert_eq!(file_digest(&fx.temp.path), before);
    FileTrustStore::open(&fx.data)
        .unwrap()
        .revoke(&fx.identity())
        .unwrap();
    let revoked = file_digest(&fx.temp.path);
    assert_error(&client.read(&uri(a.document_id())), -32000);
    assert_eq!(file_digest(&fx.temp.path), revoked);
    save_documents(&fx.project, &[&large_document(13), &b]);
    fx.grant();
    let restored = file_digest(&fx.temp.path);
    let fresh = content(&client.read(&uri(a.document_id())), &uri(a.document_id()));
    assert_eq!(fresh.get("total"), Some(&J::int(13)));
    assert_eq!(
        fresh.get("revision").unwrap().get("generation"),
        Some(&J::int(2))
    );
    client.finish();
    assert_eq!(file_digest(&fx.temp.path), restored);
}

/// 동일 fixture·500개 page의 stdio 왕복(요청 직렬화부터 응답 JSON 파싱까지).
#[test]
#[ignore = "release 수동 RTT 측정"]
fn release_page_rtt() {
    if cfg!(debug_assertions) {
        panic!("--release로 실행하세요");
    }
    let fx = Fixture::new();
    let model = large_document(50_000);
    save_documents(&fx.project, &[&model]);
    fx.grant();
    let before = file_digest(&fx.temp.path);
    let base = format!("stellar://documents/{}", model.document_id());
    let mut runs = vec![(
        "after",
        std::ffi::OsString::from(BIN),
        [Vec::<f64>::new(), Vec::new(), Vec::new()],
    )];
    // 기준 실행 파일을 지정하면 같은 fixture에서 순서를 번갈아 측정합니다.
    if let Some(binary) = std::env::var_os("STELLAR_MCP_BENCH_BASELINE") {
        runs.push(("before", binary, [Vec::new(), Vec::new(), Vec::new()]));
    }
    for turn in 0..10 {
        for index in 0..runs.len() {
            let index = (turn + index) % runs.len();
            let (_, binary, [first, next, repeated]) = &mut runs[index];
            let mut client = Client::new(
                fx.command_with_binary(&binary, &["--headless", "--project", PROJECT_ID_TEXT]),
            );
            client.initialize();
            // 구조 조회 없이 연결의 첫 resource read를 측정합니다.
            let uri = format!("{base}?limit=500");
            let start = Instant::now();
            let reply = client.read(&uri);
            first.push(start.elapsed().as_secs_f64() * 1000.0);
            let body = content(&reply, &uri);
            assert_eq!(body.get("total"), Some(&J::int(50_000)));
            assert_eq!(array(body.get("items").unwrap()).len(), 500);
            let mut cursor = text(body.get("next_cursor").unwrap()).to_string();
            for offset in (500..=2500).step_by(500) {
                let uri = format!("{base}?cursor={cursor}&limit=500");
                let start = Instant::now();
                let reply = client.read(&uri);
                next.push(start.elapsed().as_secs_f64() * 1000.0);
                let body = content(&reply, &uri);
                assert_eq!(body.get("offset"), Some(&J::int(offset)));
                assert_eq!(array(body.get("items").unwrap()).len(), 500);
                cursor = text(body.get("next_cursor").unwrap()).to_string();
            }
            let start = Instant::now();
            let reply = client.read(&uri);
            repeated.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(content(&reply, &uri).get("offset"), Some(&J::int(0)));
            client.finish();
        }
    }
    for (label, _, [first, next, repeated]) in runs {
        for (name, mut samples) in [("first", first), ("next", next), ("repeat-first", repeated)] {
            println!("{label}/{name} samples_ms={samples:.3?}");
            samples.sort_by(f64::total_cmp);
            println!(
                "{label}/{name} n={} p50_ms={:.3} p95_ms={:.3}",
                samples.len(),
                samples[(samples.len() - 1) / 2],
                samples[(samples.len() * 95).div_ceil(100) - 1],
            );
        }
    }
    assert_eq!(file_digest(&fx.temp.path), before);
}
