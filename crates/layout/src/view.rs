//! layout tree view (X.1.2).
//!
//! 레이아웃 엔진은 문서 모델 crate에 의존하지 않습니다(core-types만 의존). 문서 쪽 adapter는
//! [`LayoutSource`]를 구현해 **이미 해석된** node를 읽기 전용으로 제공하고, [`build_layout_tree`]가
//! 그 tree를 레이아웃 입력([`LayoutInput`])으로 펼칩니다. view는 원천을 `&self`로만 읽으며 문서를
//! 수정하지 않습니다(trait에 변경 API가 없음).
//!
//! # 원천이 책임지는 값
//!
//! - **ComponentInstance virtual subtree**(IX.2.2): 인스턴스 node의 [`LayoutSource::children`]은
//!   expansion view의 가상 자식(정의 tree 투영, slot 투영 포함)입니다. 정의 node를 복제하지 않고 key로만
//!   가리킵니다. 가상 node의 layout id는 [`virtual_layout_id`]로 만들 수 있습니다.
//! - **View Variant 적용 결과**(IX.7.6)와 instance override·binding: [`SourceNode::Node`]의 속성은 적용이
//!   끝난 최종 값입니다. view는 값을 다시 해석하지 않고 그대로 읽습니다.
//!
//! # view가 하는 일
//!
//! - **Fragment flatten**(VI.4.1, 아키텍처 12.4): Fragment는 자체 box·좌표계가 없으므로 결과에 나타나지
//!   않습니다. Fragment의 자식은 Fragment 자리에서(visual order 유지) 가장 가까운 box 조상의 자식이
//!   됩니다. 중첩 Fragment도 같은 규칙으로 깊이 우선 펼칩니다. 숨겨진 Fragment의 자식은 숨김으로
//!   펼칩니다(Fragment의 표시 여부가 자식 전체에 적용).
//! - **visual order**: 펼친 자식 순서를 view 전용 균등 분포 [`OrderKey`]로 다시 매깁니다. 문서
//!   OrderKey를 그대로 쓰지 않습니다(Fragment 안팎 key를 섞으면 순서가 보존되지 않음).
//! - **검증**: 결과 입력은 [`LayoutInput::validate`]를 통과해야 합니다. 같은 원천 key가 두 번 나오면
//!   (공유 자식·순환) [`ViewError::RepeatedSourceNode`], 두 key가 같은 layout id를 쓰면
//!   [`ViewError::DuplicateLayoutId`]입니다. 펼치기는 반복(비재귀)이며 방문 node 수는
//!   [`MAX_VIEW_NODES`]까지입니다.
//!
//! 결과 node 순서는 box tree pre-order(부모 다음 자식, 자식은 visual order)입니다.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;

use crate::cache::LayoutInputHasher;
use crate::error::LayoutError;
use crate::input::{LayoutChild, LayoutInput, LayoutNode};
use crate::style::Display;

/// view가 방문하는 원천 node(box + Fragment) 수 상한(프로젝트 전체 node 상한과 같은 값).
pub const MAX_VIEW_NODES: usize = 1_000_000;

/// 원천 node 하나 (X.1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceNode {
    /// 레이아웃 box를 가진 node. 속성은 View Variant·override 적용이 끝난 값입니다. `id`·`children`
    /// 필드는 view가 다시 씁니다(원천 값은 무시). 숨긴 Fragment 아래 node는 `display: none`이 됩니다.
    Node(Box<LayoutNode>),
    /// 자체 box가 없는 Fragment(VI.4.1).
    Fragment {
        /// Fragment 표시 여부. `false`면 펼친 자식이 모두 숨김입니다.
        visible: bool,
    },
}

/// 레이아웃이 읽는 원천 tree (X.1.2).
///
/// 모든 메서드는 `&self`이며 원천을 바꾸지 않아야 합니다. 같은 key에 대한 응답은 한 번의
/// [`build_layout_tree`] 동안 일정해야 합니다.
pub trait LayoutSource {
    /// 원천 node key(scene StableId, expansion 가상 id 등).
    type Key: Clone + Eq + Hash + fmt::Debug;
    /// 원천 읽기 오류.
    type Error: Clone + fmt::Debug;

    /// view root key. root는 Fragment일 수 없습니다.
    fn root(&self) -> Self::Key;

    /// node 내용.
    fn node(&self, key: &Self::Key) -> Result<SourceNode, Self::Error>;

    /// 자식 key 목록(visual order). Fragment도 자식을 가지며, ComponentInstance는 virtual subtree
    /// 자식(expansion view의 가상 root)을 돌려줍니다.
    fn children(&self, key: &Self::Key) -> Result<Vec<Self::Key>, Self::Error>;

    /// box node의 layout id. Fragment에는 호출하지 않습니다.
    fn layout_id(&self, key: &Self::Key) -> StableId;
}

/// view 구성 오류 (X.1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewError<K, E> {
    /// 원천 읽기 실패.
    Source(E),
    /// root가 Fragment입니다(Fragment는 box가 없어 layout root가 될 수 없음).
    FragmentRoot,
    /// 같은 원천 key가 두 번 나왔습니다(공유 자식 또는 순환).
    RepeatedSourceNode {
        /// 반복된 key.
        key: K,
    },
    /// 서로 다른 원천 key가 같은 layout id를 씁니다.
    DuplicateLayoutId {
        /// 중복 layout id.
        id: StableId,
    },
    /// 방문 node 수가 [`MAX_VIEW_NODES`]를 넘습니다.
    NodeLimitExceeded {
        /// 상한.
        limit: usize,
    },
    /// 펼친 입력이 레이아웃 검증을 통과하지 못했습니다.
    Layout(LayoutError),
}

impl<K: fmt::Debug, E: fmt::Debug> fmt::Display for ViewError<K, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViewError::Source(error) => write!(f, "layout 원천 읽기 실패: {error:?}"),
            ViewError::FragmentRoot => f.write_str("Fragment는 layout root가 될 수 없습니다"),
            ViewError::RepeatedSourceNode { key } => {
                write!(f, "원천 node {key:?}가 tree에 두 번 나옵니다")
            }
            ViewError::DuplicateLayoutId { id } => {
                write!(f, "서로 다른 원천 node가 layout id {id}를 함께 씁니다")
            }
            ViewError::NodeLimitExceeded { limit } => {
                write!(f, "layout view node 수가 상한 {limit}을 넘습니다")
            }
            ViewError::Layout(error) => write!(f, "layout view 검증 실패: {error}"),
        }
    }
}

impl<K: fmt::Debug, E: fmt::Debug> std::error::Error for ViewError<K, E> {}

/// 원천 `S`의 view 오류.
pub type SourceViewError<S> = ViewError<<S as LayoutSource>::Key, <S as LayoutSource>::Error>;

/// 펼친 layout tree와 원천 대응표 (X.1.2).
#[derive(Debug, Clone)]
pub struct LayoutTreeView<K> {
    input: LayoutInput,
    origin: HashMap<StableId, K>,
    ids: HashMap<K, StableId>,
    fragments: HashMap<K, StableId>,
}

impl<K: Clone + Eq + Hash> LayoutTreeView<K> {
    /// 레이아웃 입력.
    pub fn input(&self) -> &LayoutInput {
        &self.input
    }

    /// 레이아웃 입력을 꺼냅니다.
    pub fn into_input(self) -> LayoutInput {
        self.input
    }

    /// layout id의 원천 key.
    pub fn source_key(&self, id: StableId) -> Option<&K> {
        self.origin.get(&id)
    }

    /// box 원천 key의 layout id. Fragment·view 밖 key는 `None`입니다.
    pub fn layout_id(&self, key: &K) -> Option<StableId> {
        self.ids.get(key).copied()
    }

    /// Fragment의 자식을 받아 펼친 box 조상(layout id). Fragment가 아니면 `None`입니다.
    ///
    /// Fragment 자식 목록·표시 여부 변경은 이 node의 자식 변경으로 전파해야 합니다(X.7.1).
    pub fn fragment_host(&self, key: &K) -> Option<StableId> {
        self.fragments.get(key).copied()
    }

    /// box node 수.
    pub fn len(&self) -> usize {
        self.input.nodes.len()
    }

    /// box node가 없는지 여부(검증된 view는 root가 있어 항상 `false`).
    pub fn is_empty(&self) -> bool {
        self.input.nodes.is_empty()
    }
}

/// expansion 가상 node `(인스턴스 chain, 정의 node)`의 결정적 layout id (X.1.2).
///
/// 같은 정의 node라도 인스턴스 chain이 다르면 다른 id입니다. id는 domain 분리된
/// [`LayoutInputHasher`] 두 값(128비트)이며 UUID version 8·variant `10`으로 표시해 실제 문서
/// StableId(version 7)와 겹치지 않습니다. 런타임 전용이며 저장하지 않습니다.
pub fn virtual_layout_id(instances: &[StableId], source: StableId) -> StableId {
    let mut hasher = LayoutInputHasher::new();
    hasher.write_bytes(b"stellar-layout/virtual-node/v1");
    hasher.write_u64(instances.len() as u64);
    for instance in instances {
        hasher.write_stable_id(*instance);
    }
    hasher.write_stable_id(source);
    let hash = hasher.finish();
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hash.raw().to_be_bytes());
    bytes[8..].copy_from_slice(&hash.check().to_be_bytes());
    bytes[6] = (bytes[6] & 0x0F) | 0x80;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    StableId::from_bytes(bytes)
}

/// 펼치는 중인 box node(원천 key, 내용, layout id).
struct PendingBox<K> {
    key: K,
    node: LayoutNode,
    id: StableId,
}

/// view 구성 상태.
struct Builder<'s, S: LayoutSource> {
    source: &'s S,
    seen: HashSet<S::Key>,
    origin: HashMap<StableId, S::Key>,
    ids: HashMap<S::Key, StableId>,
    fragments: HashMap<S::Key, StableId>,
}

impl<S: LayoutSource> Builder<'_, S> {
    /// 원천 key를 처음 방문함을 기록합니다.
    fn visit(&mut self, key: &S::Key) -> Result<(), SourceViewError<S>> {
        if self.seen.len() >= MAX_VIEW_NODES {
            return Err(ViewError::NodeLimitExceeded {
                limit: MAX_VIEW_NODES,
            });
        }
        if !self.seen.insert(key.clone()) {
            return Err(ViewError::RepeatedSourceNode { key: key.clone() });
        }
        Ok(())
    }

    /// 아직 방문하지 않은 `pending`개를 더해도 상한 안인지 검사합니다(큰 자식 목록을 쌓기 전에 거부).
    fn check_pending(&self, pending: usize) -> Result<(), SourceViewError<S>> {
        if self.seen.len().saturating_add(pending) > MAX_VIEW_NODES {
            return Err(ViewError::NodeLimitExceeded {
                limit: MAX_VIEW_NODES,
            });
        }
        Ok(())
    }

    /// box node의 layout id를 등록합니다.
    fn register(&mut self, key: &S::Key) -> Result<StableId, SourceViewError<S>> {
        let id = self.source.layout_id(key);
        if self.origin.insert(id, key.clone()).is_some() {
            return Err(ViewError::DuplicateLayoutId { id });
        }
        self.ids.insert(key.clone(), id);
        Ok(id)
    }

    /// `host` box의 자식을 Fragment를 펼쳐 visual order로 모읍니다.
    fn flatten(
        &mut self,
        host: &PendingBox<S::Key>,
    ) -> Result<Vec<PendingBox<S::Key>>, SourceViewError<S>> {
        let children = self.source.children(&host.key).map_err(ViewError::Source)?;
        // (key, Fragment 조상 표시 여부). 뒤에서 꺼내므로 역순으로 넣습니다.
        self.check_pending(children.len())?;
        let mut pending: Vec<(S::Key, bool)> =
            children.into_iter().rev().map(|key| (key, true)).collect();
        let mut flat = Vec::new();
        while let Some((key, inherited)) = pending.pop() {
            self.visit(&key)?;
            match self.source.node(&key).map_err(ViewError::Source)? {
                SourceNode::Fragment { visible } => {
                    self.fragments.insert(key.clone(), host.id);
                    let grand = self.source.children(&key).map_err(ViewError::Source)?;
                    self.check_pending(pending.len().saturating_add(grand.len()))?;
                    let shown = inherited && visible;
                    pending.extend(grand.into_iter().rev().map(|child| (child, shown)));
                }
                SourceNode::Node(node) => {
                    let mut node = *node;
                    if !inherited {
                        node.style.display = Display::None;
                    }
                    let id = self.register(&key)?;
                    flat.push(PendingBox { key, node, id });
                }
            }
        }
        Ok(flat)
    }
}

/// 원천 tree를 Fragment를 펼친 레이아웃 입력으로 만듭니다 (X.1.2).
///
/// 원천은 읽기만 합니다. 결과 입력은 검증을 통과한 상태입니다.
pub fn build_layout_tree<S: LayoutSource>(
    source: &S,
) -> Result<LayoutTreeView<S::Key>, SourceViewError<S>> {
    let mut builder = Builder {
        source,
        seen: HashSet::new(),
        origin: HashMap::new(),
        ids: HashMap::new(),
        fragments: HashMap::new(),
    };
    let root_key = source.root();
    builder.visit(&root_key)?;
    let root_node = match source.node(&root_key).map_err(ViewError::Source)? {
        SourceNode::Node(node) => *node,
        SourceNode::Fragment { .. } => return Err(ViewError::FragmentRoot),
    };
    let root_id = builder.register(&root_key)?;
    let mut work = vec![PendingBox {
        key: root_key,
        node: root_node,
        id: root_id,
    }];
    let mut nodes = Vec::new();
    while let Some(entry) = work.pop() {
        let flat = builder.flatten(&entry)?;
        let PendingBox { node, id, .. } = entry;
        let keys = OrderKey::evenly_spaced(flat.len());
        let children = flat
            .iter()
            .zip(keys)
            .map(|(child, order_key)| LayoutChild {
                id: child.id,
                order_key,
            })
            .collect();
        nodes.push(LayoutNode {
            id,
            children,
            ..node
        });
        // 첫 자식이 먼저 나오도록 역순으로 넣어 pre-order를 만듭니다.
        work.extend(flat.into_iter().rev());
    }
    let input = LayoutInput::new(root_id, nodes);
    input.validate().map_err(ViewError::Layout)?;
    Ok(LayoutTreeView {
        input,
        origin: builder.origin,
        ids: builder.ids,
        fragments: builder.fragments,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeMap;

    use super::*;
    use crate::engine::compute_layout;
    use crate::input::LayoutContent;
    use crate::intrinsic::{ComponentInstanceIntrinsic, MeasuredContent};
    use crate::test_support::{fixed, id, len, px, style};
    use stellar_core_types::value::Insets;

    /// 원천 key: scene node 또는 (인스턴스 chain, 정의 node) 가상 node.
    #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
    enum Key {
        Scene(u64),
        Virtual(Vec<u64>, u64),
    }

    /// 원천 node 정의.
    #[derive(Debug, Clone, PartialEq)]
    enum Def {
        /// 가로 Stack(gap) container.
        Row { gap: f32, children: Vec<Key> },
        /// 고정 크기 leaf.
        Leaf { width: f32, visible: bool },
        /// Fragment.
        Fragment { visible: bool, children: Vec<Key> },
        /// component instance: 정의 `definition`의 root를 virtual subtree로 가집니다.
        Instance { definition: u64 },
    }

    /// 정의 tree(정의 id → (root node id, node id → Def)). Def의 자식 key는 정의 local id(Scene)로
    /// 적고, 읽을 때 인스턴스 chain을 붙여 Virtual key로 바꿉니다.
    #[derive(Debug, Clone, PartialEq, Default)]
    struct Source {
        root: u64,
        scene: BTreeMap<u64, Def>,
        definitions: BTreeMap<u64, (u64, BTreeMap<u64, Def>)>,
        /// View Variant(예: compact): 적용 시 Row gap을 이 값으로 바꿉니다.
        variant_gap: Option<f32>,
        /// 이번 view 구성에서 읽은 node 수(원천은 바꾸지 않고 관찰만).
        reads: Cell<u32>,
    }

    impl Source {
        fn def(&self, key: &Key) -> Result<(Def, Vec<u64>), &'static str> {
            match key {
                Key::Scene(n) => self
                    .scene
                    .get(n)
                    .cloned()
                    .map(|d| (d, Vec::new()))
                    .ok_or("없는 scene node"),
                Key::Virtual(chain, n) => {
                    let owner = chain.last().ok_or("빈 chain")?;
                    let definition =
                        match self.instance_definition(*owner, &chain[..chain.len() - 1]) {
                            Some(d) => d,
                            None => return Err("인스턴스가 아님"),
                        };
                    let (_, nodes) = self.definitions.get(&definition).ok_or("없는 정의")?;
                    nodes
                        .get(n)
                        .cloned()
                        .map(|d| (d, chain.clone()))
                        .ok_or("정의에 없는 node")
                }
            }
        }

        /// `owner` 인스턴스 node(바깥 chain `outer` 안)의 정의.
        fn instance_definition(&self, owner: u64, outer: &[u64]) -> Option<u64> {
            let def = if outer.is_empty() {
                self.scene.get(&owner)?.clone()
            } else {
                let (def, _) = self.def(&Key::Virtual(outer.to_vec(), owner)).ok()?;
                def
            };
            match def {
                Def::Instance { definition } => Some(definition),
                _ => None,
            }
        }

        fn localize(chain: &[u64], children: &[Key]) -> Vec<Key> {
            children
                .iter()
                .map(|child| match (child, chain.is_empty()) {
                    (Key::Scene(n), false) => Key::Virtual(chain.to_vec(), *n),
                    _ => child.clone(),
                })
                .collect()
        }
    }

    impl LayoutSource for Source {
        type Key = Key;
        type Error = &'static str;

        fn root(&self) -> Key {
            Key::Scene(self.root)
        }

        fn node(&self, key: &Key) -> Result<SourceNode, &'static str> {
            self.reads.set(self.reads.get() + 1);
            let (def, _) = self.def(key)?;
            Ok(match def {
                Def::Row { gap, .. } => {
                    let gap = self.variant_gap.unwrap_or(gap);
                    boxed(LayoutNode::new(id(0)).with_style(style(|s| {
                        s.display = Display::Flex;
                        s.column_gap = len(gap);
                    })))
                }
                Def::Leaf { width, visible } => {
                    boxed(LayoutNode::new(id(0)).with_style(style(|s| {
                        *s = fixed(width, 10.0);
                        if !visible {
                            s.display = Display::None;
                        }
                    })))
                }
                Def::Fragment { visible, .. } => SourceNode::Fragment { visible },
                Def::Instance { .. } => boxed(
                    LayoutNode::new(id(0))
                        .with_style(style(|s| {
                            s.padding = Insets::new(1.0, 1.0, 1.0, 1.0).expect("padding");
                        }))
                        .with_content(LayoutContent::Measured(MeasuredContent::ComponentInstance(
                            ComponentInstanceIntrinsic,
                        ))),
                ),
            })
        }

        fn children(&self, key: &Key) -> Result<Vec<Key>, &'static str> {
            let (def, chain) = self.def(key)?;
            Ok(match def {
                Def::Row { children, .. } | Def::Fragment { children, .. } => {
                    Source::localize(&chain, &children)
                }
                Def::Leaf { .. } => Vec::new(),
                Def::Instance { definition } => {
                    let (root, _) = self.definitions.get(&definition).ok_or("없는 정의")?;
                    let owner = match key {
                        Key::Scene(n) | Key::Virtual(_, n) => *n,
                    };
                    let mut inner = chain;
                    inner.push(owner);
                    vec![Key::Virtual(inner, *root)]
                }
            })
        }

        fn layout_id(&self, key: &Key) -> StableId {
            match key {
                Key::Scene(n) => id(*n),
                Key::Virtual(chain, n) => {
                    let chain: Vec<StableId> = chain.iter().map(|c| id(*c)).collect();
                    virtual_layout_id(&chain, id(*n))
                }
            }
        }
    }

    fn boxed(node: LayoutNode) -> SourceNode {
        SourceNode::Node(Box::new(node))
    }

    fn leaf(width: f32) -> Def {
        Def::Leaf {
            width,
            visible: true,
        }
    }

    fn scene(n: u64) -> Key {
        Key::Scene(n)
    }

    /// root 1 = Row[2, F10[3, F11[4, 5], 6], 7].
    fn fragment_source() -> Source {
        let mut source = Source {
            root: 1,
            ..Source::default()
        };
        source.scene.insert(
            1,
            Def::Row {
                gap: 2.0,
                children: vec![scene(2), scene(10), scene(7)],
            },
        );
        source.scene.insert(
            10,
            Def::Fragment {
                visible: true,
                children: vec![scene(3), scene(11), scene(6)],
            },
        );
        source.scene.insert(
            11,
            Def::Fragment {
                visible: true,
                children: vec![scene(4), scene(5)],
            },
        );
        for (n, width) in [
            (2, 10.0),
            (3, 11.0),
            (4, 12.0),
            (5, 13.0),
            (6, 14.0),
            (7, 15.0),
        ] {
            source.scene.insert(n, leaf(width));
        }
        source
    }

    /// 원천 tree를 재귀로 펼친 기대 visual order(테스트 기준값, 구현과 독립).
    fn expected_flat(source: &Source, key: &Key, out: &mut Vec<Key>) {
        for child in source.children(key).unwrap() {
            match source.node(&child).unwrap() {
                SourceNode::Fragment { .. } => expected_flat(source, &child, out),
                SourceNode::Node(_) => out.push(child),
            }
        }
    }

    /// view 입력에서 node의 자식을 visual order(OrderKey 순)로 읽습니다.
    fn visual_children(view: &LayoutTreeView<Key>, parent: StableId) -> Vec<Key> {
        let node = view.input().node(parent).unwrap();
        let mut children = node.children.clone();
        children.sort_by(|a, b| a.order_key.cmp(&b.order_key));
        children
            .iter()
            .map(|c| view.source_key(c.id).unwrap().clone())
            .collect()
    }

    #[test]
    fn flatten_preserves_visual_child_order_fixture() {
        let source = fragment_source();
        let view = build_layout_tree(&source).unwrap();
        // flatten 전: 원천 tree의 visual order(Fragment 자리에서 깊이 우선).
        let mut before = Vec::new();
        expected_flat(&source, &scene(1), &mut before);
        assert_eq!(
            before,
            vec![scene(2), scene(3), scene(4), scene(5), scene(6), scene(7)]
        );
        // flatten 후: layout 입력 root 자식의 visual order가 같습니다.
        assert_eq!(visual_children(&view, id(1)), before);
        // Fragment는 box가 없고, 자식은 root가 받습니다.
        assert_eq!(view.len(), 7);
        assert_eq!(view.layout_id(&scene(10)), None);
        assert_eq!(view.fragment_host(&scene(10)), Some(id(1)));
        assert_eq!(view.fragment_host(&scene(11)), Some(id(1)));
        assert_eq!(view.fragment_host(&scene(2)), None);
        // 레이아웃 결과: 가로 Stack에서 x가 visual order로 증가(gap 2).
        let result = compute_layout(view.input()).unwrap();
        let xs: Vec<i32> = before
            .iter()
            .map(|key| {
                let id = view.layout_id(key).unwrap();
                result.get(id).unwrap().local_rect().x().raw() / 64
            })
            .collect();
        assert_eq!(xs, vec![0, 12, 25, 39, 54, 70]);
        assert_eq!(result.len(), 7);
        // pre-order 입력, 결정적 구성.
        assert_eq!(view.input().nodes[0].id, id(1));
        assert_eq!(
            build_layout_tree(&source).unwrap().input().encode(),
            view.input().encode()
        );
    }

    #[test]
    fn hidden_fragment_hides_its_flattened_children() {
        let mut source = fragment_source();
        source.scene.insert(
            11,
            Def::Fragment {
                visible: false,
                children: vec![scene(4), scene(5)],
            },
        );
        let view = build_layout_tree(&source).unwrap();
        let visible = |n: u64| view.input().node(id(n)).unwrap().style.display != Display::None;
        assert!(!visible(4) && !visible(5));
        assert!(visible(3) && visible(6));
        // 숨긴 자식은 flow에서 빠져 형제가 당겨집니다.
        let result = compute_layout(view.input()).unwrap();
        assert!(result.get(id(4)).is_none());
        assert_eq!(result.get(id(6)).unwrap().local_rect().x(), px(25));
        // 자식 자신이 숨김이면 Fragment가 보여도 숨김입니다.
        let mut source = fragment_source();
        source.scene.insert(
            3,
            Def::Leaf {
                width: 11.0,
                visible: false,
            },
        );
        let view = build_layout_tree(&source).unwrap();
        assert_eq!(
            view.input().node(id(3)).unwrap().style.display,
            Display::None
        );
    }

    /// root 1 = Row[I20, I21](두 인스턴스) — 정의 100 = Row[200 leaf, 201 nested instance(정의 101)],
    /// 정의 101 = Row[300 leaf, F301[302 leaf]].
    fn component_source() -> Source {
        let mut source = Source {
            root: 1,
            ..Source::default()
        };
        source.scene.insert(
            1,
            Def::Row {
                gap: 4.0,
                children: vec![scene(20), scene(21)],
            },
        );
        source.scene.insert(20, Def::Instance { definition: 100 });
        source.scene.insert(21, Def::Instance { definition: 100 });
        let mut outer = BTreeMap::new();
        outer.insert(
            150,
            Def::Row {
                gap: 3.0,
                children: vec![scene(200), scene(201)],
            },
        );
        outer.insert(200, leaf(8.0));
        outer.insert(201, Def::Instance { definition: 101 });
        source.definitions.insert(100, (150, outer));
        let mut inner = BTreeMap::new();
        inner.insert(
            160,
            Def::Row {
                gap: 1.0,
                children: vec![scene(300), scene(301)],
            },
        );
        inner.insert(300, leaf(5.0));
        inner.insert(
            301,
            Def::Fragment {
                visible: true,
                children: vec![scene(302)],
            },
        );
        inner.insert(302, leaf(6.0));
        source.definitions.insert(101, (160, inner));
        source
    }

    #[test]
    fn component_instance_reads_virtual_subtree_without_duplicating_definitions() {
        let source = component_source();
        let snapshot = source.clone();
        let view = build_layout_tree(&source).unwrap();
        // 원천은 읽기만 했습니다(읽기 횟수 외 상태 동일).
        assert_eq!(source.scene, snapshot.scene);
        assert_eq!(source.definitions, snapshot.definitions);
        assert!(source.reads.get() > 0);
        // 정의 node는 인스턴스마다 다른 가상 layout id로 투영됩니다.
        let first = Key::Virtual(vec![20], 200);
        let second = Key::Virtual(vec![21], 200);
        let a = view.layout_id(&first).unwrap();
        let b = view.layout_id(&second).unwrap();
        assert_ne!(a, b);
        assert_eq!(view.source_key(a), Some(&first));
        assert_eq!(a.version(), 8);
        assert_eq!(a, virtual_layout_id(&[id(20)], id(200)));
        // nested instance의 정의 안 Fragment도 펼쳐집니다(chain 20 → 201).
        let nested_row = view.layout_id(&Key::Virtual(vec![20, 201], 160)).unwrap();
        assert_eq!(
            visual_children(&view, nested_row),
            vec![
                Key::Virtual(vec![20, 201], 300),
                Key::Virtual(vec![20, 201], 302)
            ]
        );
        // box 수: root + 인스턴스 2 × (instance, 정의 root, leaf, nested instance, 정의 root, leaf 2).
        assert_eq!(view.len(), 1 + 2 * 7);
        // 두 인스턴스의 subtree 결과는 같은 크기입니다.
        let result = compute_layout(view.input()).unwrap();
        let size = |key: Key| {
            let rect = result
                .get(view.layout_id(&key).unwrap())
                .unwrap()
                .local_rect();
            (rect.width(), rect.height())
        };
        assert_eq!(size(scene(20)), size(scene(21)));
        // instance 20: padding 1 + Row(8 + 3 + nested(1 + Row(5 + 1 + 6) + 1)) + 1.
        assert_eq!(size(scene(20)), (px(1 + 8 + 3 + 14 + 1), px(1 + 12 + 1)));
    }

    #[test]
    fn view_variant_values_are_read_as_resolved_by_the_source() {
        let mut source = fragment_source();
        let base = compute_layout(build_layout_tree(&source).unwrap().input()).unwrap();
        // compact variant: 원천이 gap을 0으로 해석해 돌려줍니다. view는 해석된 값을 그대로 씁니다.
        source.variant_gap = Some(0.0);
        let view = build_layout_tree(&source).unwrap();
        let compact = compute_layout(view.input()).unwrap();
        let x = |result: &crate::output::LayoutResult, n: u64| {
            result.get(id(n)).unwrap().local_rect().x()
        };
        assert_eq!(x(&base, 7), px(70));
        assert_eq!(x(&compact, 7), px(60));
        // 순서는 variant와 무관합니다.
        assert_eq!(
            visual_children(&view, id(1)),
            vec![scene(2), scene(3), scene(4), scene(5), scene(6), scene(7)]
        );
    }

    #[test]
    fn invalid_sources_are_rejected() {
        // root가 Fragment.
        let mut source = fragment_source();
        source.root = 10;
        assert_eq!(
            build_layout_tree(&source).unwrap_err(),
            ViewError::FragmentRoot
        );
        // 같은 node를 두 부모가 공유.
        let mut source = fragment_source();
        source.scene.insert(
            11,
            Def::Fragment {
                visible: true,
                children: vec![scene(4), scene(2)],
            },
        );
        assert_eq!(
            build_layout_tree(&source).unwrap_err(),
            ViewError::RepeatedSourceNode { key: scene(2) }
        );
        // 순환(Fragment가 자기 조상을 자식으로).
        let mut source = fragment_source();
        source.scene.insert(
            11,
            Def::Fragment {
                visible: true,
                children: vec![scene(10)],
            },
        );
        assert_eq!(
            build_layout_tree(&source).unwrap_err(),
            ViewError::RepeatedSourceNode { key: scene(10) }
        );
        // 원천 오류 전파.
        let mut source = fragment_source();
        source.scene.remove(&5);
        assert_eq!(
            build_layout_tree(&source).unwrap_err(),
            ViewError::Source("없는 scene node")
        );
        // leaf에 자식(레이아웃 검증 실패).
        struct LeafWithChild;
        impl LayoutSource for LeafWithChild {
            type Key = u64;
            type Error = ();
            fn root(&self) -> u64 {
                1
            }
            fn node(&self, key: &u64) -> Result<SourceNode, ()> {
                Ok(boxed(if *key == 1 {
                    LayoutNode::new(id(1)).with_content(LayoutContent::Intrinsic(
                        crate::input::IntrinsicContent {
                            width: len(1.0),
                            height: len(1.0),
                            baseline: None,
                        },
                    ))
                } else {
                    LayoutNode::new(id(2))
                }))
            }
            fn children(&self, key: &u64) -> Result<Vec<u64>, ()> {
                Ok(if *key == 1 { vec![2] } else { Vec::new() })
            }
            fn layout_id(&self, key: &u64) -> StableId {
                id(*key)
            }
        }
        assert_eq!(
            build_layout_tree(&LeafWithChild).unwrap_err(),
            ViewError::Layout(LayoutError::ChildrenOnContentLeaf { node: id(1) })
        );
        // 두 key가 같은 layout id.
        struct SameId;
        impl LayoutSource for SameId {
            type Key = u64;
            type Error = ();
            fn root(&self) -> u64 {
                1
            }
            fn node(&self, _: &u64) -> Result<SourceNode, ()> {
                Ok(boxed(LayoutNode::new(id(0))))
            }
            fn children(&self, key: &u64) -> Result<Vec<u64>, ()> {
                Ok(if *key == 1 { vec![2] } else { Vec::new() })
            }
            fn layout_id(&self, _: &u64) -> StableId {
                id(7)
            }
        }
        assert_eq!(
            build_layout_tree(&SameId).unwrap_err(),
            ViewError::DuplicateLayoutId { id: id(7) }
        );
        assert!(ViewError::<u64, ()>::FragmentRoot
            .to_string()
            .contains("Fragment"));
        // 자식 목록이 node 상한을 넘으면 방문 전에 거부합니다.
        struct Wide;
        impl LayoutSource for Wide {
            type Key = u64;
            type Error = ();
            fn root(&self) -> u64 {
                0
            }
            fn node(&self, _: &u64) -> Result<SourceNode, ()> {
                Ok(boxed(LayoutNode::new(id(0))))
            }
            fn children(&self, key: &u64) -> Result<Vec<u64>, ()> {
                Ok(if *key == 0 {
                    (1..=MAX_VIEW_NODES as u64).collect()
                } else {
                    Vec::new()
                })
            }
            fn layout_id(&self, key: &u64) -> StableId {
                id(*key)
            }
        }
        assert_eq!(
            build_layout_tree(&Wide).unwrap_err(),
            ViewError::NodeLimitExceeded {
                limit: MAX_VIEW_NODES
            }
        );
    }

    #[test]
    fn virtual_ids_are_deterministic_and_chain_sensitive() {
        let a = virtual_layout_id(&[id(1), id(2)], id(3));
        assert_eq!(a, virtual_layout_id(&[id(1), id(2)], id(3)));
        assert_ne!(a, virtual_layout_id(&[id(2), id(1)], id(3)));
        assert_ne!(a, virtual_layout_id(&[id(1)], id(3)));
        assert_ne!(a, virtual_layout_id(&[id(1), id(2)], id(4)));
        assert_eq!((a.version(), a.variant()), (8, 0b10));
    }
}
