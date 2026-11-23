//! nested component·variant·token 조건 layout fixture corpus (X.11.2).
//!
//! layout crate는 문서·component·token crate에 의존하지 않으므로, 이 테스트는 같은 계약(IX.2.2 instance expansion,
//! IX.7.6 variant override 우선순위, VIII.5 token 값)을 따르는 작은 fixture 모델을 [`LayoutSource`]로 구현하고 X.1.2
//! layout tree view로 펼쳐 레이아웃합니다.
//!
//! - **component**: Button(icon + label), Toolbar(Fragment로 묶은 Button 둘 + title + icon-only Button), Card(Toolbar
//!   header + body + Button 둘), ListItem(avatar + text + small Button). Card → Toolbar → Button으로 3단계 중첩입니다.
//! - **property**: 인스턴스 override와 명시 drilling(`$name`은 바깥 인스턴스의 property 값).
//! - **variant**: Button `size`(small·large)와 `kind`(icon-only)는 nullable 차원입니다. 일반 variant가 base를 덮고,
//!   구체 조합(large + icon-only)이 일반 variant를 덮으며, 없는 조합은 base + 일반 variant만 적용합니다.
//! - **token 조건**: density(comfortable·compact) × scale(regular·large)에 따라 간격·크기·글자 token 값이 바뀝니다.
//!
//! scene × token 조건 4개 fixture의 모든 box rect·baseline을 checked-in golden과 비교하고, 같은 scene에서
//! 조건을 바꾸는 edit를 증분 레이아웃(X.7)으로 적용해 전체 계산과 같은지 확인합니다.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Insets, Length};
use stellar_layout::engine::compute_layout;
use stellar_layout::geometry::Axis;
use stellar_layout::incremental::{IncrementalLayout, LayoutEdit};
use stellar_layout::input::{LayoutContent, LayoutInput, LayoutNode};
use stellar_layout::intrinsic::{DefaultMeasurer, MeasuredContent, TextIntrinsic};
use stellar_layout::output::LayoutResult;
use stellar_layout::style::{
    Display, FlexDirection, ItemAlignment, LayoutStyle, LengthPercentageAuto, Position,
    Size as CssSize,
};
use stellar_layout::view::{build_layout_tree, virtual_layout_id, LayoutSource, SourceNode};

const GOLDEN: &str = include_str!("fixtures/component_layout_v2.golden");

/// fixture 수(scene 4 × token 조건 4).
const FIXTURE_COUNT: usize = 16;

// --- token ---

/// layout token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    SpaceXs,
    SpaceS,
    SpaceM,
    SpaceL,
    Icon,
    Control,
    ControlSmall,
    Line,
    Char,
}

/// token 조건(view 환경).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Condition {
    compact: bool,
    large: bool,
}

impl Condition {
    const ALL: [Condition; 4] = [
        Condition {
            compact: false,
            large: false,
        },
        Condition {
            compact: true,
            large: false,
        },
        Condition {
            compact: false,
            large: true,
        },
        Condition {
            compact: true,
            large: true,
        },
    ];

    fn name(self) -> &'static str {
        match (self.compact, self.large) {
            (false, false) => "comfortable",
            (true, false) => "compact",
            (false, true) => "large",
            (true, true) => "compact-large",
        }
    }

    /// token 값(조건별 표: comfortable, compact, large, compact-large).
    fn token(self, tok: Tok) -> f32 {
        let (comfortable, compact, large, compact_large) = match tok {
            Tok::SpaceXs => (2.0, 1.0, 3.0, 2.0),
            Tok::SpaceS => (4.0, 2.0, 6.0, 3.0),
            Tok::SpaceM => (8.0, 4.0, 10.0, 6.0),
            Tok::SpaceL => (16.0, 8.0, 20.0, 12.0),
            Tok::Icon => (16.0, 16.0, 24.0, 20.0),
            Tok::Control => (32.0, 24.0, 40.0, 32.0),
            Tok::ControlSmall => (24.0, 20.0, 28.0, 24.0),
            Tok::Line => (16.0, 14.0, 20.0, 18.0),
            Tok::Char => (6.0, 6.0, 8.0, 7.0),
        };
        match (self.compact, self.large) {
            (false, false) => comfortable,
            (true, false) => compact,
            (false, true) => large,
            (true, true) => compact_large,
        }
    }
}

// --- component 모델 ---

/// 크기 지정.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Size {
    Px(f32),
    Tok(Tok),
    Content,
    Stretch,
}

/// Button variant 차원.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonSize {
    Small,
    Large,
}

/// variant 값: 고정 또는 바깥 인스턴스 property에서 drilling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariantValue<T> {
    Given(Option<T>),
    Prop(&'static str),
}

/// 인스턴스 node의 variant 선택.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VariantSelection {
    size: VariantValue<ButtonSize>,
    icon_only: VariantValue<bool>,
}

const NO_VARIANT: VariantSelection = VariantSelection {
    size: VariantValue::Given(None),
    icon_only: VariantValue::Given(None),
};

/// def·scene node 본문.
#[derive(Debug, Clone, PartialEq)]
enum Body {
    /// 가로·세로 Stack(gap, 세로 padding, 가로 padding, 너비, 높이).
    Stack {
        axis: Axis,
        gap: Tok,
        pad: (Tok, Tok),
        width: Size,
        height: Size,
        center: bool,
    },
    /// 정사각형 고정 크기(icon·avatar).
    Square(Tok),
    /// property 값을 표시하는 Text.
    Text(&'static str),
    /// Fragment(slot·묶음).
    Fragment,
    /// component 인스턴스(component, property override, variant).
    Instance {
        component: Comp,
        props: Vec<(&'static str, &'static str)>,
        variant: VariantSelection,
    },
    /// scene root Free(Fixed 크기).
    FreeRoot(f32, f32),
    /// scene root grid(열 2개 fr, 행 콘텐츠 크기).
    GridRoot,
}

/// component 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Comp {
    Button,
    Toolbar,
    Card,
    ListItem,
}

/// def·scene node.
#[derive(Debug, Clone, PartialEq)]
struct Node {
    name: &'static str,
    body: Body,
    children: Vec<u64>,
    /// Free 부모 안 위치 또는 Grid 칸(열, 행).
    place: Place,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Place {
    None,
    At(f32, f32),
    Cell(u16, u16),
}

/// variant override patch.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Patch {
    Pad(Tok),
    Height(Tok),
    Hide,
}

/// variant override 하나: 조건(size, icon_only)과 대상 def node 이름.
#[derive(Debug, Clone, Copy, PartialEq)]
struct VariantRule {
    size: Option<ButtonSize>,
    icon_only: Option<bool>,
    node: &'static str,
    patch: Patch,
}

/// component 정의.
#[derive(Debug, Clone, PartialEq)]
struct Definition {
    root: u64,
    nodes: BTreeMap<u64, Node>,
    defaults: Vec<(&'static str, &'static str)>,
    rules: Vec<VariantRule>,
}

fn node(name: &'static str, body: Body, children: &[u64]) -> Node {
    Node {
        name,
        body,
        children: children.to_vec(),
        place: Place::None,
    }
}

fn placed(mut n: Node, place: Place) -> Node {
    n.place = place;
    n
}

fn row(gap: Tok, pad: (Tok, Tok), width: Size, height: Size, center: bool) -> Body {
    Body::Stack {
        axis: Axis::Horizontal,
        gap,
        pad,
        width,
        height,
        center,
    }
}

fn column(gap: Tok, pad: (Tok, Tok), width: Size, height: Size) -> Body {
    Body::Stack {
        axis: Axis::Vertical,
        gap,
        pad,
        width,
        height,
        center: false,
    }
}

fn instance(component: Comp, props: &[(&'static str, &'static str)]) -> Body {
    Body::Instance {
        component,
        props: props.to_vec(),
        variant: NO_VARIANT,
    }
}

fn instance_with(
    component: Comp,
    props: &[(&'static str, &'static str)],
    variant: VariantSelection,
) -> Body {
    Body::Instance {
        component,
        props: props.to_vec(),
        variant,
    }
}

fn definitions() -> HashMap<Comp, Definition> {
    let mut defs = HashMap::new();
    // Button: 높이 control token, 좌우 padding M, icon + label.
    defs.insert(
        Comp::Button,
        Definition {
            root: 100,
            nodes: BTreeMap::from([
                (
                    100,
                    node(
                        "button",
                        row(
                            Tok::SpaceS,
                            (Tok::SpaceXs, Tok::SpaceM),
                            Size::Content,
                            Size::Tok(Tok::Control),
                            true,
                        ),
                        &[101, 102],
                    ),
                ),
                (101, node("icon", Body::Square(Tok::Icon), &[])),
                (102, node("label", Body::Text("label"), &[])),
            ]),
            defaults: vec![("label", "Button")],
            rules: vec![
                // 일반 variant.
                VariantRule {
                    size: Some(ButtonSize::Small),
                    icon_only: None,
                    node: "button",
                    patch: Patch::Height(Tok::ControlSmall),
                },
                VariantRule {
                    size: Some(ButtonSize::Small),
                    icon_only: None,
                    node: "button",
                    patch: Patch::Pad(Tok::SpaceS),
                },
                VariantRule {
                    size: Some(ButtonSize::Large),
                    icon_only: None,
                    node: "button",
                    patch: Patch::Pad(Tok::SpaceL),
                },
                VariantRule {
                    size: None,
                    icon_only: Some(true),
                    node: "label",
                    patch: Patch::Hide,
                },
                VariantRule {
                    size: None,
                    icon_only: Some(true),
                    node: "button",
                    patch: Patch::Pad(Tok::SpaceXs),
                },
                // 구체 조합: large + icon-only는 일반 variant padding을 덮습니다.
                VariantRule {
                    size: Some(ButtonSize::Large),
                    icon_only: Some(true),
                    node: "button",
                    patch: Patch::Pad(Tok::SpaceM),
                },
            ],
        },
    );
    // Toolbar: 너비 채움, Fragment(leading: back, forward) + title + menu(icon-only).
    defs.insert(
        Comp::Toolbar,
        Definition {
            root: 200,
            nodes: BTreeMap::from([
                (
                    200,
                    node(
                        "toolbar",
                        row(
                            Tok::SpaceM,
                            (Tok::SpaceS, Tok::SpaceS),
                            Size::Stretch,
                            Size::Content,
                            true,
                        ),
                        &[201, 204, 205],
                    ),
                ),
                (201, node("leading", Body::Fragment, &[202, 203])),
                (
                    202,
                    node("back", instance(Comp::Button, &[("label", "Back")]), &[]),
                ),
                (
                    203,
                    node(
                        "forward",
                        instance_with(
                            Comp::Button,
                            &[("label", "$next")],
                            VariantSelection {
                                size: VariantValue::Given(Some(ButtonSize::Small)),
                                icon_only: VariantValue::Given(None),
                            },
                        ),
                        &[],
                    ),
                ),
                (204, node("title", Body::Text("title"), &[])),
                (
                    205,
                    node(
                        "menu",
                        instance_with(
                            Comp::Button,
                            &[("label", "Menu")],
                            VariantSelection {
                                size: VariantValue::Prop("menuSize"),
                                icon_only: VariantValue::Given(Some(true)),
                            },
                        ),
                        &[],
                    ),
                ),
            ]),
            defaults: vec![("title", "Title"), ("next", "Next"), ("menuSize", "")],
            rules: Vec::new(),
        },
    );
    // Card: 너비 고정 360, header(Toolbar) + body + actions(cancel, confirm large).
    defs.insert(
        Comp::Card,
        Definition {
            root: 300,
            nodes: BTreeMap::from([
                (
                    300,
                    node(
                        "card",
                        column(
                            Tok::SpaceM,
                            (Tok::SpaceL, Tok::SpaceL),
                            Size::Px(360.0),
                            Size::Content,
                        ),
                        &[301, 302, 303],
                    ),
                ),
                (
                    301,
                    node(
                        "header",
                        instance(Comp::Toolbar, &[("title", "$title"), ("menuSize", "large")]),
                        &[],
                    ),
                ),
                (302, node("body", Body::Text("body"), &[])),
                (
                    303,
                    node(
                        "actions",
                        row(
                            Tok::SpaceS,
                            (Tok::SpaceXs, Tok::SpaceXs),
                            Size::Content,
                            Size::Content,
                            false,
                        ),
                        &[304, 305],
                    ),
                ),
                (
                    304,
                    node(
                        "cancel",
                        instance(Comp::Button, &[("label", "Cancel")]),
                        &[],
                    ),
                ),
                (
                    305,
                    node(
                        "confirm",
                        instance_with(
                            Comp::Button,
                            &[("label", "$confirm")],
                            VariantSelection {
                                size: VariantValue::Given(Some(ButtonSize::Large)),
                                icon_only: VariantValue::Given(None),
                            },
                        ),
                        &[],
                    ),
                ),
            ]),
            defaults: vec![
                ("title", "Card"),
                ("body", "Body text here"),
                ("confirm", "OK"),
            ],
            rules: Vec::new(),
        },
    );
    // ListItem: avatar + text + action(small, icon-only는 property로 drilling).
    defs.insert(
        Comp::ListItem,
        Definition {
            root: 400,
            nodes: BTreeMap::from([
                (
                    400,
                    node(
                        "item",
                        row(
                            Tok::SpaceS,
                            (Tok::SpaceS, Tok::SpaceS),
                            Size::Stretch,
                            Size::Content,
                            true,
                        ),
                        &[401, 402, 403],
                    ),
                ),
                (401, node("avatar", Body::Square(Tok::Control), &[])),
                (402, node("text", Body::Text("text"), &[])),
                (
                    403,
                    node(
                        "action",
                        instance_with(
                            Comp::Button,
                            &[("label", "$action")],
                            VariantSelection {
                                size: VariantValue::Given(Some(ButtonSize::Small)),
                                icon_only: VariantValue::Prop("actionIconOnly"),
                            },
                        ),
                        &[],
                    ),
                ),
            ]),
            defaults: vec![("text", "Item"), ("action", "Go"), ("actionIconOnly", "")],
            rules: Vec::new(),
        },
    );
    defs
}

/// 이번 엔진이 받는 scene(이름, root id, node). grid root scene(dashboard)은 `display: grid`를 받은 뒤 다시 넣습니다.
fn scenes() -> Vec<(&'static str, u64, BTreeMap<u64, Node>)> {
    all_scenes()
        .into_iter()
        .filter(|(name, _, _)| *name != "dashboard")
        .collect()
}

/// scene 5개(이름, root id, node).
fn all_scenes() -> Vec<(&'static str, u64, BTreeMap<u64, Node>)> {
    vec![
        (
            "button",
            1,
            BTreeMap::from([
                (1, node("root", Body::FreeRoot(400.0, 200.0), &[2, 3, 4])),
                (
                    2,
                    placed(
                        node("save", instance(Comp::Button, &[("label", "Save")]), &[]),
                        Place::At(10.0, 10.0),
                    ),
                ),
                (
                    3,
                    placed(
                        node(
                            "big-icon",
                            instance_with(
                                Comp::Button,
                                &[],
                                VariantSelection {
                                    size: VariantValue::Given(Some(ButtonSize::Large)),
                                    icon_only: VariantValue::Given(Some(true)),
                                },
                            ),
                            &[],
                        ),
                        Place::At(10.0, 80.0),
                    ),
                ),
                (
                    4,
                    placed(
                        node(
                            "small-icon",
                            instance_with(
                                Comp::Button,
                                &[],
                                VariantSelection {
                                    size: VariantValue::Given(Some(ButtonSize::Small)),
                                    icon_only: VariantValue::Given(Some(true)),
                                },
                            ),
                            &[],
                        ),
                        Place::At(120.0, 80.0),
                    ),
                ),
            ]),
        ),
        (
            "toolbar",
            1,
            BTreeMap::from([
                (
                    1,
                    node(
                        "root",
                        column(
                            Tok::SpaceM,
                            (Tok::SpaceM, Tok::SpaceM),
                            Size::Px(480.0),
                            Size::Content,
                        ),
                        &[2, 3],
                    ),
                ),
                (
                    2,
                    node("main", instance(Comp::Toolbar, &[("title", "Inbox")]), &[]),
                ),
                (
                    3,
                    node(
                        "secondary",
                        instance(
                            Comp::Toolbar,
                            &[("title", "Drafts and outbox"), ("next", "Later")],
                        ),
                        &[],
                    ),
                ),
            ]),
        ),
        (
            "card",
            1,
            BTreeMap::from([
                (1, node("root", Body::FreeRoot(600.0, 400.0), &[2])),
                (
                    2,
                    placed(
                        node(
                            "order",
                            instance(
                                Comp::Card,
                                &[("title", "Order"), ("confirm", "Confirm order")],
                            ),
                            &[],
                        ),
                        Place::At(20.0, 20.0),
                    ),
                ),
            ]),
        ),
        (
            "list",
            1,
            BTreeMap::from([
                (
                    1,
                    node(
                        "root",
                        column(
                            Tok::SpaceXs,
                            (Tok::SpaceS, Tok::SpaceS),
                            Size::Px(320.0),
                            Size::Content,
                        ),
                        &[2, 3, 4],
                    ),
                ),
                (
                    2,
                    node("first", instance(Comp::ListItem, &[("text", "First")]), &[]),
                ),
                (
                    3,
                    node(
                        "second",
                        instance(
                            Comp::ListItem,
                            &[("text", "Second item longer"), ("action", "Open")],
                        ),
                        &[],
                    ),
                ),
                (
                    4,
                    node(
                        "third",
                        instance(
                            Comp::ListItem,
                            &[("text", "Third"), ("actionIconOnly", "yes")],
                        ),
                        &[],
                    ),
                ),
            ]),
        ),
        (
            "dashboard",
            1,
            BTreeMap::from([
                (1, node("root", Body::GridRoot, &[2, 3, 4])),
                (
                    2,
                    placed(
                        node(
                            "summary",
                            instance(Comp::Card, &[("body", "Weekly totals")]),
                            &[],
                        ),
                        Place::Cell(1, 1),
                    ),
                ),
                (
                    3,
                    placed(
                        node("bar", instance(Comp::Toolbar, &[("title", "Filters")]), &[]),
                        Place::Cell(2, 1),
                    ),
                ),
                (
                    4,
                    placed(
                        node(
                            "refresh",
                            instance(Comp::Button, &[("label", "Refresh now")]),
                            &[],
                        ),
                        Place::Cell(2, 2),
                    ),
                ),
            ]),
        ),
    ]
}

// --- LayoutSource 구현 ---

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Key {
    Scene(u64),
    Virtual(Vec<u64>, u64),
}

struct Fixture<'a> {
    defs: &'a HashMap<Comp, Definition>,
    scene: &'a BTreeMap<u64, Node>,
    root: u64,
    condition: Condition,
}

/// 인스턴스 하나의 해석 결과(component, property, variant).
struct Resolved {
    component: Comp,
    props: HashMap<&'static str, String>,
    size: Option<ButtonSize>,
    icon_only: Option<bool>,
}

fn length(px: f32) -> Length {
    Length::new(px).expect("fixture 길이는 유한")
}

fn order_id(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

impl Fixture<'_> {
    /// scene 또는 def node.
    fn node(&self, key: &Key) -> Result<&Node, &'static str> {
        match key {
            Key::Scene(n) => self.scene.get(n).ok_or("없는 scene node"),
            Key::Virtual(chain, n) => {
                let resolved = self.resolve_chain(chain)?;
                self.defs[&resolved.component]
                    .nodes
                    .get(n)
                    .ok_or("정의에 없는 node")
            }
        }
    }

    /// 인스턴스 node(`instance`)를 바깥 property `outer`로 해석합니다.
    fn resolve_instance(
        &self,
        instance: &Node,
        outer: &HashMap<&'static str, String>,
    ) -> Result<Resolved, &'static str> {
        let Body::Instance {
            component,
            props,
            variant,
        } = &instance.body
        else {
            return Err("인스턴스가 아님");
        };
        let def = &self.defs[component];
        let mut values: HashMap<&'static str, String> = def
            .defaults
            .iter()
            .map(|(k, v)| (*k, (*v).to_owned()))
            .collect();
        let lookup = |value: &str| -> String {
            match value.strip_prefix('$') {
                Some(name) => outer.get(name).cloned().unwrap_or_default(),
                None => value.to_owned(),
            }
        };
        for (name, value) in props {
            values.insert(name, lookup(value));
        }
        let size = match variant.size {
            VariantValue::Given(value) => value,
            VariantValue::Prop(name) => match outer.get(name).map(String::as_str) {
                Some("small") => Some(ButtonSize::Small),
                Some("large") => Some(ButtonSize::Large),
                _ => None,
            },
        };
        let icon_only = match variant.icon_only {
            VariantValue::Given(value) => value,
            VariantValue::Prop(name) => match outer.get(name).map(String::as_str) {
                Some("") | None => None,
                Some(_) => Some(true),
            },
        };
        Ok(Resolved {
            component: *component,
            props: values,
            size,
            icon_only,
        })
    }

    /// 인스턴스 chain(scene 인스턴스 → nested 인스턴스 def node)을 바깥부터 해석합니다.
    fn resolve_chain(&self, chain: &[u64]) -> Result<Resolved, &'static str> {
        let first = chain.first().ok_or("빈 chain")?;
        let scene_instance = self.scene.get(first).ok_or("없는 scene 인스턴스")?;
        let mut resolved = self.resolve_instance(scene_instance, &HashMap::new())?;
        for nested in &chain[1..] {
            let def = &self.defs[&resolved.component];
            let node = def.nodes.get(nested).ok_or("정의에 없는 nested 인스턴스")?;
            resolved = self.resolve_instance(node, &resolved.props)?;
        }
        Ok(resolved)
    }

    /// fixture 크기 지정을 CSS 값으로 바꿉니다(속성 모델 4.3절): px·token은 `<length>`, 콘텐츠 크기는
    /// `fit-content`, 채움은 `auto`(block 부모 안에서 늘어남).
    fn size(&self, size: Size) -> CssSize {
        match size {
            Size::Px(px) => CssSize::Length(length(px)),
            Size::Tok(tok) => CssSize::Length(length(self.condition.token(tok))),
            Size::Content => CssSize::FitContent,
            Size::Stretch => CssSize::Auto,
        }
    }

    /// variant rule 적용(일반 → 구체 순서, IX.7.6).
    fn patches(&self, resolved: &Resolved, name: &str) -> Vec<Patch> {
        let def = &self.defs[&resolved.component];
        let general = def.rules.iter().filter(|rule| {
            rule.node == name
                && match (rule.size, rule.icon_only) {
                    (Some(size), None) => resolved.size == Some(size),
                    (None, Some(flag)) => resolved.icon_only == Some(flag),
                    _ => false,
                }
        });
        let concrete = def.rules.iter().filter(|rule| {
            rule.node == name
                && rule.size.is_some()
                && rule.icon_only.is_some()
                && rule.size == resolved.size
                && rule.icon_only == resolved.icon_only
        });
        general.chain(concrete).map(|rule| rule.patch).collect()
    }

    fn layout_node(&self, key: &Key) -> Result<LayoutNode, &'static str> {
        let node = self.node(key)?;
        let resolved = match key {
            Key::Virtual(chain, _) => Some(self.resolve_chain(chain)?),
            Key::Scene(_) => None,
        };
        let token = |tok: Tok| length(self.condition.token(tok));
        // 이전 엔진의 flex item 생략값(줄어들지 않음)을 그대로 쓰려고 모든 node에 flex-shrink 0을 둡니다.
        let base = LayoutStyle {
            flex_shrink: FiniteF32::ZERO,
            ..LayoutStyle::default()
        };
        let mut out = match &node.body {
            Body::Stack {
                axis,
                gap,
                pad,
                width,
                height,
                center,
            } => {
                let (v, h) = (self.condition.token(pad.0), self.condition.token(pad.1));
                let mut style = LayoutStyle {
                    display: Display::Flex,
                    flex_direction: match axis {
                        Axis::Horizontal => FlexDirection::Row,
                        Axis::Vertical => FlexDirection::Column,
                    },
                    align_items: if *center {
                        ItemAlignment::Center
                    } else {
                        ItemAlignment::FlexStart
                    },
                    width: self.size(*width),
                    height: self.size(*height),
                    padding: Insets::new(v, h, v, h).map_err(|_| "padding")?,
                    ..base.clone()
                };
                match axis {
                    Axis::Horizontal => style.column_gap = token(*gap),
                    Axis::Vertical => style.row_gap = token(*gap),
                }
                LayoutNode::new(order_id(0)).with_style(style)
            }
            Body::Square(tok) => LayoutNode::new(order_id(0)).with_style(LayoutStyle {
                width: CssSize::Length(token(*tok)),
                height: CssSize::Length(token(*tok)),
                ..base.clone()
            }),
            Body::Text(prop) => {
                let text = match &resolved {
                    Some(resolved) => resolved.props.get(prop).cloned().unwrap_or_default(),
                    None => (*prop).to_owned(),
                };
                let char_width = self.condition.token(Tok::Char);
                let line = self.condition.token(Tok::Line);
                let words = text
                    .split(' ')
                    .filter(|word| !word.is_empty())
                    .map(|word| length(word.chars().count() as f32 * char_width))
                    .collect();
                LayoutNode::new(order_id(0))
                    .with_style(LayoutStyle {
                        width: CssSize::FitContent,
                        height: CssSize::FitContent,
                        ..base.clone()
                    })
                    .with_content(LayoutContent::Measured(MeasuredContent::Text(
                        TextIntrinsic {
                            words,
                            space: length(char_width),
                            line_height: length(line),
                            ascent: length(line * 0.75),
                        },
                    )))
            }
            Body::Fragment => return Err("Fragment는 box가 아님"),
            Body::Instance { .. } => LayoutNode::new(order_id(0))
                .with_style(LayoutStyle {
                    width: CssSize::FitContent,
                    height: CssSize::FitContent,
                    ..base.clone()
                })
                .with_content(LayoutContent::Measured(MeasuredContent::ComponentInstance(
                    Default::default(),
                ))),
            Body::FreeRoot(w, h) => LayoutNode::new(order_id(0)).with_style(LayoutStyle {
                width: CssSize::Length(length(*w)),
                height: CssSize::Length(length(*h)),
                ..base.clone()
            }),
            Body::GridRoot => return Err("grid root는 아직 받지 않습니다"),
        };
        match node.place {
            Place::None => {}
            Place::At(x, y) => {
                out.style.position = Position::Absolute;
                out.style.left = LengthPercentageAuto::Length(length(x));
                out.style.top = LengthPercentageAuto::Length(length(y));
            }
            Place::Cell(..) => return Err("grid 칸 배치는 아직 받지 않습니다"),
        }
        if let Some(resolved) = &resolved {
            for patch in self.patches(resolved, node.name) {
                match patch {
                    Patch::Pad(tok) => {
                        let (top, _, bottom, _) = out.style.padding.components();
                        let h = self.condition.token(tok);
                        out.style.padding =
                            Insets::new(top, h, bottom, h).map_err(|_| "padding")?;
                    }
                    Patch::Height(tok) => out.style.height = CssSize::Length(token(tok)),
                    Patch::Hide => out.style.display = Display::None,
                }
            }
        }
        Ok(out)
    }

    /// 사람이 읽는 node 경로(golden 표시용).
    fn label(&self, key: &Key) -> String {
        match key {
            Key::Scene(n) => self.scene.get(n).map_or("?", |node| node.name).to_owned(),
            Key::Virtual(chain, n) => {
                let mut parts = vec![self
                    .scene
                    .get(&chain[0])
                    .map_or("?", |node| node.name)
                    .to_owned()];
                let mut component = match self.scene.get(&chain[0]).map(|node| &node.body) {
                    Some(Body::Instance { component, .. }) => *component,
                    _ => return "?".to_owned(),
                };
                for nested in &chain[1..] {
                    let node = &self.defs[&component].nodes[nested];
                    parts.push(node.name.to_owned());
                    if let Body::Instance {
                        component: inner, ..
                    } = node.body
                    {
                        component = inner;
                    }
                }
                parts.push(self.defs[&component].nodes[n].name.to_owned());
                parts.join("/")
            }
        }
    }
}

impl LayoutSource for Fixture<'_> {
    type Key = Key;
    type Error = &'static str;

    fn root(&self) -> Key {
        Key::Scene(self.root)
    }

    fn node(&self, key: &Key) -> Result<SourceNode, &'static str> {
        let node = Fixture::node(self, key)?;
        if node.body == Body::Fragment {
            return Ok(SourceNode::Fragment { visible: true });
        }
        Ok(SourceNode::Node(Box::new(self.layout_node(key)?)))
    }

    fn children(&self, key: &Key) -> Result<Vec<Key>, &'static str> {
        let node = Fixture::node(self, key)?;
        if let Body::Instance { component, .. } = &node.body {
            // instance box의 자식은 component 정의 root의 가상 투영입니다(IX.2.2).
            let mut chain = match key {
                Key::Scene(_) => Vec::new(),
                Key::Virtual(chain, _) => chain.clone(),
            };
            chain.push(match key {
                Key::Scene(n) | Key::Virtual(_, n) => *n,
            });
            return Ok(vec![Key::Virtual(chain, self.defs[component].root)]);
        }
        Ok(node
            .children
            .iter()
            .map(|child| match key {
                Key::Scene(_) => Key::Scene(*child),
                Key::Virtual(chain, _) => Key::Virtual(chain.clone(), *child),
            })
            .collect())
    }

    fn layout_id(&self, key: &Key) -> StableId {
        match key {
            Key::Scene(n) => order_id(*n),
            Key::Virtual(chain, n) => {
                let chain: Vec<StableId> = chain.iter().map(|c| order_id(*c)).collect();
                virtual_layout_id(&chain, order_id(*n))
            }
        }
    }
}

// --- 실행 ---

/// px 표시(1/64 px 정수라 정확한 십진 표현).
fn px(unit: stellar_layout::rounding::LayoutUnit) -> String {
    let value = f64::from(unit.raw()) / 64.0;
    format!("{value}")
}

/// fixture 하나의 golden 줄.
fn fixture_lines(name: &str, fixture: &Fixture<'_>) -> (Vec<String>, LayoutInput, LayoutResult) {
    let view = build_layout_tree(fixture).unwrap_or_else(|e| panic!("{name}: {e}"));
    let result = compute_layout(view.input()).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = vec![format!(
        "fixture {name} boxes={} diagnostics={}",
        result.len(),
        result.diagnostics().len()
    )];
    for layout_box in result.boxes() {
        let key = view.source_key(layout_box.id()).expect("view key");
        let rect = layout_box.local_rect();
        let mut line = format!(
            "  {} {},{} {}x{}",
            fixture.label(key),
            px(rect.x()),
            px(rect.y()),
            px(rect.width()),
            px(rect.height())
        );
        if let Some(baseline) = layout_box.baseline() {
            let _ = write!(line, " b={}", px(baseline));
        }
        lines.push(line);
    }
    (lines, view.into_input(), result)
}

fn golden_lines() -> Vec<String> {
    GOLDEN
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn twenty_component_variant_token_fixtures_match_expected_rect_golden() {
    let defs = definitions();
    let scenes = scenes();
    let mut actual = Vec::new();
    let mut results: Vec<Vec<u8>> = Vec::new();
    for (scene_name, root, scene) in &scenes {
        for condition in Condition::ALL {
            let fixture = Fixture {
                defs: &defs,
                scene,
                root: *root,
                condition,
            };
            let name = format!("{scene_name}-{}", condition.name());
            let (lines, _, result) = fixture_lines(&name, &fixture);
            // 같은 fixture는 다시 계산해도 같은 결과입니다(결정적).
            assert_eq!(fixture_lines(&name, &fixture).0, lines, "{name}");
            actual.extend(lines);
            results.push(result.encode());
        }
    }
    assert_eq!(results.len(), FIXTURE_COUNT);
    // fixture 결과는 서로 다릅니다(조건·variant가 실제로 레이아웃을 바꿈).
    for (i, a) in results.iter().enumerate() {
        for b in &results[i + 1..] {
            assert_ne!(a, b);
        }
    }
    let expected = golden_lines();
    assert_eq!(
        actual,
        expected,
        "golden 불일치. 실제 값:\n{}",
        actual.join("\n")
    );
}

#[test]
fn fixtures_cover_nesting_variants_and_tokens() {
    let defs = definitions();
    let scenes = scenes();
    let (_, root, card) = &scenes[2];
    let fixture = Fixture {
        defs: &defs,
        scene: card,
        root: *root,
        condition: Condition::ALL[0],
    };
    let view = build_layout_tree(&fixture).unwrap();
    // 3단계 중첩: card 인스턴스(2) → header Toolbar(301) → menu Button(205)의 label.
    let label = Key::Virtual(vec![2, 301, 205], 102);
    let id = view.layout_id(&label).expect("3단계 virtual node");
    assert_eq!(fixture.label(&label), "order/header/menu/label");
    // menu는 icon-only(label 숨김)이고 drilling된 menuSize=large로 구체 조합 padding(M)을 받습니다.
    assert_eq!(view.input().node(id).unwrap().style.display, Display::None);
    let menu_root = view
        .layout_id(&Key::Virtual(vec![2, 301, 205], 100))
        .unwrap();
    let padding = view
        .input()
        .node(menu_root)
        .unwrap()
        .style
        .padding
        .components();
    assert_eq!(padding.1, Condition::ALL[0].token(Tok::SpaceM));
    // forward는 small 일반 variant(높이 ControlSmall), label은 바깥 property drilling(next 기본값 Next).
    let forward = view
        .layout_id(&Key::Virtual(vec![2, 301, 203], 100))
        .unwrap();
    assert_eq!(
        view.input().node(forward).unwrap().style.height,
        CssSize::Length(length(Condition::ALL[0].token(Tok::ControlSmall)))
    );
    // Toolbar title은 Card title("Order")을 drilling합니다.
    let title_words = match &view
        .input()
        .node(view.layout_id(&Key::Virtual(vec![2, 301], 204)).unwrap())
        .unwrap()
        .content
    {
        LayoutContent::Measured(MeasuredContent::Text(text)) => text.words.len(),
        _ => 0,
    };
    assert_eq!(title_words, 1);
    // Fragment(leading)는 box가 없고 자식은 toolbar root가 받습니다.
    assert!(view.layout_id(&Key::Virtual(vec![2, 301], 201)).is_none());
    assert!(view
        .fragment_host(&Key::Virtual(vec![2, 301], 201))
        .is_some());
}

#[test]
fn token_condition_switches_apply_incrementally() {
    let defs = definitions();
    for (scene_name, root, scene) in &scenes() {
        let build = |condition: Condition| {
            let fixture = Fixture {
                defs: &defs,
                scene,
                root: *root,
                condition,
            };
            fixture_lines(scene_name, &fixture)
        };
        let (_, first, _) = build(Condition::ALL[0]);
        let (mut session, _) = IncrementalLayout::new(first, &DefaultMeasurer).unwrap();
        for condition in Condition::ALL.iter().skip(1).chain([&Condition::ALL[0]]) {
            let (_, next, expected) = build(*condition);
            // 바뀐 node만 edit로 넘깁니다(가상 id는 조건과 무관하게 같음).
            let edits: Vec<LayoutEdit> = next
                .nodes
                .iter()
                .filter(|node| session.node(node.id) != Some(node))
                .cloned()
                .map(LayoutEdit::upsert)
                .collect();
            assert!(!edits.is_empty(), "{scene_name} {}", condition.name());
            assert!(edits.len() <= next.nodes.len());
            let stats = session.apply(edits, &DefaultMeasurer).unwrap();
            assert_eq!(stats.full_layouts, 0, "{scene_name} {}", condition.name());
            assert_eq!(
                session.result().unwrap().encode(),
                expected.encode(),
                "{scene_name} {}",
                condition.name()
            );
        }
        assert_eq!(session.metrics().unexpected_fallback, 0);
        assert_eq!(session.metrics().full_layouts, 1);
    }
}

#[test]
fn virtual_ids_stay_stable_across_conditions() {
    let defs = definitions();
    for (_, root, scene) in &scenes() {
        let ids = |condition: Condition| {
            let fixture = Fixture {
                defs: &defs,
                scene,
                root: *root,
                condition,
            };
            let view = build_layout_tree(&fixture).unwrap();
            let mut ids: Vec<StableId> = view.input().nodes.iter().map(|n| n.id).collect();
            ids.sort();
            ids
        };
        let base = ids(Condition::ALL[0]);
        for condition in Condition::ALL {
            assert_eq!(ids(condition), base);
        }
    }
}
