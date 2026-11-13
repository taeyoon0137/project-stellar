//! 명령 구현 (XXIV.1.2).
//!
//! 각 명령은 [`Success`] 또는 [`CliError`]를 반환하고, 출력 형식은 [`crate::output`]이
//! 결정합니다. 명령은 stdout·stderr에 직접 쓰지 않습니다.

use std::path::Path;

use stellar_project::probe::{probe_project, ProbeError, ProjectProbe};
use stellar_project::root::{GitLink, RootError};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Diagnostic, Severity, Success};

/// `version`: 도구 이름·버전과 출력 스키마 버전.
pub fn version(args: &[String]) -> Result<Success, CliError> {
    if let Some(extra) = args.first() {
        return Err(CliError::usage(format!(
            "version은 인자를 받지 않습니다: {extra}"
        )));
    }
    Ok(Success {
        result: JsonValue::object(vec![
            ("name", JsonValue::string(env!("CARGO_PKG_NAME"))),
            ("version", JsonValue::string(env!("CARGO_PKG_VERSION"))),
            (
                "output_schema_version",
                JsonValue::int(crate::output::SCHEMA_VERSION),
            ),
        ]),
        diagnostics: Vec::new(),
    })
}

/// `inspect <경로>`: 프로젝트 root와 `stellar.yaml`을 읽기 전용으로 검사합니다.
///
/// 문서 청크는 읽지 않고 파일을 만들거나 고치지 않습니다(IV.5.1 probe). 결과 JSON에는 경로와
/// 의존성 repository URL을 넣지 않습니다.
pub fn inspect(args: &[String]) -> Result<Success, CliError> {
    let path = match args {
        [] => return Err(CliError::usage("inspect에는 프로젝트 경로가 필요합니다")),
        [p] if p.starts_with('-') && p != "-" => {
            return Err(CliError::usage(format!("알 수 없는 옵션: {p}")))
        }
        [p] => Path::new(p),
        [_, extra, ..] => {
            return Err(CliError::usage(format!(
                "inspect는 경로 하나만 받습니다: {extra}"
            )))
        }
    };
    let context = path.display().to_string();

    let metadata = std::fs::metadata(path).map_err(|e| {
        let status = ExitStatus::from_io_kind(e.kind());
        let (code, message) = match status {
            ExitStatus::NoInput => ("no-input", "입력 경로가 없습니다"),
            ExitStatus::NoPerm => ("no-permission", "입력 경로에 접근할 권한이 없습니다"),
            _ => ("io-error", "입력 경로를 읽을 수 없습니다"),
        };
        CliError::new(status, code, message).with_context(context.clone())
    })?;
    if !metadata.is_dir() {
        return Err(
            not_a_project("not-a-directory", "입력 경로가 디렉터리가 아닙니다")
                .with_context(context),
        );
    }

    let probe = probe_project(path).map_err(|e| probe_error(e).with_context(context))?;
    Ok(Success {
        result: probe_json(&probe),
        diagnostics: Vec::new(),
    })
}

fn not_a_project(diagnostic: &'static str, message: &str) -> CliError {
    CliError::new(
        ExitStatus::Failure,
        "not-a-stellar-project",
        "Stellar 프로젝트 root가 아닙니다",
    )
    .with_diagnostic(Diagnostic {
        severity: Severity::Error,
        code: diagnostic,
        message: message.to_string(),
    })
}

fn probe_error(error: ProbeError) -> CliError {
    match error {
        ProbeError::Root(RootError::MissingManifest) => {
            not_a_project("missing-manifest", &RootError::MissingManifest.to_string())
        }
        ProbeError::Root(RootError::MissingGit) => {
            not_a_project("missing-git", &RootError::MissingGit.to_string())
        }
        ProbeError::ManifestRead { kind } => CliError::new(
            ExitStatus::from_io_kind(kind),
            "manifest-read",
            format!("stellar.yaml을 읽을 수 없습니다: {kind:?}"),
        ),
        ProbeError::ManifestSyntax(e) => CliError::new(
            ExitStatus::DataErr,
            "manifest-syntax",
            "stellar.yaml YAML 문법 오류",
        )
        .with_diagnostic(Diagnostic {
            severity: Severity::Error,
            code: e.kind.code(),
            message: format!("{}행 {}열: {}", e.line, e.col, e.message),
        }),
        ProbeError::ManifestSchema(e) => CliError::new(
            ExitStatus::DataErr,
            "manifest-schema",
            "stellar.yaml 스키마 오류",
        )
        .with_diagnostic(Diagnostic {
            severity: Severity::Error,
            code: "schema",
            message: e.to_string(),
        }),
    }
}

fn probe_json(probe: &ProjectProbe) -> JsonValue {
    let manifest = &probe.manifest;
    let git_link = match probe.git_link {
        GitLink::Directory => "directory",
        GitLink::GitFile => "gitfile",
    };
    JsonValue::object(vec![
        (
            "manifest_schema_version",
            JsonValue::int(i64::from(manifest.schema_version)),
        ),
        (
            "project",
            JsonValue::object(vec![
                ("id", JsonValue::string(manifest.project.id.clone())),
                ("name", JsonValue::string(manifest.project.name.clone())),
            ]),
        ),
        ("git_link", JsonValue::string(git_link)),
        (
            "target_profiles",
            JsonValue::Array(
                manifest
                    .target_profiles
                    .iter()
                    .map(|p| {
                        JsonValue::object(vec![
                            ("id", JsonValue::string(p.id.clone())),
                            ("platform", JsonValue::string(p.platform.clone())),
                            ("minimum_os", JsonValue::opt_string(p.minimum_os.as_deref())),
                            ("baseline", JsonValue::opt_string(p.baseline.as_deref())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "dependencies",
            JsonValue::Array(
                manifest
                    .dependencies
                    .iter()
                    .map(|d| {
                        JsonValue::object(vec![
                            ("name", JsonValue::string(d.name.as_str())),
                            ("source", JsonValue::string(d.source.as_str())),
                            ("version", JsonValue::string(d.version.value())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}
