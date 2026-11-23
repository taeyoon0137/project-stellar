//! XXII.1.1 검증: semantic snapshot fixture, 결정론, 제외·접근성 재부모, 입력 오류.

use std::collections::BTreeMap;

use stellar_accessibility::node::{AccessibilityBinding, AccessibilityNode, AccessibilityRole};
use stellar_accessibility::snapshot::AccessibilitySnapshot;
use stellar_accessibility::tree::AccessibilityTree;
use stellar_core_types::typed_value::TypedValue;
use stellar_target_profile::capability::{CapabilityStatus, CapabilitySubject, ContractCode};
use stellar_target_profile::overrides::ValueOrigin;
use stellar_target_profile::registry::ProfileRegistryVersion;
use stellar_target_profile::validate::ViewNode;

use super::*;
use crate::fixture::{self as fx, sid, Fixture};

/// (이름, fixture 변경, 기대 오류).
type Case = (&'static str, Box<dyn Fn(&mut Fixture)>, IrError);

fn build(fixture: &Fixture, profile: &TargetProfile) -> Result<CodegenIr, IrError> {
    build_ir(&fixture.sources(), profile, &fixture.catalog)
}

#[test]
fn semantic_snapshot_fixtures_per_profile() {
    // 검증 기준(XXII.1.1): semantic snapshot fixture. 같은 View를 Web·Apple·Android profile로 변환한 IR이
    // golden과 byte 단위로 같습니다.
    let fixture = Fixture::new();
    for (name, profile) in [
        ("ir_site.snapshot", fx::web_profile()),
        ("ir_ios.snapshot", fx::ios_profile()),
        ("ir_android.snapshot", fx::android_profile()),
    ] {
        let snapshot = fixture.ir(&profile).semantic_snapshot();
        fx::check_golden(name, &snapshot);
    }
}

#[test]
fn ir_carries_layout_semantics_state_tokens_and_capability() {
    let fixture = Fixture::new();
    let ir = fixture.ir(&fx::ios_profile());

    // 구조: preorder, Fragment는 node로 남고 layout이 없음.
    let order: Vec<StableId> = ir.nodes.iter().map(|n| n.id).collect();
    assert_eq!(
        order,
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 16, 17].map(sid)
    );
    let fragment = ir.node(sid(3)).expect("fragment");
    assert!(fragment.layout.is_none());
    assert!(ir
        .nodes
        .iter()
        .filter(|n| n.id != sid(3))
        .all(|n| n.layout.is_some()));

    // layout: root는 세로 flex, 자식은 Fragment를 펼친 순서.
    let root = ir.root();
    assert_eq!(
        root.layout
            .as_ref()
            .map(|l| (l.style.display, l.style.flex_direction)),
        Some((
            stellar_layout::style::Display::Flex,
            stellar_layout::style::FlexDirection::Column
        ))
    );
    // ios는 13을 제외: 2, 3(Fragment), 6, 11, 12, 16.
    assert_eq!(root.children.len(), 6);

    // 값: ios override radius 12, variant 층 텍스트 색, token binding.
    let radius = ir.property(&fx::path(12, fx::KEY_RADIUS)).expect("radius");
    assert_eq!(radius.origin, ValueOrigin::Profile);
    assert_eq!(radius.value, TypedValue::Length(fx::len(12.0)));
    let text_color = ir
        .property(&fx::path(2, fx::KEY_TEXT_COLOR))
        .expect("text color");
    assert_eq!(
        text_color.origin,
        ValueOrigin::Variant(sid(fx::VARIANT_ERROR))
    );
    assert_eq!(
        ir.property(&fx::path(12, fx::KEY_FILL))
            .and_then(|p| p.token),
        Some(sid(fx::TOKEN_ACCENT))
    );

    // semantics: radio group, radio(초기 선택), textField, dialog.
    let role_of = |id: u8| {
        ir.node_index(sid(id))
            .and_then(|index| ir.semantics_of(index))
            .map(|entry| entry.semantics.role)
    };
    assert_eq!(role_of(6), Some(AccessibilityRole::RadioGroup));
    assert_eq!(role_of(7), None); // radio group에 흡수된 List.
    assert_eq!(role_of(8), Some(AccessibilityRole::Radio));
    assert_eq!(role_of(11), Some(AccessibilityRole::TextField));
    assert_eq!(role_of(16), Some(AccessibilityRole::Dialog));

    // state·token: id 오름차순 전체 선언.
    let state_ids: Vec<StableId> = ir.states.iter().map(|s| s.id).collect();
    assert_eq!(state_ids, [50, 51, 52].map(sid));
    let token_ids: Vec<StableId> = ir.tokens.iter().map(|t| t.id).collect();
    assert_eq!(token_ids, [70, 71, 72].map(sid));

    // capability: iOS에서 SingleSelectionGroup은 approximated, SF Symbol은 native.
    let group = ir.node(sid(6)).expect("group");
    let status = |node: &IrNode, subject: CapabilitySubject| {
        node.capabilities
            .iter()
            .find(|f| f.subject == subject)
            .and_then(|f| f.status)
    };
    assert_eq!(
        status(
            group,
            CapabilitySubject::Contract(ContractCode::SINGLE_SELECTION_GROUP)
        ),
        Some(CapabilityStatus::Approximated)
    );
    let image = ir.node(sid(4)).expect("image");
    assert_eq!(
        status(
            image,
            CapabilitySubject::Property(
                stellar_target_profile::builtin::properties::APPLE_SF_SYMBOL
            )
        ),
        Some(CapabilityStatus::Native)
    );
    assert_eq!(ir.target.registry_version.get(), 1);
}

#[test]
fn excluded_subtree_drops_values_text_layout_and_semantics() {
    let fixture = Fixture::new();
    let ios = fixture.ir(&fx::ios_profile());
    for id in [13, 15, 18] {
        assert!(ios.node(sid(id)).is_none(), "{id}는 ios에서 제외");
    }
    assert!(ios.property(&fx::path(13, fx::KEY_FILL)).is_none());
    assert!(ios
        .accessibility
        .iter()
        .all(|entry| ![sid(13), sid(15)].contains(&ios.nodes[entry.node as usize].id)));
    let site = fixture.ir(&fx::web_profile());
    assert_eq!(
        site.node(sid(15)).and_then(|n| n.text.as_deref()),
        Some("새 기능")
    );
}

#[test]
fn bindings_resolve_to_ir_values_and_initial_state() {
    let fixture = Fixture::new();
    let ir = fixture.ir(&fx::web_profile());
    let entry = |id: u8| {
        ir.node_index(sid(id))
            .and_then(|index| ir.semantics_of(index))
            .expect("entry")
    };
    // image label = 속성 값.
    let image = entry(4);
    assert_eq!(
        image
            .semantics
            .label
            .as_ref()
            .and_then(|b| ir.static_text(b)),
        Some("프로필 사진".to_string())
    );
    // radio 8은 초기 선택(상태 초기값 = item StableId 문자열), radio 10은 아님.
    let checked = |id: u8| {
        entry(id)
            .semantics
            .states
            .get(&stellar_accessibility::node::StateKind::Checked)
            .and_then(|b| ir.static_flag(b))
    };
    assert_eq!(checked(8), Some(true));
    assert_eq!(checked(10), Some(false));
    // static text 내용.
    let text = entry(9);
    assert_eq!(
        text.semantics
            .label
            .as_ref()
            .and_then(|b| ir.static_text(b)),
        Some("작게".to_string())
    );
    // 컴포넌트 Property는 정적으로 모릅니다.
    assert_eq!(
        ir.static_value(&AccessibilityBinding::ComponentProperty(sid(90))),
        None
    );
    assert_eq!(
        ir.resolve_binding(&AccessibilityBinding::NodeText(sid(99))),
        ResolvedBinding::Missing
    );
}

#[test]
fn ir_is_deterministic_and_independent_of_declaration_order() {
    let fixture = Fixture::new();
    let profile = fx::android_profile();
    let first = fixture.ir(&profile);
    assert_eq!(first, fixture.ir(&profile));

    let mut shuffled = Fixture::new();
    shuffled.states.reverse();
    shuffled.tokens.reverse();
    shuffled.layout.nodes.reverse();
    for node in &mut shuffled.layout.nodes {
        node.children.reverse();
    }
    let second = shuffled.ir(&profile);
    assert_eq!(first.semantic_snapshot(), second.semantic_snapshot());
    assert_eq!(first, second);
}

#[test]
fn excluded_accessibility_parent_lifts_children_to_nearest_kept_ancestor() {
    // 시각 tree와 다른 접근성 tree: 17(모달 안 텍스트)을 13 아래에 둔 뒤 13을 제외하면 17은 root 아래로 올라갑니다.
    let mut fixture = Fixture::new();
    let mut tree = AccessibilityTree::new();
    for (id, role) in [
        (1, AccessibilityRole::Generic),
        (13, AccessibilityRole::Group),
        (17, AccessibilityRole::StaticText),
        (2, AccessibilityRole::StaticText),
    ] {
        tree.insert(AccessibilityNode::new(sid(id), role))
            .expect("insert");
    }
    tree.push_root(sid(1));
    tree.push_child(sid(1), sid(13));
    tree.push_child(sid(13), sid(17));
    tree.push_child(sid(1), sid(2));
    fixture.accessibility = AccessibilitySnapshot::from_tree(&tree).expect("snapshot");
    let ir = fixture.ir(&fx::ios_profile());
    let summary: Vec<(StableId, Option<u32>, u32)> = ir
        .accessibility
        .iter()
        .map(|e| (ir.nodes[e.node as usize].id, e.parent, e.depth))
        .collect();
    assert_eq!(
        summary,
        vec![
            (sid(1), None, 0),
            (sid(17), Some(0), 1),
            (sid(2), Some(0), 1)
        ]
    );
    assert_eq!(ir.accessibility_children(0), vec![1, 2]);
    // 제외하지 않는 profile에서는 원래 구조.
    let site = fixture.ir(&fx::web_profile());
    assert_eq!(site.accessibility[2].parent, Some(1));
    assert_eq!(site.accessibility[2].depth, 2);
}

#[test]
fn semantic_subtrees_mark_nodes_with_accessible_descendants() {
    let fixture = Fixture::new();
    let ir = fixture.ir(&fx::web_profile());
    let has = ir.semantic_subtrees();
    let of = |id: u8| has[ir.node_index(sid(id)).expect("node") as usize];
    assert!(of(1));
    assert!(of(3)); // Fragment 아래 image.
    assert!(!of(5)); // 장식 shape.
    assert!(of(7)); // 흡수된 List 아래 radio.
    assert!(!of(18));
}

#[test]
fn element_parents_skip_fragments() {
    let fixture = Fixture::new();
    let ir = fixture.ir(&fx::web_profile());
    let parents = ir.element_parents();
    let index = |id: u8| ir.node_index(sid(id)).expect("node");
    // Fragment(3) 아래 4·5의 출력 부모는 root(1)이고, Fragment가 아닌 부모는 그대로입니다.
    assert_eq!(parents[index(4) as usize], Some(index(1)));
    assert_eq!(parents[index(5) as usize], Some(index(1)));
    assert_eq!(parents[index(3) as usize], Some(index(1)));
    assert_eq!(parents[index(9) as usize], Some(index(8)));
    assert_eq!(parents[0], None);
}

#[test]
fn accessibility_child_lists_match_per_entry_children() {
    let fixture = Fixture::new();
    for profile in [fx::web_profile(), fx::ios_profile()] {
        let ir = fixture.ir(&profile);
        let lists = ir.accessibility_child_lists();
        assert_eq!(lists.len(), ir.accessibility.len());
        for entry in 0..ir.accessibility.len() as u32 {
            assert_eq!(lists[entry as usize], ir.accessibility_children(entry));
        }
    }
}

#[test]
fn structural_errors_are_rejected() {
    let profile = fx::web_profile();
    let cases: Vec<Case> = vec![
        (
            "빈 View",
            Box::new(|f| f.view.nodes.clear()),
            IrError::EmptyView,
        ),
        (
            "중복 node",
            Box::new(|f| {
                let copy = f.view.nodes[1].clone();
                f.view.nodes.push(copy);
            }),
            IrError::DuplicateNode(sid(2)),
        ),
        (
            "root 둘",
            Box::new(|f| f.view.nodes[1].parent = None),
            IrError::MultipleRoots {
                first: sid(1),
                second: sid(2),
            },
        ),
        (
            "root 없음",
            Box::new(|f| f.view.nodes[0].parent = Some(sid(2))),
            IrError::MissingRoot,
        ),
        (
            "없는 부모",
            Box::new(|f| f.view.nodes[1].parent = Some(sid(99))),
            IrError::UnknownParent {
                node: sid(2),
                parent: sid(99),
            },
        ),
        (
            "순환",
            Box::new(|f| {
                // 9의 부모를 그 자손이 아닌 순환으로: 8 → 9 → 8.
                f.view.nodes[7].parent = Some(sid(9));
            }),
            IrError::Unreachable(sid(8)),
        ),
    ];
    for (name, mutate, expected) in cases {
        let mut fixture = Fixture::new();
        mutate(&mut fixture);
        assert_eq!(build(&fixture, &profile).err(), Some(expected), "{name}");
    }
}

#[test]
fn input_consistency_errors_are_rejected() {
    let profile = fx::web_profile();
    let cases: Vec<Case> = vec![
        (
            "layout 없음",
            Box::new(|f| {
                f.layout.nodes.retain(|n| n.id != sid(11));
                for node in &mut f.layout.nodes {
                    node.children.retain(|c| c.id != sid(11));
                }
            }),
            IrError::MissingLayout(sid(11)),
        ),
        (
            "layout 자식 순서 불일치",
            Box::new(|f| {
                let row = f
                    .layout
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == sid(7))
                    .expect("row");
                row.children.swap(0, 1);
                let (a, b) = (
                    row.children[0].order_key.clone(),
                    row.children[1].order_key.clone(),
                );
                row.children[0].order_key = b;
                row.children[1].order_key = a;
            }),
            IrError::LayoutChildrenMismatch(sid(7)),
        ),
        (
            "layout root 불일치",
            Box::new(|f| {
                f.layout.root = sid(16);
                f.layout
                    .nodes
                    .retain(|n| n.id == sid(16) || n.id == sid(17));
            }),
            IrError::LayoutRootMismatch {
                view: sid(1),
                layout: sid(16),
            },
        ),
        (
            "View 밖 속성",
            Box::new(|f| {
                f.view
                    .common
                    .set(fx::path(99, fx::KEY_FILL), fx::number(1.0));
            }),
            IrError::PropertyOutsideView(sid(99)),
        ),
        (
            "View 밖 텍스트",
            Box::new(|f| {
                f.texts.insert(sid(99), "x".to_string());
            }),
            IrError::TextOutsideView(sid(99)),
        ),
        (
            "Text가 아닌 node 텍스트",
            Box::new(|f| {
                f.texts.insert(sid(12), "x".to_string());
            }),
            IrError::TextOnNonTextNode(sid(12)),
        ),
        (
            "선언되지 않은 상태",
            Box::new(|f| f.states.retain(|s| s.id != sid(fx::STATE_SAVING))),
            IrError::UnknownState {
                node: sid(12),
                state: sid(fx::STATE_SAVING),
            },
        ),
        (
            "중복 상태",
            Box::new(|f| {
                let copy = f.states[0].clone();
                f.states.push(copy);
            }),
            IrError::DuplicateState(sid(fx::STATE_MEMO)),
        ),
        (
            "빈 상태 이름",
            Box::new(|f| f.states[0].name = " ".to_string()),
            IrError::EmptyStateName(sid(fx::STATE_MEMO)),
        ),
        (
            "선언되지 않은 token",
            Box::new(|f| {
                f.tokens.retain(|t| t.id != sid(fx::TOKEN_ACCENT));
            }),
            IrError::UnknownToken {
                node: sid(12),
                token: sid(fx::TOKEN_ACCENT),
            },
        ),
        (
            "값 없는 token binding",
            Box::new(|f| {
                f.token_bindings
                    .insert(fx::path(11, fx::KEY_FILL), sid(fx::TOKEN_ACCENT));
            }),
            IrError::TokenBindingWithoutValue(sid(11)),
        ),
        (
            "빈 token 이름",
            Box::new(|f| f.tokens[0].name = String::new()),
            IrError::EmptyTokenName(sid(fx::TOKEN_SPACING)),
        ),
        (
            "View 밖 접근성 source",
            Box::new(|f| {
                let mut tree = AccessibilityTree::new();
                tree.insert(AccessibilityNode::new(sid(99), AccessibilityRole::Generic))
                    .expect("insert");
                tree.push_root(sid(99));
                f.accessibility = AccessibilitySnapshot::from_tree(&tree).expect("snapshot");
            }),
            IrError::AccessibilityOutsideView(sid(99)),
        ),
        (
            "Fragment 접근성 source",
            Box::new(|f| {
                // 3은 Fragment라 출력 element가 없어 의미를 붙일 곳이 없습니다.
                let mut tree = AccessibilityTree::new();
                tree.insert(AccessibilityNode::new(sid(1), AccessibilityRole::Generic))
                    .expect("root");
                tree.insert(AccessibilityNode::new(sid(3), AccessibilityRole::Group))
                    .expect("fragment");
                tree.push_root(sid(1));
                tree.push_child(sid(1), sid(3));
                f.accessibility = AccessibilitySnapshot::from_tree(&tree).expect("snapshot");
            }),
            IrError::AccessibilityOnFragment(sid(3)),
        ),
    ];
    for (name, mutate, expected) in cases {
        let mut fixture = Fixture::new();
        mutate(&mut fixture);
        assert_eq!(build(&fixture, &profile).err(), Some(expected), "{name}");
    }
}

#[test]
fn unknown_registry_pin_and_invalid_layout_are_rejected() {
    let fixture = Fixture::new();
    let mut profile = fx::web_profile();
    profile.registry_version = ProfileRegistryVersion::new(9).expect("pin");
    assert_eq!(
        build(&fixture, &profile).err(),
        Some(IrError::RegistryUnavailable {
            pinned: ProfileRegistryVersion::new(9).expect("pin"),
            latest: ProfileRegistryVersion::new(1).expect("latest"),
        })
    );
    let mut broken = Fixture::new();
    // layout 자식이 layout node로 없으면 layout 검사가 실패합니다.
    broken.layout.nodes.retain(|n| n.id != sid(17));
    assert!(matches!(
        build(&broken, &fx::web_profile()),
        Err(IrError::InvalidLayout(_))
    ));
}

#[test]
fn view_without_semantics_or_tokens_still_builds() {
    // 최소 입력: root 하나.
    let view = ViewModel {
        nodes: vec![ViewNode {
            id: sid(1),
            parent: None,
            element: stellar_target_profile::capability::ElementKind::FRAME,
            contracts: vec![ContractCode::BUTTON, ContractCode::BUTTON],
        }],
        ..ViewModel::default()
    };
    let layout = LayoutInput::new(sid(1), vec![LayoutNode::new(sid(1))]);
    let snapshot = AccessibilitySnapshot::default();
    let empty_texts = BTreeMap::new();
    let empty_bindings = BTreeMap::new();
    let variants = VariantLayer::none();
    let sources = CodegenSources {
        view: &view,
        capability_map: &fx::KeyMap,
        variants: &variants,
        layout: &layout,
        accessibility: &snapshot,
        texts: &empty_texts,
        states: &[],
        tokens: &[],
        token_bindings: &empty_bindings,
    };
    let catalog = stellar_target_profile::builtin::catalog().expect("catalog");
    let ir = build_ir(&sources, &fx::web_profile(), &catalog).expect("IR");
    assert_eq!(ir.nodes.len(), 1);
    assert_eq!(ir.root().contracts, vec![ContractCode::BUTTON]);
    assert!(ir.accessibility.is_empty());
    assert!(ir
        .semantic_snapshot()
        .starts_with(snapshot::SNAPSHOT_HEADER));
}

#[test]
fn accessibility_only_input_has_no_invented_layout_profile_or_values() {
    use crate::web::{aria::map_accessibility, check::check_document};
    let nodes = vec![
        ViewNode {
            id: sid(1),
            parent: None,
            element: ElementKind::FRAME,
            contracts: vec![],
        },
        ViewNode {
            id: sid(2),
            parent: Some(sid(1)),
            element: ElementKind::TEXT,
            contracts: vec![],
        },
    ];
    let mut tree = AccessibilityTree::new();
    tree.insert(
        AccessibilityNode::new(sid(1), AccessibilityRole::Group)
            .with_label(AccessibilityBinding::text("이름 \" & < >")),
    )
    .unwrap();
    tree.insert(
        AccessibilityNode::new(sid(2), AccessibilityRole::StaticText)
            .with_label(AccessibilityBinding::NodeText(sid(2))),
    )
    .unwrap();
    tree.push_root(sid(1));
    tree.push_child(sid(1), sid(2));
    let snapshot = AccessibilitySnapshot::from_tree(&tree).unwrap();
    let texts = BTreeMap::from([(sid(2), "내용 < & >".to_string())]);
    let ir = build_accessibility_ir(&nodes, &snapshot, &texts).unwrap();
    assert!(ir
        .nodes
        .iter()
        .all(|n| n.layout.is_none() && n.properties.is_empty() && n.capabilities.is_empty()));
    assert!(ir.states.is_empty() && ir.tokens.is_empty());
    let mapped = map_accessibility(&ir);
    assert_eq!(mapped.to_html(), "<div role=\"group\" aria-label=\"이름 &quot; &amp; &lt; &gt;\">\n  <span>내용 &lt; &amp; &gt;</span>\n</div>\n");
    assert!(mapped.notes.is_empty());
    assert!(check_document(&mapped.document).is_empty());
    assert_eq!(
        build_accessibility_ir(
            &nodes,
            &snapshot,
            &BTreeMap::from([(sid(99), "outside".into())])
        ),
        Err(IrError::TextOutsideView(sid(99)))
    );
    assert_eq!(
        build_accessibility_ir(&nodes[..1], &snapshot, &BTreeMap::new()),
        Err(IrError::AccessibilityOutsideView(sid(2)))
    );
}

#[test]
fn accessibility_only_input_does_not_resolve_missing_state_or_hide_mapping_and_check_errors() {
    use crate::web::{aria::map_accessibility, check::check_document};
    let nodes = [ViewNode {
        id: sid(1),
        parent: None,
        element: ElementKind::FRAME,
        contracts: vec![],
    }];
    for label in [
        AccessibilityBinding::State(sid(7)),
        AccessibilityBinding::NodeText(sid(99)),
    ] {
        let mut tree = AccessibilityTree::new();
        tree.insert(
            AccessibilityNode::new(sid(1), AccessibilityRole::Button).with_label(label.clone()),
        )
        .unwrap();
        tree.push_root(sid(1));
        let snapshot = AccessibilitySnapshot::from_tree(&tree).unwrap();
        let ir = build_accessibility_ir(&nodes, &snapshot, &BTreeMap::new());
        if matches!(label, AccessibilityBinding::State(_)) {
            assert_eq!(
                ir,
                Err(IrError::UnknownState {
                    node: sid(1),
                    state: sid(7)
                })
            );
        } else {
            let mapped = map_accessibility(&ir.unwrap());
            assert_eq!(mapped.notes.len(), 1);
            assert!(!check_document(&mapped.document).is_empty());
        }
    }
}
