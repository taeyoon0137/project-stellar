//! XVI.11.1 종단 시나리오: 새 프로젝트 생성 → Frame·Text·Image 추가 → Component·Token 생성과 Component 사용 →
//! Save → Close → Reopen 뒤 semantic snapshot이 같은지.
//!
//! 앱 UI 없이 제품 경로만 씁니다(ADR-0111).
//!
//! | 단계 | 경로 |
//! |---|---|
//! | 새 프로젝트, Frame·Text·Image, 속성 편집, Save Now, 창·프로젝트 닫기, 다시 열기 | 앱 runtime(셸과 같은 진입점) |
//! | Component 정의 생성 | `stellar component-source create`와 같은 `stellar_ops::component_source::create` |
//! | Token set 생성 | 텍스트 원본 `design/tokens/<set>.stellar.yaml`을 codec 공개 encoder로 씀(AI·편집기가 파일을 직접 고치는 경로) |
//! | Component 사용 | `stellar apply`의 `InsertComponentInstance`(CLI가 cache generation을 쓰고 앱이 다음 열기에 원본으로 옮김) |
//! | View 변수·Variant 생성 | `edit-view-variants` command(ADR-0126)를 headless 공용 저장(`ops::snapshot::save`)으로 기록 |
//!
//! "semantic snapshot 동일"은 세 겹으로 확인합니다: 문서 semantic snapshot JSON(VI.10.4), 텍스트 원본 전체 의미
//! (`TextProject::semantic_key`: 문서 레코드·컴포넌트 library bytes·token set·이미지 record), 그리고 다시 연 창의 Save
//! Now가 쓸 것이 없다는 사실(디스크 = 창 메모리).
//!
//! View 변수·Variant는 View 원본 파일의 `variables`·`variants`와 View root subtree chunk에 저장됩니다(ADR-0126). 이
//! 시나리오는 변수 하나와 그 변수를 조건으로 쓰는 Variant 하나를 만들고 다시 열어 같은지 봅니다.

mod e2e_common;

use std::path::Path;

use e2e_common::{
    git, headless_json, json, porcelain, reopen, trusted_git, App, TempDir, TINY_PNG,
};
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Rect, Size2, Vec2};
use stellar_document::records::NodePayload;
use stellar_ops::apply::{apply, ApplyMode, ApplyRequest, CommandInput, ComponentInstanceInsert};
use stellar_ops::lock::{default_lock_dir, LockPolicy};
use stellar_ops::query::project_structure;
use stellar_ops::snapshot::open_snapshot;
use stellar_project::document_io::session::{decode_source, read_source};
use stellar_project::document_io::source_v4::project::SemanticKey;
use stellar_project::document_io::source_v4::token::{self, TokenSetSource, TokenSource};
use stellar_runtime::app::inspector::InspectorProperty;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(Vec2::new(x, y).unwrap(), Size2::new(w, h).unwrap())
}

fn new_id() -> StableId {
    stellar_project::new_ids::new_ids::<1>().expect("UUIDv7")[0]
}

/// 텍스트 원본 전체 의미(문서·컴포넌트·token·이미지).
fn source_key(root: &Path) -> SemanticKey {
    let files = read_source(root)
        .expect("원본 읽기")
        .expect("텍스트 원본 프로젝트");
    decode_source(&files)
        .expect("원본 해석")
        .semantic_key()
        .expect("의미 key")
}

/// `design/` 아래 원본 파일 이름 목록.
fn source_paths(root: &Path) -> Vec<String> {
    read_source(root)
        .unwrap()
        .map(|files| files.files.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn new_project_edit_save_close_reopen_keeps_semantic_snapshot() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("new-project");
    let root = dir.path.join("새 디자인");

    // ── 1. 새 프로젝트: 첫 문서·Page·View·Frame을 generation 1로 저장하고 최초 commit까지 합니다(IV.4).
    let app = App::start();
    assert_eq!(app.create_project(&root), 0, "최초 commit까지 성공");
    assert!(
        porcelain(&gate, &root).is_empty(),
        "생성 직후 작업 트리 clean"
    );
    let first_commit = git(&gate, &root, &["rev-parse", "HEAD"]);
    let structure = project_structure(&open_snapshot(&root).unwrap());
    assert_eq!(structure.documents.len(), 1);
    let document = structure.documents[0].id;
    let view = &structure.documents[0].pages[0].views[0];
    let (view_id, view_root) = (view.id, view.root_frame);

    // ── 2. Frame·Text·Image 추가와 속성 편집(앱 runtime).
    let project = app.open_project(&root);
    let (window, opened, generation) = app.open_window(project);
    assert_eq!((opened, generation), (document, 1));
    assert_eq!(app.window(window).active_view, Some(view_id));
    let card = app.insert_frame(window, view_root, rect(48.0, 64.0, 320.0, 200.0));
    app.edit_property(window, InspectorProperty::BorderRadius, "12px");
    app.edit_property(window, InspectorProperty::BackgroundColor, "#204080");
    let title = app.insert_text(window, card, Vec2::new(16.0, 16.0).unwrap(), "Stellar 카드");
    let png = dir.path.join("hero.png");
    std::fs::write(&png, TINY_PNG).unwrap();
    let image = app.place_png(
        window,
        card,
        Vec2::new(16.0, 64.0).unwrap(),
        Some(Size2::new(96.0, 96.0).unwrap()),
        &png,
    );
    let summary = app.window(window);
    assert_eq!(
        summary.undo_depth, 5,
        "Frame·속성 2개·Text·Image가 각각 history 항목"
    );
    assert!(summary.has_unsaved_changes);

    // ── 3. Save Now: 텍스트 원본(Git 원본)과 cache generation을 함께 씁니다(ADR-0088).
    let (saved, written) = app.save(window);
    assert!(written && saved == 2, "첫 편집 저장은 generation 2");
    assert!(!app.window(window).has_unsaved_changes);
    let paths = source_paths(&root);
    assert!(
        paths.contains(&"document.stellar.yaml".to_string()),
        "{paths:?}"
    );
    assert!(paths.iter().any(|p| p.starts_with("pages/")), "{paths:?}");
    assert!(
        paths.iter().any(|p| p.starts_with("assets/")),
        "이미지 catalog: {paths:?}"
    );
    let saved_doc = headless_json(&root, document);
    for id in [card, title, image] {
        assert!(saved_doc.contains(&id.to_string()), "{id} 저장");
    }

    // ── 4. Component 정의 생성(CLI `component-source create`)과 token set 생성(원본 파일 직접 편집).
    let component = new_id();
    let component_root = new_id();
    std::fs::create_dir_all(root.join("design/components")).unwrap();
    let info = stellar_ops::component_source::create(
        &root.join("design/components"),
        "Badge".into(),
        component,
        component_root,
    )
    .expect("컴포넌트 원본 생성");
    assert_eq!((info.id, info.root), (component, component_root));
    let component_file = root
        .join("design/components")
        .join(info.file.file_name().unwrap());
    let component_bytes = std::fs::read(&component_file).expect("컴포넌트 원본 파일");
    let brand = TokenSetSource {
        id: new_id(),
        name: "brand".into(),
        tokens: vec![TokenSource {
            id: new_id(),
            name: "primary".into(),
            expression: new_id(),
            value: TypedValue::Color(Color::new(ColorSpace::Srgb, 0.125, 0.25, 0.5, 1.0).unwrap()),
        }],
        computed: Vec::new(),
    };
    let token_path = root.join("design/tokens/brand.stellar.yaml");
    std::fs::create_dir_all(token_path.parent().unwrap()).unwrap();
    let token_text = token::encode(&brand).expect("token set encode");
    std::fs::write(&token_path, &token_text).unwrap();
    let with_library = source_key(&root);
    assert_eq!(with_library.tokens, vec![brand.clone()]);
    assert!(!with_library.components.is_empty());

    // 문서 밖 의미 파일만 바뀌었으므로 창은 stale이 아니고(ADR-0088 결정 5), 창 저장은 그 파일을 지우거나 바꾸지
    // 않습니다. 컴포넌트 원본 다시 읽기도 셸과 같은 요청입니다.
    let request = app
        .rt
        .reload_canvas_components(window)
        .expect("컴포넌트 다시 읽기");
    app.ok(request, "컴포넌트 다시 읽기");
    app.edit_property(window, InspectorProperty::Name, "카드");
    let (saved, written) = app.save(window);
    assert!(written && saved == 3);
    assert_eq!(
        std::fs::read_to_string(&token_path).unwrap(),
        token_text,
        "token 원본 보존"
    );
    assert_eq!(
        std::fs::read(&component_file).unwrap(),
        component_bytes,
        "컴포넌트 원본 보존"
    );
    let before_close = source_key(&root);
    assert_eq!(before_close.tokens, with_library.tokens);
    assert_eq!(before_close.components, with_library.components);
    let before_close_doc = headless_json(&root, document);

    // ── 5. Close: 창·프로젝트를 닫고 앱 runtime을 끝냅니다.
    app.close_window(window);
    app.close_project(project);
    app.shutdown();
    assert_eq!(source_key(&root), before_close, "닫기는 원본을 바꾸지 않음");

    // ── 6. Reopen: 새 앱 runtime에서 같은 프로젝트를 엽니다. 원본·cache가 짝이므로 cache에서 작업 문서만 읽습니다.
    let app = App::start();
    let project = app.open_project(&root);
    let (window, opened, generation) = app.open_window(project);
    assert_eq!((opened, generation), (document, 3));
    let reopened = app.window(window);
    assert!(!reopened.has_unsaved_changes);
    assert_eq!(
        (reopened.undo_depth, reopened.redo_depth),
        (0, 0),
        "history는 이어지지 않음"
    );
    assert_eq!(reopened.active_view, Some(view_id));
    assert_eq!(app.save(window), (3, false), "다시 연 창 = 디스크");
    assert_eq!(source_key(&root), before_close);
    assert_eq!(headless_json(&root, document), before_close_doc);
    let session = reopen(&root);
    assert_eq!(json(session.document().unwrap().model()), before_close_doc);
    app.close_window(window);
    app.close_project(project);
    app.shutdown();

    // ── 7. Component 사용: CLI `stellar apply`로 인스턴스를 넣고(cache generation), 앱이 다시 열 때 원본으로 옮깁니다.
    let components = stellar_ops::component_source::query(&open_snapshot(&root).unwrap())
        .expect("컴포넌트 조회");
    assert_eq!(components.entries.len(), 1);
    assert_eq!(components.entries[0].id, component);
    let snapshot = open_snapshot(&root).unwrap();
    let outcome = apply(
        &root,
        &ApplyRequest {
            base_revision: snapshot.revision(),
            document,
            commands: vec![CommandInput::InsertComponentInstance(
                ComponentInstanceInsert {
                    component,
                    source_hash: components.entries[0].hash,
                    parent: card,
                    position: Vec2::new(200.0, 16.0).unwrap(),
                },
            )],
        },
        ApplyMode::Write,
        &LockPolicy::Required {
            dir: default_lock_dir().unwrap(),
        },
    )
    .expect("인스턴스 삽입");
    assert_eq!(outcome.command_count, 1);
    let records = open_snapshot(&root)
        .unwrap()
        .document_records(document)
        .unwrap();
    let instance = records
        .nodes
        .iter()
        .find(|n| matches!(&n.payload, NodePayload::ComponentInstance(i) if i.component().id() == component))
        .expect("인스턴스 node")
        .id;
    let with_instance = headless_json(&root, document);

    let app = App::start();
    let project = app.open_project(&root);
    let (window, _, generation) = app.open_window(project);
    assert_eq!(generation, 4, "CLI가 쓴 generation");
    assert!(!app.window(window).has_unsaved_changes);
    assert!(!app.save(window).1, "열기 맞춤 쓰기 뒤 쓸 것이 없음");
    let after_cli = source_key(&root);
    assert!(
        after_cli.records.nodes.iter().any(|n| n.id == instance),
        "앱이 연 뒤 원본에도 인스턴스"
    );
    assert_eq!(after_cli.components, before_close.components);
    assert_eq!(after_cli.tokens, before_close.tokens);
    assert_eq!(headless_json(&root, document), with_instance);
    app.close_window(window);
    app.close_project(project);
    app.shutdown();

    // 한 번 더 닫고 열어도 같습니다.
    let session = reopen(&root);
    assert_eq!(json(session.document().unwrap().model()), with_instance);
    assert_eq!(source_key(&root), after_cli);

    // ── 7b. View 변수·Variant 생성: 변수 하나와 그 변수를 조건으로 쓰는 Variant 하나(override: 카드 이름)를 command 두 개로
    //       만들고 공용 저장으로 원본·cache에 씁니다. 앱이 다시 열어도 같고, 다시 연 창은 쓸 것이 없습니다.
    use stellar_commands::command::Command;
    use stellar_commands::guides::EditViewVariants;
    use stellar_commands::request::CommandRequest;
    use stellar_commands::state::DocumentState;
    use stellar_core_types::property::{PropertyKey, PropertyPath};
    use stellar_document::expressions::types::{ExpressionType, TypeSpec};
    use stellar_document::view_variant::commands::ViewVariantCommand;
    use stellar_document::view_variant::condition::VariantCondition;
    use stellar_document::view_variant::resolver::{ViewVariant, ViewVariantId};
    use stellar_document::view_variant::variable::{VariableValue, ViewVariable, ViewVariableId};
    let snapshot = open_snapshot(&root).unwrap();
    let mut state = DocumentState::new(snapshot.document_model(document).unwrap());
    let (variable, variant) = (new_id(), new_id());
    let mut overrides = stellar_document::component::variant_override::VariantOverrideSet::new();
    overrides.set_property(
        PropertyPath::with_keys(card, vec![PropertyKey(0x0110)]),
        TypedValue::Boolean(true),
    );
    for command in [
        ViewVariantCommand::CreateVariable(
            ViewVariable::new(
                ViewVariableId::new(variable),
                "dark".into(),
                TypeSpec::required(ExpressionType::Boolean),
                VariableValue::Value(TypedValue::Boolean(false)),
                [],
            )
            .unwrap(),
        ),
        ViewVariantCommand::CreateVariant(ViewVariant::new(
            ViewVariantId::new(variant),
            VariantCondition::new().equals(
                ViewVariableId::new(variable),
                VariableValue::Value(TypedValue::Boolean(true)),
            ),
            overrides.clone(),
        )),
    ] {
        let command = Command::EditViewVariants(Box::new(EditViewVariants {
            view: view_id,
            command,
        }));
        state
            .execute(&CommandRequest::at(&state, command))
            .expect("View 변수·Variant command");
    }
    let expected = state
        .model()
        .view_variants(view_id)
        .cloned()
        .expect("View model");
    stellar_ops::snapshot::save::save_documents(
        &snapshot,
        stellar_ops::snapshot::save::root_order(&snapshot),
        vec![state
            .model()
            .to_records()
            .map(|r| {
                stellar_document::model::DocumentModel::from_records(r)
                    .unwrap()
                    .0
            })
            .unwrap()],
    )
    .expect("공용 저장");
    let with_variants = source_key(&root);
    assert_eq!(with_variants.records.view_variants, vec![expected.clone()]);
    let view_file =
        std::fs::read_to_string(root.join("design/pages/page-1/view-1.stellar.yaml")).unwrap();
    assert!(
        view_file.contains("variables:") && view_file.contains("variants:"),
        "{view_file}"
    );
    let app = App::start();
    let project = app.open_project(&root);
    let (window, _, _) = app.open_window(project);
    assert!(!app.window(window).has_unsaved_changes);
    assert!(!app.save(window).1, "다시 연 창 = 디스크");
    app.close_window(window);
    app.close_project(project);
    app.shutdown();
    let session = reopen(&root);
    assert_eq!(
        session.document().unwrap().model().view_variants(view_id),
        Some(&expected)
    );
    assert_eq!(source_key(&root), with_variants);

    // ── 8. Git: 최초 commit 이후 변경은 작업 트리에만 있고 HEAD·index는 그대로입니다.
    assert_eq!(git(&gate, &root, &["rev-parse", "HEAD"]), first_commit);
    assert_eq!(git(&gate, &root, &["diff", "--cached", "--name-only"]), "");
    // 새 프로젝트의 최초 commit이 이미 텍스트 원본을 담습니다(ADR-0113 결정 4). 그 뒤 편집은 바뀐 원본 파일로 보입니다.
    assert_eq!(
        git(
            &gate,
            &root,
            &[
                "ls-tree",
                "--name-only",
                first_commit.trim(),
                "design/document.stellar.yaml"
            ]
        )
        .trim(),
        "design/document.stellar.yaml"
    );
    let status = porcelain(&gate, &root);
    for expected in [
        "design/pages/page-1/view-1.stellar.yaml",
        "design/tokens/brand.stellar.yaml",
    ] {
        assert!(
            status.iter().any(|l| l.ends_with(expected)),
            "{expected}: {status:?}"
        );
    }
    assert!(
        status.iter().any(|l| l.contains("design/components/")),
        "{status:?}"
    );
}
