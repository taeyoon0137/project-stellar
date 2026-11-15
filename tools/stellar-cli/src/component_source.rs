//! 컴포넌트별 Free Frame 트리 생성·조회·편집 (ADR-0059).
use crate::exit::ExitStatus;
use crate::json::JsonValue as J;
use crate::output::{CliError, Success};
use crate::{text_source::operation_error, views};
use std::path::Path;
use stellar_ops::component_source::{self, ComponentInfo};
use stellar_ops::core_types::{
    color::{Color, ColorSpace},
    ids::StableId,
    value::Size2,
};

fn argument(message: &str) -> CliError {
    CliError::new(ExitStatus::DataErr, "component-source-argument", message)
}
fn id(value: &str) -> Result<StableId, CliError> {
    value
        .parse()
        .map_err(|_| argument("StableId 형식이 잘못되었습니다"))
}
fn dimension(value: &str) -> Result<f32, CliError> {
    value
        .parse()
        .map_err(|_| argument("크기는 유한한 비음수여야 합니다"))
}
fn size(width: &str, height: &str) -> Result<Size2, CliError> {
    Size2::new(dimension(width)?, dimension(height)?)
        .map_err(|_| argument("크기는 유한한 비음수여야 합니다"))
}
fn fill(args: &[String]) -> Result<Option<Color>, CliError> {
    match args {
        [none] if none == "none" => Ok(None),
        [space, r, g, b, a] => {
            let space = match space.as_str() {
                "srgb" => ColorSpace::Srgb,
                "display-p3" => ColorSpace::DisplayP3,
                _ => return Err(argument("색 공간은 srgb 또는 display-p3여야 합니다")),
            };
            let channel = |s: &str| {
                s.parse::<f32>()
                    .map_err(|_| argument("유한한 RGBA 값이 필요합니다"))
            };
            Color::new(space, channel(r)?, channel(g)?, channel(b)?, channel(a)?)
                .map(Some)
                .map_err(|e| argument(&e.to_string()))
        }
        _ => Err(CliError::usage(
            "set-fill <파일> <원본 hash> none | <srgb|display-p3> <r> <g> <b> <a>",
        )),
    }
}

pub(crate) fn component_json(info: ComponentInfo, include_file: bool) -> J {
    // 원본 YAML과 같은 한 줄 색 표현입니다. 채우기가 없으면 null입니다.
    let background = info.background.map_or(J::Null, |color| {
        J::string(component_source::color_source_value(color))
    });
    let mut fields = vec![
        ("id", views::id(info.id)),
        ("root", views::id(info.root)),
        ("node_count", views::count(info.node_count)),
        ("name", J::string(info.name)),
    ];
    if include_file {
        fields.push(("file", J::string(info.file.to_string_lossy())));
    }
    fields.extend([
        ("width", J::string(info.size.width().to_string())),
        ("height", J::string(info.size.height().to_string())),
        ("background", background),
        ("source_hash", J::string(info.hash.to_string())),
    ]);
    J::object(fields)
}

pub(crate) fn run(args: &[String]) -> Result<Success, CliError> {
    const USAGE: &str = "component-source create <폴더> <이름> <id> <root-id> | check <폴더> | resize <파일> <원본 hash> <너비> <높이> | set-fill <파일> <원본 hash> <none 또는 색 공간과 RGBA> | add-frame <파일> <원본 hash> <parent-id> <새-id> <이름> <너비> <높이>";
    let result = match args {
        [op, dir, name, component, root] if op == "create" => {
            let source =
                component_source::create(Path::new(dir), name.clone(), id(component)?, id(root)?)
                    .map_err(|error| operation_error(&error).with_context(dir))?;
            component_json(source, true)
        }
        [op, dir] if op == "check" => {
            let components = component_source::check(Path::new(dir))
                .map_err(|error| operation_error(&error).with_context(dir))?;
            J::object(vec![(
                "components",
                J::Array(
                    components
                        .into_iter()
                        .map(|entry| component_json(entry, true))
                        .collect(),
                ),
            )])
        }
        [op, file, hash, width, height] if op == "resize" => {
            let size = size(width, height)?;
            component_json(
                component_source::resize(Path::new(file), hash, size)
                    .map_err(|error| operation_error(&error).with_context(file))?,
                true,
            )
        }
        [op, file, hash, value @ ..] if op == "set-fill" => component_json(
            component_source::edit_root(
                Path::new(file),
                hash,
                component_source::ComponentRootEdit::Background(fill(value)?),
            )
            .map_err(|error| operation_error(&error).with_context(file))?,
            true,
        ),
        [op, file, hash, parent, child, name, width, height] if op == "add-frame" => {
            component_json(
                component_source::add_frame(
                    Path::new(file),
                    hash,
                    id(parent)?,
                    id(child)?,
                    name.clone(),
                    size(width, height)?,
                )
                .map_err(|error| operation_error(&error).with_context(file))?,
                true,
            )
        }
        _ => return Err(CliError::usage(USAGE)),
    };
    Ok(Success {
        result,
        diagnostics: Vec::new(),
    })
}
