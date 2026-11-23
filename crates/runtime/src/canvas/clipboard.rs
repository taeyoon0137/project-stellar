//! 캔버스 클립보드: Stellar 전용 형식과 Text 일반 텍스트 (XII.10.38, ADR-0117 결정 9).
//!
//! - Copy·Cut은 다른 선택의 자손을 뺀 root들(선택 순서)을 Stellar 전용 형식([`STELLAR_PASTEBOARD_TYPE`], bytes는
//!   [`CanvasClipboard`] 봉투)과 일반 텍스트(선택 subtree의 Text 내용을 문서 순서로 줄바꿈 이어 붙임)로 냅니다. 플랫폼
//!   pasteboard 접근은 셸이 합니다.
//! - 봉투 안의 subtree는 기존 같은 문서 subtree 형식(`ClipboardSubtree`, `STELLAR-CLIP\x01`) bytes 그대로입니다.
//! - Paste 위치: 선택한 컨테이너 안 → 원본 위치가 보이면 같은 부모·같은 위치 → 화면 가운데(View root 아래).
//!   새 ID는 `ClipboardSubtree::plan_with`(IdRemapTable)로 받고, 붙여 넣은 root 전체를 새 선택으로 한 history 항목에
//!   기록합니다.
//! - 다른 문서·프로젝트의 봉투는 진단(`cross-document`, `component-missing`, `asset-missing`)과 함께 거부합니다.

use stellar_commands::command::Command;
use stellar_commands::lock::{edit_permission, EditIntent};
use stellar_commands::node_ops::{DeleteSubtree, InsertNode};
use stellar_commands::paste::ClipboardSubtree;
use stellar_commands::selection::Selection;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;

use crate::app::canvas_gesture::{parent_of, view_root};
use crate::canvas::transform::{
    node_geometry, selection_roots, transaction, union, FreshIds, TransformError,
};
use crate::canvas::CanvasScene;
use crate::window_session::{history_label, ProjectWindowSession};

/// Stellar 전용 pasteboard type 이름.
pub const STELLAR_PASTEBOARD_TYPE: &str = "app.stellar.canvas-subtree.v1";
/// 봉투 header.
const MAGIC: &[u8] = b"STELLAR-CANVAS\x01";
/// 봉투 크기 상한(bytes).
pub const MAX_ENVELOPE_BYTES: usize = 16 * 1024 * 1024;
/// root 수 상한.
pub const MAX_ROOTS: usize = 4_096;

/// 붙여넣기 진단.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PasteDiagnostic {
    /// 다른 문서(다른 프로젝트 포함)에서 복사한 봉투. 지금은 같은 문서 subtree만 붙여 넣습니다.
    CrossDocument,
    /// 대상에 없는 Component 정의를 참조하는 인스턴스.
    ComponentMissing(StableId),
    /// 대상에 없는 asset.
    AssetMissing(StableId),
}

impl PasteDiagnostic {
    /// 진단 코드.
    pub fn code(&self) -> &'static str {
        match self {
            PasteDiagnostic::CrossDocument => "cross-document",
            PasteDiagnostic::ComponentMissing(_) => "component-missing",
            PasteDiagnostic::AssetMissing(_) => "asset-missing",
        }
    }
}

/// 클립보드 오류.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipboardError {
    /// 선택이 없습니다.
    Empty,
    /// 봉투 형식이 맞지 않습니다.
    Malformed(&'static str),
    /// 잠긴 root가 있어 Cut할 수 없습니다.
    Locked(StableId),
    /// 붙여 넣을 수 없습니다(진단 목록).
    Rejected(Vec<PasteDiagnostic>),
    /// 변형·기록 오류.
    Transform(TransformError),
}

impl From<TransformError> for ClipboardError {
    fn from(error: TransformError) -> ClipboardError {
        ClipboardError::Transform(error)
    }
}

/// 봉투의 root 하나.
#[derive(Clone, Debug)]
pub struct ClipboardRoot {
    /// 복사할 때의 부모.
    pub parent: StableId,
    /// 복사할 때의 world 경계(scene에 없던 node는 `None`).
    pub bounds: Option<[f32; 4]>,
    /// subtree 스냅샷.
    pub subtree: ClipboardSubtree,
}

/// Stellar 전용 pasteboard 봉투.
#[derive(Clone, Debug)]
pub struct CanvasClipboard {
    /// 출처 프로젝트.
    pub project: StableId,
    /// 출처 문서.
    pub document: StableId,
    /// 출처 View.
    pub view: StableId,
    /// 선택 root(선택 순서).
    pub roots: Vec<ClipboardRoot>,
    /// subtree가 참조하는 Component 정의(오름차순).
    pub components: Vec<StableId>,
    /// subtree가 참조하는 asset(오름차순).
    pub assets: Vec<StableId>,
}

/// Copy·Cut 결과: 셸이 pasteboard에 쓸 두 형식.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copied {
    /// [`STELLAR_PASTEBOARD_TYPE`] bytes.
    pub envelope: Vec<u8>,
    /// 일반 텍스트(Text가 없으면 `None`).
    pub text: Option<String>,
}

fn subtree_nodes(model: &DocumentModel, root: StableId) -> Vec<StableId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        out.push(node);
        let mut children = model.children_in_order(node);
        children.reverse();
        stack.extend(children);
    }
    out
}

/// root들의 subtree에서 Text 내용을 문서 순서로 줄바꿈 이어 붙입니다. Text가 없으면 `None`입니다.
pub fn plain_text(model: &DocumentModel, roots: &[StableId]) -> Option<String> {
    let texts: Vec<&str> = roots
        .iter()
        .flat_map(|&root| subtree_nodes(model, root))
        .filter_map(|node| match model.payload(node) {
            Some(NodePayload::Text(text)) => Some(text.content()),
            _ => None,
        })
        .collect();
    (!texts.is_empty()).then(|| texts.join("\n"))
}

fn references(model: &DocumentModel, roots: &[StableId]) -> (Vec<StableId>, Vec<StableId>) {
    let mut components = std::collections::BTreeSet::new();
    let mut assets = std::collections::BTreeSet::new();
    for &root in roots {
        for node in subtree_nodes(model, root) {
            match model.payload(node) {
                Some(NodePayload::ComponentInstance(instance)) => {
                    components.insert(instance.component().id());
                }
                Some(NodePayload::Image(image)) => {
                    assets.insert(image.asset.id());
                }
                _ => {}
            }
        }
    }
    (
        components.into_iter().collect(),
        assets.into_iter().collect(),
    )
}

impl CanvasClipboard {
    /// 지금 선택을 봉투로 만듭니다. 잠긴 node도 복사합니다.
    pub fn capture(
        window: &ProjectWindowSession,
        scene: &CanvasScene,
    ) -> Result<CanvasClipboard, ClipboardError> {
        let view = window.tabs().active_view().ok_or(TransformError::Stale)?;
        let (model, _root, roots) = selection_roots(window)?;
        if roots.is_empty() {
            return Err(ClipboardError::Empty);
        }
        if roots.len() > MAX_ROOTS {
            return Err(ClipboardError::Malformed("root 수 상한"));
        }
        let mut list = Vec::with_capacity(roots.len());
        for &root in &roots {
            let subtree =
                ClipboardSubtree::capture(model, root).map_err(|_| TransformError::Stale)?;
            list.push(ClipboardRoot {
                parent: parent_of(model, root).ok_or(TransformError::Stale)?,
                bounds: node_geometry(scene, root).map(|g| g.world_bounds),
                subtree,
            });
        }
        let (components, assets) = references(model, &roots);
        Ok(CanvasClipboard {
            project: window.identity().project_id(),
            document: model.document_id(),
            view,
            roots: list,
            components,
            assets,
        })
    }

    /// 봉투 bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        for id in [self.project, self.document, self.view] {
            out.extend_from_slice(&id.to_bytes());
        }
        out.extend_from_slice(&(self.roots.len() as u32).to_le_bytes());
        for root in &self.roots {
            out.extend_from_slice(&root.parent.to_bytes());
            match root.bounds {
                Some(bounds) => {
                    out.push(1);
                    for v in bounds {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                }
                None => out.push(0),
            }
            let bytes = root.subtree.encode();
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        for list in [&self.components, &self.assets] {
            out.extend_from_slice(&(list.len() as u32).to_le_bytes());
            for id in list {
                out.extend_from_slice(&id.to_bytes());
            }
        }
        out
    }

    /// [`CanvasClipboard::encode`]의 역. 크기·형식을 검사하고 subtree는 closed command codec으로만 해석합니다.
    pub fn decode(bytes: &[u8]) -> Result<CanvasClipboard, ClipboardError> {
        if bytes.len() > MAX_ENVELOPE_BYTES || !bytes.starts_with(MAGIC) {
            return Err(ClipboardError::Malformed(
                "지원하지 않는 캔버스 클립보드 형식",
            ));
        }
        let mut input = Cursor {
            bytes,
            at: MAGIC.len(),
        };
        let project = input.id()?;
        let document = input.id()?;
        let view = input.id()?;
        let count = input.u32()? as usize;
        if count == 0 || count > MAX_ROOTS {
            return Err(ClipboardError::Malformed("root 수"));
        }
        let mut roots = Vec::with_capacity(count);
        for _ in 0..count {
            let parent = input.id()?;
            let bounds = match input.take(1)?[0] {
                0 => None,
                1 => {
                    let mut b = [0f32; 4];
                    for v in &mut b {
                        *v = f32::from_le_bytes(input.take(4)?.try_into().expect("4"));
                    }
                    if !b.iter().all(|v| v.is_finite()) {
                        return Err(ClipboardError::Malformed("경계"));
                    }
                    Some(b)
                }
                _ => return Err(ClipboardError::Malformed("경계 표지")),
            };
            let len = input.u32()? as usize;
            let subtree = ClipboardSubtree::decode(input.take(len)?)
                .map_err(|_| ClipboardError::Malformed("subtree"))?;
            roots.push(ClipboardRoot {
                parent,
                bounds,
                subtree,
            });
        }
        let mut lists = [Vec::new(), Vec::new()];
        for list in &mut lists {
            let n = input.u32()? as usize;
            if n > MAX_ENVELOPE_BYTES / 16 {
                return Err(ClipboardError::Malformed("참조 수"));
            }
            for _ in 0..n {
                list.push(input.id()?);
            }
        }
        if input.at != bytes.len() {
            return Err(ClipboardError::Malformed("남는 bytes"));
        }
        let [components, assets] = lists;
        Ok(CanvasClipboard {
            project,
            document,
            view,
            roots,
            components,
            assets,
        })
    }

    /// 대상 문서 `model`에 붙일 때의 진단. `known_component`·`known_asset`은 대상 프로젝트에 그 정의·asset이 있는지.
    pub fn diagnostics(
        &self,
        model: &DocumentModel,
        known_component: &dyn Fn(StableId) -> bool,
        known_asset: &dyn Fn(StableId) -> bool,
    ) -> Vec<PasteDiagnostic> {
        let mut out = Vec::new();
        if self.document != model.document_id() {
            out.push(PasteDiagnostic::CrossDocument);
        }
        out.extend(
            self.components
                .iter()
                .filter(|&&id| !known_component(id))
                .map(|&id| PasteDiagnostic::ComponentMissing(id)),
        );
        out.extend(
            self.assets
                .iter()
                .filter(|&&id| !known_asset(id))
                .map(|&id| PasteDiagnostic::AssetMissing(id)),
        );
        out
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ClipboardError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(ClipboardError::Malformed("bytes가 짧습니다"))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, ClipboardError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }

    fn id(&mut self) -> Result<StableId, ClipboardError> {
        Ok(StableId::from_bytes(self.take(16)?.try_into().expect("16")))
    }
}

/// Copy: 지금 선택의 두 형식.
pub fn copy(window: &ProjectWindowSession, scene: &CanvasScene) -> Result<Copied, ClipboardError> {
    let clipboard = CanvasClipboard::capture(window, scene)?;
    let (model, _, roots) = selection_roots(window)?;
    let envelope = clipboard.encode();
    if envelope.len() > MAX_ENVELOPE_BYTES {
        return Err(ClipboardError::Malformed("봉투 크기 상한"));
    }
    Ok(Copied {
        envelope,
        text: plain_text(model, &roots),
    })
}

/// Cut: Copy 뒤 root들을 지우는 `DeleteSubtree`를 history 한 항목으로 기록합니다. 잠긴 root·자손이 있으면 전체를
/// 거부합니다.
pub fn cut(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
) -> Result<Copied, ClipboardError> {
    let copied = copy(window, scene)?;
    let (model, _, roots) = selection_roots(window)?;
    for &root in &roots {
        edit_permission(model, root, EditIntent::Delete)
            .map_err(|_| ClipboardError::Locked(root))?;
    }
    let commands = roots
        .iter()
        .map(|&root| Command::DeleteSubtree(DeleteSubtree { root }))
        .collect();
    crate::canvas::transform::commit(window, commands, history_label::CUT)?;
    Ok(copied)
}

/// 붙여 넣은 자리.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PastePlacement {
    /// 선택한 컨테이너 안(원래 local 위치).
    InsideSelection(StableId),
    /// 원래 부모·원래 위치(원본 위치가 화면에 보임).
    SamePlace,
    /// View root 아래 화면 가운데.
    ViewCenter,
}

/// 붙여넣기 결과.
#[derive(Clone, Debug, PartialEq)]
pub struct Pasted {
    /// 새 root(봉투 순서). 새 선택입니다.
    pub roots: Vec<StableId>,
    /// 고른 자리.
    pub placement: PastePlacement,
    /// 거부하지 않은 진단(같은 문서의 지운 asset 등).
    pub diagnostics: Vec<PasteDiagnostic>,
}

fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// Paste. `visible`은 지금 화면의 world 영역입니다. 다른 문서 봉투는 진단과 함께 거부합니다.
pub fn paste(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    bytes: &[u8],
    visible: [f32; 4],
    known_component: &dyn Fn(StableId) -> bool,
    known_asset: &dyn Fn(StableId) -> bool,
) -> Result<Pasted, ClipboardError> {
    let clipboard = CanvasClipboard::decode(bytes)?;
    let view = window.tabs().active_view().ok_or(TransformError::Stale)?;
    let model = window.document().ok_or(TransformError::Stale)?.model();
    let diagnostics = clipboard.diagnostics(model, known_component, known_asset);
    if diagnostics.contains(&PasteDiagnostic::CrossDocument) {
        return Err(ClipboardError::Rejected(diagnostics));
    }
    let root = view_root(model, view).ok_or(TransformError::Stale)?;
    let editable_frame = |id: StableId| {
        matches!(model.payload(id), Some(NodePayload::Frame(_)))
            && edit_permission(model, id, EditIntent::Edit).is_ok()
    };
    let selected = window.view().selection().ids();
    let union_bounds = union(clipboard.roots.iter().filter_map(|r| r.bounds));
    let placement = match selected {
        [one] if editable_frame(*one) => PastePlacement::InsideSelection(*one),
        _ if clipboard.view == view
            && union_bounds.is_some_and(|b| overlaps(b, visible))
            && clipboard.roots.iter().all(|r| editable_frame(r.parent)) =>
        {
            PastePlacement::SamePlace
        }
        _ => PastePlacement::ViewCenter,
    };
    let parent_of_root = |r: &ClipboardRoot| match placement {
        PastePlacement::InsideSelection(p) => p,
        PastePlacement::SamePlace => r.parent,
        PastePlacement::ViewCenter => root,
    };
    let mut ids = FreshIds::default();
    let mut inserts: Vec<InsertNode> = Vec::with_capacity(clipboard.roots.len());
    let mut last_key: std::collections::BTreeMap<StableId, OrderKey> = Default::default();
    for r in &clipboard.roots {
        let parent = parent_of_root(r);
        let key = match last_key.get(&parent) {
            Some(previous) => OrderKey::between(Some(previous), None),
            None => model.next_child_order_key(parent),
        };
        last_key.insert(parent, key.clone());
        let insert = r
            .subtree
            .plan_with(model, parent, key, &mut || ids.fresh())
            .map_err(|_| TransformError::Stale)?;
        inserts.push(insert);
    }
    ids.finish()?;
    let new_roots: Vec<StableId> = inserts.iter().map(|i| i.nodes[0].id).collect();
    let mut commands: Vec<Command> = inserts.iter().cloned().map(Command::InsertNode).collect();
    if placement == PastePlacement::ViewCenter {
        if let Some(b) = union_bounds {
            // 상자 합의 중심을 화면 가운데로: world 이동을 View root local로 바꿔 붙여 넣은 root마다 평행 이동합니다.
            let dx = visible[0] + visible[2] / 2.0 - (b[0] + b[2] / 2.0);
            let dy = visible[1] + visible[3] / 2.0 - (b[1] + b[3] / 2.0);
            let to_local = node_geometry(scene, root)
                .and_then(|g| g.local.then(&g.parent_world).ok())
                .and_then(|w| w.inverse())
                .ok_or(TransformError::Stale)?;
            let (a, bb, c, d, _, _) = to_local.components();
            let local = [a * dx + c * dy, bb * dx + d * dy];
            // 이동 쓰기는 배치 쓰기 한 곳(place)이 정합니다. 흐름 컨테이너 root면 흐름이 자리를 정하므로 옮기지 않습니다.
            let mut move_ids = FreshIds::default();
            if !crate::canvas::flow_drag::is_flow_container(model, root) {
                for insert in &inserts {
                    if let Some(command) =
                        translate_inserted(insert, local[0], local[1], &mut || move_ids.fresh())?
                    {
                        commands.push(command);
                    }
                }
            }
            move_ids.finish()?;
        }
    }
    let tx = transaction(window, commands)?;
    let primary = new_roots.last().copied();
    window
        .apply_and_select(
            &tx,
            Selection::new(new_roots.clone(), primary),
            history_label::PASTE,
        )
        .map_err(|error| TransformError::Edit(format!("{error:?}")))?;
    Ok(Pasted {
        roots: new_roots,
        placement,
        diagnostics,
    })
}

/// 아직 문서에 없는 붙여넣기 root의 평행 이동 command. 배치 쓰기 규칙(`place::translate`)을 root record와 그 modifier만
/// 담은 임시 모델에서 계획합니다(문서 전체를 복제하지 않음). 부모의 잠금·흐름 판정은 호출자가 이미 했습니다.
fn translate_inserted(
    insert: &InsertNode,
    dx: f32,
    dy: f32,
    new_id: &mut dyn FnMut() -> StableId,
) -> Result<Option<Command>, TransformError> {
    use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
    let stale = |_| TransformError::Stale;
    let mut record = insert.nodes.first().ok_or(TransformError::Stale)?.clone();
    // 임시 모델의 정의·root id는 붙여 넣는 id와 겹치지 않는 고정 값입니다.
    let scratch = |n: u8| {
        StableId::from_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0x70, 0, 0x80, 0, 0, 0, 0, 0, 0, n,
        ])
    };
    let (doc, page, view, root) = (scratch(1), scratch(2), scratch(3), scratch(4));
    let mut definition = DocumentDefinition::new(doc, String::new());
    let mut page_definition = PageDefinition::new(page, String::new());
    page_definition
        .add_view(ViewDefinition::new(view, String::new(), root))
        .map_err(|_| TransformError::Stale)?;
    definition
        .add_page(page_definition)
        .map_err(|_| TransformError::Stale)?;
    let mut model = DocumentModel::new(definition);
    model
        .insert_node(stellar_document::records::NodeRecord {
            id: root,
            parent: None,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload: NodePayload::Frame(stellar_document::frame::FramePayload::default()),
        })
        .map_err(stale)?;
    record.parent = Some(root);
    record.visible = true;
    record.locked = false;
    let node = record.id;
    model.insert_node(record).map_err(stale)?;
    if let Some(stack) = insert.modifier_stacks.iter().find(|s| s.node == node) {
        model.set_modifier_stack(stack.clone()).map_err(stale)?;
    }
    Ok(stellar_commands::arrange::place::translate(
        &model, node, dx, dy, new_id,
    )?)
}

#[cfg(test)]
mod tests;
