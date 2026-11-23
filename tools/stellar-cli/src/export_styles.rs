//! `export-styles`: 저장 View(또는 그 안 Frame 하나)의 layout 스타일을 대상별로 JSON 결과에 담습니다
//! (IX.11.30, ADR-0089).
//!
//! - `--target web`(기본): `export-web`과 같은 투영(`stellar_ops::web_frame::export_web_frame_node`)의 기반 규칙과
//!   요소별 layout 선언(출처 포함).
//! - `--target react-native`: 같은 투영 계획을 RN 스타일 객체로 낮춘 `StyleSheet.create` 모듈
//!   (`stellar_ops::export_css::export_react_native`).
//!
//! 파일을 쓰지 않습니다. 해석·낮춤은 앱·MCP와 같은 core 함수입니다.

use std::path::Path;

use stellar_ops::core_types::ids::StableId;
use stellar_ops::export_css::{export_react_native, ElementStyle};
use stellar_ops::snapshot::open_snapshot;
use stellar_ops::style_defaults as sd;
use stellar_ops::web_frame::{export_web_frame_node, HtmlDiagnostic};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Diagnostic, Severity, Success};
use crate::views;
use crate::web_frame::html_error;

const USAGE: &str = "export-styles에는 <프로젝트 경로> <문서 id> <View id> [--frame <Frame id>] [--target web|react-native]가 필요합니다";

/// 출력 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Web,
    ReactNative,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Target::Web => "web",
            Target::ReactNative => "react-native",
        }
    }

    fn defaults(self) -> &'static str {
        match self {
            Target::Web => sd::CSS,
            Target::ReactNative => sd::REACT_NATIVE,
        }
    }
}

fn elements(styles: &[ElementStyle], declarations: bool) -> JsonValue {
    JsonValue::Array(
        styles
            .iter()
            .map(|s| {
                let mut fields = vec![
                    ("node", views::id(s.node)),
                    ("element", JsonValue::string(s.element.clone())),
                    ("class", JsonValue::string(s.class.name())),
                    ("placement", JsonValue::string(s.placement.name())),
                ];
                if declarations {
                    fields.push((
                        "declarations",
                        JsonValue::Array(
                            s.declarations
                                .iter()
                                .map(|d| {
                                    JsonValue::object(vec![
                                        ("name", JsonValue::string(d.name)),
                                        ("value", JsonValue::string(d.value.clone())),
                                        ("origin", JsonValue::string(d.origin.name())),
                                    ])
                                })
                                .collect(),
                        ),
                    ));
                }
                JsonValue::object(fields)
            })
            .collect(),
    )
}

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

/// `export-styles <프로젝트 경로> <문서 id> <View id> [--frame <Frame id>] [--target web|react-native]`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let mut positional = Vec::new();
    let mut frame = None;
    let mut target = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--frame" if frame.is_none() => {
                frame = Some(rest.next().ok_or_else(|| CliError::usage(USAGE))?)
            }
            "--target" if target.is_none() => {
                target = Some(match rest.next().map(String::as_str) {
                    Some("web") => Target::Web,
                    Some("react-native") => Target::ReactNative,
                    Some(other) => {
                        return Err(CliError::usage(format!(
                            "알 수 없는 대상: {other}(web 또는 react-native)"
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
    let [path, document, view] = positional[..] else {
        return Err(CliError::usage(USAGE));
    };
    let target = target.unwrap_or(Target::Web);
    let parse_id = |value: &str, kind: &str| -> Result<StableId, CliError> {
        value
            .parse()
            .map_err(|_| CliError::usage(format!("{kind}는 UUID 형식이어야 합니다: {value}")))
    };
    let document = parse_id(document, "문서 id")?;
    let view = parse_id(view, "View id")?;
    let frame = frame.map(|f| parse_id(f, "Frame id")).transpose()?;
    let snapshot =
        open_snapshot(Path::new(path)).map_err(|e| views::open_error(&e).with_context(path))?;
    let (web, source, extra) = match target {
        Target::Web => (
            export_web_frame_node(&snapshot, document, view, frame)
                .map_err(|e| html_error(e, document, view).with_context(path))?,
            None,
            Vec::new(),
        ),
        Target::ReactNative => {
            let rn = export_react_native(&snapshot, document, view, frame)
                .map_err(|e| html_error(e, document, view).with_context(path))?;
            (rn.web, rn.source, rn.diagnostics)
        }
    };
    let exported = !web.diagnostics.iter().any(|d| d.error);
    let mut fields = vec![
        ("revision", views::revision(web.revision)),
        ("document", views::id(web.document)),
        ("view", views::id(web.view)),
        ("root_frame", views::id(web.root_frame)),
        ("target", JsonValue::string(target.name())),
        (
            "style_defaults",
            JsonValue::object(vec![
                ("active", JsonValue::string(web.style_defaults.clone())),
                ("target", JsonValue::string(target.defaults())),
            ]),
        ),
    ];
    match target {
        Target::Web => {
            fields.push(("format", JsonValue::string(web.format)));
            fields.push(("base_rules", JsonValue::string(web.base_rules.clone())));
            fields.push(("elements", elements(&web.styles, true)));
        }
        Target::ReactNative => {
            fields.push(("source", source.map_or(JsonValue::Null, JsonValue::string)));
            fields.push(("elements", elements(&web.styles, false)));
        }
    }
    let result = JsonValue::object(fields);
    let diagnostics: Vec<_> = web
        .diagnostics
        .into_iter()
        .chain(extra)
        .map(diagnostic)
        .collect();
    if exported {
        Ok(Success {
            result,
            diagnostics,
        })
    } else {
        let mut error = CliError::new(
            ExitStatus::Failure,
            "export-styles-failed",
            "지정 View의 스타일을 내보내지 못했습니다",
        )
        .with_result(result)
        .with_context(path);
        error.diagnostics = diagnostics;
        Err(error)
    }
}
