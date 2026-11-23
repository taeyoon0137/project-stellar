//! `document.guides`: `stellar guides`와 같은 ops 조회(`read_snapshot_guides`)를 세션 snapshot에서 실행합니다
//! (XII.10.19·XII.10.20·XVI.11.1, ADR-0126). field 이름은 CLI 결과와 같습니다.

use stellar_ops::core_types::ids::StableId;
use stellar_ops::guides::{read_snapshot_guides, GuidesReadError, GuidesReport};

use super::{fail, id, revision, CachedRead, Kind};
use crate::json::JsonValue as J;
use crate::jsonrpc::RpcError;

fn arg_id(value: Option<&J>) -> Result<StableId, RpcError> {
    value
        .and_then(J::as_str)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| {
            fail(
                Kind::InvalidArguments,
                "ID 인자는 StableId 문자열이어야 합니다",
            )
        })
}

fn strings(values: &[String]) -> J {
    J::Array(values.iter().cloned().map(J::string).collect())
}

fn tracks(values: &[(String, String)]) -> J {
    J::Array(
        values
            .iter()
            .map(|(start, end)| J::Array(vec![J::string(start.clone()), J::string(end.clone())]))
            .collect(),
    )
}

fn count(value: usize) -> J {
    J::Number(crate::json::JsonNumber::from_u64(value as u64))
}

fn report(report: &GuidesReport) -> J {
    J::object(vec![
        ("revision", revision(report.revision)),
        ("document", id(report.document)),
        (
            "frames",
            J::Array(
                report
                    .frames
                    .iter()
                    .map(|frame| {
                        J::object(vec![
                            ("owner", id(frame.owner)),
                            (
                                "guides",
                                J::Array(
                                    frame
                                        .guides
                                        .iter()
                                        .map(|guide| {
                                            J::object(vec![
                                                ("id", id(guide.id)),
                                                ("axis", J::string(guide.axis)),
                                                ("position", J::string(guide.position.clone())),
                                                ("line", J::string(guide.line.clone())),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                            (
                                "grids",
                                J::Array(
                                    frame
                                        .grids
                                        .iter()
                                        .map(|grid| {
                                            J::object(vec![
                                                ("line", J::string(grid.line.clone())),
                                                ("visible", J::Bool(grid.visible)),
                                                ("columns", tracks(&grid.columns)),
                                                ("rows", tracks(&grid.rows)),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "views",
            J::Array(
                report
                    .views
                    .iter()
                    .map(|view| {
                        J::object(vec![
                            ("view", id(view.view)),
                            (
                                "variables",
                                J::Array(
                                    view.variables
                                        .iter()
                                        .map(|row| {
                                            J::object(vec![
                                                ("id", id(row.id)),
                                                ("name", J::string(row.name.clone())),
                                                ("value_type", J::string(row.value_type.clone())),
                                                (
                                                    "default",
                                                    row.default.clone().map_or(J::Null, J::string),
                                                ),
                                                ("allowed", strings(&row.allowed)),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                            (
                                "variants",
                                J::Array(
                                    view.variants
                                        .iter()
                                        .map(|row| {
                                            J::object(vec![
                                                ("id", id(row.id)),
                                                ("when", strings(&row.when)),
                                                ("property_patches", count(row.property_patches)),
                                                ("structure_patches", count(row.structure_patches)),
                                                ("slot_changes", count(row.slot_changes)),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                            ("diagnostics", strings(&view.diagnostics)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// `document.guides` 실행.
pub(super) fn read(cached: &CachedRead, args: &J) -> Result<J, RpcError> {
    let document = arg_id(args.get("document"))?;
    let node = args.get("node").map(|v| arg_id(Some(v))).transpose()?;
    let read = read_snapshot_guides(&cached.snapshot, document, node).map_err(|e| match e {
        GuidesReadError::Open(_) | GuidesReadError::Document(_) => {
            fail(Kind::NotFound, "문서를 읽을 수 없습니다")
        }
        GuidesReadError::NotAFrame(_) => {
            fail(Kind::InvalidArguments, "node가 문서의 Frame이 아닙니다")
        }
    })?;
    Ok(report(&read))
}
