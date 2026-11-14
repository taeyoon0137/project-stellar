//! 중립 코드 생성 IR [`CodegenIr`] (XXII.1.1).
//!
//! resolved View 하나를 Target Profile 하나에 대해 **플랫폼 중립** IR로 바꿉니다. Web·Apple·Android adapter
//! (XXII.2~XXII.4)는 이 IR만 읽고 문서 모델·binding graph를 직접 보지 않습니다. IR은 단방향입니다(생성 코드를
//! IR이나 문서로 되읽는 API는 없습니다, 기술 아키텍처 §17.3).
//!
//! # 입력([`CodegenSources`])
//!
//! 하위 crate의 지역 모델을 그대로 받습니다. 문서 계층이 채워 줍니다.
//!
//! | 축 | 입력 | 출처 |
//! |---|---|---|
//! | 구조·target capability | [`ViewModel`](node 문서 순서·element·contract, 공통값·profile override) | IX.10.7·IX.10.8 |
//! | 상태 변화 | [`VariantLayer`](활성 View Variant와 path별 승자 값) | IX.7.4~IX.7.6 |
//! | layout | [`LayoutInput`](Fragment flatten한 layout tree, 선택 profile로 해석한 값) | X.1.2 |
//! | semantics | [`AccessibilitySnapshot`](읽기 순서 적용 preorder) | IX.8.6 |
//! | state | [`StateDecl`](상태 변수 id·이름·초기값) | IX.5.1 |
//! | tokens | [`TokenDecl`](token id·이름·해석 값)과 PropertyPath → token binding | VIII.5·VIII.6 |
//! | 텍스트 | Text node의 해석된 내용 | VI.7 |
//!
//! # 변환 규칙
//!
//! - **구조**: View는 root가 정확히 하나인 tree여야 합니다(중복 id, 없는 부모, root 둘 이상, root에서 닿지 않는
//!   node(순환)는 오류). IR node는 root부터 preorder이고 형제는 [`ViewModel`] 문서 순서입니다.
//! - **profile 해석**: IX.10.7 resolver로 공통값 + profile override + variant 층을 적용합니다. profile에서 제외한
//!   node와 그 하위 node는 IR에 넣지 않습니다(값·텍스트·layout·접근성 entry도 버림).
//! - **layout**: Fragment가 아닌 node마다 layout node가 있어야 합니다. layout 자식 목록(OrderKey 순, 제외 node
//!   제외)은 Fragment를 펼친 IR 자식 순서와 같아야 합니다. Fragment는 layout이 없습니다.
//! - **semantics**: 접근성 entry의 source는 View node여야 합니다. source가 제외된 entry는 버리고 그 자식은 가장
//!   가까운 남은 조상 entry 아래로 올립니다(읽기 순서 유지). binding·action이 가리키는 상태 변수는 선언되어
//!   있어야 합니다.
//! - **target capability**: IX.10.8 validator 결과(Native 포함 모든 진단)를 node별로 붙입니다. profile이 pin한
//!   registry release를 앱이 모르면 IR을 만들지 않습니다.
//! - **tokens·state**: 선언 전체를 id 오름차순으로 담습니다. token binding은 IR에 값이 있는 속성에만 붙습니다.
//!
//! 같은 입력은 항상 같은 IR과 같은 [`CodegenIr::semantic_snapshot`] 텍스트를 만듭니다(hash 순서·pointer·부동소수
//! 연산을 쓰지 않음).

use std::collections::BTreeMap;
use std::fmt;

use stellar_accessibility::node::AccessibilityNode;
use stellar_accessibility::snapshot::AccessibilitySnapshot;
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Insets, Transform2D};
use stellar_layout::constraint::Constraint;
use stellar_layout::error::LayoutError;
use stellar_layout::flex::FlexItem;
use stellar_layout::free::FreePosition;
use stellar_layout::grid::GridItem;
use stellar_layout::input::{ContainerLayout, LayoutContent, LayoutInput, LayoutNode, SizingMode};
use stellar_layout::sizing::{AspectRatio, SizeLimits};
use stellar_target_profile::capability::{CapabilityPropertyId, ContractCode, ElementKind};
use stellar_target_profile::overrides::{ValueOrigin, VariantLayer};
use stellar_target_profile::registry::ProfileRegistryVersion;
use stellar_target_profile::spec::TargetProfile;
use stellar_target_profile::validate::{CapabilityFinding, PropertyCapabilityMap, ViewModel};

mod binding;
mod build;

/// semantic snapshot 텍스트와 안정 이름 표기.
pub mod snapshot;

#[cfg(test)]
mod tests;

pub use binding::ResolvedBinding;
pub use build::{build_accessibility_ir, build_ir};

/// IR을 만들 수 있는 최대 View node 수.
pub const MAX_IR_NODES: usize = 1_000_000;

/// 상태 변수 선언(IX.5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDecl {
    /// 상태 변수 StableId.
    pub id: StableId,
    /// 작성자가 붙인 이름(비어 있으면 안 됨). 식별자 변환은 adapter가 합니다.
    pub name: String,
    /// 초기값(초기 expression을 평가한 값).
    pub initial: TypedValue,
}

/// token 선언(VIII.5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenDecl {
    /// token StableId.
    pub id: StableId,
    /// token 이름(비어 있으면 안 됨).
    pub name: String,
    /// 선택 profile로 해석한 값.
    pub value: TypedValue,
}

/// IR 변환 입력. 모든 값은 IR을 만들 Target Profile로 이미 해석된 것입니다(layout·텍스트·token 값 포함).
#[derive(Clone, Copy)]
pub struct CodegenSources<'a> {
    /// View 구조·공통값·profile override.
    pub view: &'a ViewModel,
    /// PropertyPath → capability 속성 대응(문서 계층 구현).
    pub capability_map: &'a dyn PropertyCapabilityMap,
    /// 활성 View Variant 층.
    pub variants: &'a VariantLayer,
    /// Fragment flatten한 layout tree.
    pub layout: &'a LayoutInput,
    /// 중립 접근성 snapshot.
    pub accessibility: &'a AccessibilitySnapshot,
    /// Text node StableId → 해석된 텍스트 내용.
    pub texts: &'a BTreeMap<StableId, String>,
    /// 상태 변수 선언.
    pub states: &'a [StateDecl],
    /// token 선언.
    pub tokens: &'a [TokenDecl],
    /// 속성 경로 → 그 값을 정한 token.
    pub token_bindings: &'a BTreeMap<PropertyPath, StableId>,
}

impl fmt::Debug for CodegenSources<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodegenSources")
            .field("nodes", &self.view.nodes.len())
            .field("layout_nodes", &self.layout.nodes.len())
            .field("accessibility", &self.accessibility.len())
            .field("states", &self.states.len())
            .field("tokens", &self.tokens.len())
            .finish()
    }
}

/// IR을 만든 target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrTarget {
    /// Target Profile entry.
    pub profile: TargetProfile,
    /// capability 판정에 쓴 registry release(= profile pin).
    pub registry_version: ProfileRegistryVersion,
}

/// node 하나의 layout 입력(자식 목록은 IR tree가 가집니다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrLayout {
    /// 자식 배치 방식.
    pub container: ContainerLayout,
    /// 안쪽 여백.
    pub padding: Insets,
    /// 너비 sizing.
    pub width: SizingMode,
    /// 높이 sizing.
    pub height: SizingMode,
    /// 콘텐츠 intrinsic 정보.
    pub content: LayoutContent,
    /// Free 부모 안의 anchor.
    pub position: FreePosition,
    /// local transform.
    pub transform: Transform2D,
    /// Stack 부모 main 축 grow·shrink·basis.
    pub flex: FlexItem,
    /// 표시 여부.
    pub visible: bool,
    /// Grid 부모 안의 위치.
    pub grid: GridItem,
    /// Constraint 부모 안의 등식 제약.
    pub constraints: Vec<Constraint>,
    /// 축별 최소·최대 크기.
    pub limits: SizeLimits,
    /// 너비 : 높이 비율.
    pub aspect_ratio: Option<AspectRatio>,
}

impl IrLayout {
    /// layout node에서 자식 목록을 뺀 값을 복사합니다.
    pub fn from_node(node: &LayoutNode) -> IrLayout {
        IrLayout {
            container: node.container.clone(),
            padding: node.padding,
            width: node.width,
            height: node.height,
            content: node.content.clone(),
            position: node.position,
            transform: node.transform,
            flex: node.flex,
            visible: node.visible,
            grid: node.grid,
            constraints: node.constraints.clone(),
            limits: node.limits,
            aspect_ratio: node.aspect_ratio,
        }
    }
}

/// node에 해석된 속성 값 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrProperty {
    /// 속성 경로(root = 이 node).
    pub path: PropertyPath,
    /// 해석 값.
    pub value: TypedValue,
    /// 값의 출처(공통값·profile override·variant).
    pub origin: ValueOrigin,
    /// 값을 정한 token(있으면).
    pub token: Option<StableId>,
    /// capability 검사 대상 속성이면 그 id.
    pub capability: Option<CapabilityPropertyId>,
}

/// IR node 하나(시각 tree, preorder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrNode {
    /// source StableId.
    pub id: StableId,
    /// 부모 IR node index(root면 `None`).
    pub parent: Option<u32>,
    /// root부터의 깊이(root = 0).
    pub depth: u32,
    /// 자식 IR node index(문서 순서).
    pub children: Vec<u32>,
    /// element 종류.
    pub element: ElementKind,
    /// 붙은 semantic contract(번호 오름차순, 중복 없음).
    pub contracts: Vec<ContractCode>,
    /// layout(Fragment는 `None`).
    pub layout: Option<IrLayout>,
    /// Text node의 해석된 텍스트 내용.
    pub text: Option<String>,
    /// 해석된 속성 값(path 오름차순).
    pub properties: Vec<IrProperty>,
    /// 이 node를 source로 하는 접근성 entry index.
    pub semantics: Option<u32>,
    /// 선택 profile의 capability 진단(IX.10.8 결정론 순서).
    pub capabilities: Vec<CapabilityFinding>,
}

impl IrNode {
    /// 계약이 붙어 있는지.
    pub fn has_contract(&self, code: ContractCode) -> bool {
        self.contracts.binary_search(&code).is_ok()
    }

    /// 속성 값(path로 찾기).
    pub fn property(&self, path: &PropertyPath) -> Option<&IrProperty> {
        self.properties
            .binary_search_by(|p| p.path.cmp(path))
            .ok()
            .map(|index| &self.properties[index])
    }
}

/// 접근성 entry 하나(읽기 순서 preorder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrAccessibilityEntry {
    /// source IR node index.
    pub node: u32,
    /// 부모 entry index(root면 `None`).
    pub parent: Option<u32>,
    /// 접근성 tree 깊이(root = 0).
    pub depth: u32,
    /// 의미(role·label·value·state·action, 읽기 순서는 entry 순서에 반영되어 `None`).
    pub semantics: AccessibilityNode,
}

/// 상태 변수.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrState {
    /// StableId.
    pub id: StableId,
    /// 이름.
    pub name: String,
    /// 초기값.
    pub initial: TypedValue,
}

/// token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrToken {
    /// StableId.
    pub id: StableId,
    /// 이름.
    pub name: String,
    /// 해석 값.
    pub value: TypedValue,
}

/// 중립 코드 생성 IR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenIr {
    /// target.
    pub target: IrTarget,
    /// 해석된 tree와 값. 접근성 전용 투영은 이 부분만 만들 수 있습니다.
    tree: IrTree,
}

impl std::ops::Deref for CodegenIr {
    type Target = IrTree;

    fn deref(&self) -> &IrTree {
        &self.tree
    }
}

impl std::ops::DerefMut for CodegenIr {
    fn deref_mut(&mut self) -> &mut IrTree {
        &mut self.tree
    }
}

/// Target Profile과 독립된 IR tree. 접근성 전용 입력은 layout·capability·속성·상태·token을
/// 만들지 않습니다. 전체 코드 생성에는 `build_ir`가 만든 `CodegenIr`이 필요합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrTree {
    /// 시각 node(preorder, `nodes[0]`이 root).
    pub nodes: Vec<IrNode>,
    /// 접근성 entry(읽기 순서 preorder).
    pub accessibility: Vec<IrAccessibilityEntry>,
    /// 상태 변수(id 오름차순).
    pub states: Vec<IrState>,
    /// token(id 오름차순).
    pub tokens: Vec<IrToken>,
    index: BTreeMap<StableId, u32>,
}

impl IrTree {
    /// StableId의 IR node index.
    pub fn node_index(&self, id: StableId) -> Option<u32> {
        self.index.get(&id).copied()
    }

    /// StableId의 IR node.
    pub fn node(&self, id: StableId) -> Option<&IrNode> {
        self.node_index(id).map(|index| &self.nodes[index as usize])
    }

    /// root node.
    pub fn root(&self) -> &IrNode {
        &self.nodes[0]
    }

    /// 속성 값(path root의 node에서 찾기).
    pub fn property(&self, path: &PropertyPath) -> Option<&IrProperty> {
        self.node(path.root())?.property(path)
    }

    /// node의 접근성 entry.
    pub fn semantics_of(&self, node: u32) -> Option<&IrAccessibilityEntry> {
        let entry = self.nodes.get(node as usize)?.semantics?;
        self.accessibility.get(entry as usize)
    }

    /// 상태 변수.
    pub fn state(&self, id: StableId) -> Option<&IrState> {
        self.states
            .binary_search_by(|s| s.id.cmp(&id))
            .ok()
            .map(|index| &self.states[index])
    }

    /// token.
    pub fn token(&self, id: StableId) -> Option<&IrToken> {
        self.tokens
            .binary_search_by(|t| t.id.cmp(&id))
            .ok()
            .map(|index| &self.tokens[index])
    }

    /// 접근성 entry의 자식 entry index(읽기 순서).
    pub fn accessibility_children(&self, entry: u32) -> Vec<u32> {
        // entry는 preorder이고 부모 index가 항상 앞이므로 뒤쪽만 훑습니다. 자식은 depth가 하나 큰 entry 중 부모가
        // `entry`인 것입니다.
        let Some(parent) = self.accessibility.get(entry as usize) else {
            return Vec::new();
        };
        let mut children = Vec::new();
        for (index, candidate) in self
            .accessibility
            .iter()
            .enumerate()
            .skip(entry as usize + 1)
        {
            if candidate.depth <= parent.depth {
                break;
            }
            if candidate.parent == Some(entry) {
                children.push(index as u32);
            }
        }
        children
    }

    /// node마다 자신이나 하위 node가 접근성 entry를 가지는지(index = IR node index).
    pub fn semantic_subtrees(&self) -> Vec<bool> {
        let mut has = vec![false; self.nodes.len()];
        // preorder 역순이면 자식이 부모보다 먼저 처리됩니다.
        for index in (0..self.nodes.len()).rev() {
            let node = &self.nodes[index];
            if node.semantics.is_some() || node.children.iter().any(|c| has[*c as usize]) {
                has[index] = true;
            }
        }
        has
    }
}

/// IR 변환 오류. 메시지는 입력 값(텍스트·이름)을 복사하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrError {
    /// View에 node가 없습니다.
    EmptyView,
    /// node 수가 [`MAX_IR_NODES`]를 넘습니다.
    TooManyNodes,
    /// 같은 StableId node가 둘 이상입니다.
    DuplicateNode(StableId),
    /// root(부모 없음) node가 없습니다.
    MissingRoot,
    /// root가 둘 이상입니다.
    MultipleRoots {
        /// 첫 root(문서 순서).
        first: StableId,
        /// 두 번째 root.
        second: StableId,
    },
    /// 부모가 View에 없습니다.
    UnknownParent {
        /// node.
        node: StableId,
        /// 없는 부모.
        parent: StableId,
    },
    /// root에서 닿지 않는 node(부모 순환).
    Unreachable(StableId),
    /// profile이 pin한 registry release를 앱이 모릅니다.
    RegistryUnavailable {
        /// pin.
        pinned: ProfileRegistryVersion,
        /// 앱 최신 release.
        latest: ProfileRegistryVersion,
    },
    /// layout 입력 검사 실패.
    InvalidLayout(LayoutError),
    /// layout root가 View root와 다릅니다.
    LayoutRootMismatch {
        /// View root.
        view: StableId,
        /// layout root.
        layout: StableId,
    },
    /// Fragment가 아닌 node에 layout node가 없습니다.
    MissingLayout(StableId),
    /// layout 자식 순서가 Fragment를 펼친 IR 자식 순서와 다릅니다.
    LayoutChildrenMismatch(StableId),
    /// 속성 값의 root가 View node가 아닙니다.
    PropertyOutsideView(StableId),
    /// 텍스트의 대상이 View node가 아닙니다.
    TextOutsideView(StableId),
    /// Text가 아닌 node에 텍스트 내용이 있습니다.
    TextOnNonTextNode(StableId),
    /// 접근성 entry의 source가 View node가 아닙니다.
    AccessibilityOutsideView(StableId),
    /// 같은 상태 변수가 두 번 선언되었습니다.
    DuplicateState(StableId),
    /// 상태 변수 이름이 비었습니다.
    EmptyStateName(StableId),
    /// 접근성 binding·action이 선언되지 않은 상태 변수를 가리킵니다.
    UnknownState {
        /// 접근성 source node.
        node: StableId,
        /// 상태 변수.
        state: StableId,
    },
    /// 같은 token이 두 번 선언되었습니다.
    DuplicateToken(StableId),
    /// token 이름이 비었습니다.
    EmptyTokenName(StableId),
    /// token binding이 선언되지 않은 token을 가리킵니다.
    UnknownToken {
        /// 속성 경로 root.
        node: StableId,
        /// token.
        token: StableId,
    },
    /// token binding의 속성에 해석 값이 없습니다.
    TokenBindingWithoutValue(StableId),
}

impl fmt::Display for IrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrError::EmptyView => f.write_str("View에 node가 없습니다"),
            IrError::TooManyNodes => {
                write!(f, "View node가 IR 상한 {MAX_IR_NODES}개를 넘습니다")
            }
            IrError::DuplicateNode(id) => write!(f, "같은 node가 두 번 있습니다: {id}"),
            IrError::MissingRoot => f.write_str("View에 root node가 없습니다"),
            IrError::MultipleRoots { first, second } => {
                write!(f, "View root가 둘 이상입니다: {first}, {second}")
            }
            IrError::UnknownParent { node, parent } => {
                write!(f, "node {node}의 부모 {parent}가 View에 없습니다")
            }
            IrError::Unreachable(id) => write!(f, "root에서 닿지 않는 node(부모 순환): {id}"),
            IrError::RegistryUnavailable { pinned, latest } => write!(
                f,
                "profile이 pin한 capability registry release {}를 앱이 모릅니다(최신 {})",
                pinned.get(),
                latest.get()
            ),
            IrError::InvalidLayout(error) => write!(f, "layout 입력이 잘못되었습니다: {error}"),
            IrError::LayoutRootMismatch { view, layout } => {
                write!(f, "layout root {layout}가 View root {view}와 다릅니다")
            }
            IrError::MissingLayout(id) => write!(f, "node {id}에 layout node가 없습니다"),
            IrError::LayoutChildrenMismatch(id) => {
                write!(
                    f,
                    "node {id}의 layout 자식 순서가 View 자식 순서와 다릅니다"
                )
            }
            IrError::PropertyOutsideView(id) => {
                write!(f, "속성 값의 root {id}가 View node가 아닙니다")
            }
            IrError::TextOutsideView(id) => write!(f, "텍스트 대상 {id}가 View node가 아닙니다"),
            IrError::TextOnNonTextNode(id) => {
                write!(f, "Text가 아닌 node {id}에 텍스트 내용이 있습니다")
            }
            IrError::AccessibilityOutsideView(id) => {
                write!(f, "접근성 entry의 source {id}가 View node가 아닙니다")
            }
            IrError::DuplicateState(id) => write!(f, "상태 변수가 두 번 선언되었습니다: {id}"),
            IrError::EmptyStateName(id) => write!(f, "상태 변수 {id}의 이름이 비었습니다"),
            IrError::UnknownState { node, state } => write!(
                f,
                "node {node}의 접근성 의미가 선언되지 않은 상태 변수 {state}를 가리킵니다"
            ),
            IrError::DuplicateToken(id) => write!(f, "token이 두 번 선언되었습니다: {id}"),
            IrError::EmptyTokenName(id) => write!(f, "token {id}의 이름이 비었습니다"),
            IrError::UnknownToken { node, token } => write!(
                f,
                "node {node}의 token binding이 선언되지 않은 token {token}을 가리킵니다"
            ),
            IrError::TokenBindingWithoutValue(id) => {
                write!(f, "node {id}의 token binding 속성에 해석 값이 없습니다")
            }
        }
    }
}

impl std::error::Error for IrError {}
