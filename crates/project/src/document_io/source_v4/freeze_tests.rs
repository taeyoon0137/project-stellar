//! 공개 포맷 동결 전 검증(V.10.4, ADR-0131 결정 5).
//!
//! 텍스트 원본이 쓰는 모든 파일 종류(문서·View·Frame 하위 파일·catalog·Context·컴포넌트 정의·interface·리터럴·계산
//! token·추가 root 문서)를 한 프로젝트에 담아 다음을 고정합니다.
//!
//! 1. 새로 더한 파일 종류(interface·계산 token·추가 root 문서 목록)의 golden bytes. 바꾸려면
//!    `STELLAR_TEXT_SOURCE_BLESS=1`로 다시 쓰고 diff를 검토합니다.
//! 2. 같은 프로젝트는 같은 파일 집합·bytes이고, 다시 읽은 의미가 같습니다.
//! 3. 어느 파일의 어느 한 줄을 지워도 읽기가 거부하거나 의미가 달라집니다. 같은 의미로 읽히는 줄은 생략하면 기본값으로
//!    읽는 줄(ADR-0082·ADR-0086)뿐이고, 그 입력을 다시 쓰면 원래 canonical bytes로 돌아옵니다(의미 하나 = 정규형 하나).

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::{ComponentRef, TokenRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::Length;
use stellar_document::component::definition::{ComponentLibrary, ComponentPropertyId};
use stellar_document::component::event::EventKind;
use stellar_document::component::property::{ComponentProperty, PropertyVisibility};
use stellar_document::component::state_machine::{
    ActionSpec, PropertyScope, SemanticAction, SemanticActionId, StateMachine, StateVariable,
    StateVariableId, TransitionId, TransitionSpec, Trigger,
};
use stellar_document::context::fixtures::{ComponentBuilder, SceneBuilder};
use stellar_document::expressions::ast::{ExprNode, ExprNodeId, ExpressionAst, ReferenceTarget};
use stellar_document::expressions::types::{ExpressionType, TypeSpec};

use super::documents;
use super::project::{decode, encode, TextProject};
use super::tests::{golden_model, sid};
use super::token::{ComputedTokenSource, TokenSetSource, TokenSource};

/// Property(public Boolean)·Slot·상태 머신(action·Property 참조 초기값 변수·transition)을 가진 컴포넌트 library.
pub(crate) fn interface_library(sid: fn(u64) -> StableId) -> ComponentLibrary {
    let literal = |seed: u64, value: TypedValue| {
        let root = ExprNodeId::new(sid(seed));
        ExpressionAst::new(root, vec![ExprNode::literal(root, value)]).unwrap()
    };
    let component = ComponentRef::new(sid(500));
    let mut library = ComponentLibrary::new();
    let property = ComponentPropertyId::new(sid(510));
    // 정의 Property 선언은 builder 밖에서 합니다(builder는 Slot만 선언).
    let mut declared = ComponentLibrary::new();
    ComponentBuilder::new(sid(500), "Toggle", sid(501))
        .frame(sid(502), sid(501))
        .frame(sid(503), sid(502))
        .slot(sid(520), sid(502))
        .register(&mut declared);
    let entry = declared.get(component).unwrap();
    let mut definition = entry.definition().clone();
    definition.add_property(property).unwrap();
    let tree = entry.tree().clone();
    let slots: Vec<_> = entry.slots().cloned().collect();
    library.insert(definition, tree).unwrap();
    library
        .add_property(
            component,
            ComponentProperty::new(
                property,
                "initiallyOn".into(),
                TypeSpec::required(ExpressionType::Boolean),
                literal(511, TypedValue::Boolean(true)),
                PropertyVisibility::Public,
            )
            .unwrap(),
        )
        .unwrap();
    for slot in slots {
        library.add_slot(component, slot).unwrap();
    }
    let entry = library.get(component).unwrap();
    let scope = PropertyScope::new(entry);
    let mut machine = StateMachine::new();
    machine
        .add_action(SemanticAction::new(SemanticActionId::new(sid(530)), "pressed".into()).unwrap())
        .unwrap();
    let initial = {
        let root = ExprNodeId::new(sid(541));
        ExpressionAst::new(
            root,
            vec![ExprNode::reference(
                root,
                ReferenceTarget::ComponentProperty(sid(510)),
            )],
        )
        .unwrap()
    };
    machine
        .add_variable(
            StateVariable::new(
                StateVariableId::new(sid(540)),
                "on".into(),
                TypeSpec::required(ExpressionType::Boolean),
                initial,
                &scope,
            )
            .unwrap(),
        )
        .unwrap();
    machine
        .add_transition(
            entry,
            TransitionSpec {
                id: TransitionId::new(sid(550)),
                trigger: Trigger::Event {
                    kind: EventKind::Activate,
                    source: sid(501),
                },
                condition: None,
                actions: vec![
                    ActionSpec::SetState {
                        variable: StateVariableId::new(sid(540)),
                        value: literal(551, TypedValue::Boolean(false)),
                    },
                    ActionSpec::Emit {
                        action: SemanticActionId::new(sid(530)),
                    },
                ],
            },
        )
        .unwrap();
    library.set_state_machine(component, machine).unwrap();
    library
}

fn tokens() -> Vec<TokenSetSource> {
    let reference = {
        let root = ExprNodeId::new(sid(613));
        ExpressionAst::new(
            root,
            vec![ExprNode::reference(
                root,
                ReferenceTarget::Token(TokenRef::new(sid(602))),
            )],
        )
        .unwrap()
    };
    vec![TokenSetSource {
        id: sid(600),
        name: "Brand".into(),
        tokens: vec![TokenSource {
            id: sid(602),
            name: "space/4".into(),
            expression: sid(612),
            value: TypedValue::Length(Length::new(16.0).unwrap()),
        }],
        computed: vec![ComputedTokenSource {
            id: sid(603),
            name: "space/gutter".into(),
            value_type: TypeSpec::required(ExpressionType::Length),
            default: reference,
        }],
    }]
}

fn extra_document() -> stellar_document::model::DocumentModel {
    SceneBuilder::new(sid(701), sid(702), sid(703), sid(710))
        .frame(sid(711), sid(710))
        .build()
}

/// 모든 파일 종류를 담은 프로젝트의 파일 집합.
fn full_project_files() -> (TextProject, BTreeMap<String, String>) {
    let project = TextProject {
        document: golden_model(),
        components: interface_library(sid),
        token_sets: tokens(),
        images: Default::default(),
    };
    let mut files = encode(&project).unwrap().files;
    files.extend(documents::encode(project.document.document_id(), &[extra_document()]).unwrap());
    (project, files)
}

/// 파일 집합 전체를 읽은 의미 key(첫 문서·프로젝트 의미 + 추가 root 문서).
fn full_key(
    files: &BTreeMap<String, String>,
) -> Result<
    (
        super::project::SemanticKey,
        Vec<super::project::SemanticKey>,
    ),
    super::super::text_source::TextSourceError,
> {
    let project = decode(files)?;
    let extras = documents::decode(project.document.document_id(), files)?
        .iter()
        .map(documents::semantic)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((project.semantic_key()?, extras))
}

fn golden(name: &str, actual: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/document_io/source_v4/golden")
        .join(name);
    if std::env::var_os("STELLAR_TEXT_SOURCE_BLESS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (STELLAR_TEXT_SOURCE_BLESS=1로 만듦)",
            path.display()
        )
    });
    assert_eq!(actual, expected, "{name} golden bytes");
}

#[test]
fn new_file_kinds_have_fixed_golden_bytes() {
    let (_, files) = full_project_files();
    let pick = |prefix: &str| {
        files
            .iter()
            .find(|(path, _)| path.starts_with(prefix))
            .map(|(_, text)| text.as_str())
            .unwrap_or_else(|| panic!("{prefix} 파일이 없음"))
    };
    golden("interface-v4.stellar.yaml", pick("interfaces/"));
    golden("tokens-v4.stellar.yaml", pick("tokens/"));
    golden("documents-v4.stellar.yaml", pick(documents::DOCUMENTS_PATH));
}

#[test]
fn full_project_round_trips_with_the_same_bytes() {
    let (project, files) = full_project_files();
    let (key, extras) = full_key(&files).unwrap();
    assert_eq!(key, project.semantic_key().unwrap());
    assert_eq!(
        extras,
        vec![documents::semantic(&extra_document()).unwrap()]
    );
    let again = full_project_files().1;
    assert_eq!(again, files, "같은 프로젝트는 같은 파일 집합·bytes");
    // 다시 읽은 의미를 다시 쓰면 같은 bytes입니다.
    let decoded = decode(&files).unwrap();
    let extras = documents::decode(decoded.document.document_id(), &files).unwrap();
    let mut rewritten = encode(&decoded).unwrap().files;
    rewritten.extend(documents::encode(decoded.document.document_id(), &extras).unwrap());
    assert_eq!(rewritten, files);
    for kind in [
        "document.stellar.yaml",
        "pages/",
        "assets/catalog.stellar.yaml",
        "components/",
        "interfaces/",
        "tokens/",
        "documents.stellar.yaml",
        "documents/",
    ] {
        assert!(
            files.keys().any(|path| path.starts_with(kind)),
            "{kind} 파일 종류가 있어야 함"
        );
    }
}

/// 어느 파일의 어느 한 줄을 지워도 읽기가 거부하거나 의미가 달라지거나, 생략값을 기본값으로 읽는 줄이면 다시 쓴 결과가
/// 원래 canonical bytes입니다.
#[test]
fn deleting_any_single_line_is_rejected_or_changes_meaning() {
    let (_, files) = full_project_files();
    let expected = full_key(&files).unwrap();
    let mut checked = 0usize;
    let mut optional = 0usize;
    for (path, text) in &files {
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        for index in 0..lines.len() {
            let mut broken = files.clone();
            let mutated: String = lines
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, line)| *line)
                .collect();
            broken.insert(path.clone(), mutated);
            if let Ok(key) = full_key(&broken) {
                if key == expected {
                    // 생략하면 기본값으로 읽는 줄(ADR-0082·ADR-0086)입니다. 읽은 의미를 다시 쓰면 원래 canonical
                    // bytes로 돌아와야 합니다(받아들인 비canonical 입력은 정규형 하나로 모임).
                    assert_eq!(
                        rewrite(&broken),
                        files,
                        "{path}:{} 줄을 지운 입력이 정규형으로 돌아오지 않음: {:?}",
                        index + 1,
                        lines[index]
                    );
                    optional += 1;
                }
            }
            checked += 1;
        }
    }
    assert!(checked > 100, "{checked}");
    assert!(optional < checked, "{optional}/{checked}");
}

/// 파일 집합을 읽어 다시 씁니다(첫 문서·프로젝트 의미와 추가 root 문서).
fn rewrite(files: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let decoded = decode(files).unwrap();
    let extras = documents::decode(decoded.document.document_id(), files).unwrap();
    let mut out = encode(&decoded).unwrap().files;
    out.extend(documents::encode(decoded.document.document_id(), &extras).unwrap());
    out
}

/// interface 원본의 거부 규칙: 파일 이름이 정의와 다름, 모르는 field·trigger, 빈 interface, 선언 순서가 아닌 쓰기.
#[test]
fn interface_sources_reject_mismatched_or_unknown_content() {
    let (_, files) = full_project_files();
    let (path, text) = files
        .iter()
        .find(|(path, _)| path.starts_with("interfaces/"))
        .map(|(p, t)| (p.clone(), t.clone()))
        .unwrap();
    let with = |path: &str, text: String| {
        let mut broken = files.clone();
        broken.remove(&path.to_owned());
        broken.insert(path.to_owned(), text);
        broken
    };
    let renamed = {
        let mut broken = files.clone();
        let text = broken.remove(&path).unwrap();
        broken.insert("interfaces/Other-000000000000.stellar.yaml".into(), text);
        broken
    };
    for (case, broken) in [
        ("이름", renamed),
        (
            "field",
            with(&path, text.replacen("visibility:", "visible:", 1)),
        ),
        (
            "trigger",
            with(&path, text.replacen("event activate", "event explode", 1)),
        ),
        (
            "형식",
            with(
                &path,
                text.replacen("stellar-component-interface", "stellar-component", 1),
            ),
        ),
        (
            "빈 interface",
            with(
                &path,
                text.lines()
                    .take(3)
                    .chain([
                        "properties: []",
                        "slots: []",
                        "actions: []",
                        "variables: []",
                        "transitions: []",
                    ])
                    .map(|l| format!("{l}\n"))
                    .collect(),
            ),
        ),
    ] {
        assert!(full_key(&broken).is_err(), "{case}");
    }
}
