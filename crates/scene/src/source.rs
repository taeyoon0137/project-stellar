//! 증분 scene build 입력 tree (XI.2.4, XI.2.5, ADR-0084 결정 3·5).
//!
//! document 계층이 binding·modifier·layout 평가를 끝낸 node 값을 [`SceneSource`]에 둡니다. node는
//! [`SceneKey`]로 식별하고 종류별 builder 입력([`SourceKind`])과 자식 목록(paint 순서)을 가집니다.
//!
//! # 식별
//!
//! - 문서 node는 자기 StableId가 key입니다([`SceneKey::node`]).
//! - ComponentInstance의 가상 subtree node(IX.2.2)는 문서에 복제하지 않습니다. key는 (인스턴스 chain, 정의 node)에서
//!   결정적으로 만든 값입니다([`SceneKey::virtual_node`]). 가상 node가 만드는 scene node의 `source`는 가장 바깥
//!   인스턴스의 StableId입니다(입력 [`NodeInput::source`]로 호출자가 정함). 그래서 hit test는 인스턴스를 고릅니다.
//!
//! # Boolean
//!
//! Boolean modifier를 가진 Shape·Path는 [`SourceNode::boolean`]에 연산과 operand geometry를 둡니다(ADR-0084 결정 7).
//! operand node는 이 tree에 넣지 않습니다(결과에 흡수되어 단독 표시하지 않음).

use std::collections::{HashMap, HashSet};
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::boolean::{BooleanOp, BooleanOperand};
use crate::builder::{
    FragmentInput, FrameInput, ImageInput, NodeInput, PathInput, ShapeInput, TextInput,
};
use crate::codec::ByteSink;
use crate::snapshot::Fnv1a128;

/// source tree node 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SceneKey(StableId);

impl SceneKey {
    /// 문서 node의 key.
    pub const fn node(id: StableId) -> SceneKey {
        SceneKey(id)
    }

    /// 인스턴스 chain(바깥 → 안)과 정의 node로 만든 가상 node key(결정적).
    ///
    /// FNV-1a 128을 StableId 16 bytes로 쓰고 version nibble을 8(custom), variant를 RFC 9562로 둡니다. 문서 node
    /// StableId(version 7)와 겹치지 않습니다.
    pub fn virtual_node(instances: &[StableId], definition: StableId) -> SceneKey {
        let mut hash = Fnv1a128::new();
        hash.put(b"stellar.scene.virtual-node.v1\0");
        hash.put(&(instances.len() as u64).to_le_bytes());
        for instance in instances {
            hash.put(&instance.to_bytes());
        }
        hash.put(&definition.to_bytes());
        let mut bytes = hash.finish().to_be_bytes();
        bytes[6] = (bytes[6] & 0x0F) | 0x80;
        bytes[8] = (bytes[8] & 0x3F) | 0x80;
        SceneKey(StableId::from_bytes(bytes))
    }

    /// key 값.
    pub const fn id(&self) -> StableId {
        self.0
    }
}

/// node 종류별 builder 입력.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceKind {
    /// Frame(자식 scope를 엶).
    Frame(FrameInput),
    /// Shape.
    Shape(ShapeInput),
    /// Path.
    Path(PathInput),
    /// Text.
    Text(TextInput),
    /// Image.
    Image(ImageInput),
    /// Fragment(scene node 없이 자식을 부모 scope에 펼침).
    Fragment(FragmentInput),
}

impl SourceKind {
    /// 공통 입력(Fragment는 없음).
    pub fn node_input(&self) -> Option<&NodeInput> {
        match self {
            SourceKind::Frame(input) => Some(&input.node),
            SourceKind::Shape(input) => Some(&input.node),
            SourceKind::Path(input) => Some(&input.node),
            SourceKind::Text(input) => Some(&input.node),
            SourceKind::Image(input) => Some(&input.node),
            SourceKind::Fragment(_) => None,
        }
    }

    /// 자식을 가질 수 있는 종류인지(Frame·Fragment).
    pub const fn allows_children(&self) -> bool {
        matches!(self, SourceKind::Frame(_) | SourceKind::Fragment(_))
    }
}

/// Boolean 결과 입력. 결과 operand 목록은 host geometry 뒤에 [`BooleanSource::operands`]가 이어집니다.
#[derive(Debug, Clone, PartialEq)]
pub struct BooleanSource {
    /// 연산.
    pub op: BooleanOp,
    /// host 다음 operand(문서 순서, host local 좌표 transform 포함).
    pub operands: Vec<BooleanOperand>,
}

/// source tree node.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceNode {
    /// 종류별 입력.
    pub kind: SourceKind,
    /// 자식(paint 순서).
    pub children: Vec<SceneKey>,
    /// Boolean 결과(Shape·Path만).
    pub boolean: Option<BooleanSource>,
}

impl SourceNode {
    /// 자식·Boolean 없는 node.
    pub fn leaf(kind: SourceKind) -> SourceNode {
        SourceNode {
            kind,
            children: Vec::new(),
            boolean: None,
        }
    }

    /// 자식이 있는 node.
    pub fn with_children(kind: SourceKind, children: Vec<SceneKey>) -> SourceNode {
        SourceNode {
            kind,
            children,
            boolean: None,
        }
    }
}

/// source tree 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    /// 자식이나 root가 tree에 없습니다.
    MissingNode {
        /// 없는 key.
        key: SceneKey,
    },
    /// node가 부모 둘 이상(또는 root이면서 자식)에 있습니다.
    MultipleParents {
        /// key.
        key: SceneKey,
    },
    /// 자식을 가질 수 없는 종류가 자식을 가집니다.
    ChildrenNotAllowed {
        /// key.
        key: SceneKey,
    },
    /// Boolean을 Shape·Path가 아닌 종류에 두었습니다.
    BooleanNotAllowed {
        /// key.
        key: SceneKey,
    },
    /// 부모 chain이 순환합니다.
    Cycle {
        /// 순환에 든 key.
        key: SceneKey,
    },
    /// root에서 닿지 않는 node가 있습니다.
    Unreachable {
        /// key.
        key: SceneKey,
    },
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::MissingNode { key } => {
                write!(f, "scene source에 {} node가 없습니다", key.0)
            }
            SourceError::MultipleParents { key } => {
                write!(f, "scene source node {}의 부모가 둘 이상입니다", key.0)
            }
            SourceError::ChildrenNotAllowed { key } => {
                write!(f, "scene source node {}는 자식을 가질 수 없습니다", key.0)
            }
            SourceError::BooleanNotAllowed { key } => {
                write!(
                    f,
                    "scene source node {}는 Boolean 결과를 가질 수 없습니다",
                    key.0
                )
            }
            SourceError::Cycle { key } => write!(f, "scene source node {}가 순환합니다", key.0),
            SourceError::Unreachable { key } => {
                write!(f, "scene source node {}가 root에서 닿지 않습니다", key.0)
            }
        }
    }
}

impl std::error::Error for SourceError {}

/// source tree 편집.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceEdit {
    /// node를 넣거나 바꿉니다.
    Upsert(SceneKey, Box<SourceNode>),
    /// node를 지웁니다(부모의 자식 목록은 부모 Upsert로 함께 바꿉니다).
    Remove(SceneKey),
    /// 최상위 node 목록(paint 순서)을 바꿉니다.
    SetRoots(Vec<SceneKey>),
}

impl SourceEdit {
    /// node를 넣거나 바꾸는 편집.
    pub fn upsert(key: SceneKey, node: SourceNode) -> SourceEdit {
        SourceEdit::Upsert(key, Box::new(node))
    }

    /// 편집이 바꾸는 key(SetRoots는 없음).
    pub const fn key(&self) -> Option<SceneKey> {
        match self {
            SourceEdit::Upsert(key, _) | SourceEdit::Remove(key) => Some(*key),
            SourceEdit::SetRoots(_) => None,
        }
    }
}

/// 증분 scene build 입력 tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneSource {
    nodes: HashMap<SceneKey, SourceNode>,
    roots: Vec<SceneKey>,
    parents: HashMap<SceneKey, SceneKey>,
}

impl SceneSource {
    /// 빈 tree.
    pub fn new() -> SceneSource {
        SceneSource::default()
    }

    /// root 목록과 node로 tree를 만들고 검사합니다.
    pub fn from_nodes(
        roots: Vec<SceneKey>,
        nodes: impl IntoIterator<Item = (SceneKey, SourceNode)>,
    ) -> Result<SceneSource, SourceError> {
        let mut source = SceneSource::new();
        for (key, node) in nodes {
            source.upsert(key, node);
        }
        source.rebuild_parents();
        source.roots = roots;
        source.validate()?;
        Ok(source)
    }

    fn upsert(&mut self, key: SceneKey, node: SourceNode) -> Vec<(SceneKey, SceneKey)> {
        if let Some(old) = self.nodes.get(&key) {
            for child in &old.children {
                if self.parents.get(child) == Some(&key) {
                    self.parents.remove(child);
                }
            }
        }
        let mut displaced = Vec::new();
        for child in &node.children {
            if let Some(previous) = self.parents.insert(*child, key) {
                if previous != key {
                    displaced.push((*child, previous));
                }
            }
        }
        self.nodes.insert(key, node);
        displaced
    }

    fn remove(&mut self, key: SceneKey) {
        if let Some(old) = self.nodes.remove(&key) {
            for child in &old.children {
                if self.parents.get(child) == Some(&key) {
                    self.parents.remove(child);
                }
            }
        }
        self.parents.remove(&key);
    }

    /// 편집을 차례로 적용하고 바뀐 부분의 불변식을 검사합니다. 실패하면 tree는 편집 전 그대로입니다.
    ///
    /// 검사 비용은 편집한 node·그 자식·조상 chain에 비례합니다(전체 tree를 다시 걷지 않음).
    pub fn apply(&mut self, edits: &[SourceEdit]) -> Result<(), SourceError> {
        let mut undo: HashMap<SceneKey, Option<SourceNode>> = HashMap::new();
        let old_roots = self.roots.clone();
        let mut roots_changed = false;
        let mut displaced = Vec::new();
        // 지운 node와 지우기 전 부모. 부모가 아직 그 node를 나열하면 끊긴 참조입니다.
        let mut removed: Vec<(SceneKey, Option<SceneKey>)> = Vec::new();
        for edit in edits {
            if let Some(key) = edit.key() {
                undo.entry(key)
                    .or_insert_with(|| self.nodes.get(&key).cloned());
            }
            match edit {
                SourceEdit::Upsert(key, node) => {
                    displaced.extend(self.upsert(*key, (**node).clone()));
                }
                SourceEdit::Remove(key) => {
                    removed.push((*key, self.parents.get(key).copied()));
                    self.remove(*key);
                }
                SourceEdit::SetRoots(roots) => {
                    self.roots = roots.clone();
                    roots_changed = true;
                }
            }
        }
        let checked = self
            .check_removed(&removed)
            .and_then(|()| self.check_edits(&undo, &displaced, &old_roots, roots_changed));
        if let Err(error) = checked {
            // 되돌리기: 편집 전 node와 root를 복원하고 부모 index를 다시 맞춥니다.
            for (key, previous) in undo {
                match previous {
                    Some(node) => {
                        self.upsert(key, node);
                    }
                    None => self.remove(key),
                }
            }
            self.roots = old_roots;
            self.rebuild_parents();
            return Err(error);
        }
        Ok(())
    }

    fn rebuild_parents(&mut self) {
        self.parents.clear();
        for (key, node) in &self.nodes {
            for child in &node.children {
                self.parents.insert(*child, *key);
            }
        }
    }

    /// 지운 node를 부모나 root 목록이 아직 가리키는지 검사합니다.
    fn check_removed(&self, removed: &[(SceneKey, Option<SceneKey>)]) -> Result<(), SourceError> {
        for (key, parent) in removed {
            if self.nodes.contains_key(key) {
                continue;
            }
            let listed_by_parent = parent
                .and_then(|parent| self.nodes.get(&parent))
                .is_some_and(|parent| parent.children.contains(key));
            if listed_by_parent || self.roots.contains(key) {
                return Err(SourceError::MissingNode { key: *key });
            }
        }
        Ok(())
    }

    /// 편집한 부분의 불변식 검사.
    fn check_edits(
        &self,
        undo: &HashMap<SceneKey, Option<SourceNode>>,
        displaced: &[(SceneKey, SceneKey)],
        old_roots: &[SceneKey],
        roots_changed: bool,
    ) -> Result<(), SourceError> {
        // 다른 node가 가져간 자식을 이전 부모가 아직 나열하면 부모가 둘입니다.
        for (child, previous) in displaced {
            if self.parents.get(child) != Some(previous)
                && self
                    .nodes
                    .get(previous)
                    .is_some_and(|p| p.children.contains(child))
            {
                return Err(SourceError::MultipleParents { key: *child });
            }
        }
        let roots: HashSet<SceneKey> = self.roots.iter().copied().collect();
        if roots.len() != self.roots.len() {
            let mut seen = HashSet::new();
            let key = self
                .roots
                .iter()
                .find(|key| !seen.insert(**key))
                .copied()
                .unwrap_or(self.roots[0]);
            return Err(SourceError::MultipleParents { key });
        }
        // 부모를 잃었을 수 있는 node.
        let mut orphans: Vec<SceneKey> = Vec::new();
        for (key, previous) in undo {
            if let Some(previous) = previous {
                orphans.extend(previous.children.iter().copied());
            }
            let Some(node) = self.nodes.get(key) else {
                continue;
            };
            if !node.children.is_empty() && !node.kind.allows_children() {
                return Err(SourceError::ChildrenNotAllowed { key: *key });
            }
            if node.boolean.is_some()
                && !matches!(node.kind, SourceKind::Shape(_) | SourceKind::Path(_))
            {
                return Err(SourceError::BooleanNotAllowed { key: *key });
            }
            let mut listed = HashSet::new();
            for child in &node.children {
                if !self.nodes.contains_key(child) {
                    return Err(SourceError::MissingNode { key: *child });
                }
                if !listed.insert(*child) || roots.contains(child) {
                    return Err(SourceError::MultipleParents { key: *child });
                }
                if self.parents.get(child) != Some(key) {
                    return Err(SourceError::MultipleParents { key: *child });
                }
            }
            // 새로 넣은 node는 부모가 있거나 root여야 합니다.
            if !roots.contains(key) && !self.parents.contains_key(key) {
                return Err(SourceError::Unreachable { key: *key });
            }
            self.check_no_cycle(*key)?;
        }
        if roots_changed {
            orphans.extend(old_roots.iter().copied());
            for root in &self.roots {
                if !self.nodes.contains_key(root) {
                    return Err(SourceError::MissingNode { key: *root });
                }
                if self.parents.contains_key(root) {
                    return Err(SourceError::MultipleParents { key: *root });
                }
            }
        }
        for key in orphans {
            if self.nodes.contains_key(&key)
                && !roots.contains(&key)
                && !self.parents.contains_key(&key)
            {
                return Err(SourceError::Unreachable { key });
            }
        }
        Ok(())
    }

    /// `key`에서 부모를 따라 올라가 root에 닿는지(순환 없음) 확인합니다.
    fn check_no_cycle(&self, key: SceneKey) -> Result<(), SourceError> {
        let mut current = key;
        for _ in 0..=self.nodes.len() {
            match self.parents.get(&current) {
                None => return Ok(()),
                Some(parent) if *parent == key => return Err(SourceError::Cycle { key }),
                Some(parent) => current = *parent,
            }
        }
        Err(SourceError::Cycle { key })
    }

    /// tree 불변식: 모든 자식·root가 있고, 부모가 하나이며, 순환이 없고, 모든 node가 root에서 닿습니다.
    pub fn validate(&self) -> Result<(), SourceError> {
        let mut seen: HashSet<SceneKey> = HashSet::with_capacity(self.nodes.len());
        let mut stack: Vec<SceneKey> = Vec::new();
        for root in self.roots.iter().rev() {
            if !self.nodes.contains_key(root) {
                return Err(SourceError::MissingNode { key: *root });
            }
            stack.push(*root);
        }
        while let Some(key) = stack.pop() {
            if !seen.insert(key) {
                return Err(SourceError::MultipleParents { key });
            }
            let node = &self.nodes[&key];
            if !node.children.is_empty() && !node.kind.allows_children() {
                return Err(SourceError::ChildrenNotAllowed { key });
            }
            if node.boolean.is_some()
                && !matches!(node.kind, SourceKind::Shape(_) | SourceKind::Path(_))
            {
                return Err(SourceError::BooleanNotAllowed { key });
            }
            for child in node.children.iter().rev() {
                if !self.nodes.contains_key(child) {
                    return Err(SourceError::MissingNode { key: *child });
                }
                if seen.contains(child) {
                    return Err(SourceError::Cycle { key: *child });
                }
                stack.push(*child);
            }
        }
        if let Some(key) = self.nodes.keys().find(|key| !seen.contains(key)) {
            return Err(SourceError::Unreachable { key: *key });
        }
        Ok(())
    }

    /// node.
    pub fn get(&self, key: SceneKey) -> Option<&SourceNode> {
        self.nodes.get(&key)
    }

    /// 최상위 node(paint 순서).
    pub fn roots(&self) -> &[SceneKey] {
        &self.roots
    }

    /// 부모(root면 `None`).
    pub fn parent(&self, key: SceneKey) -> Option<SceneKey> {
        self.parents.get(&key).copied()
    }

    /// node 수(Fragment 포함).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 모든 key(순서 없음).
    pub fn keys(&self) -> impl Iterator<Item = SceneKey> + '_ {
        self.nodes.keys().copied()
    }
}
