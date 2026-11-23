//! XXII.2.5 검증: 출력 CSS를 custom property 상속 규칙으로 해석한 값이 IR(해석기 결과)과 같은지.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_target_profile::capability::ElementKind;

use super::*;
use crate::fixture::{check_golden, number, sid, srgb, text};
use crate::ir::{
    IrContextConsumption, IrContextKey, IrContextProvision, IrContextSource, IrContexts, IrNode,
    IrSlotCrossing, IrSlotHost, IrTree,
};

const ACCENT: u8 = 100;
const DENSITY: u8 = 101;
const LABEL: u8 = 102;

struct Names;

impl CssNames for Names {
    fn name(&self, id: StableId) -> Option<String> {
        Some(
            match id.to_bytes()[15] {
                ACCENT => "--context-accent",
                DENSITY => "--context-density",
                LABEL => "--context-label",
                70 => "--brand",
                _ => return None,
            }
            .to_string(),
        )
    }

    fn literal(&self, value: &TypedValue) -> Option<String> {
        match value {
            TypedValue::Number(n) => Some(crate::text::num(n.get())),
            TypedValue::Color(c) => Some(format!(
                "c({},{},{})",
                crate::text::num(c.r()),
                crate::text::num(c.g()),
                crate::text::num(c.b())
            )),
            _ => None,
        }
    }
}

/// (node, 부모 node) preorder 목록으로 IR tree를 만듭니다.
fn tree(shape: &[(u8, Option<u8>)]) -> IrTree {
    let mut nodes: Vec<IrNode> = Vec::new();
    let mut index: BTreeMap<u8, u32> = BTreeMap::new();
    for (n, parent) in shape {
        let parent_index = parent.map(|p| index[&p]);
        let depth = parent_index.map_or(0, |p| nodes[p as usize].depth + 1);
        let position = nodes.len() as u32;
        if let Some(p) = parent_index {
            nodes[p as usize].children.push(position);
        }
        index.insert(*n, position);
        nodes.push(IrNode {
            id: sid(*n),
            parent: parent_index,
            depth,
            children: Vec::new(),
            element: ElementKind::FRAME,
            contracts: Vec::new(),
            layout: None,
            text: None,
            properties: Vec::new(),
            semantics: None,
            capabilities: Vec::new(),
            mappings: Vec::new(),
        });
    }
    IrTree::from_nodes(nodes).expect("tree")
}

fn twice(key: u8) -> IrExpr {
    IrExpr::Binary {
        op: "*",
        lhs: Box::new(IrExpr::Context(sid(key))),
        rhs: Box::new(IrExpr::Literal(number(2.0))),
    }
}

fn provision(node: u8, key: u8, value: IrExpr, depths: &[u32]) -> IrContextProvision {
    IrContextProvision {
        node: sid(node),
        key: sid(key),
        value,
        depths: depths.to_vec(),
    }
}

fn consume(node: u8, key: u8, source: IrContextSource) -> IrContextConsumption {
    IrContextConsumption {
        node: sid(node),
        path: crate::fixture::path(node, 4),
        key: sid(key),
        source,
        slots: Vec::new(),
    }
}

/// 중첩·shadowing·같은 key 2단 재읽기·slot·깊이가 다른 provision fixture.
///
/// ```text
/// 1 root
/// ├ 2 A   accent=red(d1), density=density*2(d1)
/// │ ├ 3 B accent=blue(d2), density=density*2(d2)
/// │ │ └ 4 C   소비 accent(3), density(3)
/// │ └ 5 I instance
/// │   └ 6 X(컴포넌트 내부) accent=green(d2)
/// │     └ 7 H slot host(공개 key 없음)
/// │       └ 8 S slot 콘텐츠  소비 accent(2, slot 경계)
/// ├ 9 D   label="메모"(깊이 1·2)
/// │ └ 10  소비 label(9)
/// └ 11    소비 accent(default)
/// ```
fn fixture() -> IrTree {
    let mut ir = tree(&[
        (1, None),
        (2, Some(1)),
        (3, Some(2)),
        (4, Some(3)),
        (5, Some(2)),
        (6, Some(5)),
        (7, Some(6)),
        (8, Some(7)),
        (9, Some(1)),
        (10, Some(9)),
        (11, Some(1)),
    ]);
    let mut slot = consume(8, ACCENT, IrContextSource::Provision(sid(2)));
    slot.slots.push(IrSlotCrossing {
        host: sid(7),
        instance: sid(5),
    });
    ir.contexts = IrContexts {
        keys: vec![
            IrContextKey {
                id: sid(ACCENT),
                name: "accent".into(),
                default: IrExpr::Literal(srgb(0.5, 0.5, 0.5, 1.0)),
            },
            IrContextKey {
                id: sid(DENSITY),
                name: "density".into(),
                default: IrExpr::Literal(number(1.0)),
            },
            IrContextKey {
                id: sid(LABEL),
                name: "label".into(),
                default: IrExpr::Literal(text("기본")),
            },
        ],
        provisions: vec![
            provision(2, ACCENT, IrExpr::Literal(srgb(1.0, 0.0, 0.0, 1.0)), &[1]),
            provision(2, DENSITY, twice(DENSITY), &[1]),
            provision(3, ACCENT, IrExpr::Literal(srgb(0.0, 0.0, 1.0, 1.0)), &[2]),
            provision(3, DENSITY, twice(DENSITY), &[2]),
            provision(6, ACCENT, IrExpr::Literal(srgb(0.0, 1.0, 0.0, 1.0)), &[2]),
            provision(9, LABEL, IrExpr::Literal(text("메모")), &[1, 2]),
        ],
        consumptions: vec![
            consume(4, ACCENT, IrContextSource::Provision(sid(3))),
            consume(4, DENSITY, IrContextSource::Provision(sid(3))),
            slot,
            consume(10, LABEL, IrContextSource::Provision(sid(9))),
            consume(11, ACCENT, IrContextSource::Default),
        ],
        slots: vec![IrSlotHost {
            host: sid(7),
            instance: sid(5),
            public_keys: Vec::new(),
        }],
    };
    // 소비 경로는 key마다 다르게(같은 경로 두 번이면 IR 검사 오류).
    ir.contexts.consumptions[1].path = crate::fixture::path(4, 5);
    ir
}

fn taken() -> BTreeSet<String> {
    ["--context-accent", "--context-density", "--context-label"]
        .into_iter()
        .map(String::from)
        .collect()
}

/// custom property 상속 해석기(test 전용): 선언이 있으면 같은 element의 계산값으로 `var()`를 풀고, 없으면 부모 값을
/// 상속합니다. 순환은 빈 값(guaranteed-invalid)입니다.
struct Cascade<'a> {
    ir: &'a IrTree,
    declared: BTreeMap<u32, BTreeMap<String, String>>,
}

impl Cascade<'_> {
    fn new<'a>(ir: &'a IrTree, out: &WebContexts) -> Cascade<'a> {
        let mut declared: BTreeMap<u32, BTreeMap<String, String>> = BTreeMap::new();
        for rule in &out.rules {
            let index = ir.node_index(rule.node).expect("rule node");
            let map = declared.entry(index).or_default();
            for (name, value) in &rule.declarations {
                map.insert(name.clone(), value.clone());
            }
        }
        Cascade { ir, declared }
    }

    fn value(&self, node: u32, name: &str, visiting: &mut Vec<(u32, String)>) -> Option<String> {
        if let Some(raw) = self.declared.get(&node).and_then(|m| m.get(name)) {
            if visiting.contains(&(node, name.to_string())) {
                return None;
            }
            visiting.push((node, name.to_string()));
            let resolved = self.substitute(node, raw, visiting);
            visiting.pop();
            return resolved;
        }
        let parent = self.ir.nodes[node as usize].parent?;
        self.value(parent, name, visiting)
    }

    fn substitute(
        &self,
        node: u32,
        raw: &str,
        visiting: &mut Vec<(u32, String)>,
    ) -> Option<String> {
        let mut out = String::new();
        let mut rest = raw;
        while let Some(start) = rest.find("var(") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 4..];
            let end = after.find(')')?;
            let name = &after[..end];
            out.push_str(&self.value(node, name, visiting)?);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Some(out)
    }
}

/// IR 출처로 계산한 소비 값 텍스트(같은 key 재읽기는 IR tree의 가장 가까운 조상 provision 또는 default).
fn expected(ir: &IrTree, consumption: &IrContextConsumption) -> String {
    fn eval(ir: &IrTree, key: StableId, provider: Option<StableId>) -> String {
        let contexts = &ir.contexts;
        let expr = match provider {
            Some(node) => &contexts.provision(node, key).expect("provision").value,
            None => &contexts.key(key).expect("key").default,
        };
        let outer = || {
            let start = provider.and_then(|p| ir.node(p)).and_then(|n| n.parent);
            let mut current = start;
            while let Some(index) = current {
                let id = ir.nodes[index as usize].id;
                if contexts.provision(id, key).is_some() {
                    return eval(ir, key, Some(id));
                }
                current = ir.nodes[index as usize].parent;
            }
            eval(ir, key, None)
        };
        fn text(expr: &IrExpr, outer: &dyn Fn() -> String) -> String {
            match expr {
                IrExpr::Literal(value) => Names.literal(value).expect("literal"),
                IrExpr::Context(_) => outer(),
                IrExpr::Binary { op, lhs, rhs } => {
                    format!("calc({} {op} {})", text(lhs, outer), text(rhs, outer))
                }
                other => panic!("fixture에 없는 식 {other:?}"),
            }
        }
        text(expr, &outer)
    }
    let provider = match consumption.source {
        IrContextSource::Provision(node) => Some(node),
        IrContextSource::Default => None,
    };
    eval(ir, consumption.key, provider)
}

#[test]
fn css_inheritance_of_the_output_matches_the_ir_resolution() {
    // 검증 기준(XXII.2.5, 해석기 대조 부분은 IR 입력으로 대신): 중첩·shadowing·slot fixture의 출력 해석값이 IR이 보존한
    // 해석 결과와 같음.
    let ir = fixture();
    let out = lower_contexts(&ir, &Names, &taken());
    let cascade = Cascade::new(&ir, &out);
    let mut checked = 0;
    for consumer in &out.consumers {
        let consumption = ir
            .contexts
            .consumption_of(&consumer.path)
            .expect("consumption");
        let node = ir.node_index(consumer.node).expect("node");
        let actual = cascade
            .substitute(node, &consumer.value, &mut Vec::new())
            .expect("유효한 값");
        assert_eq!(actual, expected(&ir, consumption), "{:?}", consumer.node);
        checked += 1;
    }
    // accent 3곳(shadowing·slot·default)과 density 1곳. label은 context 구조입니다.
    assert_eq!(checked, 4);
}

#[test]
fn same_key_reads_use_depth_names_without_self_referencing_var() {
    // 검증 기준(XXII.2.5): 같은 key 재읽기 2단 중첩 fixture 출력의 자기 참조 var() 0건과 해석값 일치.
    let ir = fixture();
    let out = lower_contexts(&ir, &Names, &taken());
    for rule in &out.rules {
        for (name, value) in &rule.declarations {
            assert!(
                !value.contains(&format!("var({name})")),
                "자기 참조 {name}: {value}"
            );
        }
    }
    let cascade = Cascade::new(&ir, &out);
    let c = ir.node_index(sid(4)).unwrap();
    assert_eq!(
        cascade
            .value(c, "--context-density", &mut Vec::new())
            .as_deref(),
        Some("calc(calc(1 * 2) * 2)")
    );
}

#[test]
fn varying_depth_key_goes_to_context_structure_with_one_info_diagnostic() {
    // 검증 기준(XXII.2.5): 깊이가 instance마다 다른 fixture의 context 구조 출력과 진단 1건.
    let ir = fixture();
    let out = lower_contexts(&ir, &Names, &taken());
    let varies: Vec<_> = out
        .diagnostics
        .iter()
        .filter(|d| d.kind == ContextDiagnosticKind::DepthVaries)
        .collect();
    assert_eq!(varies.len(), 1);
    assert_eq!(varies[0].key, sid(LABEL));
    assert!(!out.css_keys.contains(&sid(LABEL)));
    assert!(out.rules.iter().all(|r| r
        .declarations
        .iter()
        .all(|(n, _)| !n.starts_with("--context-label"))));
    check_golden("web_context.js", &out.structure);
    assert!(out.structure.contains("depths: [1, 2]"));
}

#[test]
fn depth_and_outer_names_colliding_with_tokens_are_diagnosed() {
    // 검증 기준(XXII.2.5): 토큰과 이름이 겹치는 key fixture의 충돌 진단(깊이 이름 포함).
    let ir = fixture();
    let mut names = taken();
    names.insert("--context-density-1".to_string());
    names.insert("--context-accent-outer".to_string());
    let out = lower_contexts(&ir, &Names, &names);
    let collisions: Vec<(StableId, Option<String>)> = out
        .diagnostics
        .iter()
        .filter(|d| d.kind == ContextDiagnosticKind::NameCollision)
        .map(|d| (d.key, d.name.clone()))
        .collect();
    assert_eq!(
        collisions,
        vec![
            (sid(ACCENT), Some("--context-accent-outer".to_string())),
            (sid(DENSITY), Some("--context-density-1".to_string())),
        ]
    );
}

#[test]
fn output_is_deterministic_and_golden() {
    // 검증 기준(XXII.2.5): deterministic output.
    let ir = fixture();
    let a = lower_contexts(&ir, &Names, &taken());
    let b = lower_contexts(&ir, &Names, &taken());
    assert_eq!(a, b);
    let mut text = String::new();
    for rule in &a.rules {
        text.push_str(&format!("{} {{\n", rule.node));
        for (name, value) in &rule.declarations {
            text.push_str(&format!("  {name}: {value};\n"));
        }
        text.push_str("}\n");
    }
    check_golden("web_context.css.txt", &text);
}
