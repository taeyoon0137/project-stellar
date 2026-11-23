//! 읽기 전용 조회 (XXIV.4.3).
//!
//! [`ProjectSnapshot`] 하나(열기 시점에 고정한 generation)에서 문서 트리 page, node 속성,
//! token·component 목록을 만듭니다. 결과는 출력 형식과 무관한 Rust 값이며 모두 그 snapshot의
//! [`ProjectRevision`]을 함께 담습니다. 이 모듈은 파일을 쓰지 않습니다.
//!
//! ## layout longhand 값과 출처
//!
//! [`node_layout`]은 node의 layout longhand 전부를 활성 기본값 세트로 풀어(문서 crate `style_defaults::resolve`
//! 하나) 값마다 출처 `explicit` 또는 `default(<세트>)`를 붙입니다(ADR-0082 결정 5, ADR-0087 결정 3). 출처는 고정
//! 크기 enum [`LayoutSource`]이고 세트 이름은 결과에 한 번만 둡니다. [`node_detail`]도 같은 결과를 담습니다.
//!
//! ## 문서 트리 page와 cursor
//!
//! - 순서: 정의 순서(페이지 → View)로 각 View root Frame에서 시작하는 깊이 우선 전위 순회.
//!   형제는 (OrderKey, StableId) 오름차순입니다. 같은 generation이면 항상 같은 순서입니다.
//! - 한 page는 최대 [`MAX_PAGE_LIMIT`]개 node입니다. 대형 문서를 한 응답에 모두 담지 않습니다.
//! - cursor는 (문서, 다음 offset, revision)을 담은 문자열입니다([`TreeCursor`]). 다른 문서나 다른
//!   revision에서 만든 cursor는 거부합니다([`QueryError::CursorDocumentMismatch`],
//!   [`QueryError::StaleCursor`]). 그 사이 다른 writer가 저장했다면 처음부터 다시 조회해야 합니다.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::str::FromStr;

use stellar_document::css::style_defaults::{resolve, ValueSource};
use stellar_document::definitions::DocumentDefinition;
use stellar_document::layout_record::format_value;
use stellar_document::model::DocumentModel;
use stellar_semantic_diff::build::project_document_node;
use stellar_semantic_diff::value::SemanticValue;

use crate::core_types::ids::StableId;
use crate::core_types::order_key::OrderKey;
use crate::core_types::property::{PathSegment, PropertyPath};
use crate::error::DocumentReadError;
use crate::revision::ProjectRevision;
use crate::snapshot::ProjectSnapshot;

/// page 크기 기본값.
pub const DEFAULT_PAGE_LIMIT: usize = 1000;
/// page 크기 상한.
pub const MAX_PAGE_LIMIT: usize = 10_000;

/// 조회 실패.
#[derive(Debug)]
pub enum QueryError {
    /// 문서를 읽지 못했습니다.
    Document(DocumentReadError),
    /// 문서에 그 node가 없습니다.
    UnknownNode {
        /// 문서.
        document: StableId,
        /// 요청 node.
        node: StableId,
    },
    /// page 크기가 1..=[`MAX_PAGE_LIMIT`] 밖입니다.
    InvalidLimit {
        /// 요청 값.
        limit: usize,
    },
    /// cursor 형식이 잘못되었습니다.
    InvalidCursor,
    /// cursor가 다른 문서에서 만들어졌습니다.
    CursorDocumentMismatch {
        /// cursor의 문서.
        cursor: StableId,
        /// 요청 문서.
        requested: StableId,
    },
    /// cursor를 만든 뒤 프로젝트 revision이 바뀌었습니다.
    StaleCursor {
        /// cursor의 revision.
        cursor: ProjectRevision,
        /// 현재 revision.
        current: ProjectRevision,
    },
    /// cursor offset이 문서 node 수를 넘습니다.
    CursorOutOfRange {
        /// cursor offset.
        offset: usize,
        /// 문서 node 수.
        total: usize,
    },
    /// 속성 투영 실패(semantic snapshot 구성, XIV.3.1).
    Projection(String),
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryError::Document(e) => write!(f, "{e}"),
            QueryError::UnknownNode { document, node } => {
                write!(f, "문서 {document}에 node {node}가 없습니다")
            }
            QueryError::InvalidLimit { limit } => {
                write!(
                    f,
                    "page 크기 {limit}는 1..={MAX_PAGE_LIMIT} 범위여야 합니다"
                )
            }
            QueryError::InvalidCursor => f.write_str("cursor 형식이 올바르지 않습니다"),
            QueryError::CursorDocumentMismatch { cursor, requested } => write!(
                f,
                "cursor는 문서 {cursor}의 것이라 문서 {requested}에 쓸 수 없습니다"
            ),
            QueryError::StaleCursor { cursor, current } => write!(
                f,
                "cursor를 만든 뒤 revision이 바뀌었습니다(cursor {cursor}, 현재 {current})"
            ),
            QueryError::CursorOutOfRange { offset, total } => {
                write!(f, "cursor offset {offset}이 node 수 {total}를 넘습니다")
            }
            QueryError::Projection(e) => write!(f, "속성 투영 실패: {e}"),
        }
    }
}

impl std::error::Error for QueryError {}

impl From<DocumentReadError> for QueryError {
    fn from(e: DocumentReadError) -> QueryError {
        QueryError::Document(e)
    }
}

/// 문서 트리 page cursor.
///
/// 표기: `t1.<offset>.<문서 StableId>.<revision 표기>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeCursor {
    /// 문서.
    pub document: StableId,
    /// 다음 page의 첫 node 위치(전위 순회 index).
    pub offset: usize,
    /// cursor를 만든 revision.
    pub revision: ProjectRevision,
}

impl fmt::Display for TreeCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t1.{}.{}.{}", self.offset, self.document, self.revision)
    }
}

impl FromStr for TreeCursor {
    type Err = QueryError;

    fn from_str(text: &str) -> Result<TreeCursor, QueryError> {
        let rest = text.strip_prefix("t1.").ok_or(QueryError::InvalidCursor)?;
        let mut parts = rest.splitn(3, '.');
        let offset = parts.next().ok_or(QueryError::InvalidCursor)?;
        let document = parts.next().ok_or(QueryError::InvalidCursor)?;
        let revision = parts.next().ok_or(QueryError::InvalidCursor)?;
        if offset.is_empty()
            || (offset.len() > 1 && offset.starts_with('0'))
            || !offset.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(QueryError::InvalidCursor);
        }
        Ok(TreeCursor {
            offset: offset.parse().map_err(|_| QueryError::InvalidCursor)?,
            document: document.parse().map_err(|_| QueryError::InvalidCursor)?,
            revision: revision.parse().map_err(|_| QueryError::InvalidCursor)?,
        })
    }
}

/// 문서 트리의 node 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    /// node StableId.
    pub id: StableId,
    /// 부모(`None`이면 View root).
    pub parent: Option<StableId>,
    /// 소속 View.
    pub view: StableId,
    /// View root가 0인 깊이.
    pub depth: u32,
    /// node 종류 이름(`frame`, `text` 등).
    pub kind: &'static str,
    /// 표시 이름(식별에 쓰지 않음).
    pub display_name: String,
    /// 표시 여부.
    pub visible: bool,
    /// 잠금 여부.
    pub locked: bool,
    /// 직접 자식 수.
    pub child_count: usize,
}

/// 문서 트리 page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreePage {
    /// 조회한 revision.
    pub revision: ProjectRevision,
    /// 문서.
    pub document: StableId,
    /// 문서 전체 node 수.
    pub total: usize,
    /// 이 page의 첫 node 위치.
    pub offset: usize,
    /// node들(전위 순회 순).
    pub nodes: Vec<TreeNode>,
    /// 다음 page cursor. 마지막 page면 `None`.
    pub next_cursor: Option<TreeCursor>,
}

/// 한 문서의 전위 순회 index. 한 번 만들어 여러 page를 조회합니다.
#[derive(Debug, Clone)]
pub struct DocumentTree {
    revision: ProjectRevision,
    document: StableId,
    nodes: Vec<TreeNode>,
}

impl DocumentTree {
    /// snapshot의 문서 하나를 읽어 순회 index를 만듭니다.
    pub fn load(
        snapshot: &ProjectSnapshot,
        document: StableId,
    ) -> Result<DocumentTree, QueryError> {
        let model = snapshot.document_model(document)?;
        let nodes = preorder(&model);
        Ok(DocumentTree {
            revision: snapshot.revision(),
            document,
            nodes,
        })
    }

    /// 문서 전체 node 수(순회에 닿는 node).
    pub fn total(&self) -> usize {
        self.nodes.len()
    }

    /// 전위 순회 위치의 한 행을 빌립니다. 범위 밖이면 `None`입니다.
    pub fn node(&self, index: usize) -> Option<&TreeNode> {
        self.nodes.get(index)
    }

    /// page 하나를 만듭니다. `cursor`가 없으면 첫 page, `limit`은 1..=[`MAX_PAGE_LIMIT`].
    pub fn page(&self, cursor: Option<&TreeCursor>, limit: usize) -> Result<TreePage, QueryError> {
        if limit == 0 || limit > MAX_PAGE_LIMIT {
            return Err(QueryError::InvalidLimit { limit });
        }
        let offset = match cursor {
            None => 0,
            Some(cursor) => {
                if cursor.document != self.document {
                    return Err(QueryError::CursorDocumentMismatch {
                        cursor: cursor.document,
                        requested: self.document,
                    });
                }
                if cursor.revision != self.revision {
                    return Err(QueryError::StaleCursor {
                        cursor: cursor.revision,
                        current: self.revision,
                    });
                }
                cursor.offset
            }
        };
        let total = self.nodes.len();
        if offset > total || (cursor.is_some() && offset == total) {
            return Err(QueryError::CursorOutOfRange { offset, total });
        }
        let end = offset.saturating_add(limit).min(total);
        let nodes = self.nodes[offset..end].to_vec();
        let next_cursor = (end < total).then_some(TreeCursor {
            document: self.document,
            offset: end,
            revision: self.revision,
        });
        Ok(TreePage {
            revision: self.revision,
            document: self.document,
            total,
            offset,
            nodes,
            next_cursor,
        })
    }
}

/// 문서 트리 page를 조회합니다([`DocumentTree::load`] + [`DocumentTree::page`]).
pub fn document_tree_page(
    snapshot: &ProjectSnapshot,
    document: StableId,
    cursor: Option<&TreeCursor>,
    limit: usize,
) -> Result<TreePage, QueryError> {
    if limit == 0 || limit > MAX_PAGE_LIMIT {
        return Err(QueryError::InvalidLimit { limit });
    }
    if let Some(cursor) = cursor {
        // 문서를 읽기 전에 싸게 거부할 수 있는 cursor는 먼저 거부합니다.
        if cursor.document != document {
            return Err(QueryError::CursorDocumentMismatch {
                cursor: cursor.document,
                requested: document,
            });
        }
        if cursor.revision != snapshot.revision() {
            return Err(QueryError::StaleCursor {
                cursor: cursor.revision,
                current: snapshot.revision(),
            });
        }
    }
    DocumentTree::load(snapshot, document)?.page(cursor, limit)
}

/// 정의 순서 View별 깊이 우선 전위 순회(형제는 (OrderKey, StableId) 순). 반복 구현입니다.
fn preorder(model: &DocumentModel) -> Vec<TreeNode> {
    // 검증된 모델의 공통 필드만 읽습니다. 전체 payload를 영속 레코드로 복제·정렬하지 않습니다.
    let records: Vec<_> = model
        .columns()
        .iter_live()
        .map(|handle| {
            let fields = model.columns().get(handle).expect("live node");
            let parent = fields
                .parent
                .map(|handle| model.stable_id(handle).expect("검증된 부모"));
            (fields, parent)
        })
        .collect();
    let by_id: HashMap<StableId, usize> = records
        .iter()
        .enumerate()
        .map(|(i, (n, _))| (n.stable_id, i))
        .collect();
    let mut children: HashMap<StableId, Vec<(&OrderKey, StableId, usize)>> = HashMap::new();
    for (index, (node, parent)) in records.iter().enumerate() {
        if let Some(parent) = parent {
            children
                .entry(*parent)
                .or_default()
                .push((&node.order_key, node.stable_id, index));
        }
    }
    for list in children.values_mut() {
        list.sort();
    }
    let mut out = Vec::with_capacity(records.len());
    for view in model.definition().pages.iter().flat_map(|page| &page.views) {
        let Some(&root) = by_id.get(&view.root_frame) else {
            continue;
        };
        let mut stack: Vec<(usize, u32)> = vec![(root, 0)];
        while let Some((index, depth)) = stack.pop() {
            let (record, parent) = &records[index];
            let kids = children
                .get(&record.stable_id)
                .map_or(&[][..], Vec::as_slice);
            out.push(TreeNode {
                id: record.stable_id,
                parent: *parent,
                view: view.id,
                depth,
                kind: record.kind.name(),
                display_name: record.display_name.clone(),
                visible: record.visible,
                locked: record.locked,
                child_count: kids.len(),
            });
            for &(_, _, child) in kids.iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    out
}

/// 출력 형식과 무관한 속성 값(semantic snapshot 값 XIV.3.1의 표시용 사본).
#[derive(Debug, Clone, PartialEq)]
pub enum OpsValue {
    /// 불리언.
    Bool(bool),
    /// 부호 없는 정수.
    Uint(u64),
    /// 문자열.
    Text(String),
    /// StableId 참조.
    Id(StableId),
    /// StableId 목록.
    Ids(Vec<StableId>),
    /// 유한 float 목록.
    Floats(Vec<f32>),
    /// core-types 타입드 값: type tag와 canonical bytes(III.3.5).
    Typed {
        /// [`TypedValue::type_tag`](crate::core_types::typed_value::TypedValue::type_tag).
        type_tag: u8,
        /// canonical bytes.
        bytes: Vec<u8>,
    },
    /// 불투명 canonical bytes(path command·text payload 등).
    Bytes(Vec<u8>),
}

impl OpsValue {
    /// 값 종류 이름.
    pub fn type_name(&self) -> &'static str {
        match self {
            OpsValue::Bool(_) => "bool",
            OpsValue::Uint(_) => "uint",
            OpsValue::Text(_) => "text",
            OpsValue::Id(_) => "id",
            OpsValue::Ids(_) => "ids",
            OpsValue::Floats(_) => "floats",
            OpsValue::Typed { .. } => "typed",
            OpsValue::Bytes(_) => "bytes",
        }
    }
}

impl From<&SemanticValue> for OpsValue {
    fn from(value: &SemanticValue) -> OpsValue {
        match value {
            SemanticValue::Bool(v) => OpsValue::Bool(*v),
            SemanticValue::Uint(v) => OpsValue::Uint(*v),
            SemanticValue::Text(v) => OpsValue::Text(v.clone()),
            SemanticValue::Id(v) => OpsValue::Id(*v),
            SemanticValue::Ids(v) => OpsValue::Ids(v.clone()),
            SemanticValue::Floats(v) => OpsValue::Floats(v.iter().map(|f| f.get()).collect()),
            SemanticValue::Typed(v) => OpsValue::Typed {
                type_tag: v.type_tag(),
                bytes: v.to_bytes(),
            },
            SemanticValue::Bytes(v) => OpsValue::Bytes(v.clone()),
        }
    }
}

/// 속성 경로 표기: key마다 문서 registry 이름(CSS longhand 또는 Stellar 이름, `css::registry::name_of`)을 `/`로
/// 연결하고, tail이 있으면 `[index]` 또는 `{key}`입니다. registry에 없는 key(semantic snapshot 전용 `0xE0xx` 등)는
/// 4자리 16진수입니다(ADR-0075 개정).
///
/// 예: `border-top-left-radius`, `border[1]`, `0xe001`. 정확한 numeric key는 [`property_key_label`]입니다. 경로
/// root(소유 객체)는 표기에 넣지 않습니다.
pub fn property_path_label(path: &PropertyPath) -> String {
    label(path, |key| {
        stellar_document::css::registry::name_of(key)
            .map(str::to_owned)
            .unwrap_or_else(|| hex_key(key))
    })
}

/// 속성 경로의 numeric 표기: key를 4자리 16진수로 `/` 연결합니다(예: `0x0101/0x0102[3]`). JSON 출력의 보조
/// 필드(`key`)입니다.
pub fn property_key_label(path: &PropertyPath) -> String {
    label(path, hex_key)
}

fn hex_key(key: crate::core_types::property::PropertyKey) -> String {
    format!("0x{:04x}", key.0)
}

fn label(
    path: &PropertyPath,
    name: impl Fn(crate::core_types::property::PropertyKey) -> String,
) -> String {
    let mut out = path
        .keys()
        .iter()
        .map(|key| name(*key))
        .collect::<Vec<_>>()
        .join("/");
    match path.tail() {
        None => {}
        Some(PathSegment::Index(index)) => out.push_str(&format!("[{index}]")),
        Some(PathSegment::Key(key)) => out.push_str(&format!("{{{key}}}")),
    }
    out
}

/// node 속성 한 개.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeProperty {
    /// [`property_path_label`] 표기(registry 이름).
    pub path: String,
    /// [`property_key_label`] 표기(numeric key, 보조).
    pub key: String,
    /// 값.
    pub value: OpsValue,
}

/// layout longhand 값의 출처(ADR-0082 결정 5). 고정 크기이고 세트 이름은 [`NodeLayout::style_defaults`] 하나입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutSource {
    /// 사용자가 명시한 값(layout record 또는 payload 크기). 세트 값과 같아도 명시입니다.
    Explicit,
    /// 활성 기본값 세트의 값.
    Default,
}

/// node의 layout longhand 값 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutProperty {
    /// CSS longhand 이름.
    pub longhand: &'static str,
    /// 정규 CSS 값 텍스트(`layout_record::format_value`).
    pub value: String,
    /// 출처.
    pub source: LayoutSource,
}

/// node 하나의 풀린 layout longhand 전체.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeLayout {
    /// 활성 기본값 세트 이름. `default` 출처의 세트입니다.
    pub style_defaults: String,
    /// 기본값 구획 이름(`frame`·`text`·`image`·`shape`).
    pub class: &'static str,
    /// longhand 순서(`LayoutLonghand::ALL`)의 값.
    pub values: Vec<LayoutProperty>,
}

impl NodeLayout {
    /// 출처 표기: `explicit` 또는 `default(<세트 이름>)`.
    pub fn source_label(&self, source: LayoutSource) -> String {
        match source {
            LayoutSource::Explicit => "explicit".to_string(),
            LayoutSource::Default => format!("default({})", self.style_defaults),
        }
    }
}

/// [`node_layout`]을 풀 수 없는 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutUnavailable {
    /// 상자가 없는 node(Fragment)라 layout longhand가 없습니다.
    NoBox,
    /// 활성 기본값 세트를 평탄화하지 못했습니다(사용자 세트 파일 오류, 없는 세트, 순환). 자세한 진단은
    /// `stellar style-defaults show`가 냅니다. 결과에는 프로젝트 문자열 없이 안정 code만 둡니다.
    StyleDefaults {
        /// 안정 진단 code(`StyleDefaultsFileError::code`).
        code: &'static str,
    },
}

impl LayoutUnavailable {
    /// 고정 설명 문구(프로젝트 문자열 없음).
    pub fn message(&self) -> &'static str {
        match self {
            LayoutUnavailable::NoBox => "상자가 없는 node라 layout longhand가 없습니다",
            LayoutUnavailable::StyleDefaults { .. } => "활성 기본값 세트를 풀 수 없습니다",
        }
    }
}

/// 이미 읽은 문서 모델에서 node의 layout longhand를 snapshot의 활성 세트로 풉니다. node가 없으면 `None`입니다.
fn resolve_layout(
    snapshot: &ProjectSnapshot,
    model: &DocumentModel,
    node: StableId,
) -> Option<Result<NodeLayout, LayoutUnavailable>> {
    let payload = model.payload(node)?;
    let set = match snapshot.style_defaults() {
        Ok(set) => set,
        Err(e) => {
            return Some(Err(LayoutUnavailable::StyleDefaults { code: e.code() }))
        }
    };
    let Some(resolved) = resolve(set, payload, model.layout_record(node)) else {
        return Some(Err(LayoutUnavailable::NoBox));
    };
    Some(Ok(NodeLayout {
        style_defaults: set.name().to_string(),
        class: resolved.class.name(),
        values: resolved
            .values
            .iter()
            .map(|v| LayoutProperty {
                longhand: v.longhand.name(),
                value: format_value(&v.value),
                source: match v.source {
                    ValueSource::Explicit => LayoutSource::Explicit,
                    ValueSource::Default { .. } => LayoutSource::Default,
                },
            })
            .collect(),
    }))
}

/// node 하나의 layout longhand 값과 출처를 조회합니다(ADR-0082 결정 5). 바깥 `Err`는 문서·node 오류이고, 안쪽
/// `Err`는 이 node에 layout 값을 풀 수 없는 이유입니다.
pub fn node_layout(
    snapshot: &ProjectSnapshot,
    document: StableId,
    node: StableId,
) -> Result<Result<NodeLayout, LayoutUnavailable>, QueryError> {
    let model = snapshot.document_model(document)?;
    resolve_layout(snapshot, &model, node).ok_or(QueryError::UnknownNode { document, node })
}

/// node 하나의 공통 필드와 속성.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeDetail {
    /// 조회한 revision.
    pub revision: ProjectRevision,
    /// 문서.
    pub document: StableId,
    /// node.
    pub id: StableId,
    /// 부모(`None`이면 View root).
    pub parent: Option<StableId>,
    /// 형제 순서 키(bytes 16진수).
    pub order_key_hex: String,
    /// node 종류 이름.
    pub kind: &'static str,
    /// 표시 이름.
    pub display_name: String,
    /// 표시 여부.
    pub visible: bool,
    /// 잠금 여부.
    pub locked: bool,
    /// 자식 StableId((OrderKey, StableId) 순).
    pub children: Vec<StableId>,
    /// semantic snapshot 속성(XIV.3.1 투영, 경로 canonical 순).
    pub properties: Vec<NodeProperty>,
    /// 이 node가 소유한 modifier 수.
    pub modifier_count: usize,
    /// layout longhand 값과 출처([`node_layout`]과 같음).
    pub layout: Result<NodeLayout, LayoutUnavailable>,
}

/// node 하나의 공통 필드와 속성을 조회합니다.
pub fn node_detail(
    snapshot: &ProjectSnapshot,
    document: StableId,
    node: StableId,
) -> Result<NodeDetail, QueryError> {
    let model = snapshot.document_model(document)?;
    let record = model
        .record(node)
        .ok_or(QueryError::UnknownNode { document, node })?;
    let object = project_document_node(document, &record)
        .map_err(|e| QueryError::Projection(e.to_string()))?;
    let properties = object
        .properties()
        .iter()
        .map(|(path, value)| NodeProperty {
            path: property_path_label(path),
            key: property_key_label(path),
            value: OpsValue::from(value),
        })
        .collect();
    let modifier_count = model
        .modifier_stack(node)
        .map_or(0, |stack| stack.entries().len());
    Ok(NodeDetail {
        revision: snapshot.revision(),
        document,
        id: record.id,
        parent: record.parent,
        order_key_hex: record
            .order_key
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        kind: record.kind().name(),
        display_name: record.display_name.clone(),
        visible: record.visible,
        locked: record.locked,
        children: model.children_in_order(node),
        properties,
        modifier_count,
        layout: resolve_layout(snapshot, &model, node)
            .ok_or(QueryError::UnknownNode { document, node })?,
    })
}

/// 문서 정의의 페이지·View 개요.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentOutline {
    /// 문서.
    pub id: StableId,
    /// 표시 이름.
    pub display_name: String,
    /// 페이지(표시 순).
    pub pages: Vec<PageOutline>,
}

/// 페이지 개요.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageOutline {
    /// 페이지.
    pub id: StableId,
    /// 표시 이름.
    pub display_name: String,
    /// View(표시 순).
    pub views: Vec<ViewOutline>,
}

/// View 개요.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewOutline {
    /// View.
    pub id: StableId,
    /// 표시 이름.
    pub display_name: String,
    /// root Frame.
    pub root_frame: StableId,
}

impl From<&DocumentDefinition> for DocumentOutline {
    fn from(definition: &DocumentDefinition) -> DocumentOutline {
        DocumentOutline {
            id: definition.id,
            display_name: definition.display_name().to_string(),
            pages: definition
                .pages
                .iter()
                .map(|page| PageOutline {
                    id: page.id,
                    display_name: page.display_name().to_string(),
                    views: page
                        .views
                        .iter()
                        .map(|view| ViewOutline {
                            id: view.id,
                            display_name: view.display_name().to_string(),
                            root_frame: view.root_frame,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

/// 프로젝트 구조: 문서 개요와 저장 chunk 요약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectStructure {
    /// 조회한 revision.
    pub revision: ProjectRevision,
    /// 문서 개요(snapshot 문서 순).
    pub documents: Vec<DocumentOutline>,
    /// chunk kind별 개수(kind code 순, 이름 없는 kind 포함).
    pub chunk_kinds: Vec<(Option<&'static str>, u32, usize)>,
}

/// 프로젝트 구조를 조회합니다.
pub fn project_structure(snapshot: &ProjectSnapshot) -> ProjectStructure {
    ProjectStructure {
        revision: snapshot.revision(),
        documents: snapshot
            .documents()
            .iter()
            .map(DocumentOutline::from)
            .collect(),
        chunk_kinds: snapshot
            .chunk_kinds()
            .iter()
            .map(|k| (k.name, k.code, k.count))
            .collect(),
    }
}

/// token·component 같은 정의 집합 조회 결과.
///
/// generation의 token·component chunk pointer를 셉니다. 별도로 저장한 컴포넌트 YAML의
/// 프로젝트 수준 조회는 [`crate::component_source::query`]로 수행합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionSet {
    /// 조회한 revision.
    pub revision: ProjectRevision,
    /// chunk kind 이름.
    pub chunk_kind: &'static str,
    /// generation에 있는 해당 kind chunk 수.
    pub chunk_count: usize,
    /// 해석한 항목 이름(현재 저장 포맷 없음 → 비어 있음).
    pub entries: Vec<String>,
}

fn definition_set(snapshot: &ProjectSnapshot, kind: &'static str) -> DefinitionSet {
    DefinitionSet {
        revision: snapshot.revision(),
        chunk_kind: kind,
        chunk_count: snapshot
            .chunk_kinds()
            .iter()
            .find(|k| k.name == Some(kind))
            .map_or(0, |k| k.count),
        entries: Vec::new(),
    }
}

/// token 집합을 조회합니다([`DefinitionSet`] 한계 참조).
pub fn tokens(snapshot: &ProjectSnapshot) -> DefinitionSet {
    definition_set(snapshot, "token-set")
}

/// generation에 저장된 component-library chunk 개요. 프로젝트 원본은 [`crate::component_source::query`]로 조회합니다.
pub fn components(snapshot: &ProjectSnapshot) -> DefinitionSet {
    definition_set(snapshot, "component-library")
}

/// 문서별 node 수(snapshot의 모든 문서를 읽습니다).
pub fn document_node_counts(
    snapshot: &ProjectSnapshot,
) -> Result<BTreeMap<StableId, usize>, QueryError> {
    let mut out = BTreeMap::new();
    for document in snapshot.documents() {
        let records = snapshot.document_records(document.id)?;
        out.insert(document.id, records.nodes.len());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::open_snapshot;
    use crate::test_support::{
        file_digest, large_document, random_valid_document, save_documents, scaffold_project,
        TempDir,
    };
    use std::collections::BTreeSet;

    #[test]
    fn cursor_round_trips_and_rejects_malformed_text() {
        let cursor = TreeCursor {
            document: StableId::new_v7(5, 6, 7),
            offset: 1234,
            revision: ProjectRevision::Unsaved,
        };
        assert_eq!(cursor.to_string().parse::<TreeCursor>().unwrap(), cursor);
        for text in [
            "",
            "t1.",
            "t2.0.x.g0",
            "t1.01.x.g0",
            "t1.a.x.g0",
            "t1.5.not-an-id.g0",
        ] {
            assert!(text.parse::<TreeCursor>().is_err(), "{text}");
        }
    }

    #[test]
    fn two_hundred_thousand_node_document_is_split_into_pages() {
        // 검증 기준(XXIV.4.3): 20만 node 문서 page 분할 통과.
        let dir = TempDir::new("pages");
        scaffold_project(&dir.path);
        let model = large_document(200_000);
        let expected_total = model.node_count();
        assert!(expected_total >= 200_000, "{expected_total}");
        save_documents(&dir.path, &[&model]);
        let before = file_digest(&dir.path);

        let snapshot = open_snapshot(&dir.path).unwrap();
        let document = model.document_id();
        let tree = DocumentTree::load(&snapshot, document).unwrap();
        assert_eq!(tree.total(), expected_total);
        let mut seen = BTreeSet::new();
        let mut cursor = None;
        let mut pages = 0usize;
        let mut previous_depth_first: Option<StableId> = None;
        loop {
            let page = tree.page(cursor.as_ref(), MAX_PAGE_LIMIT).unwrap();
            pages += 1;
            assert_eq!(page.total, expected_total);
            assert_eq!(page.offset, (pages - 1) * MAX_PAGE_LIMIT);
            assert!(page.nodes.len() <= MAX_PAGE_LIMIT);
            assert!(!page.nodes.is_empty());
            if pages == 1 {
                assert_eq!(page.nodes[0].depth, 0, "첫 node는 View root");
                assert_eq!(page.nodes[0].parent, None);
            }
            for node in &page.nodes {
                assert!(seen.insert(node.id), "중복 node {}", node.id);
            }
            previous_depth_first = page.nodes.last().map(|n| n.id).or(previous_depth_first);
            match page.next_cursor {
                Some(next) => {
                    // cursor 문자열 왕복도 같은 page를 줍니다.
                    cursor = Some(next.to_string().parse().unwrap());
                }
                None => break,
            }
        }
        assert!(previous_depth_first.is_some());
        assert_eq!(seen.len(), expected_total);
        assert_eq!(pages, expected_total.div_ceil(MAX_PAGE_LIMIT));
        // 한 번에 여는 경로(document_tree_page)도 같은 첫 page와 cursor를 줍니다.
        let first = document_tree_page(&snapshot, document, None, MAX_PAGE_LIMIT).unwrap();
        assert_eq!(first, tree.page(None, MAX_PAGE_LIMIT).unwrap());
        assert_eq!(file_digest(&dir.path), before, "조회는 파일을 바꾸지 않음");
    }

    #[test]
    fn preorder_visits_parents_before_children_in_order_key_order() {
        let dir = TempDir::new("order");
        scaffold_project(&dir.path);
        let model = random_valid_document(91);
        save_documents(&dir.path, &[&model]);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let tree = DocumentTree::load(&snapshot, model.document_id()).unwrap();
        let page = tree.page(None, MAX_PAGE_LIMIT).unwrap();
        assert_eq!(page.total, model.node_count());
        assert!(
            page.total > 0,
            "공통 필드 비교가 빈 문서에서 통과하지 않아야 함"
        );
        assert!(tree.node(page.total).is_none());
        for (index, node) in page.nodes.iter().enumerate() {
            assert_eq!(tree.node(index), Some(node));
            let record = model.record(node.id).unwrap();
            assert_eq!(node.parent, record.parent);
            assert_eq!(node.kind, record.kind().name());
            assert_eq!(node.display_name, record.display_name);
            assert_eq!(node.visible, record.visible);
            assert_eq!(node.locked, record.locked);
        }
        let position: HashMap<StableId, usize> = page
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i))
            .collect();
        for node in &page.nodes {
            if let Some(parent) = node.parent {
                assert!(position[&parent] < position[&node.id]);
                let parent_depth = page.nodes[position[&parent]].depth;
                assert_eq!(node.depth, parent_depth + 1);
            }
            assert_eq!(model.children_in_order(node.id).len(), node.child_count);
        }
        for node in &page.nodes {
            let order: Vec<StableId> = page
                .nodes
                .iter()
                .filter(|n| n.parent == Some(node.id))
                .map(|n| n.id)
                .collect();
            assert_eq!(order, model.children_in_order(node.id), "{}", node.id);
        }
    }

    #[test]
    fn stale_foreign_and_out_of_range_cursors_are_rejected() {
        let dir = TempDir::new("cursor");
        scaffold_project(&dir.path);
        let first = random_valid_document(81);
        let second = random_valid_document(82);
        save_documents(&dir.path, &[&first, &second]);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let revision = snapshot.revision();
        let foreign = TreeCursor {
            document: second.document_id(),
            offset: 0,
            revision,
        };
        assert!(matches!(
            document_tree_page(&snapshot, first.document_id(), Some(&foreign), 10),
            Err(QueryError::CursorDocumentMismatch { .. })
        ));
        let total = first.node_count();
        let beyond = TreeCursor {
            document: first.document_id(),
            offset: total,
            revision,
        };
        assert!(matches!(
            document_tree_page(&snapshot, first.document_id(), Some(&beyond), 10),
            Err(QueryError::CursorOutOfRange { .. })
        ));
        for limit in [0, MAX_PAGE_LIMIT + 1] {
            assert!(matches!(
                document_tree_page(&snapshot, first.document_id(), None, limit),
                Err(QueryError::InvalidLimit { .. })
            ));
        }
        // 다른 writer가 저장하면 이전 cursor는 낡은 cursor입니다.
        let cursor = TreeCursor {
            document: first.document_id(),
            offset: 0,
            revision,
        };
        save_documents(&dir.path, &[&first]);
        let reopened = open_snapshot(&dir.path).unwrap();
        assert!(matches!(
            document_tree_page(&reopened, first.document_id(), Some(&cursor), 10),
            Err(QueryError::StaleCursor { .. })
        ));
    }

    #[test]
    fn node_detail_reports_common_fields_and_projected_properties() {
        let dir = TempDir::new("detail");
        scaffold_project(&dir.path);
        let model = random_valid_document(91);
        save_documents(&dir.path, &[&model]);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let records = snapshot.document_records(model.document_id()).unwrap();
        for record in &records.nodes {
            let detail = node_detail(&snapshot, model.document_id(), record.id).unwrap();
            assert_eq!(detail.id, record.id);
            assert_eq!(detail.parent, record.parent);
            assert_eq!(detail.kind, record.kind().name());
            assert_eq!(detail.children, model.children_in_order(record.id));
            assert_eq!(detail.revision, snapshot.revision());
            // 공통 필드는 semantic 속성으로도 투영됩니다(visible·locked·표시 이름 key 0xE00x).
            assert!(detail
                .properties
                .iter()
                .any(|p| p.path == "0xe001" && p.value == OpsValue::Bool(record.visible)));
            let paths: Vec<&str> = detail.properties.iter().map(|p| p.path.as_str()).collect();
            let mut sorted = paths.clone();
            sorted.dedup();
            assert_eq!(sorted.len(), paths.len(), "경로 중복 없음");
        }
        let missing = StableId::new_v7(9, 9, 9);
        assert!(matches!(
            node_detail(&snapshot, model.document_id(), missing),
            Err(QueryError::UnknownNode { .. })
        ));
    }

    #[test]
    fn structure_tokens_and_components_carry_revision() {
        let dir = TempDir::new("structure");
        scaffold_project(&dir.path);
        let model = random_valid_document(95);
        save_documents(&dir.path, &[&model]);
        let snapshot = open_snapshot(&dir.path).unwrap();
        let structure = project_structure(&snapshot);
        assert_eq!(structure.revision, snapshot.revision());
        assert_eq!(structure.documents.len(), 1);
        assert_eq!(structure.documents[0].id, model.document_id());
        let views: usize = structure.documents[0]
            .pages
            .iter()
            .map(|p| p.views.len())
            .sum();
        assert_eq!(
            views,
            model.definition().pages.iter().map(|p| p.views.len()).sum()
        );
        for set in [tokens(&snapshot), components(&snapshot)] {
            assert_eq!(set.revision, snapshot.revision());
            assert_eq!(set.chunk_count, 0);
            assert!(set.entries.is_empty());
        }
        let counts = document_node_counts(&snapshot).unwrap();
        assert_eq!(counts[&model.document_id()], model.node_count());
    }

    mod layout_provenance {
        //! IX.11.28: 값마다 출처 `explicit`·`default(<세트>)`(ADR-0082 결정 5, ADR-0087 결정 3).
        use super::*;
        use crate::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
        use crate::lock::LockPolicy;
        use stellar_commands::command::Command;
        use stellar_commands::style_defaults::UnsetProperty;
        use stellar_document::layout_record::{LayoutLonghand, LayoutRecord};
        use stellar_project::manifest_style_defaults::set_active;

        /// 저장된 프로젝트, 문서, `display: flex`·`flex-direction: column`을 명시한 root Frame.
        fn project(tmp: &TempDir) -> (std::path::PathBuf, StableId, StableId) {
            let root = tmp.path.join("project");
            std::fs::create_dir_all(&root).unwrap();
            scaffold_project(&root);
            let mut model = large_document(4);
            let frame = model.definition().pages[0].views[0].root_frame;
            let mut record = LayoutRecord::new(frame);
            for (longhand, value) in [
                (LayoutLonghand::Display, "flex"),
                (LayoutLonghand::FlexDirection, "column"),
            ] {
                record
                    .set(longhand, longhand.parse(value).unwrap())
                    .unwrap();
            }
            model.set_layout_record(record).unwrap();
            save_documents(&root, &[&model]);
            (root, model.document_id(), frame)
        }

        fn source(layout: &NodeLayout, longhand: &str) -> (String, String) {
            let value = layout
                .values
                .iter()
                .find(|v| v.longhand == longhand)
                .unwrap();
            (value.value.clone(), layout.source_label(value.source))
        }

        #[test]
        fn values_carry_explicit_or_default_source_of_the_active_set() {
            let tmp = TempDir::new("query-layout");
            let (root, document, frame) = project(&tmp);
            let snapshot = open_snapshot(&root).unwrap();
            let layout = node_layout(&snapshot, document, frame).unwrap().unwrap();
            assert_eq!(layout.style_defaults, "css");
            assert_eq!(layout.class, "frame");
            assert_eq!(layout.values.len(), LayoutLonghand::ALL.len());
            assert_eq!(source(&layout, "display"), ("flex".into(), "explicit".into()));
            assert_eq!(
                source(&layout, "flex-shrink"),
                ("1".into(), "default(css)".into())
            );
            // payload 크기는 명시 값입니다.
            assert_eq!(source(&layout, "width").1, "explicit");
            // node_detail도 같은 결과를 담습니다.
            let detail = node_detail(&snapshot, document, frame).unwrap();
            assert_eq!(detail.layout, Ok(layout));

            set_active(&root, "react-native").unwrap();
            let snapshot = open_snapshot(&root).unwrap();
            let layout = node_layout(&snapshot, document, frame).unwrap().unwrap();
            assert_eq!(
                source(&layout, "flex-shrink"),
                ("0".into(), "default(react-native)".into())
            );
            // 세트 값(column)과 같아도 명시로 남습니다.
            assert_eq!(
                source(&layout, "flex-direction"),
                ("column".into(), "explicit".into())
            );
            assert!(matches!(
                node_layout(&snapshot, document, StableId::new_v7(9, 9, 9)),
                Err(QueryError::UnknownNode { .. })
            ));
        }

        #[test]
        fn unset_property_turns_the_source_into_the_set_default() {
            let tmp = TempDir::new("query-layout-unset");
            let (root, document, frame) = project(&tmp);
            set_active(&root, "react-native").unwrap();
            let snapshot = open_snapshot(&root).unwrap();
            let command = Command::UnsetProperty(UnsetProperty {
                node: frame,
                longhand: LayoutLonghand::FlexDirection,
            });
            let request = ApplyRequest {
                base_revision: snapshot.revision(),
                document,
                commands: vec![CommandInput::Canonical {
                    kind: command.kind().name().to_string(),
                    bytes: stellar_commands::codec::encode_command(&command),
                }],
            };
            let locks = LockPolicy::Required {
                dir: tmp.path.join("locks"),
            };
            apply(&root, &request, ApplyMode::Write, &locks).unwrap();
            let snapshot = open_snapshot(&root).unwrap();
            let layout = node_layout(&snapshot, document, frame).unwrap().unwrap();
            assert_eq!(
                source(&layout, "flex-direction"),
                ("column".into(), "default(react-native)".into())
            );
            assert_eq!(source(&layout, "display"), ("flex".into(), "explicit".into()));
        }

        #[test]
        fn unflattenable_set_is_reported_per_node_without_failing_the_query() {
            let tmp = TempDir::new("query-layout-broken");
            let (root, document, frame) = project(&tmp);
            set_active(&root, "react-native").unwrap();
            // 매니페스트가 없는 사용자 세트를 고르게 직접 고칩니다.
            let manifest = root.join(stellar_project::root::MANIFEST_FILE);
            let text = std::fs::read_to_string(&manifest)
                .unwrap()
                .replace("react-native", "missing");
            std::fs::write(&manifest, text).unwrap();
            let snapshot = open_snapshot(&root).unwrap();
            let detail = node_detail(&snapshot, document, frame).unwrap();
            assert_eq!(
                detail.layout,
                Err(LayoutUnavailable::StyleDefaults {
                    code: "unknown-style-defaults"
                })
            );
            assert!(!detail.properties.is_empty());
        }
    }
}
