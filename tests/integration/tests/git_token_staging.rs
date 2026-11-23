//! XIV.14.3: 컴포넌트 정의 → token set 의존 staging 시나리오 (ADR-0080 결정 4, ADR-0127 결정 4).
//!
//! 텍스트 원본 프로젝트(ADR-0088)에서 컴포넌트 원본(`design/components/*.stellar.yaml`)의 `local-tokens`가 token set
//! 원본(`design/tokens/*.stellar.yaml`)의 token을 가리킵니다. 시스템 Git fixture(XIV.14.1)와 제품 Git 엔진
//! (`stellar_ops::git::GitSession`, `stellar git` CLI·앱 runtime과 같은 구현)으로 확인합니다.
//!
//! - 새 컴포넌트만 stage하면 그 token을 담은 새 token set이 필수 동반 단위입니다(invalid partial 차단, index 불변).
//!   승인하면 둘 다 staged이고 재검증을 통과합니다(dependency 승인 후 성공).
//! - token set만 unstage하면 index 컴포넌트의 token이 끊기므로 컴포넌트 파일이 동반입니다.
//! - 작업 트리에서 token set을 지우고 컴포넌트가 참조를 뺐을 때 token set 삭제만 stage하면 컴포넌트 파일이 동반입니다.

#![cfg(unix)] // Windows 경로는 보류 상태라 검증하지 않습니다(ADR-0006).

mod git_common;

use std::collections::BTreeSet;
use std::path::Path;

use git_common::*;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{NodeRef, TokenRef};
use stellar_core_types::typed_value::TypedValue;
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, DefinitionNode, DefinitionTree,
};
use stellar_document::frame::FramePayload;
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::encode_payload;
use stellar_document::records::NodePayload;
use stellar_git_porcelain::decode::GitPath;
use stellar_git_porcelain::fixture::GitFixture;
use stellar_ops::git::{GitError, GitItem, GitSession, SourceUnit};
use stellar_project::document_io::source_v4::project::TextProject;
use stellar_project::document_io::source_v4::token::{TokenSetSource, TokenSource};

const CARD_COMPONENT: u64 = 70;
const CARD_ROOT: u64 = 71;
const BRAND_SET: u64 = 80;
const BRAND_TOKEN: u64 = 81;

fn card_component(tokens: &[u64]) -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    let mut definition = ComponentDefinition::new(
        nid(CARD_COMPONENT),
        "Card".into(),
        NodeRef::new(nid(CARD_ROOT)),
    );
    for token in tokens {
        definition
            .local_tokens_mut()
            .insert(TokenRef::new(nid(*token)));
    }
    let mut tree = DefinitionTree::new(definition.reference());
    tree.insert(DefinitionNode {
        id: nid(CARD_ROOT),
        kind: NodeKind::Frame,
        parent: None,
        order_key: OrderKey::between(None, None),
        visible: true,
        locked: false,
        display_name: "Card".into(),
        payload: encode_payload(&NodePayload::Frame(FramePayload::default())),
    })
    .unwrap();
    library.insert(definition, tree).unwrap();
    library
}

fn brand_tokens() -> TokenSetSource {
    TokenSetSource {
        id: nid(BRAND_SET),
        name: "Brand".into(),
        tokens: vec![TokenSource {
            id: nid(BRAND_TOKEN),
            name: "accent".into(),
            expression: nid(BRAND_TOKEN + 1),
            value: TypedValue::String("blue".into()),
        }],
        computed: Vec::new(),
    }
}

/// 텍스트 원본 프로젝트를 쓰고 컴포넌트·token set 파일의 저장소 경로를 돌려줍니다. 앱 저장처럼 문서의 로컬 chunk
/// cache(generation)도 함께 씁니다(staging 파이프라인은 index를 저장 프로젝트로 다시 엽니다).
fn write(
    root: &Path,
    components: ComponentLibrary,
    token_sets: Vec<TokenSetSource>,
) -> Vec<String> {
    if !root.join("design/current").exists() {
        save_documents(root, &[&card_document()]);
    }
    let files = write_text_project(
        root,
        &TextProject {
            document: card_document(),
            components,
            token_sets,
            images: Default::default(),
        },
    );
    files
        .keys()
        .filter(|k| k.starts_with("components/") || k.starts_with("tokens/"))
        .map(|k| format!("design/{k}"))
        .collect()
}

fn unit(path: &str) -> SourceUnit {
    SourceUnit::File(GitPath::from_bytes(path.as_bytes()))
}

fn git_text(fx: &GitFixture, args: &[&str]) -> String {
    String::from_utf8(fx.git_in(fx.work(), args.iter().copied()).unwrap().stdout).unwrap()
}

fn index_bytes(fx: &GitFixture) -> Vec<u8> {
    std::fs::read(fx.work().join(".git/index")).unwrap()
}

fn token_reason(error: GitError) -> (BTreeSet<SourceUnit>, StableId, Option<StableId>) {
    match error {
        GitError::CompanionRequired {
            changes,
            units,
            reasons,
        } => {
            assert!(changes.is_empty());
            assert_eq!(reasons.len(), 1, "{reasons:?}");
            (units, reasons[0].component, reasons[0].token)
        }
        other => panic!("CompanionRequired여야 함: {other:?}"),
    }
}

#[test]
fn component_token_dependency_blocks_partial_stage_until_approved() {
    let Some(fx) = git_fixture("token-staging") else {
        return;
    };
    write_manifest(fx.work(), "Token staging");
    write(fx.work(), ComponentLibrary::new(), Vec::new());
    fx.commit_all("baseline").unwrap();

    // 작업 트리: token set Brand(accent)와 그 token을 쓰는 컴포넌트 Card를 새로 만듭니다.
    let paths = write(
        fx.work(),
        card_component(&[BRAND_TOKEN]),
        vec![brand_tokens()],
    );
    let card = paths
        .iter()
        .find(|p| p.contains("/components/"))
        .unwrap()
        .clone();
    let brand = paths
        .iter()
        .find(|p| p.contains("/tokens/"))
        .unwrap()
        .clone();
    let s = GitSession::open(fx.gated(), fx.work()).unwrap();

    // 컴포넌트만 stage: token set이 필수 동반이고 index는 그대로입니다.
    let before = index_bytes(&fx);
    let (units, component, token) = token_reason(
        s.stage(vec![GitItem::Unit(unit(&card))], vec![])
            .unwrap_err(),
    );
    assert_eq!(units, BTreeSet::from([unit(&brand)]));
    assert_eq!(component, nid(CARD_COMPONENT));
    assert_eq!(token, Some(nid(BRAND_TOKEN)));
    assert_eq!(index_bytes(&fx), before, "부분 선택 차단: index 불변");
    // 필수가 아닌 승인은 거부합니다.
    assert!(matches!(
        s.stage(
            vec![GitItem::Unit(unit(&card))],
            vec![GitItem::Unit(unit("design/document.stellar.yaml"))]
        ),
        Err(GitError::UnexpectedApproval(_))
    ));
    // token set만 stage하는 것은 동반이 없습니다(token set은 아무것도 참조하지 않음).
    s.stage(vec![GitItem::Unit(unit(&brand))], vec![]).unwrap();
    s.unstage(vec![GitItem::Unit(unit(&brand))], vec![])
        .unwrap();
    assert!(git_text(&fx, &["diff", "--cached", "--name-only"]).is_empty());

    // 승인하면 둘 다 staged이고 재검증을 통과합니다.
    s.stage(
        vec![GitItem::Unit(unit(&card))],
        vec![GitItem::Unit(unit(&brand))],
    )
    .unwrap();
    let staged: BTreeSet<String> = git_text(&fx, &["diff", "--cached", "--name-only"])
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(staged, BTreeSet::from([card.clone(), brand.clone()]));
    assert!(s.validate_staged().unwrap().is_valid());

    // token set만 unstage하면 index Card의 token이 끊기므로 Card가 동반입니다.
    let (units, component, token) = token_reason(
        s.unstage(vec![GitItem::Unit(unit(&brand))], vec![])
            .unwrap_err(),
    );
    assert_eq!(units, BTreeSet::from([unit(&card)]));
    assert_eq!(
        (component, token),
        (nid(CARD_COMPONENT), Some(nid(BRAND_TOKEN)))
    );
    let outcome = s.commit("Card와 Brand token\n").unwrap();
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);

    // 작업 트리에서 Brand를 지우고 Card가 참조를 뺍니다. Brand 삭제만 stage하면 Card가 동반입니다.
    write(fx.work(), card_component(&[]), Vec::new());
    assert!(!fx.work().join(&brand).exists());
    let s = GitSession::open(fx.gated(), fx.work()).unwrap();
    let before = index_bytes(&fx);
    let (units, component, token) = token_reason(
        s.stage(vec![GitItem::Unit(unit(&brand))], vec![])
            .unwrap_err(),
    );
    assert_eq!(units, BTreeSet::from([unit(&card)]));
    assert_eq!(
        (component, token),
        (nid(CARD_COMPONENT), Some(nid(BRAND_TOKEN)))
    );
    assert_eq!(index_bytes(&fx), before);
    s.stage(
        vec![GitItem::Unit(unit(&brand))],
        vec![GitItem::Unit(unit(&card))],
    )
    .unwrap();
    assert!(s.validate_staged().unwrap().is_valid());
    let outcome = s.commit("Brand token 삭제\n").unwrap();
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    let shown = git_text(&fx, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(shown.lines().any(|l| l == brand) && shown.lines().any(|l| l == card));
}
