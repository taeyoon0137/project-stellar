//! dirty subtree만 다시 만드는 증분 scene build (XI.2.5, XI.2.6, XII.3.7, ADR-0084 결정 3·4·6).
//!
//! [`IncrementalScene`]은 [`SceneSource`]와 마지막으로 만든 node(`Arc<SceneNode>`)를 key별로 기억합니다. 갱신은
//! [`SceneDirtySet`](자기 입력이 바뀐 node)과 source 편집을 받아 아래 규칙으로 tree를 다시 걷습니다.
//!
//! # node별 판정
//!
//! 부모 scope(world transform·clip·숨김·잠금)를 이번 build의 scope와 비교합니다.
//!
//! | 조건 | 결과 | 집계 |
//! |---|---|---|
//! | 자신·자손이 dirty가 아니고, preview가 아니며, scope와 시작 z-order가 같음 | 이전 node `Arc`를 그대로 씀 | `shared` |
//! | 위와 같지만 앞쪽 구조 변경으로 시작 z-order만 다름 | content·paint·clip `Arc`를 공유한 채 z만 다시 매김 | `renumbered` |
//! | 자신이 dirty·preview·[`SceneDirtyKind::Subtree`] 아래이거나 scope가 다름 | builder로 다시 평가 | `rebuilt` |
//! | 자손만 dirty | 자기 값은 다시 평가하되 깨끗한 자식은 공유 | `copied` |
//!
//! z-order는 [`crate::builder`]와 같은 pre-order 연속 번호입니다. 그래서 전체 build와 증분 build의 결과는 같은
//! 입력에서 [`SceneSnapshot::semantic_hash`]가 같습니다. 단일 property 편집은 node 수가 바뀌지 않으므로 뒤쪽 node를
//! 모두 공유합니다. 삽입·삭제는 뒤쪽 node의 z만 다시 매기며, 이때도 paint stack·clip·content(image·text resource
//! 참조, path `Arc`)는 공유합니다.
//!
//! # Boolean
//!
//! [`SourceNode::boolean`]이 있는 Shape·Path는 host geometry와 operand로 [`evaluate_boolean`] 결과 path를 그립니다
//! (채우기 규칙 NonZero). 평가가 실패하면 host geometry를 그대로 그리고 `boolean_failures`에 셉니다.
//!
//! # transient preview (XII.3.7)
//!
//! [`IncrementalScene::set_preview`]는 상호작용 중 key별 대체 입력([`ScenePreview`])을 주입해 새 snapshot을 만듭니다.
//! document revision은 바꾸지 않고 scene revision만 올립니다(아키텍처 §9.3의 세대 번호). preview는 source와 history에
//! 들어가지 않습니다. pointer up에서 호출자가 `CommandTransaction`을 확정하고 그 결과를 source 편집으로 반영한 뒤
//! [`IncrementalScene::clear_preview`]로 preview를 지웁니다. 취소는 preview만 지웁니다.
//!
//! # trace (XI.2.6)
//!
//! 갱신마다 [`SceneBuildTrace`](정적 event 이름과 숫자만)를 돌려줍니다. 시각은 주입한 [`MonotonicClock`]으로 잽니다.
//! node 이름·text·경로는 담지 않습니다.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};
use stellar_core_types::value::Transform2D;

use crate::boolean::{evaluate_boolean, BooleanOperand};
use crate::builder::{PathInput, SceneBuildError, SceneBuilder, SceneScope};
use crate::dirty::{SceneDirtyKind, SceneDirtySet};
use crate::geometry::FillRule;
use crate::node::{SceneNode, SceneTreeError};
use crate::revision::{SceneRevisionError, SceneRevisionMap};
use crate::snapshot::SceneSnapshot;
use crate::source::{SceneKey, SceneSource, SourceEdit, SourceError, SourceKind, SourceNode};
use crate::stack::ClipStackRef;

/// 증분 build 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncrementalError {
    /// source tree 편집 오류.
    Source(SourceError),
    /// node 평가 오류.
    Build(SceneBuildError),
    /// z-order 불변식 오류.
    Tree(SceneTreeError),
    /// revision 오류.
    Revision(SceneRevisionError),
    /// preview가 source에 없는 node를 가리킵니다.
    UnknownPreviewNode {
        /// key.
        key: SceneKey,
    },
    /// preview 입력의 종류가 source node와 다릅니다(Frame ↔ Shape 등).
    PreviewKindMismatch {
        /// key.
        key: SceneKey,
    },
}

impl fmt::Display for IncrementalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IncrementalError::Source(error) => write!(f, "scene source 오류: {error}"),
            IncrementalError::Build(error) => write!(f, "scene build 오류: {error}"),
            IncrementalError::Tree(error) => write!(f, "scene tree 오류: {error}"),
            IncrementalError::Revision(error) => write!(f, "scene revision 오류: {error}"),
            IncrementalError::UnknownPreviewNode { key } => {
                write!(f, "preview node {}가 scene source에 없습니다", key.id())
            }
            IncrementalError::PreviewKindMismatch { key } => {
                write!(f, "preview node {}의 종류가 source와 다릅니다", key.id())
            }
        }
    }
}

impl std::error::Error for IncrementalError {}

impl From<SceneBuildError> for IncrementalError {
    fn from(error: SceneBuildError) -> IncrementalError {
        IncrementalError::Build(error)
    }
}

impl From<SceneTreeError> for IncrementalError {
    fn from(error: SceneTreeError) -> IncrementalError {
        IncrementalError::Tree(error)
    }
}

/// 상호작용 중 key별 대체 입력(XII.3.7). source와 history에 들어가지 않습니다.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScenePreview {
    nodes: BTreeMap<SceneKey, SourceKind>,
}

impl ScenePreview {
    /// 빈 preview.
    pub fn new() -> ScenePreview {
        ScenePreview::default()
    }

    /// `key`의 입력을 `kind`로 대체합니다. 종류는 source node와 같아야 합니다.
    pub fn set(&mut self, key: SceneKey, kind: SourceKind) {
        self.nodes.insert(key, kind);
    }

    /// 대체 입력.
    pub fn get(&self, key: SceneKey) -> Option<&SourceKind> {
        self.nodes.get(&key)
    }

    /// 대체한 key 수.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 대체한 key.
    pub fn keys(&self) -> impl Iterator<Item = SceneKey> + '_ {
        self.nodes.keys().copied()
    }
}

/// build 한 번의 집계(scene node 기준, Fragment는 node가 아님).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SceneBuildStats {
    /// snapshot 전체 node 수.
    pub nodes: u64,
    /// 다시 평가한 node 수.
    pub rebuilt: u64,
    /// 자손 변경 때문에 다시 만든 조상 node 수(path copying).
    pub copied: u64,
    /// z-order만 다시 매긴 node 수.
    pub renumbered: u64,
    /// 이전 `Arc`를 그대로 쓴 node 수.
    pub shared: u64,
    /// 그대로 쓴 subtree root 수.
    pub shared_subtrees: u64,
    /// Boolean 평가 실패로 host geometry를 그린 node 수.
    pub boolean_failures: u64,
    /// dirty 입력 종류별 개수 `[paint, geometry, subtree]`.
    pub dirty: [u64; 3],
    /// 전체 build였는지(첫 build).
    pub full: bool,
}

/// scene build trace(XI.2.6). 정적 이름과 숫자만 담습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneBuildTrace {
    /// 만든 snapshot의 scene revision.
    pub scene_revision: u64,
    /// 시작 시각(ns).
    pub start_nanos: u64,
    /// 끝 시각(ns, T4 scene 쪽 후보).
    pub end_nanos: u64,
    /// 집계.
    pub stats: SceneBuildStats,
}

impl SceneBuildTrace {
    /// 정적 event 이름.
    pub const EVENT: &'static str = "scene.build";

    /// field 이름(순서 고정). diagnostics trace schema와 같은 목록입니다.
    pub const FIELDS: [&'static str; 13] = [
        "scene_revision",
        "start_ns",
        "end_ns",
        "duration_ns",
        "nodes",
        "rebuilt",
        "copied",
        "renumbered",
        "shared",
        "dirty_paint",
        "dirty_geometry",
        "dirty_subtree",
        "full",
    ];

    /// (정적 field 이름, 값) 목록.
    pub fn fields(&self) -> [(&'static str, u64); 13] {
        let values = [
            self.scene_revision,
            self.start_nanos,
            self.end_nanos,
            self.end_nanos.saturating_sub(self.start_nanos),
            self.stats.nodes,
            self.stats.rebuilt,
            self.stats.copied,
            self.stats.renumbered,
            self.stats.shared,
            self.stats.dirty[0],
            self.stats.dirty[1],
            self.stats.dirty[2],
            u64::from(self.stats.full),
        ];
        let mut out = [("", 0u64); 13];
        for (slot, (name, value)) in out.iter_mut().zip(Self::FIELDS.iter().zip(values)) {
            *slot = (name, value);
        }
        out
    }

    /// 고정 형식 한 줄(`scene.build name=value ...`).
    pub fn to_line(&self) -> String {
        let mut line = String::from(Self::EVENT);
        for (name, value) in self.fields() {
            let _ = write!(line, " {name}={value}");
        }
        line
    }
}

/// 이전 build 결과.
#[derive(Debug, Clone)]
struct Built {
    /// 만든 scene node(Fragment는 펼친 자식들, 그 밖은 하나).
    output: Vec<Arc<SceneNode>>,
    /// 받은 부모 scope.
    scope: ScopeKey,
    /// 첫 node의 z-order.
    first_z: u64,
    /// 만든 scene node 수(subtree 포함).
    count: u64,
}

/// 부모 scope 비교 값.
#[derive(Debug, Clone, PartialEq)]
struct ScopeKey {
    world: Transform2D,
    clip: ClipStackRef,
    hidden: bool,
    locked: bool,
}

impl ScopeKey {
    fn of(scope: &SceneScope) -> ScopeKey {
        ScopeKey {
            world: scope.world,
            clip: scope.clip.clone(),
            hidden: scope.hidden,
            locked: scope.locked,
        }
    }
}

/// 한 build의 상태.
struct Pass<'a> {
    dirty: &'a SceneDirtySet,
    /// dirty·preview node의 조상(자손이 바뀐 node).
    ancestors: HashSet<SceneKey>,
    preview: &'a ScenePreview,
    stats: SceneBuildStats,
}

/// dirty subtree만 다시 만드는 scene builder(모듈 문서).
#[derive(Debug)]
pub struct IncrementalScene {
    source: SceneSource,
    built: HashMap<SceneKey, Built>,
    revisions: SceneRevisionMap,
    snapshot: SceneSnapshot,
    preview: ScenePreview,
}

impl IncrementalScene {
    /// `source` 전체를 build합니다.
    pub fn build(
        source: SceneSource,
        document: DocumentRevision,
        clock: &dyn MonotonicClock,
    ) -> Result<(IncrementalScene, SceneBuildTrace), IncrementalError> {
        let revisions = SceneRevisionMap::new();
        // 첫 snapshot이 scene revision 1을 받도록 자리 표시 빈 snapshot은 revision을 소비하지 않습니다(0 = 아직 없음).
        let empty = SceneSnapshot::new(document, SceneRevision(0), Vec::new())?;
        let mut scene = IncrementalScene {
            source,
            built: HashMap::new(),
            revisions,
            snapshot: empty,
            preview: ScenePreview::new(),
        };
        let trace = scene.run(document, &SceneDirtySet::new(), true, clock)?;
        Ok((scene, trace))
    }

    /// source 편집과 dirty set을 반영해 새 snapshot을 만듭니다.
    ///
    /// 편집한(Upsert) key는 dirty set에 없으면 [`SceneDirtyKind::Geometry`]로 봅니다. 지운 key의 이전 결과는 버립니다.
    pub fn update(
        &mut self,
        document: DocumentRevision,
        edits: &[SourceEdit],
        dirty: &SceneDirtySet,
        clock: &dyn MonotonicClock,
    ) -> Result<SceneBuildTrace, IncrementalError> {
        self.source.apply(edits).map_err(IncrementalError::Source)?;
        let mut dirty = dirty.clone();
        for edit in edits {
            match edit {
                SourceEdit::Upsert(key, _) => {
                    if dirty.get(key.id()).is_none() {
                        dirty.insert(key.id(), SceneDirtyKind::Geometry);
                    }
                }
                SourceEdit::Remove(key) => {
                    self.built.remove(key);
                }
                SourceEdit::SetRoots(_) => {}
            }
        }
        self.run(document, &dirty, false, clock)
    }

    /// transient preview를 주입한 snapshot을 만듭니다(document revision 유지).
    pub fn set_preview(
        &mut self,
        preview: ScenePreview,
        clock: &dyn MonotonicClock,
    ) -> Result<SceneBuildTrace, IncrementalError> {
        for (key, kind) in &preview.nodes {
            let node = self
                .source
                .get(*key)
                .ok_or(IncrementalError::UnknownPreviewNode { key: *key })?;
            if std::mem::discriminant(&node.kind) != std::mem::discriminant(kind) {
                return Err(IncrementalError::PreviewKindMismatch { key: *key });
            }
        }
        // 이전 preview node는 원래 입력으로 돌아가야 하므로 dirty로 봅니다.
        let mut dirty = SceneDirtySet::new();
        for key in self.preview.keys() {
            dirty.insert(key.id(), SceneDirtyKind::Geometry);
        }
        self.preview = preview;
        let document = self.snapshot.document_revision();
        self.run(document, &dirty, false, clock)
    }

    /// preview를 지운 snapshot을 만듭니다(취소 또는 확정 뒤).
    pub fn clear_preview(
        &mut self,
        clock: &dyn MonotonicClock,
    ) -> Result<SceneBuildTrace, IncrementalError> {
        self.set_preview(ScenePreview::new(), clock)
    }

    /// 마지막 snapshot.
    pub const fn snapshot(&self) -> &SceneSnapshot {
        &self.snapshot
    }

    /// 현재 source tree.
    pub const fn source(&self) -> &SceneSource {
        &self.source
    }

    /// 현재 preview.
    pub const fn preview(&self) -> &ScenePreview {
        &self.preview
    }

    fn run(
        &mut self,
        document: DocumentRevision,
        dirty: &SceneDirtySet,
        full: bool,
        clock: &dyn MonotonicClock,
    ) -> Result<SceneBuildTrace, IncrementalError> {
        let start_nanos = clock.now_nanos();
        let preview = std::mem::take(&mut self.preview);
        let mut ancestors = HashSet::new();
        let changed = dirty
            .iter()
            .map(|(id, _)| SceneKey::node(id))
            .chain(preview.keys());
        for key in changed {
            let mut current = self.source.parent(key);
            while let Some(parent) = current {
                if !ancestors.insert(parent) {
                    break;
                }
                current = self.source.parent(parent);
            }
        }
        let mut pass = Pass {
            dirty,
            ancestors,
            preview: &preview,
            stats: SceneBuildStats {
                dirty: dirty.counts(),
                full,
                ..SceneBuildStats::default()
            },
        };
        let result = self.build_roots(&mut pass);
        let stats = pass.stats;
        self.preview = preview;
        let roots = result?;
        let snapshot = self
            .revisions
            .generate(document, roots)
            .map_err(|error| match error {
                crate::revision::SceneGenerateError::Revision(e) => IncrementalError::Revision(e),
                crate::revision::SceneGenerateError::Tree(e) => IncrementalError::Tree(e),
                crate::revision::SceneGenerateError::Edit(e) => match e {
                    crate::node::SceneEditError::Tree(t) => IncrementalError::Tree(t),
                    _ => IncrementalError::Tree(SceneTreeError::ZOrderNotIncreasing {
                        previous: crate::node::ZOrder(0),
                        next: crate::node::ZOrder(0),
                    }),
                },
            })?;
        let mut stats = stats;
        stats.nodes = snapshot.node_count();
        let scene_revision = snapshot.scene_revision().0;
        self.snapshot = snapshot;
        Ok(SceneBuildTrace {
            scene_revision,
            start_nanos,
            end_nanos: clock.now_nanos(),
            stats,
        })
    }

    fn build_roots(
        &mut self,
        pass: &mut Pass<'_>,
    ) -> Result<Vec<Arc<SceneNode>>, IncrementalError> {
        let mut builder = SceneBuilder::new();
        let mut root = builder.root_scope();
        let roots: Vec<SceneKey> = self.source.roots().to_vec();
        for key in roots {
            self.visit(key, &mut root, &mut builder, false, pass)?;
        }
        Ok(builder.finish(root))
    }

    /// `key` subtree를 `parent` scope에 쌓습니다.
    fn visit(
        &mut self,
        key: SceneKey,
        parent: &mut SceneScope,
        builder: &mut SceneBuilder,
        forced: bool,
        pass: &mut Pass<'_>,
    ) -> Result<(), IncrementalError> {
        let dirty = pass.dirty.get(key.id());
        let previewed = pass.preview.get(key).is_some();
        let scope = ScopeKey::of(parent);
        let first_z = builder.next_z();
        let clean = !forced && dirty.is_none() && !previewed && !pass.ancestors.contains(&key);
        if clean {
            if let Some(previous) = self.built.get(&key).filter(|b| b.scope == scope) {
                if previous.first_z == first_z {
                    parent.nodes.extend(previous.output.iter().cloned());
                    builder.skip(previous.count)?;
                    pass.stats.shared += previous.count;
                    pass.stats.shared_subtrees += 1;
                    return Ok(());
                }
                let mut output = Vec::with_capacity(previous.output.len());
                for node in &previous.output {
                    output.push(renumber(node, builder)?);
                }
                let count = previous.count;
                pass.stats.renumbered += count;
                parent.nodes.extend(output.iter().cloned());
                self.built.insert(
                    key,
                    Built {
                        output,
                        scope,
                        first_z,
                        count,
                    },
                );
                return Ok(());
            }
        }
        let evaluated = forced
            || dirty.is_some()
            || previewed
            || self.built.get(&key).is_none_or(|b| b.scope != scope);
        let force_children = forced || dirty == Some(SceneDirtyKind::Subtree);
        let node: SourceNode = self
            .source
            .get(key)
            .cloned()
            .ok_or(IncrementalError::Source(SourceError::MissingNode { key }))?;
        let kind = pass.preview.get(key).cloned().unwrap_or(node.kind);
        let start = parent.nodes.len();
        match &kind {
            SourceKind::Frame(input) => {
                let mut frame = builder.begin_frame(parent, input)?;
                for child in &node.children {
                    self.visit(*child, frame.scope_mut(), builder, force_children, pass)?;
                }
                builder.finish_frame(parent, frame)?;
            }
            SourceKind::Fragment(input) => {
                let mut fragment = builder.begin_fragment(parent, input);
                for child in &node.children {
                    self.visit(*child, fragment.scope_mut(), builder, force_children, pass)?;
                }
                builder.finish_fragment(parent, fragment);
            }
            SourceKind::Shape(input) => match &node.boolean {
                Some(boolean) => {
                    let host = BooleanOperand {
                        geometry: input.geometry.clone(),
                        fill_rule: FillRule::NonZero,
                        transform: Transform2D::identity(),
                    };
                    let path = boolean_path(host, boolean, &input.node, input.layout_rect, pass);
                    builder.build_path(parent, &path)?;
                }
                None => {
                    builder.build_shape(parent, input)?;
                }
            },
            SourceKind::Path(input) => match &node.boolean {
                Some(boolean) => {
                    let host = BooleanOperand {
                        geometry: input.geometry.clone(),
                        fill_rule: input.fill_rule,
                        transform: Transform2D::identity(),
                    };
                    let path = boolean_path(host, boolean, &input.node, input.layout_rect, pass);
                    builder.build_path(parent, &path)?;
                }
                None => {
                    builder.build_path(parent, input)?;
                }
            },
            SourceKind::Text(input) => {
                builder.build_text(parent, input)?;
            }
            SourceKind::Image(input) => {
                builder.build_image(parent, input)?;
            }
        }
        let count = builder.next_z() - first_z;
        if !matches!(kind, SourceKind::Fragment(_)) {
            if evaluated {
                pass.stats.rebuilt += 1;
            } else {
                pass.stats.copied += 1;
            }
        }
        let output = parent.nodes[start..].to_vec();
        self.built.insert(
            key,
            Built {
                output,
                scope,
                first_z,
                count,
            },
        );
        Ok(())
    }
}

/// Boolean 결과 path 입력. 실패하면 host geometry를 씁니다.
fn boolean_path(
    host: BooleanOperand,
    boolean: &crate::source::BooleanSource,
    node: &crate::builder::NodeInput,
    layout_rect: stellar_core_types::value::Rect,
    pass: &mut Pass<'_>,
) -> PathInput {
    let fill_rule = host.fill_rule;
    let fallback = host.geometry.clone();
    let mut operands = Vec::with_capacity(boolean.operands.len() + 1);
    operands.push(host);
    operands.extend(boolean.operands.iter().cloned());
    let (geometry, fill_rule) = match evaluate_boolean(boolean.op, &operands) {
        Ok(geometry) => (geometry, FillRule::NonZero),
        Err(_) => {
            pass.stats.boolean_failures += 1;
            (fallback, fill_rule)
        }
    };
    PathInput {
        node: node.clone(),
        layout_rect,
        geometry,
        fill_rule,
    }
}

/// z-order만 다시 매긴 subtree. content·paint·clip은 공유합니다.
fn renumber(
    node: &Arc<SceneNode>,
    builder: &mut SceneBuilder,
) -> Result<Arc<SceneNode>, IncrementalError> {
    let mut spec = node.spec();
    spec.z = builder.allocate_z()?;
    let mut children = Vec::with_capacity(node.child_count());
    for child in node.children() {
        children.push(renumber(child, builder)?);
    }
    Ok(Arc::new(SceneNode::new(spec, children)?))
}

#[cfg(test)]
mod tests;
