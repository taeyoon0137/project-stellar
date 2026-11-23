//! `stellar diagnose`: 무결성 진단 필터·pagination(XXV.3.1, ADR-0095).

use std::collections::BTreeSet;
use std::process::{Command, Output, Stdio};

use stellar_cli::json::{self, JsonValue};
use stellar_ops::test_support::{
    document_with_unresolved_clips, file_digest, random_valid_document, save_documents,
    scaffold_project, TempDir,
};

fn stellar(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args(args)
        .env_clear()
        .env("HOME", std::env::temp_dir())
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn envelope(output: &Output, exit: i32) -> JsonValue {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::str::from_utf8(&output.stdout).unwrap();
    json::parse(text.trim_end(), json::JsonLimits::default()).unwrap()
}

fn result(value: &JsonValue) -> &JsonValue {
    value.get("result").expect("result")
}

fn num(value: &JsonValue, key: &str) -> u64 {
    value
        .get(key)
        .and_then(|v| match v {
            JsonValue::Number(n) => n.as_str().parse().ok(),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{key}"))
}

fn items(value: &JsonValue) -> &[JsonValue] {
    match result(value).get("items") {
        Some(JsonValue::Array(items)) => items,
        other => panic!("items: {other:?}"),
    }
}

fn str_of<'a>(value: &'a JsonValue, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .unwrap_or_else(|| panic!("{key}"))
}

#[test]
fn diagnose_lists_filters_and_pages_without_writing() {
    let dir = TempDir::new("cli-diagnose");
    scaffold_project(&dir.path);
    let broken = document_with_unresolved_clips(25);
    let clean = random_valid_document(301);
    save_documents(&dir.path, &[&broken, &clean]);
    let path = dir.path.to_str().unwrap();
    let before = file_digest(&dir.path);

    let all = envelope(&stellar(&["diagnose", path]), 0);
    let r = result(&all);
    assert_eq!(num(r, "documents_checked"), 2);
    let summary = r.get("summary").unwrap();
    assert_eq!(num(summary, "total"), 25);
    assert_eq!(num(summary.get("by_kind").unwrap(), "missing-node"), 25);
    assert_eq!(num(summary.get("by_severity").unwrap(), "warning"), 25);
    assert_eq!(items(&all).len(), 25);
    assert!(matches!(r.get("next_cursor"), Some(JsonValue::Null)));
    let first = &items(&all)[0];
    assert_eq!(str_of(first, "kind"), "missing-node");
    assert_eq!(str_of(first, "code"), "unresolved-reference");
    assert_eq!(str_of(first, "origin"), "references");
    assert_eq!(str_of(first, "id").len(), 16);
    assert!(str_of(first, "property").contains("clipping"), "{first:?}");
    let actions: Vec<&str> = match first.get("remediation") {
        Some(JsonValue::Array(a)) => a.iter().map(|x| str_of(x, "action")).collect(),
        _ => panic!("remediation"),
    };
    assert_eq!(actions, vec!["reveal-source", "reveal-property"]);

    // 10개씩 page: 3 page, 중복·누락 없이 같은 순서.
    let mut ids = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let mut args = vec!["diagnose", path, "--limit", "10"];
        if let Some(c) = &cursor {
            args.extend(["--cursor", c.as_str()]);
        }
        let page = envelope(&stellar(&args), 0);
        pages += 1;
        ids.extend(items(&page).iter().map(|i| str_of(i, "id").to_string()));
        match result(&page).get("next_cursor") {
            Some(JsonValue::String(next)) => cursor = Some(next.clone()),
            _ => break,
        }
    }
    assert_eq!(pages, 3);
    let expected: Vec<String> = items(&all)
        .iter()
        .map(|i| str_of(i, "id").to_string())
        .collect();
    assert_eq!(ids, expected);
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 25);

    // 필터: 종류·문서·프로젝트 단위·심각도.
    let fonts = envelope(&stellar(&["diagnose", path, "--kind", "missing-font"]), 0);
    assert_eq!(num(result(&fonts).get("filter").unwrap(), "matching"), 0);
    assert!(items(&fonts).is_empty());
    let clean_id = clean.definition().id.to_string();
    let only_clean = envelope(&stellar(&["diagnose", path, "--document", &clean_id]), 0);
    assert!(items(&only_clean).is_empty());
    let broken_id = broken.definition().id.to_string();
    let only_broken = envelope(
        &stellar(&["diagnose", path, "--document", &broken_id, "--limit", "5"]),
        0,
    );
    assert_eq!(
        num(result(&only_broken).get("filter").unwrap(), "matching"),
        25
    );
    assert_eq!(items(&only_broken).len(), 5);
    let project = envelope(&stellar(&["diagnose", path, "--project"]), 0);
    assert!(items(&project).is_empty());
    let errors = envelope(&stellar(&["diagnose", path, "--severity", "error"]), 0);
    assert!(items(&errors).is_empty());
    // 맞는 조건은 그 진단을 그대로 남깁니다(빈 결과만 보는 필터 test가 아님).
    for args in [
        vec!["diagnose", path, "--kind", "missing-node"],
        vec!["diagnose", path, "--severity", "warning"],
        vec![
            "diagnose",
            path,
            "--kind",
            "missing-font",
            "--kind",
            "missing-node",
        ],
    ] {
        let page = envelope(&stellar(&args), 0);
        assert_eq!(
            num(result(&page).get("filter").unwrap(), "matching"),
            25,
            "{args:?}"
        );
        assert_eq!(items(&page).len(), 25, "{args:?}");
    }
    let source = str_of(first, "source").to_string();
    let by_source = envelope(&stellar(&["diagnose", path, "--source", &source]), 0);
    assert!(!items(&by_source).is_empty());
    assert!(items(&by_source)
        .iter()
        .all(|i| str_of(i, "source") == source));

    assert_eq!(file_digest(&dir.path), before, "읽기 전용");

    // 사용법 오류.
    for bad in [
        vec!["diagnose"],
        vec!["diagnose", path, "--kind", "nope"],
        vec!["diagnose", path, "--severity", "fatal"],
        vec!["diagnose", path, "--limit", "0"],
        vec!["diagnose", path, "--document", "x"],
        vec!["diagnose", path, "--project", "--document", &broken_id],
        vec!["diagnose", path, "--cursor", "nope"],
        vec!["diagnose", path, "extra"],
    ] {
        assert_eq!(stellar(&bad).status.code(), Some(64), "{bad:?}");
    }

    // 프로젝트가 바뀌면 이전 cursor는 거부합니다.
    let first_page = envelope(&stellar(&["diagnose", path, "--limit", "10"]), 0);
    let Some(JsonValue::String(cursor)) = result(&first_page).get("next_cursor").cloned() else {
        panic!("cursor");
    };
    save_documents(&dir.path, &[&broken]);
    let stale = envelope(&stellar(&["diagnose", path, "--cursor", &cursor]), 65);
    assert_eq!(str_of(stale.get("error").unwrap(), "code"), "stale-cursor");
}
