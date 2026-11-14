//! 접근성 생성기 입력: 시각 source tree (IX.8.3).
//!
//! crate 그래프상 이 crate는 document에 의존하지 않습니다. 생성기는 문서 node tree를 직접 읽지 않고, 상위
//! adapter가 문서에서 만들어 넘기는 이 **중립 source tree**를 읽습니다. 각 field는 문서 모델의 다음 값에
//! 대응합니다.
//!
//! | field | 문서 모델 |
//! |---|---|
//! | `id` | node StableId |
//! | `kind` | `NodeKind`: Frame → `Frame`, Fragment → `Fragment`, Shape·Path → `Graphic`, Text → `Text`, Image → `Image`, ComponentInstance → `ComponentInstance` |
//! | `children` | 시각 자식 순서(OrderKey 정렬 후) |
//! | `hidden` | 렌더 가시성이 꺼짐(자신과 subtree가 보이지 않음) |
//! | `decorative` | 작성자가 정보 없는 장식으로 표시 |
//! | `label` | 작성자 지정 접근 가능한 이름(상수·속성 binding) |
//! | `reading_order` | 작성자 지정 형제 읽기 순서 |

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::node::AccessibilityBinding;
use crate::tree::SourceIndex;

/// 시각 node 종류(접근성 생성에 필요한 구분만).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceKind {
    /// Frame(layout container).
    Frame,
    /// Fragment(렌더·layout 필드 없는 그룹 marker, VI.4.1).
    Fragment,
    /// 텍스트.
    Text,
    /// 이미지.
    Image,
    /// 벡터 그래픽(Shape·Path).
    Graphic,
    /// 컴포넌트 인스턴스(내부는 펼친 자식으로 전달).
    ComponentInstance,
}

/// 시각 node 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceNode {
    /// StableId.
    pub id: StableId,
    /// 종류.
    pub kind: SourceKind,
    /// 시각 자식(순서대로).
    pub children: Vec<StableId>,
    /// 숨김(자신과 subtree가 보이지 않음).
    pub hidden: bool,
    /// 장식(자신은 정보가 없음).
    pub decorative: bool,
    /// 작성자 지정 접근 가능한 이름.
    pub label: Option<AccessibilityBinding>,
    /// 작성자 지정 형제 읽기 순서.
    pub reading_order: Option<u32>,
}

impl SourceNode {
    /// 자식·표시·label 없는 node.
    pub fn new(id: StableId, kind: SourceKind) -> SourceNode {
        SourceNode {
            id,
            kind,
            children: Vec::new(),
            hidden: false,
            decorative: false,
            label: None,
            reading_order: None,
        }
    }

    /// 자식을 설정한 사본(빌더).
    pub fn with_children(mut self, children: impl IntoIterator<Item = StableId>) -> SourceNode {
        self.children = children.into_iter().collect();
        self
    }

    /// 숨김으로 표시한 사본(빌더).
    pub fn hidden(mut self) -> SourceNode {
        self.hidden = true;
        self
    }

    /// 장식으로 표시한 사본(빌더).
    pub fn decorative(mut self) -> SourceNode {
        self.decorative = true;
        self
    }

    /// label을 설정한 사본(빌더).
    pub fn with_label(mut self, label: AccessibilityBinding) -> SourceNode {
        self.label = Some(label);
        self
    }

    /// 읽기 순서를 설정한 사본(빌더).
    pub fn with_reading_order(mut self, order: u32) -> SourceNode {
        self.reading_order = Some(order);
        self
    }
}

/// source tree 변경 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    /// 같은 StableId node가 이미 있습니다.
    DuplicateNode {
        /// node.
        id: StableId,
    },
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::DuplicateNode { id } => write!(f, "source node {id}가 이미 있습니다"),
        }
    }
}

impl std::error::Error for SourceError {}

/// 시각 source tree: root 목록과 StableId → node.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceTree {
    roots: Vec<StableId>,
    nodes: BTreeMap<StableId, SourceNode>,
}

impl SourceTree {
    /// 빈 tree.
    pub fn new() -> SourceTree {
        SourceTree::default()
    }

    /// node를 추가합니다.
    pub fn insert(&mut self, node: SourceNode) -> Result<(), SourceError> {
        if self.nodes.contains_key(&node.id) {
            return Err(SourceError::DuplicateNode { id: node.id });
        }
        self.nodes.insert(node.id, node);
        Ok(())
    }

    /// root 목록 끝에 추가합니다.
    pub fn push_root(&mut self, id: StableId) {
        self.roots.push(id);
    }

    /// root 목록.
    pub fn roots(&self) -> &[StableId] {
        &self.roots
    }

    /// node.
    pub fn node(&self, id: StableId) -> Option<&SourceNode> {
        self.nodes.get(&id)
    }

    /// node 목록(StableId 오름차순).
    pub fn nodes(&self) -> impl Iterator<Item = &SourceNode> + '_ {
        self.nodes.values()
    }

    /// node 수.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl SourceIndex for SourceTree {
    fn contains_source(&self, id: StableId) -> bool {
        self.nodes.contains_key(&id)
    }
}
