//! stellar-mcp stdio 통합 테스트 공용 fixture(XIX.9.3·XIX.9.6·XIX.9.9·XIX.9.10).
//!
//! 프로젝트·신뢰 저장소·HOME은 모두 임시 폴더입니다. 실제 `stellar-mcp` 실행 파일을 `--headless`로 띄우고
//! 한 줄 JSON-RPC로 대화합니다.
#![allow(dead_code, unused_imports)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

pub use stellar_mcp::json::{self, JsonLimits, JsonValue as J};
use stellar_mcp::server::MAX_MESSAGE_BYTES;
pub use stellar_mcp::test_support::{
    file_digest, large_document, project_id, save_documents, scaffold_project, FileTrustStore,
    ProjectIdentity, TempDir,
};

pub const BIN: &str = env!("CARGO_BIN_EXE_stellar-mcp");
pub const STRUCTURE: &str = "stellar://project/structure";

pub struct Fixture {
    pub temp: TempDir,
    pub project: PathBuf,
    pub home: PathBuf,
    pub data: PathBuf,
}

impl Fixture {
    pub fn new(label: &str) -> Self {
        let temp = TempDir::new(label);
        let project = temp.path.join("project");
        let home = temp.path.join("home");
        let base = if cfg!(target_os = "macos") {
            home.join("Library/Application Support")
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

    pub fn identity(&self) -> ProjectIdentity {
        ProjectIdentity::new(project_id(), &self.project).unwrap()
    }

    pub fn grant(&self) {
        FileTrustStore::open(&self.data)
            .unwrap()
            .grant(&self.identity())
            .unwrap();
    }

    pub fn revoke(&self) {
        FileTrustStore::open(&self.data)
            .unwrap()
            .revoke(&self.identity())
            .unwrap();
    }

    pub fn command(&self, binary: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Command {
        let mut command = Command::new(binary);
        command
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .current_dir(&self.home);
        command
    }

    /// 고정 identity·hook 없는 Git을 프로젝트에서 실행합니다.
    pub fn git(&self, args: &[&str]) -> String {
        let output = self
            .command("git", &[])
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

    pub fn launch(&self) -> Client {
        Client::new(self.command(
            BIN,
            &["--headless", "--project", self.project.to_str().unwrap()],
        ))
    }
}

pub struct Client {
    child: Child,
    input: Option<ChildStdin>,
    replies: Receiver<String>,
    id: i64,
}

impl Client {
    pub fn new(mut command: Command) -> Self {
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

    pub fn send(&mut self, value: J) {
        writeln!(self.input.as_mut().unwrap(), "{}", json::write(&value)).unwrap();
    }

    /// 준비한 frame bytes를 그대로 보냅니다(응답을 기다리지 않음).
    pub fn send_raw(&mut self, bytes: &[u8]) {
        self.input.as_mut().unwrap().write_all(bytes).unwrap();
    }

    /// 다음 응답 한 줄.
    pub fn next_reply(&mut self) -> J {
        let line = self
            .replies
            .recv_timeout(Duration::from_secs(30))
            .expect("stdio 응답 timeout");
        json::parse(&line, JsonLimits::default()).unwrap()
    }

    /// 응답 원문 한 줄.
    pub fn raw_call(&mut self, method: &str, params: J) -> String {
        self.id += 1;
        self.send(J::object(vec![
            ("jsonrpc", J::string("2.0")),
            ("id", J::int(self.id)),
            ("method", J::string(method)),
            ("params", params),
        ]));
        let line = self
            .replies
            .recv_timeout(Duration::from_secs(30))
            .expect("stdio 응답 timeout");
        assert!(line.len() <= MAX_MESSAGE_BYTES, "stdout frame bound");
        line
    }

    pub fn call(&mut self, method: &str, params: J) -> J {
        let line = self.raw_call(method, params);
        let reply = json::parse(
            &line,
            JsonLimits {
                max_bytes: MAX_MESSAGE_BYTES,
                max_depth: 64,
                max_nodes: 1 << 20,
            },
        )
        .unwrap();
        assert_eq!(reply.get("id"), Some(&J::int(self.id)));
        reply
    }

    pub fn initialize(&mut self) -> J {
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
        self.send(J::object(vec![
            ("jsonrpc", J::string("2.0")),
            ("method", J::string("notifications/initialized")),
        ]));
        reply
    }

    pub fn tool(&mut self, name: &str, arguments: J) -> J {
        self.call(
            "tools/call",
            J::object(vec![("name", J::string(name)), ("arguments", arguments)]),
        )
    }

    pub fn read(&mut self, uri: &str) -> J {
        self.call("resources/read", J::object(vec![("uri", J::string(uri))]))
    }

    /// stdin을 닫고 정상 종료(0)를 기다립니다.
    pub fn finish(&mut self) {
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

pub fn empty() -> J {
    J::object(vec![])
}

pub fn text(value: &J) -> &str {
    value.as_str().unwrap()
}

pub fn array(value: &J) -> &[J] {
    value.as_array().unwrap()
}

/// 성공한 tool 결과의 내용 JSON.
pub fn tool_body(reply: &J) -> J {
    let result = reply
        .get("result")
        .unwrap_or_else(|| panic!("tool 실패: {reply:?}"));
    let content = array(result.get("content").unwrap());
    assert_eq!(content.len(), 1);
    let structured = result.get("structuredContent").unwrap();
    let serialized =
        json::parse(text(content[0].get("text").unwrap()), JsonLimits::default()).unwrap();
    assert_eq!(&serialized, structured, "text block = structuredContent");
    assert_eq!(structured.get("kind").map(text), Some("ok"));
    structured.get("data").unwrap().clone()
}

/// 실패 응답의 종류(`kind`). tool 실행 실패는 `isError` 결과의 `structuredContent`, protocol·resource 실패는
/// JSON-RPC `error.data`에서 읽습니다. 둘 다 XXIV.1.2 종료 코드(`exit`)가 결과 종류와 맞는지 확인합니다.
pub fn failure_kind(reply: &J) -> String {
    let (kind, exit) = match reply.get("result") {
        Some(result) => {
            assert_eq!(result.get("isError"), Some(&J::Bool(true)), "{reply:?}");
            let structured = result.get("structuredContent").unwrap();
            let content = array(result.get("content").unwrap());
            let serialized =
                json::parse(text(content[0].get("text").unwrap()), JsonLimits::default()).unwrap();
            assert_eq!(&serialized, structured);
            (structured.get("kind"), structured.get("exit"))
        }
        None => {
            let data = reply
                .get("error")
                .and_then(|e| e.get("data"))
                .unwrap_or_else(|| panic!("실패 종류 없음: {reply:?}"));
            (data.get("kind"), data.get("exit"))
        }
    };
    let kind = kind.and_then(J::as_str).unwrap().to_string();
    let expected = stellar_mcp::agent_protocol::outcome::ResultKind::from_name(&kind)
        .unwrap()
        .exit();
    let exit = exit.unwrap();
    assert_eq!(exit.get("code"), Some(&J::int(i64::from(expected.code))));
    assert_eq!(exit.get("name").map(text), Some(expected.name));
    kind
}

/// resource 내용 JSON.
pub fn resource_body(reply: &J) -> J {
    let result = reply
        .get("result")
        .unwrap_or_else(|| panic!("resource 실패: {reply:?}"));
    let contents = array(result.get("contents").unwrap());
    json::parse(
        text(contents[0].get("text").unwrap()),
        JsonLimits::default(),
    )
    .unwrap()
}

/// 문서 하나(Frame root + 그룹 Frame + Fragment)를 저장하고 Git 저장소로 만든 뒤 첫 commit을 합니다.
pub fn committed_project(fx: &Fixture, nodes: usize) -> stellar_mcp::test_support::FixtureDocument {
    let document = large_document(nodes);
    save_documents(&fx.project, &[&document]);
    fx.git(&["init", "-q", "-b", "main"]);
    fx.git(&["add", "-A"]);
    fx.git(&["commit", "-qm", "base"]);
    document
}
