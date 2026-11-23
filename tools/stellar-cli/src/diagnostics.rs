//! `diagnostics`: 로컬 진단 store의 전송 전 preview, 수동 번들 내보내기, 금지 필드 검사 (XVI.8, ADR-0109).
//!
//! 모든 판정은 `stellar_diagnostics`(store·bundle·scanner·consent)가 합니다. 이 module은 인자를 해석하고 결과를
//! envelope JSON으로 옮길 뿐, 형식·검사 규칙을 따로 두지 않습니다.
//!
//! - 진단 디렉터리 기본값은 `<OS 사용자 데이터>/xyz.taeyoon.stellar/diagnostics`이고 `--data-dir`로 바꿉니다.
//! - `--project <경로>`는 그 경로와 폴더 이름을 scanner 문맥 값으로 더합니다(앱이 열린 프로젝트를 넘기는 것과 같음).
//! - 결과 JSON에는 경로를 넣지 않습니다(경로는 stderr context).
//! - 네트워크 전송은 하지 않습니다.

use std::path::{Path, PathBuf};

use stellar_diagnostics::bundle::{
    export_bundle, preview_payload, BundleEnvironment, PayloadPreview,
};
use stellar_diagnostics::consent::{read_consent, ConsentLoad, CONSENT_FILE};
use stellar_diagnostics::scanner::{scan_bundle, ForbiddenCategory, ScanContext, ScanReport};
use stellar_diagnostics::store::{
    DiagnosticStore, StoreError, Version3, DIAGNOSTICS_DIR, MAX_DIAGNOSTIC_BYTES, STORE_FILE,
};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::views;

const USAGE: &str = "diagnostics에는 preview | export --output <파일> | scan <파일> 중 하나와 선택 [--data-dir <디렉터리>] [--project <경로>]...가 필요합니다";

/// 공통 옵션.
#[derive(Default)]
struct Options {
    data_dir: Option<PathBuf>,
    projects: Vec<PathBuf>,
    output: Option<PathBuf>,
    positional: Vec<String>,
}

fn parse_options(args: &[String], allow_output: bool) -> Result<Options, CliError> {
    let mut options = Options::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| {
            iter.next()
                .map(PathBuf::from)
                .ok_or_else(|| CliError::usage(format!("{name}에는 값이 필요합니다")))
        };
        match arg.as_str() {
            "--data-dir" if options.data_dir.is_none() => {
                options.data_dir = Some(value("--data-dir")?);
            }
            "--project" => options.projects.push(value("--project")?),
            "--output" if allow_output && options.output.is_none() => {
                options.output = Some(value("--output")?);
            }
            "--data-dir" | "--output" => {
                return Err(CliError::usage(format!("{arg}는 한 번만 줄 수 있습니다")))
            }
            option if option.starts_with('-') && option != "-" => {
                return Err(CliError::usage(format!("알 수 없는 옵션: {option}")))
            }
            positional => options.positional.push(positional.to_owned()),
        }
    }
    Ok(options)
}

/// 진단 디렉터리. `--data-dir`가 없으면 OS 사용자 데이터의 앱 디렉터리 아래입니다.
fn data_dir(options: &Options) -> Result<PathBuf, CliError> {
    if let Some(dir) = &options.data_dir {
        return Ok(dir.clone());
    }
    stellar_ops::trusted::os_trust_data_dir()
        .map(|dir| dir.join(DIAGNOSTICS_DIR))
        .ok_or_else(|| {
            CliError::new(
                ExitStatus::Unavailable,
                "no-data-dir",
                "OS 사용자 데이터 위치를 찾지 못했습니다. --data-dir로 진단 디렉터리를 주세요",
            )
        })
}

/// `--project` 경로를 문맥 값으로 더합니다.
fn scan_context(options: &Options) -> ScanContext {
    let mut ctx = ScanContext::new();
    for project in &options.projects {
        let mut paths = vec![project.clone()];
        if let Ok(canonical) = std::fs::canonicalize(project) {
            paths.push(canonical);
        }
        for path in paths {
            if let Some(text) = path.to_str() {
                ctx.add(ForbiddenCategory::ProjectPath, text);
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                ctx.add(ForbiddenCategory::ProjectName, name);
            }
        }
    }
    ctx
}

fn store_error(error: &StoreError, context: &Path) -> CliError {
    let status = match error {
        StoreError::Io(kind) => ExitStatus::from_io_kind(*kind),
        StoreError::TooLarge | StoreError::Encoding | StoreError::Line(_) => ExitStatus::DataErr,
    };
    CliError::new(status, "diagnostics-unreadable", error.to_string())
        .with_context(context.display().to_string())
}

/// 이 도구의 version(`stellar version`과 같은 값).
fn app_version() -> Version3 {
    Version3::parse(env!("CARGO_PKG_VERSION")).unwrap_or_default()
}

fn findings_json(report: &ScanReport) -> JsonValue {
    JsonValue::Array(
        report
            .findings
            .iter()
            .map(|f| {
                JsonValue::object(vec![
                    ("line", views::count(f.line)),
                    ("category", JsonValue::string(f.category.name())),
                    ("rule", JsonValue::string(f.rule)),
                ])
            })
            .collect(),
    )
}

fn forbidden(report: &ScanReport, message: String) -> CliError {
    CliError::new(ExitStatus::Failure, "forbidden-content", message).with_result(JsonValue::object(
        vec![
            ("clean", JsonValue::Bool(false)),
            ("findings", findings_json(report)),
        ],
    ))
}

/// store를 읽어 preview를 만듭니다.
fn build_preview(options: &Options) -> Result<(PathBuf, PayloadPreview), CliError> {
    let dir = data_dir(options)?;
    let store_path = dir.join(STORE_FILE);
    let store =
        DiagnosticStore::read_file(&store_path).map_err(|e| store_error(&e, &store_path))?;
    let preview = preview_payload(
        &BundleEnvironment::current(app_version()),
        &store,
        &scan_context(options),
    )
    .map_err(|e| forbidden(&e.report, e.to_string()))?;
    Ok((dir, preview))
}

fn preview_json(preview: &PayloadPreview) -> Vec<(&'static str, JsonValue)> {
    vec![
        ("records", views::count(preview.records())),
        ("bytes", views::count(preview.len())),
        ("payload", JsonValue::string(preview.text())),
    ]
}

/// `diagnostics preview`: 전송·내보내기될 bytes를 그대로 보여 줍니다.
fn preview(args: &[String]) -> Result<Success, CliError> {
    let options = parse_options(args, false)?;
    if let Some(extra) = options.positional.first() {
        return Err(CliError::usage(format!(
            "preview는 위치 인자를 받지 않습니다: {extra}"
        )));
    }
    let (dir, preview) = build_preview(&options)?;
    let consent = read_consent(&dir.join(CONSENT_FILE));
    let mut result = vec![
        ("consent", JsonValue::string(consent.record().state.name())),
        (
            "consent_file",
            JsonValue::string(match consent {
                ConsentLoad::Missing => "missing",
                ConsentLoad::Loaded(_) => "loaded",
                ConsentLoad::Unreadable => "unreadable",
            }),
        ),
        (
            "automatic_upload",
            JsonValue::Bool(consent.record().state.allows_automatic_upload()),
        ),
    ];
    result.extend(preview_json(&preview));
    Ok(Success {
        result: JsonValue::object(result),
        diagnostics: Vec::new(),
    })
}

/// `diagnostics export --output <파일>`: preview bytes를 새 파일로 내보냅니다.
fn export(args: &[String]) -> Result<Success, CliError> {
    let options = parse_options(args, true)?;
    if let Some(extra) = options.positional.first() {
        return Err(CliError::usage(format!(
            "export는 위치 인자를 받지 않습니다: {extra}"
        )));
    }
    let output = options
        .output
        .clone()
        .ok_or_else(|| CliError::usage("export에는 --output <파일>이 필요합니다"))?;
    let (_, preview) = build_preview(&options)?;
    export_bundle(&preview, &output).map_err(|e| {
        CliError::new(
            match e.kind() {
                std::io::ErrorKind::AlreadyExists => ExitStatus::CantCreate,
                kind => ExitStatus::from_io_kind(kind),
            },
            "export-failed",
            format!("진단 번들을 쓰지 못했습니다: {}", e.kind()),
        )
        .with_context(output.display().to_string())
    })?;
    Ok(Success {
        result: JsonValue::object(preview_json(&preview)),
        diagnostics: Vec::new(),
    })
}

/// `diagnostics scan <파일>`: 파일을 진단 번들로 검사합니다(탐지가 있으면 종료 코드 1).
fn scan(args: &[String]) -> Result<Success, CliError> {
    let options = parse_options(args, false)?;
    if options.data_dir.is_some() {
        return Err(CliError::usage("scan은 --data-dir를 받지 않습니다"));
    }
    let [path] = options.positional.as_slice() else {
        return Err(CliError::usage("scan에는 <파일> 하나가 필요합니다"));
    };
    let path = Path::new(path);
    let bytes = read_input(path)?;
    let report = scan_bundle(&bytes, &scan_context(&options));
    if !report.is_clean() {
        return Err(forbidden(
            &report,
            format!(
                "수집 금지 정보 또는 schema 밖 내용 {}건",
                report.findings.len()
            ),
        ));
    }
    Ok(Success {
        result: JsonValue::object(vec![
            ("clean", JsonValue::Bool(true)),
            ("findings", findings_json(&report)),
        ]),
        diagnostics: Vec::new(),
    })
}

fn read_input(path: &Path) -> Result<Vec<u8>, CliError> {
    use std::io::Read as _;
    let io_error = |e: std::io::Error| {
        CliError::new(
            ExitStatus::from_io_kind(e.kind()),
            "input-unreadable",
            format!("검사할 파일을 읽지 못했습니다: {}", e.kind()),
        )
        .with_context(path.display().to_string())
    };
    let file = std::fs::File::open(path).map_err(io_error)?;
    let mut bytes = Vec::new();
    file.take(MAX_DIAGNOSTIC_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    Ok(bytes)
}

/// `diagnostics <하위 명령> ...`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(CliError::usage(USAGE));
    };
    match sub.as_str() {
        "preview" => preview(rest),
        "export" => export(rest),
        "scan" => scan(rest),
        other => Err(CliError::usage(format!(
            "알 수 없는 diagnostics 하위 명령: {other}. {USAGE}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;
    use stellar_diagnostics::bundle::encode_bundle;
    use stellar_diagnostics::store::{
        AppCrash, CrashKind, Feature, RendererBackend, RendererError, RendererStage, ThreadRole,
    };

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Dir {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "stellar-cli-diag-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Dir(path)
        }

        fn arg(&self) -> String {
            self.0.to_str().unwrap().to_owned()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(super) fn cli(args: &[&str]) -> (i32, JsonValue, String) {
        let mut all = vec!["diagnostics".to_owned()];
        all.extend(args.iter().map(|a| a.to_string()));
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = crate::cli::run(&all, &mut out, &mut err);
        let text = String::from_utf8(out).unwrap();
        let doc = if text.is_empty() {
            JsonValue::Null
        } else {
            json::parse(text.trim_end(), json::JsonLimits::default()).unwrap()
        };
        (code, doc, String::from_utf8(err).unwrap())
    }

    fn result(doc: &JsonValue) -> &JsonValue {
        doc.get("result").unwrap()
    }

    fn write_store(dir: &Dir) -> DiagnosticStore {
        let mut store = DiagnosticStore::default();
        store.push_crash(AppCrash {
            at_ms: 1_763_856_000_000,
            kind: CrashKind::Panic,
            code: 0,
            thread: ThreadRole::Document,
            uptime_ms: 5_000,
        });
        store.push_renderer_error(RendererError {
            at_ms: 1_763_856_000_100,
            backend: RendererBackend::Metal,
            stage: RendererStage::Present,
            code: 3,
        });
        store.record_feature_failure(Feature::Save, 2, 1_763_856_000_200);
        store.write_file(&dir.0.join(STORE_FILE)).unwrap();
        store
    }

    #[test]
    fn preview_shows_the_exported_bytes() {
        // 검증 기준(XVI.8.4): preview bytes = 내보낸 bytes = core 번들 bytes.
        let dir = Dir::new("preview");
        let store = write_store(&dir);
        let (code, doc, err) = cli(&["preview", "--data-dir", &dir.arg()]);
        assert_eq!(code, 0, "{err}");
        let payload = result(&doc)
            .get("payload")
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            payload.as_bytes(),
            encode_bundle(&BundleEnvironment::current(app_version()), &store).as_slice()
        );
        assert_eq!(result(&doc).get("records").unwrap().as_i64(), Some(3));
        assert_eq!(
            result(&doc).get("consent").unwrap().as_str(),
            Some("not_asked")
        );
        assert_eq!(
            result(&doc).get("automatic_upload").unwrap().as_bool(),
            Some(false)
        );
        let out = dir.0.join("bundle.txt");
        let (code, doc, err) = cli(&[
            "export",
            "--data-dir",
            &dir.arg(),
            "--output",
            out.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(std::fs::read(&out).unwrap(), payload.as_bytes());
        assert_eq!(
            result(&doc).get("payload").unwrap().as_str(),
            Some(payload.as_str())
        );
        // 결과 JSON에 경로가 없습니다.
        assert!(!json::write(&doc).contains(&dir.arg()));
        // 같은 파일로 다시 내보내면 덮어쓰지 않습니다(73).
        let (code, _, _) = cli(&[
            "export",
            "--data-dir",
            &dir.arg(),
            "--output",
            out.to_str().unwrap(),
        ]);
        assert_eq!(code, 73);
        // 내보낸 번들은 scanner를 통과합니다.
        let (code, doc, err) = cli(&["scan", out.to_str().unwrap()]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(result(&doc).get("clean").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn empty_store_previews_an_empty_bundle() {
        let dir = Dir::new("empty");
        let (code, doc, err) = cli(&["preview", "--data-dir", &dir.arg()]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(result(&doc).get("records").unwrap().as_i64(), Some(0));
        // preview는 아무 파일도 만들지 않습니다.
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
    }

    #[test]
    fn scan_reports_findings_without_echo() {
        let dir = Dir::new("scan");
        write_store(&dir);
        let out = dir.0.join("bundle.txt");
        let (code, _, _) = cli(&[
            "export",
            "--data-dir",
            &dir.arg(),
            "--output",
            out.to_str().unwrap(),
        ]);
        assert_eq!(code, 0);
        let tampered = std::fs::read_to_string(&out)
            .unwrap()
            .replace("thread=document", "thread=/Users/alice/acme-site")
            .replace("end records", "acme-site\nend records");
        let bad = dir.0.join("bad.txt");
        std::fs::write(&bad, &tampered).unwrap();
        let project = dir.0.join("acme-site");
        let (code, doc, _) = cli(&[
            "scan",
            bad.to_str().unwrap(),
            "--project",
            project.to_str().unwrap(),
        ]);
        assert_eq!(code, 1);
        let findings = result(&doc).get("findings").unwrap().as_array().unwrap();
        let categories: Vec<&str> = findings
            .iter()
            .map(|f| f.get("category").unwrap().as_str().unwrap())
            .collect();
        assert!(categories.contains(&"project_path"), "{categories:?}");
        assert!(categories.contains(&"project_name"), "{categories:?}");
        assert!(categories.contains(&"unrecognized"), "{categories:?}");
        let text = json::write(&doc);
        assert!(!text.contains("alice") && !text.contains("acme"), "{text}");
    }

    #[test]
    fn corrupt_store_and_usage_errors() {
        let dir = Dir::new("corrupt");
        std::fs::write(
            dir.0.join(STORE_FILE),
            "stellar.diagnostic-store v1\nmeta x\n",
        )
        .unwrap();
        let (code, doc, _) = cli(&["preview", "--data-dir", &dir.arg()]);
        assert_eq!(code, 65);
        assert_eq!(
            doc.get("error")
                .and_then(|e| e.get("code"))
                .and_then(JsonValue::as_str),
            Some("diagnostics-unreadable")
        );
        let (code, _, _) = cli(&["scan", dir.0.join("missing").to_str().unwrap()]);
        assert_eq!(code, 66);
        for args in [
            &[][..],
            &["frobnicate"][..],
            &["preview", "extra"][..],
            &["preview", "--output", "x"][..],
            &["preview", "--data-dir"][..],
            &["preview", "--data-dir", "a", "--data-dir", "b"][..],
            &["export"][..],
            &["export", "--output", "a", "--output", "b"][..],
            &["scan"][..],
            &["scan", "a", "b"][..],
            &["scan", "a", "--data-dir", "d"][..],
            &["scan", "a", "--bogus"][..],
        ] {
            let (code, doc, err) = cli(args);
            assert_eq!(code, 64, "{args:?}");
            assert_eq!(doc, JsonValue::Null, "{args:?}");
            assert!(err.contains("--help"), "{args:?}");
        }
    }
}
