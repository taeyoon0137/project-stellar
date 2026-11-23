//! `export png`: 저장 View를 PNG 파일로 내보냅니다(XX.2.3, ADR-0116 결정 5).
//!
//! 저장 generation의 문서 View를 앱과 같은 scene 변환으로 그리고(`stellar_ops::export_png::view`), CPU target으로 렌더해
//! temp + atomic rename으로 저장합니다. 결과 JSON은 경로·크기·profile·gamut 요약·빠진 node·확인한 진단입니다.
//!
//! - 변환이 넣지 못한 node(예: headless Text layout 없음)가 있으면 실패합니다. `--omit-skipped`는 그 node를 빼고 그리는 것을
//!   확인합니다.
//! - 확인이 필요한 사전 진단(없는 이미지, 검사하지 못한 target profile)이 있으면 실패합니다. `--accept-issues`로 확인합니다.
//! - CPU target이 아직 그리지 않는 효과(drop shadow, layer blur, Normal이 아닌 blend)는 렌더 실패이고 파일을 쓰지 않습니다.

use std::path::{Path, PathBuf};

use stellar_ops::core_types::ids::StableId;
use stellar_ops::export_png::view::{export_view_png, SkippedNode, ViewPngError, ViewPngRequest};
use stellar_ops::snapshot::open_snapshot;

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::views;

const USAGE: &str = "export png에는 <프로젝트 경로> <문서 id> <View id> --output <파일> [--scale <배율>] [--profile srgb|display-p3] [--background <CSS 색>] [--accept-issues] [--omit-skipped]가 필요합니다";

fn skipped_json(nodes: &[SkippedNode]) -> JsonValue {
    JsonValue::Array(
        nodes
            .iter()
            .map(|node| {
                JsonValue::object(vec![
                    ("node", views::id(node.node)),
                    ("reason", JsonValue::string(node.reason.code())),
                ])
            })
            .collect(),
    )
}

fn strings(items: &[String]) -> JsonValue {
    JsonValue::Array(items.iter().cloned().map(JsonValue::string).collect())
}

fn ids(items: &[StableId]) -> JsonValue {
    JsonValue::Array(items.iter().copied().map(views::id).collect())
}

fn absolute(path: &str) -> Result<PathBuf, CliError> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|e| {
        CliError::new(
            ExitStatus::IoErr,
            "io",
            format!("현재 디렉터리를 읽지 못했습니다: {e}"),
        )
    })?;
    Ok(cwd.join(path))
}

fn error(error: ViewPngError, path: &str) -> CliError {
    let message = error.to_string();
    let failed = match error {
        ViewPngError::Unsaved => CliError::new(ExitStatus::NoInput, "unsaved", message),
        ViewPngError::Document(_) => CliError::new(ExitStatus::DataErr, "document", message),
        ViewPngError::UnknownView(_) => CliError::new(ExitStatus::NoInput, "unknown-view", message),
        ViewPngError::InvalidRequest(_) => CliError::usage(message),
        ViewPngError::Scene(_) => CliError::new(ExitStatus::DataErr, "png-scene", message),
        ViewPngError::SkippedNodes(nodes) => {
            CliError::new(ExitStatus::Failure, "png-skipped-nodes", message)
                .with_result(JsonValue::object(vec![("skipped", skipped_json(&nodes))]))
        }
        ViewPngError::UnsupportedImage { .. } => {
            CliError::new(ExitStatus::Failure, "png-unsupported-image", message)
        }
        ViewPngError::Capture(_) => CliError::new(ExitStatus::Failure, "png-capture", message),
        ViewPngError::ConfirmationRequired(issues) => {
            CliError::new(ExitStatus::Failure, "png-confirmation-required", message)
                .with_result(JsonValue::object(vec![("issues", strings(&issues))]))
        }
        ViewPngError::Rejected(_) => CliError::new(ExitStatus::Failure, "png-rejected", message),
        ViewPngError::Save(_) => {
            CliError::new(ExitStatus::CantCreate, "png-export-failed", message)
        }
    };
    failed.with_context(path)
}

/// `export png <프로젝트 경로> <문서 id> <View id> --output <파일> [옵션]`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let mut positional = Vec::new();
    let mut output = None;
    let mut scale = None;
    let mut profile = None;
    let mut background = None;
    let mut accept_issues = false;
    let mut omit_skipped = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |slot: &mut Option<String>| -> Result<(), CliError> {
            if slot.is_some() {
                return Err(CliError::usage(format!("{arg}는 한 번만 씁니다")));
            }
            *slot = Some(iter.next().ok_or_else(|| CliError::usage(USAGE))?.clone());
            Ok(())
        };
        match arg.as_str() {
            "--output" => value(&mut output)?,
            "--scale" => value(&mut scale)?,
            "--profile" => value(&mut profile)?,
            "--background" => value(&mut background)?,
            "--accept-issues" => accept_issues = true,
            "--omit-skipped" => omit_skipped = true,
            option if option.starts_with('-') => {
                return Err(CliError::usage(format!("알 수 없는 옵션: {option}")))
            }
            _ => positional.push(arg),
        }
    }
    let [path, document, view] = positional[..] else {
        return Err(CliError::usage(USAGE));
    };
    let output = output.ok_or_else(|| CliError::usage(USAGE))?;
    let parse_id = |value: &str, what: &str| -> Result<StableId, CliError> {
        value
            .parse()
            .map_err(|_| CliError::usage(format!("{what}는 UUID 형식이어야 합니다: {value}")))
    };
    let document = parse_id(document, "문서 id")?;
    let view = parse_id(view, "View id")?;
    let scale = match scale {
        None => 1.0,
        Some(text) => text
            .parse::<f32>()
            .map_err(|_| CliError::usage(format!("배율은 수여야 합니다: {text}")))?,
    };
    let request = ViewPngRequest {
        document,
        view,
        scale,
        profile: profile.unwrap_or_else(|| "srgb".to_string()),
        background,
        output: absolute(&output)?,
        accept_issues,
        omit_skipped,
    };
    let snapshot =
        open_snapshot(Path::new(path)).map_err(|e| views::open_error(&e).with_context(path))?;
    let outcome = export_view_png(&snapshot, &request).map_err(|e| error(e, path))?;
    let saved = &outcome.saved;
    let result = JsonValue::object(vec![
        ("revision", views::revision(snapshot.revision())),
        ("document", views::id(document)),
        ("view", views::id(view)),
        (
            "path",
            JsonValue::string(saved.path.to_string_lossy().into_owned()),
        ),
        ("bytes", JsonValue::int(saved.bytes as i64)),
        ("width", JsonValue::int(i64::from(saved.width))),
        ("height", JsonValue::int(i64::from(saved.height))),
        ("profile", JsonValue::string(saved.profile.name())),
        (
            "gamut",
            JsonValue::object(vec![
                (
                    "clipped_colors",
                    JsonValue::int(saved.gamut.clipped_colors as i64),
                ),
                ("clipped_nodes", ids(&saved.gamut.clipped_nodes)),
                (
                    "extended_colors",
                    JsonValue::int(saved.gamut.extended_colors as i64),
                ),
                ("extended_nodes", ids(&saved.gamut.extended_nodes)),
            ]),
        ),
        ("skipped", skipped_json(&outcome.skipped)),
        ("accepted_issues", strings(&outcome.accepted)),
    ]);
    Ok(Success {
        result,
        diagnostics: Vec::new(),
    })
}
