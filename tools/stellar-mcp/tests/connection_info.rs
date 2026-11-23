//! XIX.9.8 검증: MCP client 등록 정보 golden 일치, 다른 도구 설정 변경 0.
//!
//! - `stellar-mcp --connection-info --project <경로>` 출력이 `tests/fixtures/connection-info.json`과
//!   같습니다(`<COMMAND>`는 빌드된 실행 파일, `<PROJECT>`는 정규화한 프로젝트의 절대 경로). 등록
//!   기본값은 그 프로젝트의 headless 서버입니다(ADR-0005 §3).
//! - 출력한 `command`·`args`·`env`만으로(client의 HOME 위에) 실행한 서버가 initialize에 응답하고,
//!   `serverInfo`와 협상 version이 등록 정보와 일치합니다(등록 정보가 실행에 충분함).
//! - 프로젝트 없이 `--connection-info`만 주거나 manifest가 없는 경로를 주면 사용법 오류(64)입니다.
//! - 임시 HOME에 MCP client 설정 파일 모양의 파일을 두고 실행해도 HOME·작업 디렉터리의 파일
//!   목록·내용·수정 시각이 그대로입니다.
//! - 브리지 소스에 파일 생성·수정·삭제 API가 없습니다.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use stellar_mcp::json::{parse, write, JsonLimits, JsonValue};
use stellar_mcp::test_support::{project_id, scaffold_project, FileTrustStore, ProjectIdentity};

const BIN: &str = env!("CARGO_BIN_EXE_stellar-mcp");
const GOLDEN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/connection-info.json"
));

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!("smcp-ci-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 디렉터리 아래 모든 항목의 (상대 경로 → (내용, 수정 시각)). 디렉터리는 내용 없이 기록합니다.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Option<Vec<u8>>, SystemTime)> {
    let mut map = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            let content = if meta.is_dir() {
                stack.push(path.clone());
                None
            } else {
                Some(std::fs::read(&path).unwrap())
            };
            map.insert(rel, (content, meta.modified().unwrap()));
        }
    }
    map
}

/// `project`의 등록 정보를 상대 경로(`cwd` 기준)로 요청합니다. 출력은 정규화한 절대 경로여야 합니다.
fn run_connection_info(home: &Path, cwd: &Path, project: &Path) -> std::process::Output {
    Command::new(BIN)
        .arg("--connection-info")
        .arg("--project")
        .arg(relative_to(project, cwd))
        .env_clear()
        .env("HOME", home)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// `cwd`에서 `target`으로 가는 `..` 상대 경로.
fn relative_to(target: &Path, cwd: &Path) -> PathBuf {
    let depth = cwd.canonicalize().unwrap().components().count() - 1;
    let mut path: PathBuf = std::iter::repeat_n("..", depth).collect();
    path.push(target.canonicalize().unwrap().strip_prefix("/").unwrap());
    path
}

/// 임시 Stellar 프로젝트(manifest와 첫 저장 세대).
fn project(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    scaffold_project(&dir.0);
    dir
}

#[test]
fn connection_info_matches_golden_and_changes_no_other_tool_config() {
    let home = TempDir::new("home");
    let cwd = TempDir::new("cwd");
    let project = project("golden-project");
    // MCP client 설정 파일 모양의 파일(특정 도구 형식이 아닌 일반 이름).
    for (rel, body) in [
        (
            ".config/example-mcp-client/config.json",
            "{\"mcpServers\":{}}",
        ),
        (
            "Library/Application Support/ExampleClient/mcp.json",
            "{\"servers\":[]}",
        ),
        (".example-agent/settings.toml", "[mcp]\n"),
    ] {
        let path = home.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
    }
    let before_home = snapshot(&home.0);
    let before_cwd = snapshot(&cwd.0);
    let before_project = snapshot(&project.0);

    let output = run_connection_info(&home.0, &cwd.0, &project.0);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    let stdout = String::from_utf8(output.stdout).unwrap();

    let command = Path::new(BIN).canonicalize().unwrap();
    let quoted = write(&JsonValue::string(command.to_str().unwrap()));
    let root = project.0.canonicalize().unwrap();
    let root = write(&JsonValue::string(root.to_str().unwrap()));
    assert_eq!(
        stdout,
        GOLDEN
            .replace("\"<COMMAND>\"", &quoted)
            .replace("\"<PROJECT>\"", &root)
    );
    assert_eq!(stdout.matches('\n').count(), 1, "JSON 한 줄");

    assert_eq!(snapshot(&home.0), before_home, "HOME 아래 변경 0");
    assert_eq!(snapshot(&cwd.0), before_cwd, "작업 디렉터리 변경 0");
    assert_eq!(snapshot(&project.0), before_project, "프로젝트 변경 0");
    assert!(before_cwd.is_empty());
}

#[test]
fn connection_info_is_sufficient_to_launch_the_server() {
    let home = TempDir::new("launch");
    let project = project("launch-project");
    // headless는 신뢰한 프로젝트만 엽니다. 신뢰 등록은 사용자가 앱에서 한 것으로 둡니다.
    let data = home
        .0
        .join("Library/Application Support/xyz.taeyoon.stellar");
    FileTrustStore::open(&data)
        .unwrap()
        .grant(&ProjectIdentity::new(project_id(), &project.0.canonicalize().unwrap()).unwrap())
        .unwrap();
    let output = run_connection_info(&home.0, &home.0, &project.0);
    assert_eq!(output.status.code(), Some(0), "{:?}", output.stderr);
    let info = parse(
        String::from_utf8(output.stdout).unwrap().trim_end(),
        JsonLimits::default(),
    )
    .unwrap();
    assert_eq!(
        info.get("transport").and_then(JsonValue::as_str),
        Some("stdio")
    );
    let command = info.get("command").and_then(JsonValue::as_str).unwrap();
    assert!(Path::new(command).is_absolute());
    let args: Vec<&str> = match info.get("args") {
        Some(JsonValue::Array(items)) => items.iter().filter_map(JsonValue::as_str).collect(),
        other => panic!("args: {other:?}"),
    };
    let env: Vec<(String, String)> = match info.get("env").and_then(JsonValue::as_object) {
        Some(entries) => entries
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect(),
        None => panic!("env object"),
    };

    assert_eq!(&args[..2], ["--headless", "--project"]);
    // client처럼 등록 정보만으로 subprocess를 실행합니다. client는 자기 HOME 위에 `env`를 더합니다.
    let mut child = Command::new(command)
        .args(&args)
        .env_clear()
        .env("HOME", &home.0)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}
"#,
            )
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    let reply = parse(
        String::from_utf8(output.stdout).unwrap().trim_end(),
        JsonLimits::default(),
    )
    .unwrap();
    let result = reply.get("result").unwrap();
    assert_eq!(result.get("serverInfo"), info.get("serverInfo"));
    let negotiated = result.get("protocolVersion").unwrap();
    match info.get("protocolVersions") {
        Some(JsonValue::Array(versions)) => assert!(versions.contains(negotiated)),
        other => panic!("protocolVersions: {other:?}"),
    }
}

#[test]
fn connection_info_requires_a_stellar_project() {
    let home = TempDir::new("needs-project");
    let output = Command::new(BIN)
        .arg("--connection-info")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
    // manifest가 없는 폴더와 없는 경로도 등록 정보를 내지 않습니다.
    for path in [home.0.clone(), home.0.join("missing")] {
        let output = Command::new(BIN)
            .arg("--connection-info")
            .arg("--project")
            .arg(&path)
            .env_clear()
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(64), "{}", path.display());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn connection_info_rejects_extra_arguments() {
    let output = Command::new(BIN)
        .args(["--connection-info", "--write"])
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
}

#[test]
fn bridge_sources_have_no_file_mutation_api() {
    // 이 파일 자체가 걸리지 않도록 패턴을 조립합니다.
    let forbidden: Vec<String> = [
        ["fs::", "write"],
        ["File::", "create"],
        ["Open", "Options"],
        ["create_", "dir"],
        ["remove_", "file"],
        ["remove_", "dir"],
        ["fs::", "rename"],
        ["fs::", "copy"],
        ["set_", "permissions"],
        ["sym", "link("],
    ]
    .iter()
    .map(|parts| parts.concat())
    .collect();
    let mut stack = vec![
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/mcp-core/src"),
    ];
    let mut files = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            files += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            // 파일 끝의 테스트 module만 제외합니다. 중간 계측 field·statement 뒤 제품 코드는 검사합니다.
            let shipped = text.split("\n#[cfg(test)]\nmod tests").next().unwrap_or("");
            for pattern in &forbidden {
                assert!(
                    !shipped.contains(pattern.as_str()),
                    "{}: 파일 변경 API 금지({pattern})",
                    path.display()
                );
            }
        }
    }
    assert!(files > 0);
}
