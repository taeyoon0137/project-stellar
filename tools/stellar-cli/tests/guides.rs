//! `stellar guides`: 저장된 Frame 가이드·레이아웃 그리드와 View 변수·Variant 조회(XII.10.19·XII.10.20·XVI.11.1, ADR-0126).

use std::path::Path;
use std::process::{Command, Stdio};

use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_ops::commands::document::expressions::types::{ExpressionType, TypeSpec};
use stellar_ops::commands::document::guides::{
    GuideAxis, GuideEdit, LayoutGrid, NodeGuides, RulerGuide,
};
use stellar_ops::commands::document::view_variant::commands::{
    ViewVariantCommand, ViewVariantModel,
};
use stellar_ops::commands::document::view_variant::condition::VariantCondition;
use stellar_ops::commands::document::view_variant::resolver::{ViewVariant, ViewVariantId};
use stellar_ops::commands::document::view_variant::variable::{
    VariableValue, ViewVariable, ViewVariableId,
};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::core_types::typed_value::TypedValue;
use stellar_ops::test_support::{large_document, save_documents, scaffold_project, TempDir};

fn stellar(args: &[&str], home: &Path) -> (i32, JsonValue) {
    let output = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    if stdout.trim().is_empty() {
        // 사용법 오류는 stderr에만 씁니다.
        return (output.status.code().unwrap(), JsonValue::Null);
    }
    let envelope = parse(stdout.trim_end(), JsonLimits::default())
        .unwrap_or_else(|e| panic!("{e:?}: {stdout}"));
    (output.status.code().unwrap(), envelope)
}

fn array<'a>(value: &'a JsonValue, key: &str) -> &'a [JsonValue] {
    match value.get(key) {
        Some(JsonValue::Array(items)) => items,
        other => panic!("{key}: {other:?}"),
    }
}

fn str_of<'a>(value: &'a JsonValue, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .unwrap_or_else(|| panic!("{key}: {value:?}"))
}

fn sid(n: u64) -> StableId {
    StableId::new_v7(6_100_000_000_000 + n, 0x0c0d, n)
}

#[test]
fn guides_reads_saved_guides_grids_and_view_variables() {
    let dir = TempDir::new("guides-cli");
    let root = dir.path.join("project");
    let home = dir.path.join("home");
    scaffold_project(&root);
    let mut model = large_document(3);
    let document = model.document_id();
    let view = model.definition().pages[0].views[0].clone();
    let mut guides = NodeGuides::new(view.root_frame);
    guides
        .apply(&GuideEdit::Add(
            RulerGuide::new(sid(1), GuideAxis::X, 120.0).unwrap(),
        ))
        .unwrap();
    guides
        .apply(&GuideEdit::SetGrids(vec![LayoutGrid::columns(
            12, 20.0, 32.0,
        )
        .unwrap()]))
        .unwrap();
    model.set_node_guides(guides).unwrap();
    let mut variants = ViewVariantModel::new(view.id);
    for command in [
        ViewVariantCommand::CreateVariable(
            ViewVariable::new(
                ViewVariableId::new(sid(2)),
                "dark".into(),
                TypeSpec::required(ExpressionType::Boolean),
                VariableValue::Value(TypedValue::Boolean(false)),
                [],
            )
            .unwrap(),
        ),
        ViewVariantCommand::CreateVariant(ViewVariant::new(
            ViewVariantId::new(sid(3)),
            VariantCondition::new().equals(
                ViewVariableId::new(sid(2)),
                VariableValue::Value(TypedValue::Boolean(true)),
            ),
            Default::default(),
        )),
    ] {
        command.apply(&mut variants).unwrap();
    }
    model.set_view_variants(variants).unwrap();
    save_documents(&root, &[&model]);
    let root_path = root.to_str().unwrap();
    let document_text = document.to_string();

    let (code, envelope) = stellar(&["guides", root_path, &document_text], &home);
    assert_eq!(code, 0, "{envelope:?}");
    let result = envelope.get("result").unwrap();
    let frames = array(result, "frames");
    assert_eq!(frames.len(), 1);
    assert_eq!(str_of(&frames[0], "owner"), view.root_frame.to_string());
    let guide = &array(&frames[0], "guides")[0];
    assert_eq!(str_of(guide, "axis"), "x");
    assert_eq!(str_of(guide, "position"), "120px");
    assert_eq!(str_of(guide, "line"), format!("guide x 120px {}", sid(1)));
    let grid = &array(&frames[0], "grids")[0];
    assert_eq!(
        str_of(grid, "line"),
        "grid columns 12 auto gap 20px margin 32px stretch"
    );
    assert_eq!(array(grid, "columns").len(), 12);
    let views = array(result, "views");
    assert_eq!(views.len(), 1);
    let variable = &array(&views[0], "variables")[0];
    assert_eq!(str_of(variable, "name"), "dark");
    assert_eq!(str_of(variable, "default"), "boolean false");
    let variant = &array(&views[0], "variants")[0];
    assert_eq!(
        array(variant, "when")[0].as_str(),
        Some(format!("{} = boolean true", sid(2)).as_str())
    );

    // --node는 그 Frame만, Frame이 아닌 id는 실패입니다.
    let root_text = view.root_frame.to_string();
    let (code, envelope) = stellar(
        &["guides", root_path, &document_text, "--node", &root_text],
        &home,
    );
    assert_eq!(code, 0, "{envelope:?}");
    let result = envelope.get("result").unwrap();
    assert_eq!(array(result, "frames").len(), 1);
    assert!(array(result, "views").is_empty());
    let missing = sid(99).to_string();
    let (code, _) = stellar(
        &["guides", root_path, &document_text, "--node", &missing],
        &home,
    );
    assert_ne!(code, 0);
    let (code, _) = stellar(&["guides", root_path], &home);
    assert_eq!(code, 64);
}
