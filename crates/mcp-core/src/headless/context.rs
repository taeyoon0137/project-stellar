//! `document.context`: `stellar context`와 같은 ops 조회(`read_snapshot_context`)를 세션 snapshot에서 실행합니다
//! (XXX.7.11, ADR-0114 결정 6). field 이름은 CLI 결과와 같습니다. 판정·해석을 따로 두지 않습니다.

use stellar_ops::context::{
    read_snapshot_context, ContextReadError, ContextReport, ContextSite, ContextSiteReport,
    ContextSource,
};
use stellar_ops::core_types::ids::StableId;

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

fn ids(values: &[StableId]) -> J {
    J::Array(values.iter().map(|v| id(*v)).collect())
}

fn opt_text(value: &Option<String>) -> J {
    value
        .as_ref()
        .map_or(J::Null, |text| J::string(text.clone()))
}

fn source(source: &ContextSource) -> J {
    match source {
        ContextSource::Default => J::object(vec![("kind", J::string("default"))]),
        ContextSource::Provision {
            provision,
            provider,
            instances,
        } => J::object(vec![
            ("kind", J::string("provision")),
            ("provision", id(*provision)),
            ("provider", id(*provider)),
            ("instances", ids(instances)),
        ]),
    }
}

fn site(site: &ContextSite) -> J {
    J::object(vec![
        ("node", id(site.node)),
        ("instances", ids(&site.instances)),
    ])
}

fn members(entries: &[(StableId, Vec<StableId>)], owner: &'static str) -> J {
    J::Array(
        entries
            .iter()
            .map(|(owner_id, keys)| J::object(vec![(owner, id(*owner_id)), ("keys", ids(keys))]))
            .collect(),
    )
}

fn site_report(report: &ContextSiteReport) -> J {
    J::object(vec![
        ("site", site(&report.site)),
        (
            "resolutions",
            J::Array(
                report
                    .resolutions
                    .iter()
                    .map(|row| {
                        J::object(vec![
                            ("key", id(row.key)),
                            ("name", J::string(row.name.clone())),
                            ("value", opt_text(&row.value)),
                            ("error", opt_text(&row.error)),
                            ("source", source(&row.source)),
                            ("path", J::Array(row.path.iter().map(site).collect())),
                            ("via_slots", ids(&row.via_slots)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "candidates",
            J::Array(
                report
                    .candidates
                    .iter()
                    .map(|row| {
                        J::object(vec![
                            ("key", id(row.key)),
                            ("name", J::string(row.name.clone())),
                            ("value_type", J::string(row.value_type.clone())),
                            ("needs_declaration", J::Bool(row.needs_declaration)),
                            ("source", source(&row.source)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn report(report: &ContextReport) -> J {
    J::object(vec![
        ("revision", revision(report.revision)),
        ("document", id(report.document)),
        (
            "keys",
            J::Array(
                report
                    .keys
                    .iter()
                    .map(|row| {
                        J::object(vec![
                            ("id", id(row.id)),
                            ("name", J::string(row.name.clone())),
                            ("scope", J::string(row.scope.clone())),
                            ("value_type", J::string(row.value_type.clone())),
                            ("default", J::string(row.default.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "provisions",
            J::Array(
                report
                    .provisions
                    .iter()
                    .map(|row| {
                        J::object(vec![
                            ("id", id(row.id)),
                            ("owner", J::string(row.owner.clone())),
                            ("provider", id(row.provider)),
                            ("key", id(row.key)),
                            ("value", J::string(row.value.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("requirements", members(&report.requirements, "component")),
        ("slot_keys", members(&report.slot_keys, "slot")),
        (
            "diagnostics",
            J::Array(
                report
                    .diagnostics
                    .iter()
                    .map(|row| {
                        J::object(vec![
                            ("code", J::string(row.code)),
                            ("severity", J::string(row.severity)),
                            ("message", J::string(row.message.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("site", report.site.as_ref().map_or(J::Null, site_report)),
    ])
}

/// `document.context` 실행.
pub(super) fn read(cached: &CachedRead, args: &J) -> Result<J, RpcError> {
    let document = arg_id(args.get("document"))?;
    let node = args.get("node").map(|v| arg_id(Some(v))).transpose()?;
    let instances = match args.get("instances") {
        None => Vec::new(),
        Some(J::Array(items)) => items
            .iter()
            .map(|item| arg_id(Some(item)))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(fail(
                Kind::InvalidArguments,
                "instances는 StableId 배열이어야 합니다",
            ))
        }
    };
    if node.is_none() && !instances.is_empty() {
        return Err(fail(
            Kind::InvalidArguments,
            "instances는 node와 함께 씁니다",
        ));
    }
    let at = node.map(|node| ContextSite::virtual_node(instances, node));
    let read =
        read_snapshot_context(&cached.snapshot, document, at.as_ref()).map_err(|e| match e {
            ContextReadError::Open(_) | ContextReadError::Document(_) => {
                fail(Kind::NotFound, "대상 문서를 읽을 수 없습니다")
            }
            ContextReadError::Components(_) => {
                fail(Kind::Unavailable, "컴포넌트 원본을 읽을 수 없습니다")
            }
            ContextReadError::Site(_) => fail(Kind::NotFound, "위치를 해석할 수 없습니다"),
        })?;
    Ok(report(&read))
}
