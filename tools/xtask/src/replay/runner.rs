//! core-only 입력 재생 runner (XVI.2.2, ADR-0084 결정 6·9).
//!
//! UI 없이 재생 event마다 아래 순서를 실행하고 T0~T5를 기록합니다.
//!
//! | stage | 재생에서 하는 일 |
//! |---|---|
//! | T0 | event의 ManualClock timeline 시각 |
//! | T1 | 좌표 변환, 누름이면 canvas hit test(transient handle 먼저), 상호작용 상태 기계 |
//! | T2 | 뗌 확정이면 `CommandTransaction` 하나 실행(이동은 preview만이라 command 없음) |
//! | T3 | 확정 command가 바꾼 node의 증분 layout과 layout rect 무효화 |
//! | T4 | scene 갱신(preview 주입 또는 확정 반영), 확정 뒤 공간 인덱스, render graph compile·batching |
//! | T5 | core에는 GPU 제출이 없어 T4와 같습니다 |
//!
//! T1~T5는 T0에 그 event 처리 시작부터의 실제 경과를 더한 값입니다. 그래서 trace는 ManualClock timeline에 맞춰
//! 정렬되고 구간 값은 실제 계산 시간입니다. 같은 재생은 trace 수와 마지막 scene semantic hash가 같습니다.

use std::time::Instant;

use stellar_bench_fixtures::limits::{visible_nodes, VisibleFixture};
use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::request::CommandRequest;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::{Size2, Transform2D, Vec2};
use stellar_diagnostics::interaction::{
    CacheState, CompletedTrace, DocumentSizeBucket, InteractionTrace, InteractionTraceIds, Stage,
    TraceDimensions, VisibleNodeBucket,
};
use stellar_diagnostics::trace_schema::{validate, TraceRecord};
use stellar_document::frame::keys as frame_keys;
use stellar_document::model::DocumentModel;
use stellar_renderer_core::batching::TargetInfo;
use stellar_renderer_core::clip_plan::ClipCapabilities;
use stellar_renderer_core::graph_trace::{
    compile_frame_traced_with, RenderGraphCompileTrace, TraceClock,
};
use stellar_renderer_core::pass_compiler::{CompileInput, EffectBoundaries, FrameCompiler};
use stellar_renderer_core::render_graph::{ClearColor, SampleCount, TextureFormat};
use stellar_runtime::canvas::scene_source::{
    document_scene_source, scene_edits, scene_invalidations, DocumentSceneSource,
    SceneSourceOptions,
};
use stellar_scene::dirty::SceneDirtySet;
use stellar_scene::incremental::{IncrementalScene, SceneBuildTrace, ScenePreview};
use stellar_scene::interaction::{
    InteractionConfig, InteractionController, InteractionOutcome, InteractionPreview,
    InteractionStep, InteractionTool, PressContext, PressTarget,
};
use stellar_scene::narrow_phase::{LayoutBoxesOnly, NarrowPhaseOptions};
use stellar_scene::source::{SceneKey, SourceEdit, SourceKind};
use stellar_scene::spatial::SceneSpatialIndex;
use stellar_scene::transient::{hit_test_canvas, CanvasHit, HandleMetrics, TransientLayer};
use stellar_scene::visibility::Viewport;

use super::file::{FixtureSpec, ReplayFile, ReplayInput, ReplayScenario};
use super::layout::ReplayLayout;

/// 프로세스 단조 시계(실제 경과 측정).
#[derive(Debug, Clone, Copy)]
pub struct WallClock {
    origin: Instant,
}

impl WallClock {
    /// 지금을 0으로 하는 시계.
    pub fn new() -> WallClock {
        WallClock {
            origin: Instant::now(),
        }
    }
}

impl Default for WallClock {
    fn default() -> WallClock {
        WallClock::new()
    }
}

impl MonotonicClock for WallClock {
    fn now_nanos(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

impl TraceClock for WallClock {
    fn now_nanos(&self) -> u64 {
        MonotonicClock::now_nanos(self)
    }
}

/// 재생 결과.
#[derive(Debug, Clone)]
pub struct ReplayRun {
    /// scenario.
    pub scenario: ReplayScenario,
    /// 완료 trace(event 순).
    pub traces: Vec<CompletedTrace>,
    /// 마지막 scene semantic hash.
    pub scene_hash: u128,
    /// 마지막 document revision.
    pub document_revision: DocumentRevision,
    /// scene build trace 수.
    pub scene_builds: usize,
    /// render graph compile 수.
    pub graph_compiles: usize,
    /// scene build 시간 p99(ns, 첫 전체 build 제외).
    pub scene_build_p99_nanos: u64,
    /// render graph compile·batching 시간 p99(ns).
    pub graph_compile_p99_nanos: u64,
}

/// fixture를 만듭니다.
pub fn load_fixture(spec: &FixtureSpec) -> Result<VisibleFixture, String> {
    let target = usize::try_from(spec.size).map_err(|_| "fixture 규모가 너무 큽니다".to_owned())?;
    visible_nodes(spec.seed, target).map_err(|e| format!("fixture 생성 실패: {e:?}"))
}

/// fixture 문서 사본(재생마다 새 DocumentState).
fn copy_model(model: &DocumentModel) -> Result<DocumentModel, String> {
    let records = model
        .to_records()
        .map_err(|e| format!("fixture 사본: {e:?}"))?;
    DocumentModel::from_records(records)
        .map(|(model, _)| model)
        .map_err(|e| format!("fixture 사본: {e:?}"))
}

/// 표준 scenario의 재생 파일(60 Hz 이동 60개).
pub fn standard_replay(
    scenario: ReplayScenario,
    spec: FixtureSpec,
    fixture: &VisibleFixture,
) -> Result<ReplayFile, String> {
    let model = &fixture.built.model;
    let card = *model
        .children_in_order(fixture.root)
        .first()
        .ok_or("fixture root에 자식이 없습니다")?;
    let (origin, size) = match model.payload(card) {
        Some(stellar_document::records::NodePayload::Frame(frame)) => {
            let (_, _, _, _, tx, ty) = frame.transform.components();
            ((tx, ty), (frame.size.width(), frame.size.height()))
        }
        _ => return Err("첫 card가 Frame이 아닙니다".to_owned()),
    };
    let (start, step) = match scenario {
        ReplayScenario::Pan => (
            (
                fixture.viewport.size().width() / 2.0,
                fixture.viewport.size().height() / 2.0,
            ),
            (6.0, 3.0),
        ),
        ReplayScenario::Drag => (
            (origin.0 + size.0 / 2.0, origin.1 + size.1 / 2.0),
            (2.0, 1.5),
        ),
        ReplayScenario::Resize => ((origin.0 + size.0, origin.1 + size.1), (1.0, 0.75)),
    };
    const FRAME_NANOS: u64 = 8_333_333;
    const MOVES: u64 = 60;
    let point = |i: u64| (start.0 + step.0 * i as f32, start.1 + step.1 * i as f32);
    let mut events = vec![super::file::ReplayEvent {
        at_nanos: 0,
        input: ReplayInput::Down {
            x: start.0,
            y: start.1,
        },
    }];
    for i in 1..=MOVES {
        let (x, y) = point(i);
        events.push(super::file::ReplayEvent {
            at_nanos: i * FRAME_NANOS,
            input: ReplayInput::Move { x, y },
        });
    }
    let (x, y) = point(MOVES);
    events.push(super::file::ReplayEvent {
        at_nanos: (MOVES + 1) * FRAME_NANOS,
        input: ReplayInput::Up { x, y },
    });
    Ok(ReplayFile {
        fixture: spec,
        fixture_hash: *fixture.content_hash.digest(),
        viewport: (
            fixture.viewport.size().width(),
            fixture.viewport.size().height(),
        ),
        scenario,
        events,
    })
}

/// 재생 상태.
struct Pipeline {
    state: DocumentState,
    root: StableId,
    source: DocumentSceneSource,
    scene: IncrementalScene,
    layout: ReplayLayout,
    index: SceneSpatialIndex,
    camera: Transform2D,
    size: Size2,
    controller: InteractionController,
    tool: InteractionTool,
    selection: Vec<StableId>,
    selection_box: Option<[Vec2; 4]>,
    layer: TransientLayer,
    wall: WallClock,
    scene_traces: Vec<SceneBuildTrace>,
    graph_traces: Vec<RenderGraphCompileTrace>,
    /// 바뀐 scene 부분만 다시 compile하는 render graph compiler(ADR-0090).
    compiler: FrameCompiler,
    transactions: u64,
}

/// source tree에서 `key`의 world transform(조상 local transform 합성).
fn world_of(source: &stellar_scene::source::SceneSource, key: SceneKey) -> Option<Transform2D> {
    let local = |key: SceneKey| match &source.get(key)?.kind {
        SourceKind::Fragment(_) => Some(Transform2D::identity()),
        other => other.node_input().map(|n| n.transform),
    };
    let mut world = local(key)?;
    let mut current = source.parent(key);
    while let Some(parent) = current {
        world = world.then(&local(parent)?).ok()?;
        current = source.parent(parent);
    }
    Some(world)
}

/// 부모 world 변환(root면 항등).
fn parent_world(source: &stellar_scene::source::SceneSource, key: SceneKey) -> Option<Transform2D> {
    match source.parent(key) {
        Some(parent) => world_of(source, parent),
        None => Some(Transform2D::identity()),
    }
}

/// world 변환 `delta`를 적용한 local transform: `local · parent · delta · parent⁻¹`.
fn moved_local(local: Transform2D, parent: Transform2D, delta: Transform2D) -> Option<Transform2D> {
    local
        .then(&parent)
        .ok()?
        .then(&delta)
        .ok()?
        .then(&parent.inverse()?)
        .ok()
}

impl Pipeline {
    fn new(
        model: DocumentModel,
        root: StableId,
        viewport: (f32, f32),
        scenario: ReplayScenario,
    ) -> Result<Pipeline, String> {
        let state = DocumentState::new(model);
        let source = document_scene_source(state.model(), root, SceneSourceOptions::default())
            .map_err(|e| format!("scene source: {e}"))?;
        let wall = WallClock::new();
        let (scene, first) =
            IncrementalScene::build(source.source.clone(), state.revision(), &wall)
                .map_err(|e| format!("scene build: {e}"))?;
        let (layout, _) = ReplayLayout::new(&source.source)?;
        let index = SceneSpatialIndex::build(scene.snapshot());
        let size = Size2::new(viewport.0, viewport.1).map_err(|_| "viewport 크기".to_owned())?;
        let tool = match scenario {
            ReplayScenario::Pan => InteractionTool::Hand,
            ReplayScenario::Drag | ReplayScenario::Resize => InteractionTool::Move,
        };
        let mut pipeline = Pipeline {
            state,
            root,
            source,
            scene,
            layout,
            index,
            camera: Transform2D::identity(),
            size,
            controller: InteractionController::new(InteractionConfig::default()),
            tool,
            selection: Vec::new(),
            selection_box: None,
            layer: TransientLayer::new(),
            wall,
            scene_traces: vec![first],
            graph_traces: Vec::new(),
            compiler: FrameCompiler::new(),
            transactions: 0,
        };
        if matches!(scenario, ReplayScenario::Drag | ReplayScenario::Resize) {
            // 재생은 첫 card를 선택한 상태에서 시작합니다.
            let card = *pipeline
                .state
                .model()
                .children_in_order(root)
                .first()
                .ok_or("root에 자식이 없습니다")?;
            pipeline.select(card)?;
        }
        Ok(pipeline)
    }

    fn select(&mut self, node: StableId) -> Result<(), String> {
        self.selection = vec![node];
        let key = SceneKey::node(node);
        let world = world_of(&self.source.source, key).ok_or("선택 node world 변환")?;
        let Some(SourceKind::Frame(input)) = self.source.source.get(key).map(|n| &n.kind) else {
            return Err("선택 node가 Frame이 아닙니다".to_owned());
        };
        let (w, h) = (input.size.width(), input.size.height());
        let corner = |x: f32, y: f32| {
            world
                .apply(Vec2::new(x, y).map_err(|_| "좌표")?)
                .map_err(|_| "좌표")
        };
        let quad = [
            corner(0.0, 0.0)?,
            corner(w, 0.0)?,
            corner(w, h)?,
            corner(0.0, h)?,
        ];
        self.selection_box = Some(quad);
        self.layer.clear();
        let metrics = HandleMetrics::new(4.0, 3.0, 8.0).map_err(|e| e.to_string())?;
        self.layer.push_selection_box(node, quad, metrics);
        Ok(())
    }

    fn viewport(&self) -> Result<Viewport, String> {
        Viewport::new(self.size, self.camera).map_err(|e| format!("viewport: {e:?}"))
    }

    /// 누른 node를 View root의 직계 자식(편집 범위)으로 올립니다.
    fn edit_scope(&self, node: StableId) -> StableId {
        let mut current = SceneKey::node(node);
        while let Some(parent) = self.source.source.parent(current) {
            if parent.id() == self.root {
                return current.id();
            }
            current = parent;
        }
        node
    }

    fn render(&mut self) -> Result<(), String> {
        let viewport = self.viewport()?;
        let input = CompileInput {
            snapshot: self.scene.snapshot(),
            viewport: &viewport,
            visible: None,
            surface_format: TextureFormat::Bgra8Unorm,
            clear: ClearColor::TRANSPARENT,
            boundaries: EffectBoundaries::Minimal,
            clip_caps: ClipCapabilities {
                stencil_bits: 8,
                samples: SampleCount::X4,
            },
        };
        let surface = TargetInfo {
            format: TextureFormat::Bgra8Unorm,
            samples: SampleCount::X4,
        };
        compile_frame_traced_with(
            &mut self.compiler,
            &input,
            surface,
            &self.wall,
            &mut self.graph_traces,
        )
        .map_err(|e| format!("render graph: {e:?}"))?;
        Ok(())
    }

    /// 대상 node의 preview 입력.
    fn preview_for(
        &self,
        kind: stellar_scene::interaction::InteractionKind,
        transform: Transform2D,
    ) -> Result<ScenePreview, String> {
        let mut preview = ScenePreview::new();
        for target in &self.selection {
            let key = SceneKey::node(*target);
            let Some(node) = self.source.source.get(key) else {
                continue;
            };
            let parent = parent_world(&self.source.source, key).ok_or("부모 world")?;
            let SourceKind::Frame(mut input) = node.kind.clone() else {
                continue;
            };
            match kind {
                stellar_scene::interaction::InteractionKind::Resize => {
                    let quad = self.selection_box.ok_or("선택 상자")?;
                    let (origin, size) = resized_box(quad, transform)?;
                    let local = parent
                        .inverse()
                        .ok_or("부모 역변환")?
                        .apply(origin)
                        .map_err(|_| "좌표")?;
                    let (a, b, c, d, _, _) = input.node.transform.components();
                    input.node.transform =
                        Transform2D::new(a, b, c, d, local.x(), local.y()).map_err(|_| "변환")?;
                    input.size = size;
                }
                _ => {
                    input.node.transform =
                        moved_local(input.node.transform, parent, transform).ok_or("이동 변환")?;
                }
            }
            preview.set(key, SourceKind::Frame(input));
        }
        Ok(preview)
    }

    /// 확정 command(뗌).
    fn commit_commands(&self, outcome: &InteractionOutcome) -> Result<Vec<Command>, String> {
        let mut commands = Vec::new();
        let kind = match outcome {
            InteractionOutcome::Drag { transform, .. } => (
                stellar_scene::interaction::InteractionKind::Drag,
                *transform,
            ),
            InteractionOutcome::Resize { transform, .. } => (
                stellar_scene::interaction::InteractionKind::Resize,
                *transform,
            ),
            _ => return Ok(commands),
        };
        let preview = self.preview_for(kind.0, kind.1)?;
        for key in preview.keys() {
            let Some(SourceKind::Frame(input)) = preview.get(key) else {
                continue;
            };
            commands.push(Command::SetProperty(SetProperty {
                node: key.id(),
                key: frame_keys::TRANSFORM,
                value: PropertyValue::Transform(input.node.transform),
            }));
            if kind.0 == stellar_scene::interaction::InteractionKind::Resize {
                commands.extend(
                    SetProperty::frame_dimensions(key.id(), input.size)
                        .into_iter()
                        .map(Command::SetProperty),
                );
            }
        }
        Ok(commands)
    }

    /// event 하나를 처리하고 trace를 돌려줍니다.
    fn event(
        &mut self,
        ids: &mut InteractionTraceIds,
        dims: TraceDimensions,
        at: u64,
        input: ReplayInput,
    ) -> Result<CompletedTrace, String> {
        let start = MonotonicClock::now_nanos(&self.wall);
        let elapsed = |wall: &WallClock| at + MonotonicClock::now_nanos(wall).saturating_sub(start);
        let mut trace = InteractionTrace::begin(ids.issue(), dims, at);
        let err = |e: stellar_diagnostics::interaction::TraceError| e.to_string();
        let mut preview: Option<ScenePreview> = None;
        let mut outcome: Option<InteractionOutcome> = None;
        // T1: 좌표 변환·hit test·상태 기계.
        match input {
            ReplayInput::Down { x, y } => {
                let view = Vec2::new(x, y).map_err(|_| "좌표")?;
                let viewport = self.viewport()?;
                let hit = hit_test_canvas(
                    &self.layer,
                    &self.index,
                    &viewport,
                    view,
                    &NarrowPhaseOptions::default(),
                    &LayoutBoxesOnly,
                );
                let target = match hit {
                    CanvasHit::Handle(hit) => PressTarget::Handle(hit.handle.kind),
                    CanvasHit::Node(node) => PressTarget::Node(self.edit_scope(node.source)),
                    CanvasHit::Empty => PressTarget::Empty,
                };
                let context = PressContext {
                    tool: self.tool,
                    camera: self.camera,
                    target,
                    selection: &self.selection,
                    selection_box: self.selection_box,
                };
                self.controller
                    .pointer_down(view, &context)
                    .map_err(|e| format!("pointer down: {e}"))?;
            }
            ReplayInput::Move { x, y } => {
                let view = Vec2::new(x, y).map_err(|_| "좌표")?;
                match self
                    .controller
                    .pointer_move(view)
                    .map_err(|e| format!("pointer move: {e}"))?
                {
                    InteractionStep::Updated(InteractionPreview::Camera { camera, .. }) => {
                        self.camera = camera
                    }
                    InteractionStep::Updated(InteractionPreview::Transform { kind, transform }) => {
                        preview = Some(self.preview_for(kind, transform)?);
                    }
                    _ => {}
                }
            }
            ReplayInput::Up { x, y } => {
                let view = Vec2::new(x, y).map_err(|_| "좌표")?;
                match self.controller.pointer_up(view) {
                    InteractionStep::Completed(InteractionOutcome::Pan { camera }) => {
                        self.camera = camera
                    }
                    InteractionStep::Completed(done) => outcome = Some(done),
                    _ => {}
                }
            }
            ReplayInput::Wheel { dx, dy } => {
                let pan = Transform2D::new(1.0, 0.0, 0.0, 1.0, -dx, -dy).map_err(|_| "wheel")?;
                self.camera = self.camera.then(&pan).map_err(|_| "camera")?;
            }
        }
        trace.mark(Stage::T1, elapsed(&self.wall)).map_err(err)?;
        // T2: 확정 command.
        let mut changes = None;
        if let Some(done) = &outcome {
            let commands = self.commit_commands(done)?;
            if !commands.is_empty() {
                self.transactions += 1;
                let id = CommandId::from_stable_id(StableId::new_v7(
                    self.transactions,
                    0,
                    self.transactions,
                ));
                let mut tx = CommandTransaction::new(id, CommandOrigin::User);
                for command in commands {
                    tx = tx.with_request(CommandRequest::new(self.state.revision(), command));
                }
                let applied = self
                    .state
                    .execute_transaction(&tx)
                    .map_err(|e| format!("transaction: {e:?}"))?;
                changes = Some(applied.changes);
            }
        }
        trace.mark(Stage::T2, elapsed(&self.wall)).map_err(err)?;
        // T3: 증분 layout.
        let mut commit: Option<(Vec<SourceEdit>, SceneDirtySet)> = None;
        if let Some(changes) = &changes {
            let model = self.state.model();
            let mut invalidations = scene_invalidations(model, changes);
            let edits = scene_edits(
                model,
                &mut self.source,
                changes,
                SceneSourceOptions::default(),
            )
            .map_err(|e| format!("scene edits: {e}"))?;
            let changed: Vec<SceneKey> = edits.iter().filter_map(SourceEdit::key).collect();
            let (_, layout_invalidations) = self.layout.apply(&self.source.source, &changed)?;
            invalidations.extend(layout_invalidations);
            let dirty = SceneDirtySet::from_invalidations(&invalidations, &self.source);
            commit = Some((edits, dirty));
        }
        trace.mark(Stage::T3, elapsed(&self.wall)).map_err(err)?;
        // T4: scene·render graph.
        if let Some((edits, dirty)) = commit {
            let built = self
                .scene
                .commit(self.state.revision(), &edits, &dirty, &self.wall)
                .map_err(|e| format!("scene commit: {e}"))?;
            self.scene_traces.push(built);
            self.index.refresh(self.scene.snapshot());
            if let Some(node) = self.selection.first().copied() {
                self.select(node)?;
            }
        } else if let Some(preview) = preview {
            let built = self
                .scene
                .set_preview(preview, &self.wall)
                .map_err(|e| format!("scene preview: {e}"))?;
            self.scene_traces.push(built);
        } else if outcome.is_some() && !self.scene.preview().is_empty() {
            let built = self
                .scene
                .clear_preview(&self.wall)
                .map_err(|e| format!("scene: {e}"))?;
            self.scene_traces.push(built);
        }
        self.render()?;
        let t4 = elapsed(&self.wall);
        trace.mark(Stage::T4, t4).map_err(err)?;
        // T5: core에는 GPU 제출이 없습니다.
        trace.mark(Stage::T5, t4).map_err(err)?;
        trace.set_queue_depth(0);
        trace.validate().map_err(err)
    }
}

/// Resize 변환으로 선택 상자를 옮긴 world 축 정렬 상자(origin, size).
fn resized_box(quad: [Vec2; 4], transform: Transform2D) -> Result<(Vec2, Size2), String> {
    let mut min = (f32::INFINITY, f32::INFINITY);
    let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for corner in quad {
        let p = transform.apply(corner).map_err(|_| "resize 좌표")?;
        min = (min.0.min(p.x()), min.1.min(p.y()));
        max = (max.0.max(p.x()), max.1.max(p.y()));
    }
    Ok((
        Vec2::new(min.0, min.1).map_err(|_| "resize 좌표")?,
        Size2::new(max.0 - min.0, max.1 - min.1).map_err(|_| "resize 크기")?,
    ))
}

/// 재생 파일을 실행합니다. fixture content hash가 파일과 다르면 거부합니다.
pub fn run_replay(file: &ReplayFile, fixture: &VisibleFixture) -> Result<ReplayRun, String> {
    if *fixture.content_hash.digest() != file.fixture_hash {
        return Err("fixture content hash가 재생 파일과 다릅니다".to_owned());
    }
    let model = copy_model(&fixture.built.model)?;
    let nodes = model.node_count() as u64;
    let mut pipeline = Pipeline::new(model, fixture.root, file.viewport, file.scenario)?;
    let dims = TraceDimensions {
        kind: file.scenario.kind(),
        document: DocumentSizeBucket::of(nodes),
        visible: VisibleNodeBucket::of(fixture.visible_nodes as u64),
        cache: CacheState::Warm,
    };
    let mut ids = InteractionTraceIds::new();
    let mut traces = Vec::with_capacity(file.events.len());
    for event in &file.events {
        traces.push(pipeline.event(&mut ids, dims, event.at_nanos, event.input)?);
    }
    // 단계 trace는 schema를 지켜야 합니다(XI.2.6).
    for trace in &pipeline.scene_traces {
        let fields = trace.fields();
        validate(&TraceRecord {
            event: SceneBuildTrace::EVENT,
            fields: &fields,
        })
        .map_err(|e| e.to_string())?;
    }
    for trace in &pipeline.graph_traces {
        let fields = trace.fields();
        validate(&TraceRecord {
            event: RenderGraphCompileTrace::EVENT,
            fields: &fields,
        })
        .map_err(|e| e.to_string())?;
    }
    let p99 = |mut values: Vec<u64>| {
        values.sort_unstable();
        stellar_diagnostics::analyzer::percentile(&values, 99)
    };
    let scene_build_p99_nanos = p99(pipeline
        .scene_traces
        .iter()
        .skip(1)
        .map(|t| t.end_nanos.saturating_sub(t.start_nanos))
        .collect());
    let graph_compile_p99_nanos = p99(pipeline.graph_traces.iter().map(|t| t.cpu_nanos).collect());
    Ok(ReplayRun {
        scene_build_p99_nanos,
        graph_compile_p99_nanos,
        scenario: file.scenario,
        traces,
        scene_hash: pipeline.scene.snapshot().semantic_hash().0,
        document_revision: pipeline.state.revision(),
        scene_builds: pipeline.scene_traces.len(),
        graph_compiles: pipeline.graph_traces.len(),
    })
}
