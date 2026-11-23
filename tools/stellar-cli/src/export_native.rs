//! `export swiftui|appkit|compose`: 저장 View(또는 그 안 Frame 하나)를 SwiftUI·AppKit·Compose 소스로 JSON 결과에
//! 담습니다(XXII.3·XXII.4, ADR-0107 결정 8).
//!
//! 구조·배치·layout 값은 `export-web`과 같은 투영을 쓰고(`stellar_ops::export_native::export_native`), 생성은
//! `stellar-codegen` adapter가 맡습니다. 파일을 쓰지 않습니다.

use std::path::Path;

use stellar_ops::core_types::ids::StableId;
use stellar_ops::export_native::{export_native, NativeTarget};
use stellar_ops::snapshot::open_snapshot;
use stellar_ops::web_frame::HtmlDiagnostic;

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Diagnostic, Severity, Success};
use crate::views;
use crate::web_frame::html_error;

const USAGE: &str = "export에는 <swiftui|appkit|compose> <프로젝트 경로> <문서 id> <View id> [--frame <Frame id>] [--platform ios|macos]가 필요합니다";

fn diagnostic(d: HtmlDiagnostic) -> Diagnostic {
    Diagnostic {
        severity: if d.error {
            Severity::Error
        } else {
            Severity::Warning
        },
        code: d.code,
        message: d.message,
    }
}

/// `export <swiftui|appkit|compose> <프로젝트 경로> <문서 id> <View id> [--frame <Frame id>] [--platform ios|macos]`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let Some((kind, rest)) = args.split_first() else {
        return Err(CliError::usage(USAGE));
    };
    let mut positional = Vec::new();
    let mut frame = None;
    // `--platform macos`면 true.
    let mut platform: Option<bool> = None;
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--frame" if frame.is_none() => {
                frame = Some(iter.next().ok_or_else(|| CliError::usage(USAGE))?)
            }
            "--platform" if platform.is_none() => {
                platform = Some(match iter.next().map(String::as_str) {
                    Some("ios") => false,
                    Some("macos") => true,
                    Some(other) => {
                        return Err(CliError::usage(format!(
                            "알 수 없는 플랫폼: {other}(ios 또는 macos)"
                        )))
                    }
                    None => return Err(CliError::usage(USAGE)),
                })
            }
            option if option.starts_with('-') => {
                return Err(CliError::usage(format!("알 수 없는 옵션: {option}")))
            }
            _ => positional.push(arg),
        }
    }
    let target = match (kind.as_str(), platform) {
        ("swiftui", Some(true)) => NativeTarget::SwiftUiMacOs,
        ("swiftui", _) => NativeTarget::SwiftUiIos,
        ("appkit", None | Some(true)) => NativeTarget::AppKit,
        ("compose", None) => NativeTarget::Compose,
        ("appkit" | "compose", Some(_)) => {
            return Err(CliError::usage(
                "--platform은 swiftui에만 씁니다(appkit은 macOS, compose는 Android)",
            ))
        }
        (other, _) => {
            return Err(CliError::usage(format!(
                "알 수 없는 대상: {other}(swiftui, appkit, compose)"
            )))
        }
    };
    let [path, document, view] = positional[..] else {
        return Err(CliError::usage(USAGE));
    };
    let parse_id = |value: &str, what: &str| -> Result<StableId, CliError> {
        value
            .parse()
            .map_err(|_| CliError::usage(format!("{what}는 UUID 형식이어야 합니다: {value}")))
    };
    let document = parse_id(document, "문서 id")?;
    let view = parse_id(view, "View id")?;
    let frame = frame.map(|f| parse_id(f, "Frame id")).transpose()?;
    let snapshot =
        open_snapshot(Path::new(path)).map_err(|e| views::open_error(&e).with_context(path))?;
    let export = export_native(&snapshot, document, view, frame, target)
        .map_err(|e| html_error(e, document, view).with_context(path))?;
    let exported = export.source.is_some();
    let result = JsonValue::object(vec![
        ("revision", views::revision(export.revision)),
        ("document", views::id(export.document)),
        ("view", views::id(export.view)),
        ("root_frame", views::id(export.root_frame)),
        ("target", JsonValue::string(target.name())),
        ("profile", JsonValue::string(export.profile.clone())),
        (
            "source",
            export
                .source
                .clone()
                .map_or(JsonValue::Null, JsonValue::string),
        ),
    ]);
    let diagnostics: Vec<_> = export.diagnostics.into_iter().map(diagnostic).collect();
    if exported {
        Ok(Success {
            result,
            diagnostics,
        })
    } else {
        let mut error = CliError::new(
            ExitStatus::Failure,
            "export-native-failed",
            "지정 View를 대상 소스로 내보내지 못했습니다",
        )
        .with_result(result)
        .with_context(path);
        error.diagnostics = diagnostics;
        Err(error)
    }
}
