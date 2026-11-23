//! `stellar cache status|rebuild|gc|migrate [--convert] <프로젝트>`(ADR-0122, ADR-0131). 동작은
//! `stellar_ops::git_cache`가 합니다.

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use stellar_ops::git_cache::{self, CacheCommandError, GcReport, LegacyConversion, Rebuild};

const USAGE: &str = "cache status|rebuild|gc|migrate [--convert] <프로젝트>";

fn count(n: usize) -> JsonValue {
    JsonValue::int(i64::try_from(n).unwrap_or(i64::MAX))
}

fn rebuild_json(rebuild: Rebuild) -> JsonValue {
    match rebuild {
        Rebuild::Fresh => JsonValue::string("fresh"),
        Rebuild::NoTextSource => JsonValue::string("no-text-source"),
        Rebuild::Rebuilt {
            generation,
            manifest,
        } => JsonValue::object(vec![
            (
                "generation",
                JsonValue::int(i64::try_from(generation).unwrap_or(i64::MAX)),
            ),
            ("manifest", JsonValue::string(manifest.to_string())),
        ]),
    }
}

fn gc_json(gc: &GcReport) -> JsonValue {
    JsonValue::object(vec![
        ("retained", count(gc.retained.len())),
        ("deleted", count(gc.deleted)),
        ("kept_in_grace", count(gc.kept_in_grace)),
        ("carry_over", count(gc.carry_over.len())),
    ])
}

fn conversion_json(converted: Option<&LegacyConversion>) -> JsonValue {
    match converted {
        None => JsonValue::Null,
        Some(converted) => JsonValue::object(vec![
            (
                "manifest",
                JsonValue::string(converted.manifest.to_string()),
            ),
            ("documents", count(converted.documents)),
            ("components", count(converted.components)),
            ("written", count(converted.written)),
        ]),
    }
}

/// 인자를 `(작업, --convert 여부, 프로젝트)`로 읽습니다. `--convert`는 `migrate`에만 받습니다.
fn parse(args: &[String]) -> Option<(&str, bool, &String)> {
    match args {
        [operation, project] if !project.starts_with("--") => {
            Some((operation.as_str(), false, project))
        }
        [operation, flag, project] if operation == "migrate" && flag == "--convert" => {
            Some((operation.as_str(), true, project))
        }
        _ => None,
    }
}

/// `cache` 명령을 실행합니다.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let usage = || CliError::usage(USAGE);
    let (operation, convert, project) = parse(args).ok_or_else(usage)?;
    let root = std::path::Path::new(project);
    let fail = |error: CacheCommandError| {
        let message = error.to_string();
        match error {
            CacheCommandError::Lock(e) => crate::project_cmds::lock_error(&e),
            CacheCommandError::Path(kind) => {
                CliError::new(ExitStatus::from_io_kind(kind), "path", message)
            }
            CacheCommandError::Observe(_) | CacheCommandError::Cache(_) => {
                CliError::new(ExitStatus::Failure, "cache-error", message)
            }
        }
        .with_context(project.clone())
    };
    let lock = || -> Result<stellar_ops::lock::LockPolicy, CliError> {
        Ok(stellar_ops::lock::LockPolicy::Required {
            dir: stellar_ops::lock::default_lock_dir()
                .map_err(|e| crate::project_cmds::lock_error(&e))?,
        })
    };
    let result = match operation {
        "status" => {
            let status = git_cache::status(root).map_err(fail)?;
            JsonValue::object(vec![
                ("operation", JsonValue::string("status")),
                ("location", JsonValue::string(status.location)),
                ("plan", JsonValue::string(status.plan)),
                ("text_source", JsonValue::Bool(status.text_source)),
                (
                    "current",
                    JsonValue::opt_string(status.current.map(|h| h.to_string()).as_deref()),
                ),
                ("generations", count(status.generations)),
                ("legacy_moved", count(status.migrated.moved.moved)),
            ])
        }
        "rebuild" => {
            let rebuilt = git_cache::rebuild(root, &lock()?).map_err(fail)?;
            JsonValue::object(vec![
                ("operation", JsonValue::string("rebuild")),
                ("rebuild", rebuild_json(rebuilt)),
            ])
        }
        "gc" => {
            let report =
                git_cache::gc(root, &lock()?, std::time::SystemTime::now()).map_err(fail)?;
            JsonValue::object(vec![
                ("operation", JsonValue::string("gc")),
                ("gc", gc_json(&report)),
            ])
        }
        "migrate" => {
            let report =
                git_cache::migrate_with(root, &lock()?, std::time::SystemTime::now(), convert)
                    .map_err(fail)?;
            JsonValue::object(vec![
                ("operation", JsonValue::string("migrate")),
                ("legacy_moved", count(report.layout.moved.moved)),
                (
                    "converted",
                    conversion_json(report.layout.converted.as_ref()),
                ),
                (
                    "untracked",
                    match &report.untracked {
                        Ok(paths) => JsonValue::Array(
                            paths.iter().map(|p| JsonValue::string(p.clone())).collect(),
                        ),
                        Err(_) => JsonValue::Null,
                    },
                ),
                (
                    "git",
                    JsonValue::opt_string(report.untracked.as_ref().err().map(String::as_str)),
                ),
                ("rebuild", rebuild_json(report.rebuilt)),
                ("gc", gc_json(&report.gc)),
            ])
        }
        _ => return Err(usage()),
    };
    Ok(Success {
        result,
        diagnostics: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_flag_is_only_for_migrate() {
        let args = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(matches!(
            parse(&args(&["migrate", "--convert", "p"])),
            Some(("migrate", true, _))
        ));
        assert!(matches!(
            parse(&args(&["migrate", "p"])),
            Some(("migrate", false, _))
        ));
        assert!(parse(&args(&["status", "--convert", "p"])).is_none());
        assert!(parse(&args(&["migrate", "--convert"])).is_none());
        assert!(parse(&args(&["migrate", "p", "--convert"])).is_none());
    }
}
