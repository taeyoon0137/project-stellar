//! 플러그인 command가 일반 편집 경로를 쓰는지 (XXI.4.6).
//!
//! 같은 내용의 프로젝트 둘에 같은 transaction을 하나는 `command.apply`(플러그인), 하나는 `stellar apply`가 부르는
//! `ops::apply::apply`로 적용합니다. 결과 revision, semantic diff, Git 작업 트리의 semantic 변경이 같아야 합니다.

mod support;

use std::path::Path;
use std::process::Command;

use stellar_ops::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
use stellar_ops::lock::LockPolicy;
use stellar_ops::revision::ProjectRevision;
use stellar_plugin_host::json::{write, JsonValue};
use stellar_plugin_host::permission::Permission;
use support::{json, Fixture};

fn git(root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn committed_fixture(label: &str) -> Option<Fixture> {
    let fx = Fixture::new(label, 4);
    (fx.git_init()
        && git(&fx.project, &["add", "-A"])
        && git(&fx.project, &["commit", "-q", "-m", "base"]))
    .then_some(fx)
}

fn semantic_changes(status: &JsonValue) -> String {
    let changes = status
        .get("semantic")
        .and_then(|s| s.get("changes"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    write(&changes)
}

#[test]
fn plugin_apply_matches_cli_apply_path() {
    let (Some(plugin_fx), Some(cli_fx)) = (
        committed_fixture("plugin-path-a"),
        committed_fixture("plugin-path-b"),
    ) else {
        eprintln!("시스템 Git이 없어 건너뜁니다");
        return;
    };
    let dispatcher = plugin_fx.dispatcher(&Permission::ALL);
    let snapshot = dispatcher
        .call("document.read_snapshot", &json("{}"))
        .unwrap();
    let revision = snapshot
        .get("revision")
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_owned();
    let document = snapshot
        .get("documents")
        .and_then(JsonValue::as_array)
        .and_then(|d| d[0].get("id"))
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_owned();
    let page = dispatcher
        .call(
            "query.tree_page",
            &json(&format!(r#"{{"document":"{document}"}}"#)),
        )
        .unwrap();
    let node = page.get("nodes").and_then(JsonValue::as_array).unwrap()[2]
        .get("id")
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_owned();

    // 플러그인 경로(쓰기).
    let applied = dispatcher
        .call(
            "command.apply",
            &json(&format!(
                r#"{{"base_revision":"{revision}","document":"{document}","commands":[{{"kind":"set-visible","node":"{node}","visible":false}},{{"kind":"rename","node":"{node}","name":"plugin"}}]}}"#
            )),
        )
        .unwrap();
    assert_eq!(
        applied.get("mode").and_then(JsonValue::as_str),
        Some("write")
    );

    // CLI 경로(`stellar apply`가 부르는 같은 함수).
    let request = ApplyRequest {
        base_revision: revision.parse::<ProjectRevision>().unwrap(),
        document: document.parse().unwrap(),
        commands: vec![
            CommandInput::SetVisible {
                node: node.parse().unwrap(),
                visible: false,
            },
            CommandInput::Rename {
                node: node.parse().unwrap(),
                name: "plugin".to_owned(),
            },
        ],
    };
    let outcome = apply(
        &cli_fx.project,
        &request,
        ApplyMode::Write,
        &LockPolicy::Required {
            dir: cli_fx.locks.clone(),
        },
    )
    .unwrap();

    assert_eq!(
        applied.get("revision").and_then(JsonValue::as_str),
        Some(outcome.revision.to_string().as_str())
    );
    assert_eq!(
        applied.get("change_count").and_then(JsonValue::as_i64),
        Some(outcome.diff.entries.len() as i64)
    );
    let plugin_kinds: Vec<&str> = applied
        .get("changes")
        .and_then(JsonValue::as_array)
        .unwrap()
        .iter()
        .filter_map(|c| c.get("kind").and_then(JsonValue::as_str))
        .collect();
    let cli_kinds: Vec<&str> = outcome.diff.entries.iter().map(|e| e.kind).collect();
    assert_eq!(plugin_kinds, cli_kinds);
    assert!(!cli_kinds.is_empty());

    // Git 작업 트리에서 보는 semantic 변경도 같습니다.
    let plugin_status = dispatcher.call("git.status", &json("{}")).unwrap();
    let cli_status = cli_fx
        .dispatcher(&[Permission::GitRead])
        .call("git.status", &json("{}"))
        .unwrap();
    assert_eq!(
        semantic_changes(&plugin_status),
        semantic_changes(&cli_status)
    );
    assert_ne!(semantic_changes(&plugin_status), "[]");
}
