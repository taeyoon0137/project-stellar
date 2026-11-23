//! XXII.5.1 검증: same input byte-identical, 파일 순서, 식별자 escape, formatting.

use stellar_codegen::ir::{
    IrAction, IrContextConsumption, IrContextKey, IrContextProvision, IrContextSource, IrContexts,
    IrEnvironmentValue, IrExpr, IrInteractionState, IrMotion, IrMotionStart, IrOwner, IrProperty,
    IrReducedMotion, IrState, IrStateMachine, IrTiming, IrToken, IrTransition, IrTrigger,
};
use stellar_target_profile::capability::{ContractCode, ElementKind};
use stellar_target_profile::overrides::ValueOrigin;

use super::*;
use crate::core_types::color::{Color, ColorSpace};
use crate::core_types::property::{PropertyKey, PropertyPath};
use crate::core_types::typed_value::TypedValue;
use crate::core_types::value::Length;
use crate::export_web_tokens::tests::{fixture_tokens, id, node};

fn rgb(r: u8, g: u8, b: u8) -> TypedValue {
    let c = |v: u8| f32::from(v) / 255.0;
    TypedValue::Color(Color::new(ColorSpace::Srgb, c(r), c(g), c(b), 1.0).expect("color"))
}

fn background() -> PropertyKey {
    stellar_document::css::registry::entries()
        .into_iter()
        .find(|e| e.scope.name() == "frame" && e.name == "background-color")
        .expect("background-color")
        .key
}

/// root(1) ─ Button(2, 상호작용 토큰 배경, 텍스트) ─ Text(3, `</script>` 텍스트) ─ Frame(4, Context 소비).
pub(crate) fn fixture() -> IrTree {
    let mut root = node(1, None, 0, ElementKind::FRAME);
    root.children = vec![1, 2, 3];
    let mut button = node(2, Some(0), 1, ElementKind::FRAME);
    button.contracts = vec![ContractCode::BUTTON];
    button.text = Some("<저장 & \"확인\">".into());
    button.properties.push(IrProperty {
        path: PropertyPath::new(id(2)).push(background()),
        value: rgb(255, 255, 255),
        origin: ValueOrigin::Common,
        token: Some(id(7)),
        capability: None,
    });
    let mut text = node(3, Some(0), 1, ElementKind::TEXT);
    text.text = Some("a </script> b".into());
    let frame = node(4, Some(0), 1, ElementKind::FRAME);
    let mut ir = IrTree::from_nodes(vec![root, button, text, frame]).expect("tree");
    let mut tokens = fixture_tokens();
    tokens.push(IrToken {
        id: id(40),
        name: "he said \"hi\"/x.y".into(),
        value: TypedValue::Length(Length::new(2.0).expect("len")),
        alias: None,
        variants: Vec::new(),
    });
    tokens.sort_by_key(|t| t.id);
    // 7(hover-fill)은 상호작용 상태 토큰입니다.
    assert!(tokens.iter().any(|t| t.id == id(7)
        && t.variants[0].when
            == vec![IrEnvironmentValue::InteractionState(
                IrInteractionState::Hover
            )]));
    ir.tokens = tokens;
    ir.states = vec![IrState {
        id: id(50),
        name: "is \"saving\"".into(),
        initial: TypedValue::Boolean(false),
    }];
    ir.machines = vec![IrStateMachine {
        owner: IrOwner::View(id(60)),
        variables: vec![id(50)],
        actions: Vec::new(),
        transitions: vec![IrTransition {
            id: id(61),
            trigger: IrTrigger::Event {
                kind: "activate",
                source: id(2),
            },
            condition: None,
            actions: vec![IrAction::SetState {
                variable: id(50),
                value: IrExpr::Literal(TypedValue::Boolean(true)),
            }],
        }],
    }];
    ir.motions = vec![IrMotion {
        id: id(70),
        owner: IrOwner::View(id(60)),
        targets: vec![PropertyPath::new(id(2)).push(background())],
        start: IrMotionStart::Becomes {
            state: id(50),
            value: TypedValue::Boolean(true),
        },
        duration_ms: 150.0,
        delay_ms: 0.0,
        timing: IrTiming::Linear,
        reduced_motion: IrReducedMotion::Instant,
    }];
    ir.contexts = IrContexts {
        keys: vec![IrContextKey {
            id: id(80),
            name: "accent".into(),
            default: IrExpr::Literal(rgb(0, 0, 0)),
        }],
        provisions: vec![IrContextProvision {
            node: id(1),
            key: id(80),
            value: IrExpr::Token(id(2)),
            depths: vec![1],
        }],
        consumptions: vec![IrContextConsumption {
            node: id(4),
            path: PropertyPath::new(id(4)).push(background()),
            key: id(80),
            source: IrContextSource::Provision(id(1)),
            slots: Vec::new(),
        }],
        slots: Vec::new(),
    };
    ir
}

#[test]
fn same_input_is_byte_identical_and_files_are_in_fixed_order() {
    // 검증 기준(XXII.5.1): same input byte-identical.
    let a = export_web(&fixture(), None).expect("export");
    let b = export_web(&fixture(), None).expect("export");
    assert_eq!(a.files.manifest_text(), b.files.manifest_text());
    let paths: Vec<&str> = a.files.iter().map(|(p, _)| p).collect();
    assert_eq!(
        paths,
        [
            "index.html",
            "state.js",
            "styles.css",
            "tokens.css",
            "tokens.js"
        ]
    );
    let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/export_web/fixture/web_export.txt");
    if std::env::var_os("STELLAR_UPDATE_SNAPSHOT_FIXTURES").is_some() {
        std::fs::write(&golden, a.files.manifest_text()).expect("갱신");
    }
    assert_eq!(
        a.files.manifest_text(),
        std::fs::read_to_string(golden).expect("golden")
    );
}

#[test]
fn identifiers_are_escaped_and_selectors_point_at_ids() {
    let out = export_web(&fixture(), None).expect("export");
    let html = out.files.get("index.html").expect("html");
    let styles = out.files.get("styles.css").expect("styles");
    // 모든 selector의 element id가 HTML에 있습니다.
    for word in styles.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '#')) {
        if let Some(element) = word.strip_prefix("#s-") {
            assert!(
                html.contains(&format!("id=\"s-{element}\"")),
                "selector #s-{element}의 id가 없습니다"
            );
        }
    }
    // HTML 텍스트는 escape, JavaScript 문자열은 JSON escape(`</` 포함), 토큰 이름은 CSS identifier escape입니다.
    assert!(html.contains("&lt;저장 &amp; \"확인\"&gt;"));
    assert!(html.contains("a &lt;/script&gt; b"));
    let tokens = out.files.get("tokens.css").expect("tokens");
    assert!(
        tokens.contains("--he-said-\\\"hi\\\"-x-y: 2px;"),
        "{tokens}"
    );
    for (path, content) in out.files.iter() {
        assert!(
            content.ends_with('\n') && !content.ends_with("\n\n"),
            "{path}"
        );
        assert!(!content.contains('\r'), "{path}");
        assert!(content.contains("stellar:generated"), "{path}");
        if path.ends_with(".js") {
            assert!(!content.contains("</script"), "{path}");
        }
    }
}
