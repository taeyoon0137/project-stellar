//! Target adapter용 중립 접근성 snapshot (IX.8.6).
//!
//! [`AccessibilitySnapshot`]은 Target Adapter(Apple·Windows·Web·Android, 기술 아키텍처 §18)가 플랫폼 접근성 구조로
//! 변환할 때 읽는 **플랫폼 중립** 표현입니다. 플랫폼 API 호출은 이 crate가 하지 않습니다(Phase XII).
//!
//! - **읽기 순서 적용**: 형제마다 명시 읽기 순서가 있는 node를 순서 값 오름차순(같은 값은 트리 순서)으로 먼저,
//!   나머지를 트리 순서로 둡니다(tabindex 규칙과 같음). snapshot의 node는 이미 이 순서이므로 읽기 순서 field는
//!   비웁니다.
//! - **preorder 평탄화**: entry는 root부터 preorder이고 부모 entry index를 가집니다. adapter는 entry를 앞에서부터
//!   한 번 읽어 플랫폼 tree를 만들 수 있습니다.
//! - **결정론**: 정수는 고정 폭 little-endian, 문자열은 UTF-8, map은 tag 오름차순입니다. `usize`·pointer·hash
//!   순서·부동소수점 연산을 쓰지 않으므로 같은 tree는 어느 플랫폼에서나 같은 bytes를 만듭니다(ADR-0006 4번:
//!   정수 연산만으로 정해지는 결과).
//!
//! binary 형식:
//!
//! ```text
//! snapshot := magic "STAX" ++ version:u16 (= 1) ++ count:u32 ++ entry*
//! entry    := source[16] ++ opt_u32(parent index) ++ semantics
//! semantics:= role:u8 ++ opt(label) ++ opt(value) ++ states ++ actions   (IX.8.1 node 본문에서 source·읽기 순서 제외)
//! ```
//!
//! 디코더는 magic·버전·잘림·잉여 bytes·중복 source를 거부하고, 부모 index가 preorder 경로(현재 조상) 위에
//! 있는지 검사하며, 재인코딩 bytes가 입력과 같을 때만 받아들입니다.

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::codec::{put_count, put_id, put_opt_u32, put_u16, CodecError, Reader};
use crate::node::AccessibilityNode;
use crate::tree::{AccessibilityTree, TreeIssue};

/// snapshot magic.
pub const SNAPSHOT_MAGIC: [u8; 4] = *b"STAX";

/// snapshot binary 형식 버전.
pub const SNAPSHOT_FORMAT_VERSION: u16 = 1;

/// snapshot entry 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEntry {
    /// 부모 entry index(root면 `None`). 항상 자신의 index보다 작습니다.
    pub parent: Option<u32>,
    /// root부터의 깊이(root = 0).
    pub depth: u32,
    /// 의미 node(읽기 순서는 이미 entry 순서에 반영되어 `None`).
    pub node: AccessibilityNode,
}

/// snapshot 생성 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// tree 구조가 forest가 아닙니다(IX.8.2 검사 결과).
    InvalidTree {
        /// 구조 문제.
        issues: Vec<TreeIssue>,
    },
    /// entry 수가 `u32` index 범위를 넘습니다.
    TooManyEntries,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::InvalidTree { issues } => {
                write!(
                    f,
                    "접근성 tree 구조 문제 {}건으로 snapshot을 만들 수 없습니다",
                    issues.len()
                )
            }
            SnapshotError::TooManyEntries => f.write_str("snapshot entry 수가 u32 범위를 넘습니다"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// 플랫폼 중립 접근성 snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessibilitySnapshot {
    entries: Vec<SnapshotEntry>,
}

/// 형제를 읽기 순서로 정렬합니다: 명시 순서(값 오름차순, 같은 값은 트리 순서) 다음 나머지(트리 순서).
fn in_reading_order(tree: &AccessibilityTree, siblings: &[StableId]) -> Vec<StableId> {
    let mut explicit: Vec<(u32, usize, StableId)> = Vec::new();
    let mut implicit: Vec<StableId> = Vec::new();
    for (position, sibling) in siblings.iter().enumerate() {
        match tree.node(*sibling).and_then(|node| node.reading_order) {
            Some(order) => explicit.push((order, position, *sibling)),
            None => implicit.push(*sibling),
        }
    }
    explicit.sort_unstable();
    explicit
        .into_iter()
        .map(|(_, _, sibling)| sibling)
        .chain(implicit)
        .collect()
}

impl AccessibilitySnapshot {
    /// 구조가 유효한 tree에서 snapshot을 만듭니다(source 검사는 호출자가 `validate`로 먼저 합니다).
    pub fn from_tree(tree: &AccessibilityTree) -> Result<AccessibilitySnapshot, SnapshotError> {
        let issues = tree.validate_structure();
        if !issues.is_empty() {
            return Err(SnapshotError::InvalidTree { issues });
        }
        let mut entries = Vec::with_capacity(tree.len());
        // (node, 부모 entry index, 깊이). 구조가 forest이므로 각 node를 한 번만 방문합니다.
        let mut stack: Vec<(StableId, Option<u32>, u32)> = in_reading_order(tree, tree.roots())
            .into_iter()
            .rev()
            .map(|root| (root, None, 0))
            .collect();
        while let Some((source, parent, depth)) = stack.pop() {
            let Some(node) = tree.node(source) else {
                continue;
            };
            let index = u32::try_from(entries.len()).map_err(|_| SnapshotError::TooManyEntries)?;
            let mut node = node.clone();
            node.reading_order = None;
            entries.push(SnapshotEntry {
                parent,
                depth,
                node,
            });
            let children = in_reading_order(tree, tree.children(source));
            stack.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| (child, Some(index), depth.saturating_add(1))),
            );
        }
        Ok(AccessibilitySnapshot { entries })
    }

    /// entry 목록(preorder, 읽기 순서 적용).
    pub fn entries(&self) -> &[SnapshotEntry] {
        &self.entries
    }

    /// entry 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `index` entry의 자식 entry index(읽기 순서). preorder이므로 뒤따르는 더 깊은 entry만 훑습니다.
    pub fn children(&self, index: usize) -> Vec<usize> {
        let Some(entry) = self.entries.get(index) else {
            return Vec::new();
        };
        let mut children = Vec::new();
        for (offset, candidate) in self.entries[index + 1..].iter().enumerate() {
            if candidate.depth <= entry.depth {
                break;
            }
            if candidate.parent.map(|p| p as usize) == Some(index) {
                children.push(index + 1 + offset);
            }
        }
        children
    }

    /// canonical bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&SNAPSHOT_MAGIC);
        put_u16(&mut buf, SNAPSHOT_FORMAT_VERSION);
        put_count(&mut buf, self.entries.len());
        for entry in &self.entries {
            put_id(&mut buf, entry.node.source);
            put_opt_u32(&mut buf, entry.parent);
            entry.node.encode_semantics(&mut buf);
        }
        buf
    }

    /// [`AccessibilitySnapshot::to_bytes`]의 역.
    pub fn from_bytes(bytes: &[u8]) -> Result<AccessibilitySnapshot, CodecError> {
        let mut reader = Reader::new(bytes);
        if reader.take(4)? != SNAPSHOT_MAGIC {
            return Err(CodecError::new("snapshot magic 불일치"));
        }
        if reader.u16()? != SNAPSHOT_FORMAT_VERSION {
            return Err(CodecError::new("지원하지 않는 snapshot 형식 버전"));
        }
        // entry 최소 크기: source 16 + parent flag 1 + role 1 + label·value flag 2 + 개수 2×4.
        let count = reader.count(28)?;
        let mut entries: Vec<SnapshotEntry> = Vec::with_capacity(count);
        let mut sources = BTreeSet::new();
        // 현재 preorder 경로(조상 entry index).
        let mut path: Vec<u32> = Vec::new();
        for index in 0..count {
            let source = reader.id()?;
            if !sources.insert(source) {
                return Err(CodecError::new("snapshot source 중복"));
            }
            let parent = reader.opt_u32()?;
            let node = AccessibilityNode::decode_semantics(&mut reader, source)?;
            match parent {
                None => path.clear(),
                Some(parent) => {
                    while path.last().is_some_and(|last| *last != parent) {
                        path.pop();
                    }
                    if path.is_empty() {
                        return Err(CodecError::new("부모 index가 preorder 조상이 아님"));
                    }
                }
            }
            let depth = u32::try_from(path.len())
                .map_err(|_| CodecError::new("snapshot 깊이가 u32 범위를 넘음"))?;
            let index =
                u32::try_from(index).map_err(|_| CodecError::new("snapshot index 범위 초과"))?;
            path.push(index);
            entries.push(SnapshotEntry {
                parent,
                depth,
                node,
            });
        }
        reader.expect_end()?;
        let snapshot = AccessibilitySnapshot { entries };
        if snapshot.to_bytes() != bytes {
            return Err(CodecError::new("canonical 형식이 아님"));
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{SelectionMode, SourceControl};
    use crate::generate::generate;
    use crate::node::{AccessibilityBinding, AccessibilityRole};
    use crate::source::{SourceKind, SourceNode, SourceTree};
    use crate::test_support::id;
    use stellar_core_types::property::{PropertyKey, PropertyPath};

    /// 양 플랫폼 공통 golden fixture(이름 → snapshot bytes hex).
    const GOLDEN: &str = include_str!("../tests/fixtures/snapshot_v1.golden");

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn golden(name: &str) -> String {
        GOLDEN
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .find_map(|line| {
                let (key, value) = line.split_once(' ')?;
                (key == name).then(|| value.trim().to_string())
            })
            .unwrap_or_default()
    }

    /// ```text
    /// 1 Frame ─ 2 SingleSelectionGroup("크기") ─ 3 List ─ 4 Button ─ 6 Text
    ///         │                                        └ 5 Button("큼")
    ///         ├ 7 TextInput(placeholder "메모", 읽기 순서 2)
    ///         └ 9 Image(label = 속성 경로, 읽기 순서 1)
    /// ```
    fn fixture_source() -> SourceTree {
        let mut source = SourceTree::new();
        for node in [
            SourceNode::new(id(1), SourceKind::Frame).with_children([id(2), id(7), id(9)]),
            SourceNode::new(id(2), SourceKind::Frame)
                .with_label(AccessibilityBinding::text("크기"))
                .with_control(SourceControl::single_selection_group(id(3), Some(id(50))))
                .with_children([id(3)]),
            SourceNode::new(id(3), SourceKind::Frame)
                .with_control(SourceControl::list(id(3), SelectionMode::Single, None))
                .with_children([id(4), id(5)]),
            SourceNode::new(id(4), SourceKind::Frame)
                .with_control(SourceControl::button(id(13)))
                .with_children([id(6)]),
            SourceNode::new(id(5), SourceKind::Frame)
                .with_label(AccessibilityBinding::text("큼"))
                .with_control(SourceControl::button(id(14))),
            SourceNode::new(id(6), SourceKind::Text),
            SourceNode::new(id(7), SourceKind::Frame)
                .with_control(SourceControl {
                    semantics: crate::control::ControlSemantics::TextInput {
                        value: Some(id(30)),
                        placeholder: Some(AccessibilityBinding::text("메모")),
                        focused: None,
                    },
                    focusable: true,
                })
                .with_reading_order(2),
            SourceNode::new(id(9), SourceKind::Image)
                .with_label(AccessibilityBinding::Property(PropertyPath::with_keys(
                    id(9),
                    vec![PropertyKey(3)],
                )))
                .with_reading_order(1),
        ] {
            source.insert(node).expect("source");
        }
        source.push_root(id(1));
        source
    }

    fn fixture_snapshot() -> AccessibilitySnapshot {
        let source = fixture_source();
        let tree = generate(&source).expect("생성");
        assert_eq!(tree.validate(&source), Vec::new());
        AccessibilitySnapshot::from_tree(&tree).expect("snapshot")
    }

    #[test]
    fn snapshot_applies_reading_order_in_preorder() {
        let snapshot = fixture_snapshot();
        let order: Vec<(StableId, AccessibilityRole, Option<u32>, u32)> = snapshot
            .entries()
            .iter()
            .map(|entry| {
                (
                    entry.node.source,
                    entry.node.role,
                    entry.parent,
                    entry.depth,
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![
                (id(1), AccessibilityRole::Generic, None, 0),
                // 명시 읽기 순서 1·2가 먼저, 나머지(group)는 트리 순서로 뒤.
                (id(9), AccessibilityRole::Image, Some(0), 1),
                (id(7), AccessibilityRole::TextField, Some(0), 1),
                (id(2), AccessibilityRole::RadioGroup, Some(0), 1),
                (id(4), AccessibilityRole::Radio, Some(3), 2),
                (id(6), AccessibilityRole::StaticText, Some(4), 3),
                (id(5), AccessibilityRole::Radio, Some(3), 2),
            ]
        );
        assert!(snapshot
            .entries()
            .iter()
            .all(|entry| entry.node.reading_order.is_none()));
        assert_eq!(snapshot.children(0), vec![1, 2, 3]);
        assert_eq!(snapshot.children(3), vec![4, 6]);
        assert_eq!(snapshot.children(6), Vec::<usize>::new());
        assert_eq!(snapshot.children(99), Vec::<usize>::new());
    }

    #[test]
    fn snapshot_bytes_match_cross_platform_golden() {
        let bytes = fixture_snapshot().to_bytes();
        assert_eq!(hex(&bytes), golden("radio_group_text_input_image"));
        // 빈 snapshot.
        assert_eq!(
            hex(&AccessibilitySnapshot::default().to_bytes()),
            golden("empty")
        );
        // tree 구성 순서(삽입·형제 추가 순서)와 무관하게 같은 bytes.
        let source = fixture_source();
        let tree = generate(&source).expect("생성");
        let mut rebuilt = AccessibilityTree::new();
        let mut nodes: Vec<AccessibilityNode> = tree.nodes().cloned().collect();
        nodes.reverse();
        for node in nodes {
            rebuilt.insert(node).expect("node");
        }
        let mut parents: Vec<StableId> = tree.nodes().map(|node| node.source).collect();
        parents.reverse();
        for parent in parents {
            rebuilt.set_children(parent, tree.children(parent).to_vec());
        }
        for root in tree.roots() {
            rebuilt.push_root(*root);
        }
        assert_eq!(
            AccessibilitySnapshot::from_tree(&rebuilt)
                .expect("snapshot")
                .to_bytes(),
            bytes
        );
    }

    #[test]
    fn snapshot_round_trips_and_rejects_malformed_bytes() {
        let snapshot = fixture_snapshot();
        let bytes = snapshot.to_bytes();
        assert_eq!(AccessibilitySnapshot::from_bytes(&bytes), Ok(snapshot));
        for len in 0..bytes.len() {
            assert!(
                AccessibilitySnapshot::from_bytes(&bytes[..len]).is_err(),
                "len {len}"
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(AccessibilitySnapshot::from_bytes(&trailing).is_err());
        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert_eq!(
            AccessibilitySnapshot::from_bytes(&magic),
            Err(CodecError::new("snapshot magic 불일치"))
        );

        // 부모 index가 현재 조상이 아님: root 다음 entry가 존재하지 않는 index 5를 부모로 가짐.
        let mut buf = Vec::new();
        buf.extend_from_slice(&SNAPSHOT_MAGIC);
        put_u16(&mut buf, SNAPSHOT_FORMAT_VERSION);
        put_count(&mut buf, 2);
        for (n, parent) in [(1u8, None), (2u8, Some(5u32))] {
            put_id(&mut buf, id(n));
            put_opt_u32(&mut buf, parent);
            AccessibilityNode::new(id(n), AccessibilityRole::Generic).encode_semantics(&mut buf);
        }
        assert_eq!(
            AccessibilitySnapshot::from_bytes(&buf),
            Err(CodecError::new("부모 index가 preorder 조상이 아님"))
        );

        // source 중복.
        let mut buf = Vec::new();
        buf.extend_from_slice(&SNAPSHOT_MAGIC);
        put_u16(&mut buf, SNAPSHOT_FORMAT_VERSION);
        put_count(&mut buf, 2);
        for parent in [None, Some(0u32)] {
            put_id(&mut buf, id(1));
            put_opt_u32(&mut buf, parent);
            AccessibilityNode::new(id(1), AccessibilityRole::Generic).encode_semantics(&mut buf);
        }
        assert_eq!(
            AccessibilitySnapshot::from_bytes(&buf),
            Err(CodecError::new("snapshot source 중복"))
        );
    }

    #[test]
    fn invalid_tree_is_rejected() {
        let mut tree = AccessibilityTree::new();
        tree.insert(AccessibilityNode::new(id(1), AccessibilityRole::Group))
            .expect("1");
        tree.push_root(id(1));
        tree.push_child(id(1), id(1));
        assert!(matches!(
            AccessibilitySnapshot::from_tree(&tree),
            Err(SnapshotError::InvalidTree { .. })
        ));
    }
}
