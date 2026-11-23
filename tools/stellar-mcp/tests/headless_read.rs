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
    corrupt_view_tree_chunks, document_with_unresolved_clips, file_digest, large_document,
    project_id, random_valid_document, save_documents, scaffold_project, FileTrustStore,
    ProjectIdentity, TempDir, PROJECT_ID_TEXT,
};

const BIN: &str = env!("CARGO_BIN_EXE_stellar-mcp");
const STRUCTURE: &str = "stellar://project/structure";
const COMPONENTS: &str = "stellar://components";
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
    fn git(&self, args: &[&str]) -> String {
        let output = self
            .command_with_binary("git", &[])
            .current_dir(&self.project)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
    fn root(&self) -> &str {
        self.project.to_str().unwrap()
    }
    fn launch(&self) -> Client {
        self.launch_at(&self.project)
    }
    fn launch_at(&self, root: &std::path::Path) -> Client {
        Client::new(self.command(&["--headless", "--project", root.to_str().unwrap()]))
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
        self.initialize_with_tools(true);
    }
    fn initialize_with_tools(&mut self, tools: bool) {
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
        let capabilities = reply.get("result").unwrap().get("capabilities").unwrap();
        assert_eq!(capabilities.get("resources"), Some(&empty()));
        if tools {
            assert_eq!(capabilities.get("tools"), Some(&empty()));
        }
        assert_error(&self.call("resources/list", empty()), -32002);
        self.send(J::object(vec![
            ("jsonrpc", J::string("2.0")),
            ("method", J::string("notifications/initialized")),
        ]));
    }
    fn validate(&mut self, arguments: J) -> J {
        self.call(
            "tools/call",
            J::object(vec![
                ("name", J::string("document.validate")),
                ("arguments", arguments),
            ]),
        )
    }
    fn diff(&mut self, arguments: J) -> J {
        self.call(
            "tools/call",
            J::object(vec![
                ("name", J::string("document.semantic_diff")),
                ("arguments", arguments),
            ]),
        )
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
    let result = reply
        .get("result")
        .unwrap_or_else(|| panic!("{uri}: {reply:?}"));
    let contents = array(result.get("contents").unwrap());
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
    assert_eq!(resources.len(), 2);
    assert_eq!(resources[1].get("uri").map(text), Some(COMPONENTS));
    assert_eq!(resources[1].get("name").map(text), Some("components"));
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
    assert_eq!(templates.len(), 3);
    assert_eq!(
        templates[2].get("uriTemplate").map(text),
        Some("stellar://components/{component}{?cursor,limit}")
    );
    assert_eq!(
        templates[1].get("uriTemplate").map(text),
        Some("stellar://components{?cursor,limit}")
    );
    assert_eq!(
        templates[0].get("uriTemplate").map(text),
        Some(resources::resource("document").unwrap().uri)
    );
    let tools = client.call("tools/list", empty());
    let tools = array(tools.get("result").unwrap().get("tools").unwrap());
    assert_eq!(tools.len(), 2);
    assert_eq!(
        tools[0].get("name").map(text),
        Some("document.semantic_diff")
    );
    assert_eq!(tools[1].get("name").map(text), Some("document.validate"));
    assert_eq!(
        tools[0].get("annotations").unwrap().get("readOnlyHint"),
        Some(&J::Bool(true))
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
        "stellar://components?path=other".into(),
        "stellar://components/other".into(),
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
        "document.render_preview",
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
            -32602,
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

fn tool_page(reply: &J) -> J {
    let parts = array(
        reply
            .get("result")
            .expect("tool result")
            .get("content")
            .unwrap(),
    );
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].get("type").map(text), Some("text"));
    let text = text(parts[0].get("text").unwrap());
    assert!(text.len() <= paging::MAX_PAGE_BYTES);
    json::parse(text, JsonLimits::default()).unwrap()
}

#[test]
fn component_pages_cover_large_libraries_and_reject_a_changed_source_cursor() {
    let fx = Fixture::new();
    save_documents(&fx.project, &[&large_document(2)]);
    let first_file = stellar_mcp::test_support::component_diff_fixture(
        &fx.project.join("design/components"),
        2200,
    );
    fx.grant();
    let mut client = fx.launch();
    client.initialize();
    let mut uri = COMPONENTS.to_string();
    let mut seen = std::collections::BTreeSet::new();
    let mut first_cursor = None;
    loop {
        let body = content(&client.read(&uri), &uri);
        assert_eq!(body.get("total"), Some(&J::int(2200)));
        if seen.is_empty() {
            // 고정 URI도 다른 page resource와 같은 기본 개수를 씁니다.
            assert_eq!(
                body.get("limit").and_then(J::as_i64),
                Some(paging::DEFAULT_PAGE_ITEMS as i64)
            );
            assert_eq!(
                array(body.get("entries").unwrap()).len(),
                paging::DEFAULT_PAGE_ITEMS
            );
        }
        assert_eq!(
            body.get("offset").and_then(J::as_i64),
            Some(seen.len() as i64)
        );
        for entry in array(body.get("entries").unwrap()) {
            assert!(seen.insert(text(entry.get("id").unwrap()).to_owned()));
        }
        let Some(cursor) = body.get("next_cursor").and_then(J::as_str) else {
            break;
        };
        first_cursor.get_or_insert_with(|| cursor.to_owned());
        uri = format!("{COMPONENTS}?cursor={cursor}&limit=1600");
    }
    assert_eq!(seen.len(), 2200);
    let mut bytes = std::fs::read(&first_file).unwrap();
    bytes.extend_from_slice(b"\n# edited externally\n");
    std::fs::write(first_file, bytes).unwrap();
    let stale = format!("{COMPONENTS}?cursor={}", first_cursor.unwrap());
    assert_error(&client.read(&stale), -32602);
    let uri = format!("{COMPONENTS}?limit=1");
    let first = content(&client.read(&uri), &uri);
    assert_eq!(array(first.get("entries").unwrap()).len(), 1);
}

#[test]
fn component_resource_tracks_saved_yaml_across_checkouts_in_one_session() {
    use std::fs;
    const ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";
    const ROOT: &str = "0195c530-3acd-7bc1-8414-c213f61cb966";
    const CHILD: &str = "0195c530-3acd-7bc1-8414-c213f61cb970";
    const FILE: &str = "Card-8c5619572809.stellar.yaml";
    let fx = Fixture::new();
    let document = large_document(2);
    save_documents(&fx.project, &[&document]);
    fx.grant();
    fx.git(&["init", "-q", "-b", "main"]);
    let mut client = fx.launch();
    client.initialize();
    let revision = content(&client.read(STRUCTURE), STRUCTURE)
        .get("revision")
        .unwrap()
        .clone();
    let dir = fx.project.join("design/components");
    let read = |client: &mut Client| {
        let before = file_digest(&fx.temp.path);
        let body = content(&client.read(COMPONENTS), COMPONENTS);
        assert_eq!(file_digest(&fx.temp.path), before);
        assert!(!json::write(&body).contains(fx.project.to_str().unwrap()));
        assert_eq!(body.get("revision"), Some(&revision));
        assert_eq!(body.get("chunk_count"), Some(&J::int(0)));
        body
    };
    assert!(array(read(&mut client).get("entries").unwrap()).is_empty());
    assert!(!dir.exists());
    fs::create_dir_all(&dir).unwrap();
    // component-source v1/v2의 최소 Free Frame 레코드. HEAD와 저장 generation을 독립적으로 변경합니다.
    let frame = |id: &str, parent: &str, size: &str| {
        format!(
            r#"id: {id}
parent: {parent}
order_key: "80"
visible: "true"
locked: "false"
display_name: Frame
kind: frame
payload:
  size: "{size}"
  transform: "1 0 0 1 0 0"
  clipping:
    kind: frame-bounds
  background: none
  border: none
  corner_radius: "0 0 0 0"
  layout_mode: none
"#
        )
    };
    let indent = |record: String, spaces: &str| {
        record
            .lines()
            .map(|s| format!("{spaces}{s}\n"))
            .collect::<String>()
    };
    let v1 = format!(
        "format: stellar-component\nschema_version: 1\ncomponent:\n  id: {ID}\n  name: Card\n  root: {ROOT}\nroot_node:\n{}",
        indent(frame(ROOT, "none", "160 40"), "  ")
    );
    let path = dir.join(FILE);
    fs::write(&path, &v1).unwrap();
    fx.git(&["add", "-A"]);
    fx.git(&["commit", "-qm", "root Frame"]);
    let checked = |client: &mut Client, node_count, width| {
        let body = read(client);
        let items = array(body.get("entries").unwrap());
        assert_eq!(items.len(), 1);
        let entry = &items[0];
        assert_eq!(entry.get("id").map(text), Some(ID));
        assert_eq!(entry.get("name").map(text), Some("Card"));
        assert_eq!(entry.get("root").map(text), Some(ROOT));
        assert_eq!(entry.get("node_count"), Some(&J::int(node_count)));
        assert_eq!(
            entry.get("source_hash").map(text),
            Some(file_digest(&dir)[FILE].as_str())
        );
        assert_eq!(entry.get("width"), Some(&J::string(width)));
        assert_eq!(entry.get("height").map(text), Some("40"));
        body
    };
    let main = checked(&mut client, 1, "160");
    assert_eq!(
        array(main.get("entries").unwrap())[0].get("background"),
        Some(&J::Null)
    );
    let colored = v1.replace(
        "    background: none",
        "    background:\n      space: display-p3\n      rgba: \"1.2 -0.1 0.5 0\"",
    );
    assert_ne!(colored, v1);
    fs::write(&path, colored).unwrap();
    let filled = checked(&mut client, 1, "160");
    // generation은 그대로이고 YAML 원본 집합 hash만 바뀝니다.
    assert_eq!(filled.get("revision"), main.get("revision"));
    assert_ne!(
        filled.get("component_source_hash"),
        main.get("component_source_hash")
    );
    let background = array(filled.get("entries").unwrap())[0]
        .get("background")
        .unwrap();
    assert_eq!(background, &J::string("display-p3 1.2 -0.1 0.5 0"));
    fx.git(&["restore", "--", "design/components"]);
    assert_eq!(checked(&mut client, 1, "160"), main);
    fx.git(&["switch", "-qc", "nested"]);
    let v2 = v1.replace("schema_version: 1", "schema_version: 2")
        + "children:\n  -\n"
        + &indent(frame(CHILD, ROOT, "24 12"), "    ");
    fs::write(&path, &v2).unwrap();
    fx.git(&["add", "--", "design/components"]);
    fx.git(&["commit", "-qm", "nested Frame"]);
    let nested = checked(&mut client, 2, "160");
    let detail = format!("{COMPONENTS}/{ID}");
    let first_page = format!("{detail}?limit=1");
    let root_page = content(&client.read(&first_page), &first_page);
    assert_eq!(
        root_page.get("source_hash"),
        nested.get("entries").unwrap().as_array().unwrap()[0].get("source_hash")
    );
    let root = &array(root_page.get("nodes").unwrap())[0];
    assert_eq!(root.get("id").map(text), Some(ROOT));
    assert_eq!(root.get("parent"), Some(&J::Null));
    assert_eq!(root.get("display_name").map(text), Some("Frame"));
    assert_eq!(
        root.get("payload").unwrap().get("width").map(text),
        Some("160")
    );
    let cursor = text(root_page.get("next_cursor").unwrap());
    let next_page = format!("{detail}?cursor={cursor}");
    let child_page = content(&client.read(&next_page), &next_page);
    let child = &array(child_page.get("nodes").unwrap())[0];
    assert_eq!(child.get("id").map(text), Some(CHILD));
    assert_eq!(child.get("parent").map(text), Some(ROOT));
    assert_eq!(
        child.get("payload").unwrap().get("height").map(text),
        Some("12")
    );
    assert_error(&client.read(&format!("{COMPONENTS}/{ROOT}")), -32602);
    let nested_head = fx.git(&["rev-parse", "HEAD"]);
    assert_eq!(
        tool_page(&client.diff(empty())).get("total"),
        Some(&J::int(0))
    );
    fs::write(
        &path,
        v2.replace("160 40", "181 40").replace("24 12", "30 14"),
    )
    .unwrap();
    let before = file_digest(&fx.temp.path);
    assert_error(&client.read(&next_page), -32602);
    let updated = content(&client.read(&detail), &detail);
    assert_ne!(updated.get("source_hash"), root_page.get("source_hash"));
    assert_eq!(
        array(updated.get("nodes").unwrap())[1]
            .get("payload")
            .unwrap()
            .get("width")
            .map(text),
        Some("30")
    );
    let first = tool_page(&client.diff(J::object(vec![("limit", J::int(1))])));
    assert_eq!(first.get("total"), Some(&J::int(3)));
    assert_eq!(first.get("revision"), Some(&revision));
    assert_eq!(
        first.get("base_commit").map(text),
        Some(nested_head.as_str())
    );
    let cursor = first.get("next_cursor").unwrap().clone();
    let next = tool_page(&client.diff(J::object(vec![("cursor", cursor.clone())])));
    assert_eq!(next.get("next_cursor"), Some(&J::Null));
    let changes: Vec<_> = array(first.get("items").unwrap())
        .iter()
        .chain(array(next.get("items").unwrap()))
        .collect();
    assert_eq!(changes.len(), 3);
    for (entry, (object, path)) in
        changes
            .into_iter()
            .zip([(ROOT, "width"), (CHILD, "width"), (CHILD, "height")])
    {
        assert_eq!(entry.get("document"), Some(&J::Null));
        assert_eq!(entry.get("component").map(text), Some(ID));
        assert_eq!(entry.get("object").map(text), Some(object));
        assert_eq!(entry.get("path").map(text), Some(path));
        assert_eq!(entry.get("kind").map(text), Some("set-property"));
    }
    assert_eq!(file_digest(&fx.temp.path), before);
    fs::write(&path, v2.replace("160 40", "180 40")).unwrap();
    assert_error(
        &client.diff(J::object(vec![("cursor", cursor.clone())])),
        -32602,
    );
    let updated = tool_page(&client.diff(empty()));
    assert_eq!(updated.get("total"), Some(&J::int(1)));
    assert_ne!(
        updated.get("component_source_hash"),
        first.get("component_source_hash")
    );
    fs::write(&path, &v2).unwrap();
    // 고정된 HEAD가 실제로 바뀌면 현재 파일과 generation이 같아도 이전 커서는 폐기합니다.
    fx.git(&["symbolic-ref", "HEAD", "refs/heads/main"]);
    assert_error(&client.diff(J::object(vec![("cursor", cursor)])), -32602);
    let switched = tool_page(&client.diff(empty()));
    assert_eq!(switched.get("total"), Some(&J::int(1)));
    assert_eq!(
        array(switched.get("items").unwrap())[0]
            .get("object")
            .map(text),
        Some(CHILD)
    );
    fx.git(&["symbolic-ref", "HEAD", "refs/heads/nested"]);

    assert_ne!(nested, main);
    fx.git(&["switch", "-q", "main"]);
    assert_eq!(checked(&mut client, 1, "160"), main);
    fx.git(&["switch", "-q", "nested"]);
    assert_eq!(checked(&mut client, 2, "160"), nested);
    fs::write(&path, v2.replace("160 40", "184.5 40")).unwrap();
    assert_ne!(checked(&mut client, 2, "184.5"), nested);

    // 한 파일만 바꾼 경로도 현재 문서 또는 다른 정의가 소유한 ID를 가져올 수 없습니다.
    let colliding = v2.replace(CHILD, &document.document_id().to_string());
    fs::write(&path, colliding).unwrap();
    let before = file_digest(&fx.temp.path);
    assert_error(&client.diff(empty()), -32000);
    assert_eq!(file_digest(&fx.temp.path), before);
    fs::write(&path, v2.replace("160 40", "184.5 40")).unwrap();

    // 전체 SHA·generation을 그대로 둔 작업 파일 손상, 바이트 상한은 fail closed.
    let edited = fs::read(&path).unwrap();
    fs::write(&path, "format: unsupported").unwrap();
    assert_error(&client.diff(empty()), -32000);
    fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert_error(&client.diff(empty()), -32000);

    // HEAD의 과대 blob도 working 파일로 대체해서 비교하지 않습니다.
    fx.git(&["add", "--", "design/components"]);
    fx.git(&["commit", "-qm", "oversized source"]);
    fs::write(&path, &edited).unwrap();
    let over = file_digest(&fx.temp.path);
    assert_error(&client.diff(empty()), -32000);
    assert_eq!(file_digest(&fx.temp.path), over);
    fx.git(&["reset", "--soft", &nested_head]);
    fx.git(&["restore", "--staged", "--", "design/components"]);
    assert_eq!(
        tool_page(&client.diff(empty())).get("total"),
        Some(&J::int(1))
    );

    #[cfg(unix)]
    {
        // 작업 트리에 정상 파일을 돌려놔도 고정한 commit의 symlink는 거부합니다.
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("../../stellar.yaml", &path).unwrap();
        fx.git(&["add", "--", "design/components"]);
        fx.git(&["commit", "-qm", "source symlink"]);
        fs::remove_file(&path).unwrap();
        fs::write(&path, &edited).unwrap();
        let switched = file_digest(&fx.temp.path);
        assert_error(&client.diff(empty()), -32000);
        assert_eq!(file_digest(&fx.temp.path), switched);
    }

    assert_error(
        &client.call(
            "resources/read",
            J::object(vec![
                ("uri", J::string(COMPONENTS)),
                ("path", J::string("design")),
            ]),
        ),
        -32602,
    );
    FileTrustStore::open(&fx.data)
        .unwrap()
        .revoke(&fx.identity())
        .unwrap();
    let revoked = file_digest(&fx.temp.path);
    assert_error(&client.read(COMPONENTS), -32000);
    client.finish();
    assert_eq!(file_digest(&fx.temp.path), revoked);
}

#[test]
fn head_diff_pages_follow_real_branches_and_preserve_all_inputs() {
    let fx = Fixture::new();
    fx.git(&["init", "-q", "-b", "main"]);
    let mut document = large_document(4);
    let nodes = document.to_records().unwrap().nodes;
    save_documents(&fx.project, &[&document]);
    fx.grant();
    let mut client = fx.launch();
    client.initialize();
    assert_error(&client.diff(empty()), -32000); // unborn HEAD
    fx.git(&["add", "-A"]);
    fx.git(&["commit", "-qm", "main"]);
    let main = fx.git(&["rev-parse", "HEAD"]);
    let before = file_digest(&fx.temp.path);
    let clean = tool_page(&client.diff(empty()));
    assert_eq!(clean.get("total"), Some(&J::int(0)));
    assert_eq!(clean.get("base_commit").map(text), Some(main.as_str()));
    assert_eq!(file_digest(&fx.temp.path), before);

    fx.git(&["switch", "-qc", "feature"]);
    document
        .set_display_name(nodes[0].id, "브랜치 이름".into())
        .unwrap();
    save_documents(&fx.project, &[&document]);
    fx.git(&["add", "-A"]);
    fx.git(&["commit", "-qm", "feature"]);
    let feature = fx.git(&["rev-parse", "HEAD"]);
    for (node, name) in nodes.iter().take(2).zip(["변경 A", "변경 B"]) {
        document.set_display_name(node.id, name.into()).unwrap();
    }
    save_documents(&fx.project, &[&document]);
    let before = file_digest(&fx.temp.path);
    let first = tool_page(&client.diff(J::object(vec![
        ("base", J::string("HEAD")),
        ("limit", J::int(1)),
    ])));
    assert_eq!(first.get("total"), Some(&J::int(2)));
    assert_eq!(first.get("base_commit").map(text), Some(feature.as_str()));
    let cursor = first.get("next_cursor").unwrap().clone();
    let second = tool_page(&client.diff(J::object(vec![("cursor", cursor.clone())])));
    assert_eq!(second.get("offset"), Some(&J::int(1)));
    assert_eq!(second.get("next_cursor"), Some(&J::Null));
    let changes: Vec<_> = [&first, &second]
        .into_iter()
        .map(|page| &array(page.get("items").unwrap())[0])
        .collect();
    let rename = changes
        .iter()
        .find(|e| e.get("object") == Some(&J::string(nodes[0].id.to_string())))
        .unwrap();
    assert_eq!(
        rename.get("old").unwrap().get("value").map(text),
        Some("브랜치 이름")
    );
    assert_eq!(
        rename.get("new").unwrap().get("value").map(text),
        Some("변경 A")
    );
    for bad in [
        J::object(vec![("base", J::string("feature"))]),
        J::object(vec![("path", J::string("other"))]),
        J::object(vec![("limit", J::int(2001))]),
    ] {
        assert_error(&client.diff(bad), -32602);
    }
    assert_eq!(file_digest(&fx.temp.path), before);

    // 작업 트리와 index는 그대로 두고 실제 branch를 이동합니다.
    fx.git(&["symbolic-ref", "HEAD", "refs/heads/main"]);
    let after_ref = file_digest(&fx.temp.path);
    assert_error(&client.diff(J::object(vec![("cursor", cursor)])), -32602);
    let switched = tool_page(&client.diff(J::object(vec![("limit", J::int(1))])));
    assert_eq!(switched.get("base_commit").map(text), Some(main.as_str()));
    assert_eq!(switched.get("revision"), first.get("revision"));
    assert_eq!(file_digest(&fx.temp.path), after_ref);
    document
        .set_display_name(nodes[1].id, "새 저장".into())
        .unwrap();
    save_documents(&fx.project, &[&document]);
    assert_error(
        &client.diff(J::object(vec![(
            "cursor",
            switched.get("next_cursor").unwrap().clone(),
        )])),
        -32602,
    );

    // 긴 문자열도 count limit보다 실제 JSON byte 한도가 먼저 페이지를 끊습니다.
    for node in nodes.iter().take(3) {
        document
            .set_display_name(node.id, "이름".repeat(38_000))
            .unwrap();
    }
    save_documents(&fx.project, &[&document]);
    let large = tool_page(&client.diff(empty()));
    assert_eq!(large.get("total"), Some(&J::int(3)));
    assert_eq!(array(large.get("items").unwrap()).len(), 2);
    let tail = tool_page(&client.diff(J::object(vec![(
        "cursor",
        large.get("next_cursor").unwrap().clone(),
    )])));
    assert_eq!(array(tail.get("items").unwrap()).len(), 1);
    document
        .set_display_name(nodes[0].id, "가".repeat(180_000))
        .unwrap();
    save_documents(&fx.project, &[&document]);
    assert_error(&client.diff(empty()), -32602);
    for node in nodes.iter().take(3) {
        document.set_display_name(node.id, "복원".into()).unwrap();
    }
    save_documents(&fx.project, &[&document]);

    // 별도 branch의 다른 프로젝트 ID로 커밋한 매니페스트를 기준으로 사용하지 않습니다.
    fx.git(&["switch", "-qc", "foreign"]);
    let manifest = fx.project.join("stellar.yaml");
    let original = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, original.replace(PROJECT_ID_TEXT, OTHER)).unwrap();
    fx.git(&["add", "stellar.yaml"]);
    fx.git(&["commit", "-qm", "different project"]);
    std::fs::write(manifest, original).unwrap();
    let before = file_digest(&fx.temp.path);
    assert_error(&client.diff(empty()), -32000);
    assert_eq!(file_digest(&fx.temp.path), before);
    fx.git(&["symbolic-ref", "HEAD", "refs/heads/main"]);
    assert!(tool_page(&client.diff(empty())).get("items").is_some());
    FileTrustStore::open(&fx.data)
        .unwrap()
        .revoke(&fx.identity())
        .unwrap();
    let revoked = file_digest(&fx.temp.path);
    assert_error(&client.diff(empty()), -32000);
    client.finish();
    assert_eq!(file_digest(&fx.temp.path), revoked);
}

#[test]
fn validation_pages_share_the_cli_validator_and_only_read_the_selected_project() {
    let fx = Fixture::new();
    let warned = document_with_unresolved_clips(5);
    let other = random_valid_document(26);
    save_documents(&fx.project, &[&warned, &other]);
    fx.grant();
    let before = file_digest(&fx.temp.path);
    let mut client = fx.launch();
    assert_error(&client.validate(empty()), -32002);
    client.initialize();
    let project = tool_page(&client.validate(empty()));
    assert_eq!(project.get("documents_checked"), Some(&J::int(2)));
    assert_eq!(project.get("document"), Some(&J::Null));

    let document = warned.document_id().to_string();
    let mut arguments = vec![("document", J::string(&document)), ("limit", J::int(2))];
    let mut cursor = None;
    let mut messages = std::collections::BTreeSet::new();
    for offset in [0, 2, 4] {
        let mut args = arguments.clone();
        if let Some(previous) = &cursor {
            args.push(("cursor", J::string(previous)));
        }
        let page = tool_page(&client.validate(J::object(args)));
        assert_eq!(page.get("valid"), Some(&J::Bool(true)));
        assert_eq!(page.get("documents_checked"), Some(&J::int(1)));
        assert_eq!(page.get("errors"), Some(&J::int(0)));
        assert_eq!(page.get("warnings"), Some(&J::int(5)));
        assert_eq!(page.get("total"), Some(&J::int(5)));
        assert_eq!(page.get("offset"), Some(&J::int(offset)));
        assert_eq!(page.get("revision"), project.get("revision"));
        let items = array(page.get("items").unwrap());
        assert_eq!(items.len(), if offset == 4 { 1 } else { 2 });
        for finding in items {
            assert_eq!(finding.get("document").map(text), Some(document.as_str()));
            assert_eq!(finding.get("severity").map(text), Some("warning"));
            assert_eq!(finding.get("code").map(text), Some("unresolved-reference"));
            assert!(messages.insert(text(finding.get("message").unwrap()).to_owned()));
        }
        cursor = page.get("next_cursor").unwrap().as_str().map(str::to_owned);
        if offset == 0 {
            assert_error(
                &client.validate(J::object(vec![(
                    "cursor",
                    J::string(cursor.as_ref().unwrap()),
                )])),
                -32602,
            );
        }
    }
    assert!(cursor.is_none());
    assert_eq!(messages.len(), 5);
    let first = tool_page(&client.validate(J::object(arguments.clone())));
    arguments.push(("cursor", first.get("next_cursor").unwrap().clone()));
    for bad in [
        J::object(vec![("document", J::string(OTHER))]),
        J::object(vec![("path", J::string("."))]),
        J::object(vec![("limit", J::int(0))]),
        J::object(vec![("limit", J::int(2001))]),
        J::object(vec![("limit", J::string("2"))]),
    ] {
        assert_error(&client.validate(bad), -32602);
    }
    assert_eq!(file_digest(&fx.temp.path), before);
    save_documents(&fx.project, &[&warned]);
    let saved = file_digest(&fx.temp.path);
    assert_error(&client.validate(J::object(arguments)), -32602);
    assert_eq!(file_digest(&fx.temp.path), saved);
    FileTrustStore::open(&fx.data)
        .unwrap()
        .revoke(&fx.identity())
        .unwrap();
    let revoked = file_digest(&fx.temp.path);
    assert_error(&client.validate(empty()), -32000);
    assert_error(&client.call("tools/list", empty()), -32000);
    client.finish();
    assert_eq!(file_digest(&fx.temp.path), revoked);
}

#[test]
fn validation_reports_clean_and_unreadable_saved_documents_without_writes() {
    let fx = Fixture::new();
    save_documents(&fx.project, &[&large_document(5)]);
    fx.grant();
    let read = || {
        let before = file_digest(&fx.temp.path);
        let mut client = fx.launch();
        client.initialize();
        let reply = client.call(
            "tools/call",
            J::object(vec![("name", J::string("document.validate"))]),
        );
        let page = tool_page(&reply);
        client.finish();
        assert_eq!(file_digest(&fx.temp.path), before);
        page
    };
    let clean = read();
    assert_eq!(clean.get("valid"), Some(&J::Bool(true)));
    assert_eq!(clean.get("next_cursor"), Some(&J::Null));
    assert!(array(clean.get("items").unwrap()).is_empty());
    assert!(corrupt_view_tree_chunks(&fx.project) > 0);
    let damaged = read();
    assert_eq!(damaged.get("valid"), Some(&J::Bool(false)));
    assert_eq!(damaged.get("errors"), Some(&J::int(1)));
    assert_eq!(
        array(damaged.get("items").unwrap())[0]
            .get("code")
            .map(text),
        Some("document-load")
    );
    assert!(!json::write(&damaged).contains(fx.project.to_str().unwrap()));
}

#[test]
fn missing_or_changed_authority_never_starts_a_read_session() {
    let fx = Fixture::new();
    let denied = |args: &[&str], code| {
        let before = file_digest(&fx.temp.path);
        let output = fx.command(args).stdin(Stdio::null()).output().unwrap();
        assert_eq!(output.status.code(), Some(code), "{:?}", output.stderr);
        assert!(output.stdout.is_empty());
        assert_eq!(file_digest(&fx.temp.path), before);
    };
    denied(&["--headless", "--project", fx.root()], 77);
    // 실행 인자는 절대 프로젝트 루트입니다(ADR-0005 §3). 프로젝트 ID나 상대 경로로는 고르지 않습니다.
    denied(&["--headless", "--project", PROJECT_ID_TEXT], 64);
    denied(&["--headless", "--project", "project"], 64);
    denied(&["--headless", "--project", fx.root(), "--attach"], 64);
    fx.grant();
    // 신뢰하지 않은 다른 폴더와 없는 경로는 프로젝트 파일을 읽기 전에 거부합니다.
    let copy = fx.temp.path.join("copy");
    scaffold_project(&copy);
    denied(&["--headless", "--project", copy.to_str().unwrap()], 77);
    let missing = fx.temp.path.join("missing");
    denied(&["--headless", "--project", missing.to_str().unwrap()], 77);
    let manifest = fx.project.join("stellar.yaml");
    let old = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, old.replace(PROJECT_ID_TEXT, OTHER)).unwrap();
    denied(&["--headless", "--project", fx.root()], 77);
    std::fs::write(&manifest, old).unwrap();
    let store = FileTrustStore::open(&fx.data).unwrap();
    std::fs::write(store.file_path(), "invalid store").unwrap();
    denied(&["--headless", "--project", fx.root()], 77);
}

#[test]
fn each_trusted_worktree_of_one_project_id_is_selected_by_its_root() {
    // 같은 프로젝트 ID의 작업 폴더 두 개가 모두 신뢰돼 있어도 루트 인자마다 그 폴더만 읽습니다.
    let fx = Fixture::new();
    let main = large_document(3);
    save_documents(&fx.project, &[&main]);
    fx.grant();
    let worktree = fx.temp.path.join("worktree");
    scaffold_project(&worktree);
    let other = random_valid_document(91);
    assert_ne!(main.document_id(), other.document_id());
    save_documents(&worktree, &[&other]);
    FileTrustStore::open(&fx.data)
        .unwrap()
        .grant(&ProjectIdentity::new(project_id(), &worktree).unwrap())
        .unwrap();
    let before = file_digest(&fx.temp.path);
    for (root, model) in [(&fx.project, &main), (&worktree, &other)] {
        let mut client = fx.launch_at(root);
        client.initialize();
        let structure = content(&client.read(STRUCTURE), STRUCTURE);
        let documents = array(structure.get("documents").unwrap());
        assert_eq!(documents.len(), 1);
        assert_eq!(
            documents[0].get("id").map(text),
            Some(model.document_id().to_string().as_str())
        );
        client.finish();
    }
    assert_eq!(file_digest(&fx.temp.path), before);
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
            let (label, binary, [first, next, repeated]) = &mut runs[index];
            let mut client = Client::new(
                fx.command_with_binary(&binary, &["--headless", "--project", fx.root()]),
            );
            client.initialize_with_tools(*label == "after");
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
