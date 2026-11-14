//! 컴포넌트별 root Frame 원본 생성·조회·크기 편집 (ADR-0059).
use crate::exit::ExitStatus;
use crate::json::JsonValue as J;
use crate::output::{CliError, Success};
use crate::{text_source::operation_error, views};
use std::path::Path;
use stellar_ops::component_source::{self, ComponentInfo};
use stellar_ops::core_types::{ids::StableId, value::Size2};

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
fn json(info: ComponentInfo) -> J {
    J::object(vec![
        ("id", views::id(info.id)),
        ("root", views::id(info.root)),
        ("name", J::string(info.name)),
        ("file", J::string(info.file.to_string_lossy())),
        ("width", J::string(info.size.width().to_string())),
        ("height", J::string(info.size.height().to_string())),
        ("source_hash", J::string(info.hash.to_string())),
    ])
}

pub(crate) fn run(args: &[String]) -> Result<Success, CliError> {
    const USAGE: &str = "component-source create <폴더> <이름> <id> <root-id> | check <폴더> | resize <파일> <원본 hash> <너비> <높이>";
    let result = match args {
        [op, dir, name, component, root] if op == "create" => {
            let source =
                component_source::create(Path::new(dir), name.clone(), id(component)?, id(root)?)
                    .map_err(|error| operation_error(error).with_context(dir))?;
            json(source)
        }
        [op, dir] if op == "check" => {
            let components = component_source::check(Path::new(dir))
                .map_err(|error| operation_error(error).with_context(dir))?;
            J::object(vec![(
                "components",
                J::Array(components.into_iter().map(json).collect()),
            )])
        }
        [op, file, hash, width, height] if op == "resize" => {
            let size = Size2::new(dimension(width)?, dimension(height)?)
                .map_err(|_| argument("크기는 유한한 비음수여야 합니다"))?;
            json(
                component_source::resize(Path::new(file), hash, size)
                    .map_err(|error| operation_error(error).with_context(file))?,
            )
        }
        _ => return Err(CliError::usage(USAGE)),
    };
    Ok(Success {
        result,
        diagnostics: Vec::new(),
    })
}
