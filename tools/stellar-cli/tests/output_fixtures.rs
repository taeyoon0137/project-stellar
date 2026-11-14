//! XXIV.1.2 검증: 종료 코드·JSON 출력 스키마 success/error fixture.
//!
//! 빌드된 실행 파일을 headless(환경 변수 없음)로 실행하고 다음을 확인합니다.
//!
//! - 종료 코드가 fixture 기대값과 같고 JSON `exit_code`와도 같습니다.
//! - stdout은 golden fixture(`tests/fixtures/results/*.json`)와 byte 단위로 같고, JSON 한 줄뿐입니다.
//! - stdout JSON은 엄격한 reader로 파싱되고 `schema/cli-result.v1.schema.json`을 만족합니다.
//! - 성공은 stderr가 비어 있고, 오류는 stderr에 사람용 진단(오류 코드 포함)이 있습니다.
//! - stdout에는 입력 경로가 들어가지 않습니다(경로는 stderr에만).
//! - XXIV.4.3: `inspect --structure`·`query` 결과는 golden과 같고 revision을 담으며, 실행 전후
//!   프로젝트 파일 hash가 같습니다. 옵션 없는 `inspect`는 XXIV.1.2 fixture와 그대로 같습니다.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use stellar_cli::exit::ExitStatus;
use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_cli::output::ENVELOPE_KEYS;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-cli-out-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 프로젝트 디렉터리를 만듭니다. `manifest`가 있으면 해당 fixture를 `stellar.yaml`로 복사합니다.
fn project(dir: &TempDir, manifest: Option<&str>, git: bool) -> PathBuf {
    let root = dir.path.join("project");
    std::fs::create_dir_all(&root).unwrap();
    if let Some(name) = manifest {
        std::fs::copy(
            Path::new(FIXTURES).join("projects").join(name),
            root.join("stellar.yaml"),
        )
        .unwrap();
    }
    if git {
        std::fs::create_dir_all(root.join(".git")).unwrap();
    }
    root
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn schema() -> JsonValue {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/schema/cli-result.v1.schema.json"
    ))
    .unwrap();
    parse(&text, JsonLimits::default()).unwrap()
}

/// 이 스키마가 쓰는 JSON Schema 부분집합(type·const·enum·required·properties·
/// additionalProperties·items·oneOf)으로 값을 검사합니다.
fn validate(value: &JsonValue, schema: &JsonValue, at: &str) -> Result<(), String> {
    if let Some(expected) = schema.get("const") {
        if value != expected {
            return Err(format!("{at}: const 불일치"));
        }
    }
    if let Some(options) = schema.get("enum").and_then(JsonValue::as_array) {
        if !options.contains(value) {
            return Err(format!("{at}: enum 밖의 값 {value:?}"));
        }
    }
    if let Some(types) = schema.get("type") {
        let names: Vec<&str> = match types {
            JsonValue::String(s) => vec![s.as_str()],
            JsonValue::Array(items) => items.iter().filter_map(JsonValue::as_str).collect(),
            _ => return Err(format!("{at}: 잘못된 type 선언")),
        };
        let actual = match value {
            JsonValue::Null => "null",
            JsonValue::Bool(_) => "boolean",
            JsonValue::Number(_) => "number",
            JsonValue::String(_) => "string",
            JsonValue::Array(_) => "array",
            JsonValue::Object(_) => "object",
        };
        if !names.contains(&actual) {
            return Err(format!("{at}: type {actual}은 {names:?}가 아님"));
        }
    }
    if let Some(options) = schema.get("oneOf").and_then(JsonValue::as_array) {
        let matches = options
            .iter()
            .filter(|o| validate(value, o, at).is_ok())
            .count();
        if matches != 1 {
            return Err(format!("{at}: oneOf {matches}개 일치"));
        }
    }
    if let Some(pairs) = value.as_object() {
        let properties = schema.get("properties");
        if let Some(required) = schema.get("required").and_then(JsonValue::as_array) {
            for key in required.iter().filter_map(JsonValue::as_str) {
                if value.get(key).is_none() {
                    return Err(format!("{at}: 필수 key {key} 없음"));
                }
            }
        }
        for (key, item) in pairs {
            match properties.and_then(|p| p.get(key)) {
                Some(sub) => validate(item, sub, &format!("{at}.{key}"))?,
                None if schema.get("additionalProperties") == Some(&JsonValue::Bool(false)) => {
                    return Err(format!("{at}: 허용되지 않은 key {key}"))
                }
                None => {}
            }
        }
    }
    if let (Some(items), Some(sub)) = (value.as_array(), schema.get("items")) {
        for (i, item) in items.iter().enumerate() {
            validate(item, sub, &format!("{at}[{i}]"))?;
        }
    }
    Ok(())
}

/// 실행 결과가 fixture·스키마·stdout/stderr 계약을 만족하는지 검사합니다.
fn check(run: &Run, fixture: &str, expected: ExitStatus, private: &[&Path]) {
    assert_eq!(
        run.code,
        i32::from(expected.code()),
        "{fixture}: 종료 코드, stderr: {}",
        run.stderr
    );
    let golden_path = Path::new(FIXTURES).join("results").join(fixture);
    // 새 golden을 만들 때만 STELLAR_CLI_BLESS=1로 실행합니다(검토 후 commit).
    if std::env::var_os("STELLAR_CLI_BLESS").is_some() {
        std::fs::write(&golden_path, &run.stdout).unwrap();
    }
    let golden = std::fs::read_to_string(&golden_path).unwrap();
    assert_eq!(run.stdout, golden, "{fixture}: stdout golden 불일치");
    assert!(run.stdout.ends_with('\n'), "{fixture}: 개행으로 끝남");
    assert_eq!(run.stdout.matches('\n').count(), 1, "{fixture}: JSON 한 줄");

    let doc = parse(run.stdout.trim_end(), JsonLimits::default()).unwrap();
    validate(&doc, &schema(), "$").unwrap_or_else(|e| panic!("{fixture}: {e}"));
    let keys: Vec<&str> = doc
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(keys, ENVELOPE_KEYS, "{fixture}: key 순서");
    assert_eq!(
        doc.get("exit_code").and_then(JsonValue::as_i64),
        Some(i64::from(expected.code()))
    );
    assert_eq!(
        doc.get("ok").and_then(JsonValue::as_bool),
        Some(expected == ExitStatus::Ok)
    );

    if expected == ExitStatus::Ok {
        // 성공의 stderr는 JSON diagnostics(경고·정보)의 사람용 줄뿐입니다. 진단이 없으면 비어 있습니다.
        let diagnostics = doc
            .get("diagnostics")
            .and_then(JsonValue::as_array)
            .map_or(0, <[_]>::len);
        assert_eq!(
            run.stderr.lines().count(),
            diagnostics,
            "{fixture}: 성공 stderr는 진단 줄만: {}",
            run.stderr
        );
        for line in run.stderr.lines() {
            assert!(line.starts_with("stellar "), "{fixture}: {line}");
        }
    } else {
        let code = doc
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(JsonValue::as_str)
            .unwrap();
        assert!(
            run.stderr.contains(code),
            "{fixture}: stderr에 오류 코드 {code}: {}",
            run.stderr
        );
        // XXIV.4.1: stderr 진단 접두어는 실행 이름 `stellar`입니다.
        for line in run.stderr.lines() {
            assert!(
                line.starts_with("stellar ") && !line.starts_with("stellar-cli"),
                "{fixture}: stderr 접두어: {line}"
            );
        }
    }
    for path in private {
        let text = path.display().to_string();
        assert!(!run.stdout.contains(&text), "{fixture}: stdout에 경로 금지");
    }
}

#[test]
fn version_success_fixture() {
    check(&run(&["version"]), "version.json", ExitStatus::Ok, &[]);
    check(&run(&["--version"]), "version.json", ExitStatus::Ok, &[]);
}

#[test]
fn inspect_success_fixture() {
    let dir = TempDir::new("valid");
    let root = project(&dir, Some("valid.stellar.yaml"), true);
    let run = run(&["inspect", root.to_str().unwrap()]);
    check(&run, "inspect-valid.json", ExitStatus::Ok, &[&root]);
    // 의존성 repository URL은 결과에 넣지 않습니다.
    assert!(!run.stdout.contains("example.com"));
}

#[test]
fn inspect_domain_failure_fixtures_exit_1() {
    let dir = TempDir::new("no-manifest");
    let root = project(&dir, None, true);
    let run_result = run(&["inspect", root.to_str().unwrap()]);
    check(
        &run_result,
        "inspect-not-a-project.json",
        ExitStatus::Failure,
        &[&root],
    );
    assert!(
        run_result.stderr.contains(root.to_str().unwrap()),
        "경로는 stderr에만"
    );

    let dir = TempDir::new("no-git");
    let root = project(&dir, Some("valid.stellar.yaml"), false);
    check(
        &run(&["inspect", root.to_str().unwrap()]),
        "inspect-missing-git.json",
        ExitStatus::Failure,
        &[&root],
    );

    let dir = TempDir::new("file");
    let root = project(&dir, Some("valid.stellar.yaml"), true);
    let file = root.join("stellar.yaml");
    check(
        &run(&["inspect", file.to_str().unwrap()]),
        "inspect-not-a-directory.json",
        ExitStatus::Failure,
        &[&file],
    );
}

#[test]
fn inspect_data_error_fixtures_exit_65() {
    let dir = TempDir::new("corrupt");
    let root = project(&dir, Some("corrupt.stellar.yaml"), true);
    check(
        &run(&["inspect", root.to_str().unwrap()]),
        "inspect-corrupt-manifest.json",
        ExitStatus::DataErr,
        &[&root],
    );

    let dir = TempDir::new("unknown-key");
    let root = project(&dir, Some("unknown-key.stellar.yaml"), true);
    check(
        &run(&["inspect", root.to_str().unwrap()]),
        "inspect-unknown-key.json",
        ExitStatus::DataErr,
        &[&root],
    );
}

#[test]
fn inspect_missing_path_fixture_exit_66() {
    let dir = TempDir::new("missing");
    let missing = dir.path.join("does-not-exist");
    check(
        &run(&["inspect", missing.to_str().unwrap()]),
        "inspect-no-input.json",
        ExitStatus::NoInput,
        &[&missing],
    );
}

#[cfg(unix)]
#[test]
fn inspect_unreadable_manifest_fixture_exit_77() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("noperm");
    let root = project(&dir, Some("valid.stellar.yaml"), true);
    let manifest = root.join("stellar.yaml");
    std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&manifest).is_ok() {
        // 권한 검사를 우회하는 사용자(root)로 실행 중이면 77을 재현할 수 없습니다.
        eprintln!("권한 검사를 우회하는 사용자: 77 fixture 생략");
        return;
    }
    let run_result = run(&["inspect", root.to_str().unwrap()]);
    std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o644)).unwrap();
    check(
        &run_result,
        "inspect-no-permission.json",
        ExitStatus::NoPerm,
        &[&root],
    );
}

#[test]
fn usage_errors_exit_64_and_write_nothing_to_stdout() {
    for args in [
        &[][..],
        &["unknown"][..],
        &["inspect"][..],
        &["inspect", "a", "b"][..],
        &["inspect", "--bogus"][..],
        &["version", "extra"][..],
    ] {
        let run = run(args);
        assert_eq!(run.code, 64, "{args:?}");
        assert!(run.stdout.is_empty(), "{args:?}: stdout 비어 있음");
        assert!(run.stderr.contains("--help"), "{args:?}: {}", run.stderr);
    }
}

#[test]
fn schema_document_matches_code_contract() {
    let schema = schema();
    let required: Vec<&str> = schema
        .get("required")
        .and_then(JsonValue::as_array)
        .unwrap()
        .iter()
        .filter_map(JsonValue::as_str)
        .collect();
    assert_eq!(required, ENVELOPE_KEYS);
    let properties: Vec<&str> = schema
        .get("properties")
        .and_then(JsonValue::as_object)
        .unwrap()
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(properties, ENVELOPE_KEYS);

    let enum_of = |key: &str| -> Vec<JsonValue> {
        schema
            .get("properties")
            .and_then(|p| p.get(key))
            .and_then(|p| p.get("enum"))
            .and_then(JsonValue::as_array)
            .unwrap()
            .to_vec()
    };
    let codes: Vec<JsonValue> = ExitStatus::ALL
        .iter()
        .map(|s| JsonValue::int(i64::from(s.code())))
        .collect();
    assert_eq!(enum_of("exit_code"), codes);
    let names: Vec<JsonValue> = ExitStatus::ALL
        .iter()
        .map(|s| JsonValue::string(s.name()))
        .collect();
    assert_eq!(enum_of("exit_name"), names);
}

#[test]
fn schema_validator_rejects_contract_violations() {
    // 검사기 자체가 위반을 잡는지 확인합니다(빈 검사로 통과하는 것을 방지).
    let schema = schema();
    let golden = std::fs::read_to_string(Path::new(FIXTURES).join("results/version.json")).unwrap();
    let good = parse(golden.trim_end(), JsonLimits::default()).unwrap();
    assert!(validate(&good, &schema, "$").is_ok());

    let JsonValue::Object(pairs) = good.clone() else {
        unreachable!()
    };
    let mut extra = pairs.clone();
    extra.push(("log".into(), JsonValue::string("x")));
    let mut wrong_code = pairs.clone();
    wrong_code[4].1 = JsonValue::int(2);
    let mut missing = pairs.clone();
    missing.retain(|(k, _)| k != "diagnostics");
    for bad in [extra, wrong_code, missing] {
        assert!(validate(&JsonValue::Object(bad), &schema, "$").is_err());
    }
}

// --- XXIV.4.3: inspect --structure, query ---

use stellar_ops::test_support::{
    file_digest, large_document, random_valid_document, save_documents, scaffold_project,
    FixtureDocument,
};

/// fixture 문서 seed(node가 여러 개인 View를 가진 seed).
const QUERY_SEED: u64 = 7;

/// 저장된 generation을 가진 fixture 프로젝트(문서 하나).
fn saved_project(dir: &TempDir) -> (PathBuf, FixtureDocument) {
    let root = dir.path.join("project");
    scaffold_project(&root);
    let model = random_valid_document(QUERY_SEED);
    assert!(
        model.node_count() >= 8,
        "fixture node 수 {}",
        model.node_count()
    );
    save_documents(&root, &[&model]);
    (root, model)
}

fn result_of(run: &Run) -> JsonValue {
    // 대형 page(최대 10000 node) 결과도 읽도록 reader 상한을 넓힙니다.
    let limits = JsonLimits {
        max_bytes: 16 * 1024 * 1024,
        ..JsonLimits::default()
    };
    parse(run.stdout.trim_end(), limits)
        .unwrap()
        .get("result")
        .cloned()
        .unwrap()
}

fn first_root_frame(model: &FixtureDocument) -> String {
    model
        .definition()
        .pages
        .iter()
        .flat_map(|p| p.views.iter())
        .find(|v| model.contains(v.root_frame))
        .map(|v| v.root_frame.to_string())
        .unwrap()
}

#[test]
fn inspect_without_option_keeps_the_xxiv_1_2_result() {
    // 기존 inspect fixture(XXIV.1.2)는 저장된 generation이 있어도 byte 단위로 같습니다.
    let dir = TempDir::new("inspect-compat");
    let (root, _) = saved_project(&dir);
    std::fs::copy(
        Path::new(FIXTURES)
            .join("projects")
            .join("valid.stellar.yaml"),
        root.join("stellar.yaml"),
    )
    .unwrap();
    let run = run(&["inspect", root.to_str().unwrap()]);
    check(&run, "inspect-valid.json", ExitStatus::Ok, &[&root]);
}

#[test]
fn inspect_structure_and_query_success_fixtures() {
    let dir = TempDir::new("query");
    let (root, model) = saved_project(&dir);
    let path = root.to_str().unwrap();
    let document = model.document_id().to_string();
    let before = file_digest(&root);

    let structure = run(&["inspect", path, "--structure"]);
    check(
        &structure,
        "inspect-structure.json",
        ExitStatus::Ok,
        &[&root],
    );
    // 기존 inspect key가 같은 순서로 앞에 있고 새 key는 뒤에 붙습니다.
    let keys: Vec<String> = result_of(&structure)
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, _)| k.clone())
        .collect();
    assert_eq!(
        keys,
        [
            "manifest_schema_version",
            "project",
            "git_link",
            "target_profiles",
            "dependencies",
            "revision",
            "documents",
            "chunks"
        ]
    );

    check(
        &run(&["query", path, "project"]),
        "query-project.json",
        ExitStatus::Ok,
        &[&root],
    );
    check(
        &run(&["query", path, "tokens"]),
        "query-tokens.json",
        ExitStatus::Ok,
        &[&root],
    );
    check(
        &run(&["query", path, "components"]),
        "query-components.json",
        ExitStatus::Ok,
        &[&root],
    );
    let first = run(&["query", path, "tree", &document, "--limit", "3"]);
    check(&first, "query-tree-first.json", ExitStatus::Ok, &[&root]);
    let cursor = result_of(&first)
        .get("next_cursor")
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_string();
    let next = run(&[
        "query", path, "tree", &document, "--cursor", &cursor, "--limit", "3",
    ]);
    check(&next, "query-tree-next.json", ExitStatus::Ok, &[&root]);
    let node = first_root_frame(&model);
    check(
        &run(&["query", path, "node", &document, &node]),
        "query-node.json",
        ExitStatus::Ok,
        &[&root],
    );

    // 모든 결과의 revision은 같은 generation 표기입니다.
    let token = result_of(&structure)
        .get("revision")
        .and_then(|r| r.get("token"))
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_string();
    assert!(token.starts_with("g1:sha256:"), "{token}");
    assert_eq!(
        result_of(&first)
            .get("revision")
            .and_then(|r| r.get("token"))
            .and_then(JsonValue::as_str),
        Some(token.as_str())
    );
    // 실행 전후 프로젝트 파일 hash 동일(파일을 쓰지 않음).
    assert_eq!(file_digest(&root), before);
}

#[test]
fn query_error_fixtures() {
    let dir = TempDir::new("query-errors");
    let (root, model) = saved_project(&dir);
    let path = root.to_str().unwrap();
    let document = model.document_id().to_string();
    let missing = "0195c530-3acd-7bc1-8414-c213f61cb999";

    check(
        &run(&["query", path, "tree", missing]),
        "query-unknown-document.json",
        ExitStatus::NoInput,
        &[&root],
    );
    check(
        &run(&["query", path, "node", &document, missing]),
        "query-unknown-node.json",
        ExitStatus::NoInput,
        &[&root],
    );
    check(
        &run(&["query", path, "tree", &document, "--cursor", "t1.x"]),
        "query-invalid-cursor.json",
        ExitStatus::DataErr,
        &[&root],
    );
    // 다른 writer가 저장한 뒤의 낡은 cursor는 거부합니다.
    let first = run(&["query", path, "tree", &document, "--limit", "2"]);
    let cursor = result_of(&first)
        .get("next_cursor")
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_string();
    save_documents(&root, &[&model, &random_valid_document(QUERY_SEED + 1)]);
    check(
        &run(&["query", path, "tree", &document, "--cursor", &cursor]),
        "query-stale-cursor.json",
        ExitStatus::DataErr,
        &[&root],
    );
    // 저장된 generation이 없는 프로젝트는 g0 revision과 빈 구조를 돌려줍니다.
    let empty = TempDir::new("query-unsaved");
    let unsaved = empty.path.join("project");
    scaffold_project(&unsaved);
    check(
        &run(&["query", unsaved.to_str().unwrap(), "project"]),
        "query-project-unsaved.json",
        ExitStatus::Ok,
        &[&unsaved],
    );
}

#[test]
fn query_tree_splits_a_two_hundred_thousand_node_document() {
    // 검증 기준(XXIV.4.3): 20만 node 문서 page 분할(명령 경로). 전체 순회는 stellar-ops 단위
    // 테스트가 하고, 여기서는 첫 page와 마지막 page를 명령으로 확인합니다.
    let dir = TempDir::new("query-large");
    let root = dir.path.join("project");
    scaffold_project(&root);
    let model = large_document(200_000);
    save_documents(&root, &[&model]);
    let path = root.to_str().unwrap();
    let document = model.document_id().to_string();
    let first = run(&["query", path, "tree", &document, "--limit", "10000"]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let page = result_of(&first);
    assert_eq!(page.get("total").and_then(JsonValue::as_i64), Some(200_000));
    assert_eq!(
        page.get("nodes")
            .and_then(JsonValue::as_array)
            .map(<[_]>::len),
        Some(10_000)
    );
    let cursor = page.get("next_cursor").and_then(JsonValue::as_str).unwrap();
    assert!(cursor.starts_with("t1.10000."), "{cursor}");
    // 같은 revision의 cursor로 마지막 page로 이동합니다.
    let last_cursor = cursor.replacen("t1.10000.", "t1.195000.", 1);
    let last = run(&[
        "query",
        path,
        "tree",
        &document,
        "--cursor",
        &last_cursor,
        "--limit",
        "10000",
    ]);
    assert_eq!(last.code, 0, "{}", last.stderr);
    let page = result_of(&last);
    assert_eq!(
        page.get("nodes")
            .and_then(JsonValue::as_array)
            .map(<[_]>::len),
        Some(5_000)
    );
    assert_eq!(page.get("next_cursor"), Some(&JsonValue::Null));
    // 한 응답의 크기는 page 크기로 제한됩니다(전체 문서를 한 번에 보내지 않음).
    assert!(
        first.stdout.len() < 4 * 1024 * 1024,
        "{}",
        first.stdout.len()
    );
}

// --- XXIV.4.4: apply ---

/// 표준 입력을 주고 실행합니다. `home`이 있으면 그 경로만 `HOME`으로 둡니다(저장 잠금 위치).
fn run_with_stdin(args: &[&str], input: &str, home: Option<&Path>) -> Run {
    use std::io::Write as _;
    let mut command = Command::new(env!("CARGO_BIN_EXE_stellar"));
    command.args(args).env_clear();
    if let Some(home) = home {
        command.env("HOME", home);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn revision_token(root: &Path) -> String {
    let run = run(&["query", root.to_str().unwrap(), "project"]);
    result_of(&run)
        .get("revision")
        .and_then(|r| r.get("token"))
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_string()
}

/// 부모가 있는(View root가 아닌) 첫 node.
fn first_child(model: &FixtureDocument) -> String {
    let root = first_root_frame(model);
    let root: stellar_ops::core_types::ids::StableId = root.parse().unwrap();
    model.children_in_order(root)[0].to_string()
}

fn transaction(base: &str, document: &str, commands: &str) -> String {
    format!(
        r#"{{"format":"stellar-apply","format_version":1,"base_revision":"{base}","document":"{document}","commands":[{commands}]}}"#
    )
}

#[test]
fn apply_dry_run_and_write_fixtures() {
    let dir = TempDir::new("apply");
    let (root, model) = saved_project(&dir);
    let path = root.to_str().unwrap();
    let document = model.document_id().to_string();
    let node = first_child(&model);
    let base = revision_token(&root);
    let input = transaction(
        &base,
        &document,
        &format!(
            r#"{{"kind":"set-visible","node":"{node}","visible":false}},{{"kind":"rename","node":"{node}","name":"headless"}}"#
        ),
    );
    let file = dir.path.join("transaction.json");
    std::fs::write(&file, &input).unwrap();

    // dry-run: semantic diff만 출력하고 revision·파일은 그대로입니다.
    let before = file_digest(&root);
    let dry = run(&["apply", path, file.to_str().unwrap(), "--dry-run"]);
    check(&dry, "apply-dry-run.json", ExitStatus::Ok, &[&root, &file]);
    assert_eq!(file_digest(&root), before, "dry-run은 파일을 쓰지 않음");
    assert_eq!(revision_token(&root), base, "dry-run 전후 revision 동일");

    // 표준 입력으로 받은 같은 transaction을 세대 전환 저장으로 기록합니다.
    let home = dir.path.join("home");
    let write = run_with_stdin(&["apply", path, "-"], &input, Some(&home));
    check(&write, "apply-write.json", ExitStatus::Ok, &[&root]);
    let after = revision_token(&root);
    assert!(after.starts_with("g2:"), "{after}");
    assert_eq!(
        result_of(&write)
            .get("revision")
            .and_then(|r| r.get("token"))
            .and_then(JsonValue::as_str),
        Some(after.as_str())
    );
    // 적용 후 reopen한 node가 바뀐 값을 가집니다.
    let detail = result_of(&run(&["query", path, "node", &document, &node]));
    let node_json = detail.get("node").unwrap();
    assert_eq!(node_json.get("visible"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        node_json.get("name").and_then(JsonValue::as_str),
        Some("headless")
    );
    // dry-run과 write의 변경 목록이 같습니다.
    assert_eq!(
        result_of(&dry).get("changes"),
        result_of(&write).get("changes")
    );
}

#[test]
fn apply_rejection_fixtures_write_nothing() {
    let dir = TempDir::new("apply-errors");
    let (root, model) = saved_project(&dir);
    let path = root.to_str().unwrap();
    let document = model.document_id().to_string();
    let node = first_child(&model);
    let base = revision_token(&root);
    let before = file_digest(&root);
    let ok = format!(r#"{{"kind":"set-locked","node":"{node}","locked":true}}"#);

    let cases = [
        (
            transaction("g0", &document, &ok),
            "apply-base-mismatch.json",
            ExitStatus::DataErr,
        ),
        (
            transaction(
                &base,
                &document,
                &format!(r#"{ok},{{"kind":"run-script","source":"rm -rf /"}}"#),
            ),
            "apply-unknown-command.json",
            ExitStatus::DataErr,
        ),
        (
            transaction(
                &base,
                &document,
                &format!(
                    r#"{ok},{{"kind":"delete-subtree","root":"0195c530-3acd-7bc1-8414-c213f61cb999"}}"#
                ),
            ),
            "apply-rejected.json",
            ExitStatus::Failure,
        ),
        (
            transaction(
                &base,
                &document,
                &format!(r#"{{"kind":"set-locked","node":"{node}","locked":true,"extra":1}}"#),
            ),
            "apply-invalid-transaction.json",
            ExitStatus::DataErr,
        ),
    ];
    let home = dir.path.join("home");
    for (input, fixture, status) in cases {
        let result = run_with_stdin(&["apply", path, "-"], &input, Some(&home));
        check(&result, fixture, status, &[&root]);
    }
    assert_eq!(
        file_digest(&root),
        before,
        "거부된 apply는 파일을 쓰지 않음"
    );
    assert_eq!(revision_token(&root), base);
}

// --- XXIV.4.5: 교차 프로세스 저장 잠금 ---

/// `HOME`을 `home`으로 둔 `stellar`의 저장 잠금 디렉터리(macOS).
#[cfg(target_os = "macos")]
fn lock_dir(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Application Support")
        .join(stellar_ops::lock::APP_DATA_DIR)
        .join(stellar_ops::lock::LOCKS_DIR)
}

#[test]
fn apply_write_without_user_data_dir_is_config_error_and_dry_run_still_works() {
    let dir = TempDir::new("apply-no-home");
    let (root, model) = saved_project(&dir);
    let path = root.to_str().unwrap();
    let base = revision_token(&root);
    let node = first_child(&model);
    let input = transaction(
        &base,
        &model.document_id().to_string(),
        &format!(r#"{{"kind":"set-locked","node":"{node}","locked":true}}"#),
    );
    let before = file_digest(&root);
    check(
        &run_with_stdin(&["apply", path, "-"], &input, None),
        "apply-no-user-data-dir.json",
        ExitStatus::Config,
        &[&root],
    );
    let dry = run_with_stdin(&["apply", path, "-", "--dry-run"], &input, None);
    assert_eq!(dry.code, 0, "{}", dry.stderr);
    assert_eq!(file_digest(&root), before);
}

#[cfg(target_os = "macos")]
#[test]
fn apply_exits_75_while_another_writer_holds_the_save_lock() {
    // 검증 기준(XXIV.4.5): 잠금 경합 시 exit 75, 파일 변경 0.
    let dir = TempDir::new("apply-lock-busy");
    let (root, model) = saved_project(&dir);
    let path = root.to_str().unwrap();
    let home = dir.path.join("home");
    let base = revision_token(&root);
    let node = first_child(&model);
    let input = transaction(
        &base,
        &model.document_id().to_string(),
        &format!(r#"{{"kind":"set-locked","node":"{node}","locked":true}}"#),
    );
    let project_id = stellar_ops::test_support::project_id();
    let held =
        stellar_ops::lock::SaveLock::try_acquire(&lock_dir(&home), project_id, &root).unwrap();
    let before = file_digest(&root);
    check(
        &run_with_stdin(&["apply", path, "-"], &input, Some(&home)),
        "apply-lock-busy.json",
        ExitStatus::TempFail,
        &[&root, &home],
    );
    assert_eq!(file_digest(&root), before, "잠금 경합은 아무것도 쓰지 않음");
    drop(held);
    let ok = run_with_stdin(&["apply", path, "-"], &input, Some(&home));
    assert_eq!(ok.code, 0, "{}", ok.stderr);
    // 잠금 파일은 프로젝트 밖(OS 사용자 데이터)에만 있습니다.
    assert!(!file_digest(&root).keys().any(|k| k.ends_with(".lock")));
    assert_eq!(std::fs::read_dir(lock_dir(&home)).unwrap().count(), 1);
}

#[test]
fn concurrent_apply_processes_lose_no_update() {
    // 검증 기준(XXIV.4.5): 동시 쓰기 fixture 손실 0. 여러 `stellar apply` 프로세스가 서로 다른
    // node 이름을 바꾸고, 75(잠금 사용 중)·65 `base-revision-mismatch`(잠금 밖에서 읽은 base가
    // 낡음)면 최신 revision으로 다시 시도합니다. 잠금이 generation 비교와 교체를 감싸므로 계획 뒤
    // 교체(65 `concurrent-write`, V.5.2 optimistic check)는 생기면 안 되며, 생기면 실패입니다.
    let dir = TempDir::new("apply-concurrent");
    let (root, model) = saved_project(&dir);
    let home = dir.path.join("home");
    let document = model.document_id().to_string();
    let records = model.to_records().unwrap();
    let targets: Vec<String> = records
        .nodes
        .iter()
        .filter(|n| n.parent.is_some())
        .map(|n| n.id.to_string())
        .take(4)
        .collect();
    assert_eq!(targets.len(), 4);
    let mut retries = 0usize;
    std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let (root, home, document) = (&root, &home, &document);
                scope.spawn(move || {
                    let mut retried = 0usize;
                    for _ in 0..500 {
                        let base = revision_token(root);
                        let input = transaction(
                            &base,
                            document,
                            &format!(r#"{{"kind":"rename","node":"{node}","name":"proc-{i}"}}"#),
                        );
                        let result = run_with_stdin(
                            &["apply", root.to_str().unwrap(), "-"],
                            &input,
                            Some(home),
                        );
                        let error_code = (result.code != 0).then(|| {
                            parse(result.stdout.trim_end(), JsonLimits::default())
                                .unwrap()
                                .get("error")
                                .and_then(|e| e.get("code"))
                                .and_then(JsonValue::as_str)
                                .map(str::to_string)
                                .unwrap_or_default()
                        });
                        match (result.code, error_code.as_deref()) {
                            (0, _) => return retried,
                            (75, Some("save-lock-busy")) | (65, Some("base-revision-mismatch")) => {
                                retried += 1
                            }
                            (other, code) => {
                                panic!("예상하지 못한 종료 {other} {code:?}: {}", result.stderr)
                            }
                        }
                    }
                    panic!("writer {i}가 끝나지 못했습니다");
                })
            })
            .collect();
        for handle in handles {
            retries += handle.join().unwrap();
        }
    });
    let token = revision_token(&root);
    assert!(
        token.starts_with("g5:"),
        "writer 4개 → generation 5: {token} (재시도 {retries})"
    );
    for (i, node) in targets.iter().enumerate() {
        let detail = result_of(&run(&[
            "query",
            root.to_str().unwrap(),
            "node",
            &document,
            node,
        ]));
        assert_eq!(
            detail
                .get("node")
                .and_then(|n| n.get("name"))
                .and_then(JsonValue::as_str),
            Some(format!("proc-{i}").as_str()),
            "손실 0"
        );
    }
}

// --- XXIV.1.3: validate, semantic-diff, semantic-merge, migrate ---

/// 같은 문서로 저장한 세 사본(base·ours·theirs)과 문서.
fn three_projects(dir: &TempDir) -> (Vec<PathBuf>, FixtureDocument) {
    let roots: Vec<PathBuf> = ["base", "ours", "theirs"]
        .iter()
        .map(|name| {
            let root = dir.path.join(name);
            scaffold_project(&root);
            save_documents(&root, &[&random_valid_document(QUERY_SEED)]);
            root
        })
        .collect();
    (roots, random_valid_document(QUERY_SEED))
}

/// `stellar apply`로 command 하나를 적용합니다.
fn apply_one(root: &Path, home: &Path, document: &str, command: &str) {
    let input = transaction(&revision_token(root), document, command);
    let result = run_with_stdin(&["apply", root.to_str().unwrap(), "-"], &input, Some(home));
    assert_eq!(result.code, 0, "{}", result.stderr);
}

fn children(model: &FixtureDocument) -> Vec<String> {
    let root: stellar_ops::core_types::ids::StableId = first_root_frame(model).parse().unwrap();
    model
        .children_in_order(root)
        .iter()
        .map(|id| id.to_string())
        .collect()
}

#[test]
fn validate_corpus_fixtures() {
    // 유효 corpus: 문서 여러 개를 가진 프로젝트, 저장 전 프로젝트.
    let dir = TempDir::new("validate");
    let root = dir.path.join("project");
    scaffold_project(&root);
    let documents: Vec<FixtureDocument> = (0..3).map(|i| random_valid_document(300 + i)).collect();
    let refs: Vec<&FixtureDocument> = documents.iter().collect();
    save_documents(&root, &refs);
    let before = file_digest(&root);
    check(
        &run(&["validate", root.to_str().unwrap()]),
        "validate-valid.json",
        ExitStatus::Ok,
        &[&root],
    );
    assert_eq!(file_digest(&root), before, "validate는 파일을 쓰지 않음");
    let unsaved = dir.path.join("unsaved");
    scaffold_project(&unsaved);
    check(
        &run(&["validate", unsaved.to_str().unwrap()]),
        "validate-unsaved.json",
        ExitStatus::Ok,
        &[&unsaved],
    );
    // 손상 corpus: 문서 내용 chunk를 망가뜨리면 종료 코드 1이고 결과 object에 근거가 남습니다.
    assert!(stellar_ops::test_support::corrupt_view_tree_chunks(&root) > 0);
    let broken = run(&["validate", root.to_str().unwrap()]);
    check(
        &broken,
        "validate-corrupt.json",
        ExitStatus::Failure,
        &[&root],
    );
    let result = result_of(&broken);
    assert_eq!(result.get("valid"), Some(&JsonValue::Bool(false)));
    assert!(
        result
            .get("error_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0)
            >= 1
    );
}

#[test]
fn semantic_diff_and_merge_fixtures() {
    let dir = TempDir::new("semantic");
    let home = dir.path.join("home");
    let (roots, model) = three_projects(&dir);
    let document = model.document_id().to_string();
    let kids = children(&model);
    assert!(kids.len() >= 2);
    let [base, ours, theirs] = [&roots[0], &roots[1], &roots[2]];
    let s = |p: &PathBuf| p.to_str().unwrap().to_string();

    check(
        &run(&["semantic-diff", &s(base), &s(ours)]),
        "semantic-diff-none.json",
        ExitStatus::Ok,
        &[base, ours],
    );
    apply_one(
        ours,
        &home,
        &document,
        &format!(r#"{{"kind":"rename","node":"{}","name":"ours"}}"#, kids[0]),
    );
    apply_one(
        theirs,
        &home,
        &document,
        &format!(
            r#"{{"kind":"set-locked","node":"{}","locked":true}}"#,
            kids[1]
        ),
    );
    let digests: Vec<_> = roots.iter().map(|r| file_digest(r)).collect();
    check(
        &run(&["semantic-diff", &s(base), &s(ours)]),
        "semantic-diff.json",
        ExitStatus::Ok,
        &[base, ours],
    );
    check(
        &run(&["semantic-merge", &s(base), &s(ours), &s(theirs)]),
        "semantic-merge-clean.json",
        ExitStatus::Ok,
        &[base, ours, theirs],
    );
    // 같은 속성을 다른 값으로 바꾸면 미해결 conflict(종료 코드 1)이고 세 값이 결과에 남습니다.
    apply_one(
        theirs,
        &home,
        &document,
        &format!(
            r#"{{"kind":"rename","node":"{}","name":"theirs"}}"#,
            kids[0]
        ),
    );
    let digests_after: Vec<_> = roots.iter().map(|r| file_digest(r)).collect();
    let conflict = run(&["semantic-merge", &s(base), &s(ours), &s(theirs)]);
    check(
        &conflict,
        "semantic-merge-conflict.json",
        ExitStatus::Failure,
        &[base, ours, theirs],
    );
    let result = result_of(&conflict);
    assert_eq!(result.get("clean"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        result
            .get("conflicts")
            .and_then(JsonValue::as_array)
            .map(<[_]>::len),
        Some(1)
    );
    // diff·merge는 입력 프로젝트를 바꾸지 않습니다.
    assert_eq!(digests_after[0], digests[0]);
    assert_eq!(digests_after[1], digests[1]);
    let now: Vec<_> = roots.iter().map(|r| file_digest(r)).collect();
    assert_eq!(now, digests_after);
}

#[test]
fn migrate_corpus_fixtures() {
    let dir = TempDir::new("migrate");
    let home = dir.path.join("home");
    // 현재 스키마(1): --check와 실행 모두 최신, 파일 변경 0.
    let current = dir.path.join("current");
    scaffold_project(&current);
    save_documents(&current, &[&random_valid_document(QUERY_SEED)]);
    let before = file_digest(&current);
    check(
        &run(&["migrate", current.to_str().unwrap(), "--check"]),
        "migrate-check-up-to-date.json",
        ExitStatus::Ok,
        &[&current],
    );
    check(
        &run_with_stdin(&["migrate", current.to_str().unwrap()], "", Some(&home)),
        "migrate-up-to-date.json",
        ExitStatus::Ok,
        &[&current, &home],
    );
    assert_eq!(file_digest(&current), before);
    // 이 도구보다 새 스키마(2): 판정 65, 쓰기 거부, 파일 변경 0.
    let newer = dir.path.join("newer");
    scaffold_project(&newer);
    stellar_ops::test_support::save_documents_with_schema(
        &newer,
        &[&random_valid_document(QUERY_SEED)],
        2,
    );
    let before = file_digest(&newer);
    check(
        &run(&["migrate", newer.to_str().unwrap(), "--check"]),
        "migrate-check-newer.json",
        ExitStatus::DataErr,
        &[&newer],
    );
    check(
        &run_with_stdin(&["migrate", newer.to_str().unwrap()], "", Some(&home)),
        "migrate-newer.json",
        ExitStatus::DataErr,
        &[&newer, &home],
    );
    assert_eq!(file_digest(&newer), before);
    // 저장 전 프로젝트: 할 일 없음.
    let unsaved = dir.path.join("unsaved");
    scaffold_project(&unsaved);
    check(
        &run(&["migrate", unsaved.to_str().unwrap(), "--check"]),
        "migrate-check-unsaved.json",
        ExitStatus::Ok,
        &[&unsaved],
    );
}

// --- XXIV.3.1: CI mode ---

#[test]
fn ci_mode_fixtures() {
    let dir = TempDir::new("ci");
    // 통과: 저장된 유효 프로젝트.
    let (root, _) = saved_project(&dir);
    let before = file_digest(&root);
    check(
        &run(&["ci", root.to_str().unwrap()]),
        "ci-pass.json",
        ExitStatus::Ok,
        &[&root],
    );
    assert_eq!(file_digest(&root), before, "ci는 파일을 쓰지 않음");

    // 실패: 손상 문서(validation) + 새 스키마(migration).
    let broken = dir.path.join("broken");
    scaffold_project(&broken);
    stellar_ops::test_support::save_documents_with_schema(
        &broken,
        &[&random_valid_document(QUERY_SEED)],
        2,
    );
    assert!(stellar_ops::test_support::corrupt_view_tree_chunks(&broken) > 0);
    let failed = run(&["ci", broken.to_str().unwrap()]);
    check(&failed, "ci-invalid.json", ExitStatus::Failure, &[&broken]);
    let checks = result_of(&failed);
    let statuses: Vec<(&str, &str)> = checks
        .get("checks")
        .and_then(JsonValue::as_array)
        .unwrap()
        .iter()
        .map(|c| {
            (
                c.get("name").and_then(JsonValue::as_str).unwrap(),
                c.get("status").and_then(JsonValue::as_str).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        statuses,
        [
            ("lockfile", "pass"),
            ("validation", "fail"),
            ("migration", "fail")
        ]
    );

    // 실패: 의존성이 선언됐지만 lock entry가 없음.
    let deps = dir.path.join("deps");
    scaffold_project(&deps);
    std::fs::copy(
        Path::new(FIXTURES)
            .join("projects")
            .join("valid.stellar.yaml"),
        deps.join("stellar.yaml"),
    )
    .unwrap();
    let lock = run(&["ci", deps.to_str().unwrap()]);
    check(&lock, "ci-lockfile.json", ExitStatus::Failure, &[&deps]);
    assert!(
        !lock.stdout.contains("example.com"),
        "repository URL 미포함"
    );

    // 프로젝트가 아니면(손상된 매니페스트) CI도 실패합니다.
    let corrupt = dir.path.join("corrupt");
    scaffold_project(&corrupt);
    std::fs::copy(
        Path::new(FIXTURES)
            .join("projects")
            .join("corrupt.stellar.yaml"),
        corrupt.join("stellar.yaml"),
    )
    .unwrap();
    let invalid = run(&["ci", corrupt.to_str().unwrap()]);
    assert_eq!(invalid.code, 65, "{}", invalid.stderr);
}

// --- XXIV.3.2: diff-summary artifact ---

#[test]
fn diff_summary_redacts_project_text_names_and_paths_by_default() {
    // 검증 기준(XXIV.3.2): privacy fixture.
    let dir = TempDir::new("summary-privacy-secret-dir");
    let home = dir.path.join("home");
    let (roots, model) = three_projects(&dir);
    let [base, target] = [&roots[0], &roots[1]];
    for root in [base, target] {
        let manifest = root.join("stellar.yaml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        assert!(text.contains("name: Fixture"));
        std::fs::write(
            &manifest,
            text.replace("name: Fixture", "name: SECRET-PROJECT"),
        )
        .unwrap();
    }
    let document = model.document_id().to_string();
    let kids = children(&model);
    let original_name = model.record(kids[0].parse().unwrap()).unwrap().display_name;
    assert!(!original_name.is_empty());
    apply_one(
        target,
        &home,
        &document,
        &format!(
            r#"{{"kind":"rename","node":"{}","name":"SECRET-RENAME"}},{{"kind":"set-visible","node":"{}","visible":false}}"#,
            kids[0], kids[0]
        ),
    );
    let secrets = [
        "SECRET-RENAME",
        "SECRET-PROJECT",
        original_name.as_str(),
        "summary-privacy-secret-dir",
    ];
    let (b, t) = (base.to_str().unwrap(), target.to_str().unwrap());

    // 기본(redaction): 값의 종류만 남고 프로젝트 텍스트·이름·경로가 없습니다.
    let redacted = run(&["diff-summary", b, t]);
    check(
        &redacted,
        "diff-summary-redacted.json",
        ExitStatus::Ok,
        &[base, target],
    );
    for secret in secrets {
        assert!(!redacted.stdout.contains(secret), "stdout에 {secret:?}");
    }
    let summary = result_of(&redacted);
    assert_eq!(
        summary.get("redaction").and_then(JsonValue::as_str),
        Some("project-content")
    );
    assert_eq!(
        summary.get("redacted_values").and_then(JsonValue::as_i64),
        Some(2)
    );

    // artifact 파일: 같은 내용, stdout에는 크기·hash만.
    let artifacts = TempDir::new("summary-artifacts");
    let output = artifacts.path.join("diff-summary.json");
    let written = run(&["diff-summary", b, t, "--output", output.to_str().unwrap()]);
    assert_eq!(written.code, 0, "{}", written.stderr);
    assert!(
        !written.stdout.contains("diff-summary.json"),
        "stdout에 경로 금지"
    );
    let artifact = std::fs::read_to_string(&output).unwrap();
    for secret in secrets {
        assert!(!artifact.contains(secret), "artifact에 {secret:?}");
    }
    let artifact_doc = parse(artifact.trim_end(), JsonLimits::default()).unwrap();
    assert_eq!(&artifact_doc, &summary, "artifact = stdout 요약");
    assert_eq!(
        result_of(&written)
            .get("artifact")
            .and_then(|a| a.get("hash"))
            .and_then(JsonValue::as_str)
            .map(str::to_string),
        Some(stellar_ops::summary::artifact_hash(artifact.as_bytes()))
    );

    // 명시적 --include-content만 값을 포함합니다(경로·프로젝트 이름은 여전히 없음).
    let plain = run(&["diff-summary", b, t, "--include-content"]);
    assert_eq!(plain.code, 0, "{}", plain.stderr);
    assert!(plain.stdout.contains("SECRET-RENAME"));
    assert!(!plain.stdout.contains("SECRET-PROJECT"));
    assert!(!plain.stdout.contains("summary-privacy-secret-dir"));

    // 입력 프로젝트 안에는 artifact를 쓰지 않습니다.
    let before = file_digest(base);
    let inside = base.join("diff.json");
    let refused = run(&["diff-summary", b, t, "--output", inside.to_str().unwrap()]);
    assert_eq!(refused.code, 73, "{}", refused.stderr);
    assert!(refused.stdout.contains("output-inside-project"));
    assert_eq!(file_digest(base), before);
}
