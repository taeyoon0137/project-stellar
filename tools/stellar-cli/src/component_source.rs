//! 컴포넌트별 Free Frame 트리 생성·조회·편집 (ADR-0059).
use crate::exit::ExitStatus;
use crate::json::JsonValue as J;
use crate::output::{CliError, Success};
use crate::{text_source::operation_error, views};
use std::path::Path;
use stellar_ops::component_source::{self, ComponentInfo};
use stellar_ops::core_types::{
    ids::StableId,
    value::{Size2, Vec2},
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
pub(crate) fn component_json(info: ComponentInfo, include_file: bool) -> J {
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
        ("source_hash", J::string(info.hash.to_string())),
    ]);
    J::object(fields)
}

pub(crate) fn run(args: &[String]) -> Result<Success, CliError> {
    const USAGE: &str = "component-source create <폴더> <이름> <id> <root-id> | check <폴더> | resize <파일> <원본 hash> <너비> <높이> | resize-frame <파일> <원본 hash> <node-id> <너비> <높이> | set-position <파일> <원본 hash> <node-id> <x> <y> | set-visible <파일> <원본 hash> <node-id> <true|false> | set-locked <파일> <원본 hash> <node-id> <true|false> | remove-frame <파일> <원본 hash> <node-id> | rename-frame <파일> <원본 hash> <node-id> <이름> | add-frame <파일> <원본 hash> <parent-id> <새-id> <이름> <너비> <높이>";
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
        [op, file, hash, node, width, height] if op == "resize-frame" => component_json(
            component_source::resize_frame(Path::new(file), hash, id(node)?, size(width, height)?)
                .map_err(|error| operation_error(error).with_context(file))?,
            true,
        ),
        [op, file, hash, node, x, y] if op == "set-position" => {
            let coordinate = |value: &str| {
                value
                    .parse::<f32>()
                    .map_err(|_| argument("좌표는 유한한 수여야 합니다"))
            };
            let position = Vec2::new(coordinate(x)?, coordinate(y)?)
                .map_err(|_| argument("좌표는 유한한 수여야 합니다"))?;
            component_json(
                component_source::set_position(Path::new(file), hash, id(node)?, position)
                    .map_err(|error| operation_error(error).with_context(file))?,
                true,
            )
        }
        [op, file, hash, node, visible] if op == "set-visible" => component_json(
            component_source::set_visible(
                Path::new(file),
                hash,
                id(node)?,
                visible
                    .parse::<bool>()
                    .map_err(|_| argument("표시 여부는 true 또는 false여야 합니다"))?,
            )
            .map_err(|error| operation_error(error).with_context(file))?,
            true,
        ),
        [op, file, hash, node, locked] if op == "set-locked" => component_json(
            component_source::set_locked(
                Path::new(file),
                hash,
                id(node)?,
                locked
                    .parse::<bool>()
                    .map_err(|_| argument("잠금 여부는 true 또는 false여야 합니다"))?,
            )
            .map_err(|error| operation_error(error).with_context(file))?,
            true,
        ),
        [op, file, hash, node] if op == "remove-frame" => component_json(
            component_source::remove_frame(Path::new(file), hash, id(node)?)
                .map_err(|error| operation_error(error).with_context(file))?,
            true,
        ),
        [op, file, hash, node, name] if op == "rename-frame" => component_json(
            component_source::rename_frame(Path::new(file), hash, id(node)?, name.clone())
                .map_err(|error| operation_error(error).with_context(file))?,
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
