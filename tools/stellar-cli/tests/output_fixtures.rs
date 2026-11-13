//! XXIV.1.2 검증: 종료 코드·JSON 출력 스키마 success/error fixture.
//!
//! 빌드된 실행 파일을 headless(환경 변수 없음)로 실행하고 다음을 확인합니다.
//!
//! - 종료 코드가 fixture 기대값과 같고 JSON `exit_code`와도 같습니다.
//! - stdout은 golden fixture(`tests/fixtures/results/*.json`)와 byte 단위로 같고, JSON 한 줄뿐입니다.
//! - stdout JSON은 엄격한 reader로 파싱되고 `schema/cli-result.v1.schema.json`을 만족합니다.
//! - 성공은 stderr가 비어 있고, 오류는 stderr에 사람용 진단(오류 코드 포함)이 있습니다.
//! - stdout에는 입력 경로가 들어가지 않습니다(경로는 stderr에만).

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
    assert_eq!(run.code, i32::from(expected.code()), "{fixture}: 종료 코드");
    let golden =
        std::fs::read_to_string(Path::new(FIXTURES).join("results").join(fixture)).unwrap();
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
        assert!(run.stderr.is_empty(), "{fixture}: 성공은 stderr 없음");
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
