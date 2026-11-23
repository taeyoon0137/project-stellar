//! core-only 입력 재생 runner (XVI.2.2, XVI.2.5, XVI.2.6, ADR-0084 결정 6·9, ADR-0096 결정 3·4·5).
//!
//! UI 없이 재생 event마다 아래 순서를 실행하고 T0~T5를 기록합니다.
//!
//! | stage | 재생에서 하는 일 |
//! |---|---|
//! | T0 | event의 ManualClock timeline 시각 |
//! | T1 | 좌표 변환, 누름·hover면 canvas hit test(transient handle 먼저), 상호작용 상태 기계, key 해석 |
//! | T2 | 뗌 확정이면 `CommandTransaction` 하나를 history에 기록, undo·redo면 history 재생(이동은 preview만) |
//! | T3 | 바뀐 node의 scene source 편집(View Variant 포함), 증분 layout과 layout rect 무효화 |
//! | T4 | scene 갱신(preview 주입 또는 확정 반영), 확정 뒤 공간 인덱스, render graph compile·batching |
//! | T5 | core에는 GPU 제출이 없어 T4와 같습니다 |
//!
//! T1~T5는 T0에 그 event 처리 시작부터의 실제 경과를 더한 값입니다. 그래서 trace는 ManualClock timeline에 맞춰
//! 정렬되고 구간 값은 실제 계산 시간입니다. 같은 재생은 trace 수와 마지막 scene semantic hash가 같습니다.
//!
//! # 시나리오(XVI.2.5)
//!
//! [`standard_replay`]가 시나리오마다 판정 sample `samples`개 이상(기본 1,000)을 만드는 재생 파일을 씁니다.
//!
//! | 시나리오 | 도구·입력 |
//! |---|---|
//! | pan | Hand, 화면 가운데 누르고 이동 |
//! | zoom | Zoom, 화면 가운데 누르고 좌우 scrub |
//! | drag | Move, 선택한 첫 card 가운데 누르고 이동 |
//! | resize | Move, 선택한 첫 card 오른쪽 아래 모서리 handle |
//! | rotate | Move, 선택한 첫 card 오른쪽 위 회전 영역을 누르고 중심 둘레로 이동 |
//! | hover | 누르지 않은 이동, 이동마다 hit test |
//! | select | Move, 화면 안 card를 차례로 클릭 |
//! | variant-change | `commit n`: 짝수 단계는 card 10개 중 1개를 숨기는 View Variant, 홀수 단계는 Variant 없음 |
//! | undo | setup: 첫 card를 `samples`번 끌어 history를 쌓음. 측정: `cmd+z` |
//! | redo | setup: undo와 같고 모두 undo. 측정: `cmd+shift+z` |
//!
//! Text Edit·Token Propagation·Component Update는 엔진 경로가 아직 문서 → scene에 이어지지 않아
//! ([`CoreReplay::NotConnected`]) 재생 파일을 만들지 않습니다.
//!
//! # cache setup(XVI.2.6)
//!
//! core replay의 cache는 증분 scene build의 node 결과(그리고 그 위의 공간 인덱스·증분 layout)입니다. `warm`은 문서를
//! 연 뒤 만든 결과와 setup event가 남긴 결과를 그대로 두고 측정합니다. `cold`는 측정 첫 event에서 그 결과를 버리고
//! scene·공간 인덱스·layout을 처음부터 만듭니다. 측정 구간의 scene build마다 hit(`shared`)·miss(`rebuilt`)를 모아
//! [`check_cache_setup`]으로 선언과 맞는지 판정합니다.

use std::time::Instant;

use stellar_bench_fixtures::limits::{visible_nodes, VisibleFixture};
use stellar_commands::change::{node_keys, ChangeRecorder, ChangeSet};
use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::request::CommandRequest;
use stellar_commands::selection::WindowViewState;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::{StableId, WindowSessionId};
use stellar_core_types::property::PropertyPath;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{Size2, Transform2D, Vec2};
use stellar_diagnostics::interaction::{
    CacheState, CompletedTrace, DocumentSizeBucket, InteractionTrace, InteractionTraceIds, Stage,
    TraceDimensions, VisibleNodeBucket,
};
use stellar_diagnostics::latency_scenario::{
    check_cache_setup, CacheBuild, CacheObservation, CoreReplay,
};
use stellar_diagnostics::trace_schema::{validate, TraceRecord};
use stellar_document::component::variant_override::VariantOverrideSet;
use stellar_document::frame::keys as frame_keys;
use stellar_document::model::DocumentModel;
use stellar_document::modifier_metadata::InvalidationSet;
use stellar_history::history::History;
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
use stellar_scene::dirty::{SceneDirtySet, SceneInvalidation};
use stellar_scene::incremental::{IncrementalScene, SceneBuildTrace, ScenePreview};
use stellar_scene::interaction::{
    InteractionConfig, InteractionController, InteractionKind as GestureKind, InteractionOutcome,
    InteractionPreview, InteractionStep, InteractionTool, PressContext, PressTarget,
};
use stellar_scene::narrow_phase::{LayoutBoxesOnly, NarrowPhaseOptions};
use stellar_scene::source::{SceneKey, SourceEdit, SourceKind};
use stellar_scene::spatial::SceneSpatialIndex;
use stellar_scene::transient::{hit_test_canvas, CanvasHit, HandleMetrics, TransientLayer};
use stellar_scene::visibility::Viewport;

use super::file::{FixtureSpec, Modifiers, ReplayEvent, ReplayFile, ReplayInput, ReplayScenario};

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
    /// 측정 구간 완료 trace(event 순, setup event 제외).
    pub traces: Vec<CompletedTrace>,
    /// 측정 전 setup event 수.
    pub setup_events: usize,
    /// 선언한 cache setup.
    pub cache: CacheState,
    /// 측정 구간 cache 관측.
    pub cache_observation: CacheObservation,
    /// 선언과 관측이 맞는지.
    pub cache_consistent: bool,
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

/// root 직계 Frame card의 (world 원점, 크기). 문서 순서.
fn cards(fixture: &VisibleFixture) -> Vec<((f32, f32), (f32, f32))> {
    let model = &fixture.built.model;
    model
        .children_in_order(fixture.root)
        .into_iter()
        .filter_map(|card| match model.payload(card) {
            Some(stellar_document::records::NodePayload::Frame(frame)) => {
                let (_, _, _, _, tx, ty) = frame.transform.components();
                Some(((tx, ty), (frame.size.width(), frame.size.height())))
            }
            _ => None,
        })
        .collect()
}

/// 표준 재생의 frame 간격(120 Hz).
pub const FRAME_NANOS: u64 = 8_333_333;

/// 판정 sample 기본 개수(performance-budgets `scenario_min_samples`와 같은 값).
pub const DEFAULT_SAMPLES: u64 = 1_000;

/// 0 → `amplitude` → 0으로 오가는 삼각파(제스처가 화면 밖으로 나가지 않게).
fn triangle(i: u64, amplitude: u64) -> f32 {
    let period = 2 * amplitude.max(1);
    let phase = i % period;
    (if phase <= amplitude {
        phase
    } else {
        period - phase
    }) as f32
}

/// event 목록 작성기.
struct Timeline {
    events: Vec<ReplayEvent>,
}

impl Timeline {
    fn push(&mut self, input: ReplayInput) {
        let at_nanos = self.events.len() as u64 * FRAME_NANOS;
        self.events.push(ReplayEvent { at_nanos, input });
    }

    fn gesture(&mut self, start: (f32, f32), moves: u64, at: impl Fn(u64) -> (f32, f32)) {
        self.push(ReplayInput::Down {
            x: start.0,
            y: start.1,
        });
        let mut last = start;
        for i in 1..=moves {
            last = at(i);
            self.push(ReplayInput::Move {
                x: last.0,
                y: last.1,
            });
        }
        self.push(ReplayInput::Up {
            x: last.0,
            y: last.1,
        });
    }

    /// 첫 card를 `count`번 끌어 놓습니다(짝수 번째는 오른쪽 아래, 홀수 번째는 되돌아옴).
    fn drags(&mut self, center: (f32, f32), count: u64) {
        let mut at = center;
        for i in 0..count {
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            let start = at;
            self.gesture(start, 2, |k| {
                (
                    start.0 + sign * 2.0 * k as f32,
                    start.1 + sign * 1.5 * k as f32,
                )
            });
            at = (start.0 + sign * 4.0, start.1 + sign * 3.0);
        }
    }

    fn key(&mut self, key: &str, modifiers: Modifiers) {
        self.push(ReplayInput::Key {
            key: key.to_owned(),
            modifiers,
        });
    }
}

const UNDO: Modifiers = Modifiers {
    cmd: true,
    ctrl: false,
    alt: false,
    shift: false,
};

const REDO: Modifiers = Modifiers {
    cmd: true,
    ctrl: false,
    alt: false,
    shift: true,
};

/// 표준 scenario의 재생 파일(XVI.2.5). 판정 sample이 `samples`개 이상입니다.
///
/// 엔진 경로가 이어지지 않은 시나리오([`CoreReplay::NotConnected`])는 거부합니다.
pub fn standard_replay(
    scenario: ReplayScenario,
    spec: FixtureSpec,
    fixture: &VisibleFixture,
    samples: u64,
    cache: CacheState,
) -> Result<ReplayFile, String> {
    if let CoreReplay::NotConnected { missing } = scenario.core_replay() {
        return Err(format!(
            "{} 시나리오는 core replay가 아직 실행할 수 없습니다: {missing}",
            scenario.name()
        ));
    }
    let samples = samples.max(1);
    let all = cards(fixture);
    let &(origin, size) = all.first().ok_or("fixture root에 Frame card가 없습니다")?;
    let view = (
        fixture.viewport.size().width(),
        fixture.viewport.size().height(),
    );
    let center = (origin.0 + size.0 / 2.0, origin.1 + size.1 / 2.0);
    let screen = (view.0 / 2.0, view.1 / 2.0);
    let mut timeline = Timeline { events: Vec::new() };
    let mut measure_from = 0;
    match scenario {
        ReplayScenario::Pan => timeline.gesture(screen, samples, |i| {
            (
                screen.0 + 1.5 * triangle(i, 200),
                screen.1 + triangle(i, 200),
            )
        }),
        ReplayScenario::Zoom => {
            timeline.gesture(screen, samples, |i| (screen.0 + triangle(i, 300), screen.1))
        }
        ReplayScenario::Drag => timeline.gesture(center, samples, |i| {
            (
                center.0 + 2.0 * triangle(i, 60),
                center.1 + 1.5 * triangle(i, 60),
            )
        }),
        ReplayScenario::Resize => {
            let corner = (origin.0 + size.0, origin.1 + size.1);
            timeline.gesture(corner, samples, |i| {
                (
                    corner.0 + triangle(i, 80),
                    corner.1 + 0.75 * triangle(i, 80),
                )
            })
        }
        ReplayScenario::Rotate => {
            // 오른쪽 위 모서리 밖 회전 영역(모서리 square handle 밖, 회전 반지름 안).
            let start = (origin.0 + size.0 + 5.0, origin.1 - 5.0);
            let radius = ((start.0 - center.0).powi(2) + (start.1 - center.1).powi(2)).sqrt();
            let a0 = (start.1 - center.1).atan2(start.0 - center.0);
            timeline.gesture(start, samples, |i| {
                let a = a0 + 0.05 * triangle(i, 30);
                (center.0 + radius * a.cos(), center.1 + radius * a.sin())
            })
        }
        ReplayScenario::Hover => {
            for i in 0..samples {
                timeline.push(ReplayInput::Move {
                    x: ((i * 37) % view.0.max(1.0) as u64) as f32 + 0.5,
                    y: ((i * 53) % view.1.max(1.0) as u64) as f32 + 0.5,
                });
            }
        }
        ReplayScenario::Select => {
            let inside: Vec<(f32, f32)> = all
                .iter()
                .map(|(o, s)| (o.0 + s.0 / 2.0, o.1 + s.1 / 2.0))
                .filter(|c| c.0 > 0.0 && c.1 > 0.0 && c.0 < view.0 && c.1 < view.1)
                .take(16)
                .collect();
            if inside.len() < 2 {
                return Err("화면 안 card가 2개보다 적습니다".to_owned());
            }
            for i in 0..samples.div_ceil(2) {
                let c = inside[i as usize % inside.len()];
                timeline.push(ReplayInput::Down { x: c.0, y: c.1 });
                timeline.push(ReplayInput::Up { x: c.0, y: c.1 });
            }
        }
        ReplayScenario::VariantChange => {
            for step in 0..samples {
                timeline.push(ReplayInput::Commit { step });
            }
        }
        ReplayScenario::Undo => {
            timeline.drags(center, samples);
            measure_from = timeline.events.len();
            for _ in 0..samples {
                timeline.key("z", UNDO);
            }
        }
        ReplayScenario::Redo => {
            timeline.drags(center, samples);
            for _ in 0..samples {
                timeline.key("z", UNDO);
            }
            measure_from = timeline.events.len();
            for _ in 0..samples {
                timeline.key("z", REDO);
            }
        }
        ReplayScenario::TextEdit
        | ReplayScenario::TokenPropagation
        | ReplayScenario::ComponentUpdate => unreachable!("NotConnected은 위에서 거부"),
    }
    Ok(ReplayFile {
        fixture: spec,
        fixture_hash: *fixture.content_hash.digest(),
        viewport: view,
        scenario,
        cache,
        measure_from,
        events: timeline.events,
    })
}

/// Variant Change 시나리오의 Variant 두 개: card 10개 중 1개를 숨김, Variant 없음.
fn variant_sets(model: &DocumentModel, root: StableId) -> [VariantOverrideSet; 2] {
    let mut hidden = VariantOverrideSet::new();
    for card in model.children_in_order(root).into_iter().step_by(10) {
        hidden.set_property(
            PropertyPath::with_keys(card, vec![node_keys::VISIBLE]),
            TypedValue::Boolean(false),
        );
    }
    [hidden, VariantOverrideSet::new()]
}

/// 재생 상태.
struct Pipeline {
    scenario: ReplayScenario,
    state: DocumentState,
    history: History,
    view: WindowViewState,
    root: StableId,
    source: DocumentSceneSource,
    scene: IncrementalScene,
    index: SceneSpatialIndex,
    camera: Transform2D,
    size: Size2,
    controller: InteractionController,
    tool: InteractionTool,
    selection: Vec<StableId>,
    selection_box: Option<[Vec2; 4]>,
    hovered: Option<StableId>,
    layer: TransientLayer,
    variants: [VariantOverrideSet; 2],
    variant: Option<usize>,
    wall: WallClock,
    scene_traces: Vec<SceneBuildTrace>,
    graph_traces: Vec<RenderGraphCompileTrace>,
    /// 바뀐 scene 부분만 다시 compile하는 render graph compiler(ADR-0090).
    compiler: FrameCompiler,
    transactions: u64,
    /// 측정 구간인지.
    measuring: bool,
    /// 이 event에서 cache를 비우고 처음부터 만듭니다(cold 측정 시작).
    cold_pending: bool,
    observation: CacheObservation,
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

/// 한 event의 변경(문서 change set과 scene 쪽 추가 무효화).
struct EventChange {
    changes: ChangeSet,
    extra: Vec<SceneInvalidation>,
}

impl Pipeline {
    fn new(
        model: DocumentModel,
        root: StableId,
        viewport: (f32, f32),
        scenario: ReplayScenario,
    ) -> Result<Pipeline, String> {
        let variants = variant_sets(&model, root);
        let state = DocumentState::new(model);
        let source = document_scene_source(state.model(), root, SceneSourceOptions::default())
            .map_err(|e| format!("scene source: {e}"))?;
        let wall = WallClock::new();
        let (scene, first) =
            IncrementalScene::build(source.source.clone(), state.revision(), &wall)
                .map_err(|e| format!("scene build: {e}"))?;
        let index = SceneSpatialIndex::build(scene.snapshot());
        let size = Size2::new(viewport.0, viewport.1).map_err(|_| "viewport 크기".to_owned())?;
        let tool = match scenario {
            ReplayScenario::Pan => InteractionTool::Hand,
            ReplayScenario::Zoom => InteractionTool::Zoom,
            _ => InteractionTool::Move,
        };
        let window = WindowSessionId::from_stable_id(StableId::new_v7(1, 0, 1));
        let mut pipeline = Pipeline {
            scenario,
            state,
            history: History::new(),
            view: WindowViewState::new(window),
            root,
            source,
            scene,
            index,
            camera: Transform2D::identity(),
            size,
            controller: InteractionController::new(InteractionConfig::default()),
            tool,
            selection: Vec::new(),
            selection_box: None,
            hovered: None,
            layer: TransientLayer::new(),
            variants,
            variant: None,
            wall,
            scene_traces: vec![first],
            graph_traces: Vec::new(),
            compiler: FrameCompiler::new(),
            transactions: 0,
            measuring: false,
            cold_pending: false,
            observation: CacheObservation::default(),
        };
        if matches!(
            scenario,
            ReplayScenario::Drag
                | ReplayScenario::Resize
                | ReplayScenario::Rotate
                | ReplayScenario::Undo
                | ReplayScenario::Redo
        ) {
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

    fn clear_selection(&mut self) {
        self.selection.clear();
        self.selection_box = None;
        self.layer.clear();
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

    fn hit(&self, view: Vec2) -> Result<CanvasHit, String> {
        let viewport = self.viewport()?;
        Ok(hit_test_canvas(
            &self.layer,
            &self.index,
            &viewport,
            view,
            &NarrowPhaseOptions::default(),
            &LayoutBoxesOnly,
        ))
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

    /// scene build trace를 남기고, 측정 구간이면 cache 관측에 더합니다.
    fn record_build(&mut self, trace: SceneBuildTrace) {
        if self.measuring {
            self.observation.record(CacheBuild {
                hits: trace.stats.shared,
                misses: trace.stats.rebuilt,
                full: trace.stats.full,
            });
        }
        self.scene_traces.push(trace);
    }

    /// 대상 node의 preview 입력.
    fn preview_for(
        &self,
        kind: GestureKind,
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
                GestureKind::Resize => {
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
            InteractionOutcome::Drag { transform, .. } => (GestureKind::Drag, *transform),
            InteractionOutcome::Resize { transform, .. } => (GestureKind::Resize, *transform),
            InteractionOutcome::Rotate { transform, .. } => (GestureKind::Rotate, *transform),
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
            if kind.0 == GestureKind::Resize {
                commands.extend(
                    SetProperty::frame_dimensions(key.id(), input.size)
                        .into_iter()
                        .map(Command::SetProperty),
                );
            }
        }
        Ok(commands)
    }

    /// 확정 command를 transaction 하나로 history에 기록합니다.
    fn record_commands(&mut self, commands: Vec<Command>) -> Result<Option<ChangeSet>, String> {
        if commands.is_empty() {
            return Ok(None);
        }
        self.transactions += 1;
        let id =
            CommandId::from_stable_id(StableId::new_v7(self.transactions, 0, self.transactions));
        let mut tx = CommandTransaction::new(id, CommandOrigin::User);
        for command in commands {
            tx = tx.with_request(CommandRequest::new(self.state.revision(), command));
        }
        self.history
            .record_document(&mut self.state, &tx, None)
            .map(Some)
            .map_err(|e| format!("transaction: {e:?}"))
    }

    /// View Variant 확정 단계(짝수: 숨김 Variant, 홀수: Variant 없음).
    fn switch_variant(&mut self, step: u64) -> EventChange {
        let next = step.is_multiple_of(2).then_some(0);
        let mut recorder = ChangeRecorder::new();
        let mut extra = Vec::new();
        // 이전·다음 Variant가 건드리는 node 모두가 다시 펼쳐야 할 subtree입니다.
        let touched: std::collections::BTreeSet<StableId> = [self.variant, next]
            .into_iter()
            .flatten()
            .flat_map(|i| {
                self.variants[i]
                    .property_patches()
                    .map(|(path, _)| path.root())
                    .collect::<Vec<_>>()
            })
            .collect();
        for node in touched {
            recorder.node_property(
                node,
                node_keys::VISIBLE,
                InvalidationSet::GEOMETRY_AND_PAINT,
            );
            extra.push(SceneInvalidation::Subtree { node });
        }
        self.variant = next;
        EventChange {
            changes: recorder.finish(self.state.revision()),
            extra,
        }
    }

    /// event 하나를 처리하고 trace를 돌려줍니다.
    fn event(
        &mut self,
        ids: &mut InteractionTraceIds,
        dims: TraceDimensions,
        at: u64,
        input: &ReplayInput,
    ) -> Result<CompletedTrace, String> {
        let start = MonotonicClock::now_nanos(&self.wall);
        let elapsed = |wall: &WallClock| at + MonotonicClock::now_nanos(wall).saturating_sub(start);
        let mut trace = InteractionTrace::begin(ids.issue(), dims, at);
        let err = |e: stellar_diagnostics::interaction::TraceError| e.to_string();
        let mut preview: Option<ScenePreview> = None;
        let mut outcome: Option<InteractionOutcome> = None;
        let mut history_step: Option<bool> = None;
        let mut variant_step: Option<u64> = None;
        // T1: 좌표 변환·hit test·상태 기계·key 해석.
        match *input {
            ReplayInput::Down { x, y } => {
                let view = Vec2::new(x, y).map_err(|_| "좌표")?;
                let target = match self.hit(view)? {
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
                if self.controller.is_active() {
                    match self
                        .controller
                        .pointer_move(view)
                        .map_err(|e| format!("pointer move: {e}"))?
                    {
                        InteractionStep::Updated(InteractionPreview::Camera { camera, .. }) => {
                            self.camera = camera
                        }
                        InteractionStep::Updated(InteractionPreview::Transform {
                            kind,
                            transform,
                        }) => {
                            preview = Some(self.preview_for(kind, transform)?);
                        }
                        _ => {}
                    }
                } else {
                    // hover: 누르지 않은 이동은 hit test로 가리킨 node를 정합니다.
                    self.hovered = match self.hit(view)? {
                        CanvasHit::Node(node) => Some(self.edit_scope(node.source)),
                        CanvasHit::Handle(hit) => Some(hit.handle.target),
                        CanvasHit::Empty => None,
                    };
                }
            }
            ReplayInput::Up { x, y } => {
                let view = Vec2::new(x, y).map_err(|_| "좌표")?;
                match self.controller.pointer_up(view) {
                    InteractionStep::Completed(
                        InteractionOutcome::Pan { camera } | InteractionOutcome::Zoom { camera },
                    ) => self.camera = camera,
                    InteractionStep::Completed(InteractionOutcome::Click { target }) => {
                        match target {
                            Some(node) => self.select(node)?,
                            None => self.clear_selection(),
                        }
                    }
                    InteractionStep::Completed(done) => outcome = Some(done),
                    _ => {}
                }
            }
            ReplayInput::Wheel { dx, dy } => {
                let pan = Transform2D::new(1.0, 0.0, 0.0, 1.0, -dx, -dy).map_err(|_| "wheel")?;
                self.camera = self.camera.then(&pan).map_err(|_| "camera")?;
            }
            ReplayInput::Key { ref key, modifiers } => {
                // 해석하지 않는 key는 아무 일도 하지 않는 sample이 되어 통계를 흐리므로 거부합니다.
                if key == "z" && modifiers == UNDO {
                    history_step = Some(true);
                } else if key == "z" && modifiers == REDO {
                    history_step = Some(false);
                } else {
                    return Err(format!(
                        "core replay가 해석하지 않는 key입니다: {key} {}",
                        modifiers.to_text()
                    ));
                }
            }
            ReplayInput::Commit { step } => {
                // 확정 단계의 뜻은 시나리오가 정합니다. 지금은 Variant Change만 정의합니다.
                if self.scenario != ReplayScenario::VariantChange {
                    return Err(format!(
                        "{} 시나리오는 commit 단계를 정의하지 않습니다",
                        self.scenario.name()
                    ));
                }
                variant_step = Some(step)
            }
            ReplayInput::Text { .. } => {
                return Err(
                    "text 입력은 core replay가 아직 실행할 수 없습니다(Text scene 입력 미연결)"
                        .to_owned(),
                )
            }
        }
        trace.mark(Stage::T1, elapsed(&self.wall)).map_err(err)?;
        // T2: 확정 command 또는 history 재생.
        let mut change: Option<EventChange> = None;
        if let Some(done) = &outcome {
            let commands = self.commit_commands(done)?;
            if let Some(changes) = self.record_commands(commands)? {
                change = Some(EventChange {
                    changes,
                    extra: Vec::new(),
                });
            }
        }
        if let Some(undo) = history_step {
            let replayed = if undo {
                self.history.undo(&mut self.state, &mut self.view)
            } else {
                self.history.redo(&mut self.state, &mut self.view)
            }
            .map_err(|e| format!("history: {e:?}"))?;
            if let Some(changes) = replayed.and_then(|o| o.changes) {
                change = Some(EventChange {
                    changes,
                    extra: Vec::new(),
                });
            }
        }
        if let Some(step) = variant_step {
            change = Some(self.switch_variant(step));
        }
        trace.mark(Stage::T2, elapsed(&self.wall)).map_err(err)?;
        // T3: scene source 편집과 증분 layout.
        let mut commit: Option<(Vec<SourceEdit>, SceneDirtySet)> = None;
        if let Some(EventChange { changes, extra }) = &change {
            let model = self.state.model();
            let mut invalidations = scene_invalidations(model, changes);
            invalidations.extend(extra.iter().copied());
            let options = SceneSourceOptions {
                variant: self.variant.map(|i| &self.variants[i]),
                ..SceneSourceOptions::default()
            };
            let edits = scene_edits(model, &mut self.source, changes, options)
                .map_err(|e| format!("scene edits: {e}"))?;
            // 증분 layout은 scene_edits가 같은 change set으로 갱신했습니다(ADR-0125 결정 6).
            if !self.cold_pending {
                invalidations.extend(super::layout::invalidations(&self.source));
            }
            let dirty = SceneDirtySet::from_invalidations(&invalidations, &self.source);
            commit = Some((edits, dirty));
        }
        if self.cold_pending {
            // cold: 이전 layout 결과를 버리고 처음부터 배치합니다.
            let options = SceneSourceOptions {
                variant: self.variant.map(|i| &self.variants[i]),
                ..SceneSourceOptions::default()
            };
            super::layout::rebuild(&mut self.source, self.state.model(), options);
        }
        trace.mark(Stage::T3, elapsed(&self.wall)).map_err(err)?;
        // T4: scene·render graph.
        let reselect = commit.is_some();
        if self.cold_pending {
            // cold: 이전 scene 결과·공간 인덱스를 버리고 처음부터 만듭니다. 이 event의 source 편집은 이미
            // source에 들어 있습니다.
            let (scene, built) = IncrementalScene::build(
                self.source.source.clone(),
                self.state.revision(),
                &self.wall,
            )
            .map_err(|e| format!("scene build: {e}"))?;
            self.scene = scene;
            self.record_build(built);
            self.index = SceneSpatialIndex::build(self.scene.snapshot());
            if let Some(preview) = preview {
                let built = self
                    .scene
                    .set_preview(preview, &self.wall)
                    .map_err(|e| format!("scene preview: {e}"))?;
                self.record_build(built);
            }
            self.cold_pending = false;
        } else if let Some((edits, dirty)) = commit {
            let built = self
                .scene
                .commit(self.state.revision(), &edits, &dirty, &self.wall)
                .map_err(|e| format!("scene commit: {e}"))?;
            self.record_build(built);
            self.index.refresh(self.scene.snapshot());
        } else if let Some(preview) = preview {
            let built = self
                .scene
                .set_preview(preview, &self.wall)
                .map_err(|e| format!("scene preview: {e}"))?;
            self.record_build(built);
        } else if outcome.is_some() && !self.scene.preview().is_empty() {
            let built = self
                .scene
                .clear_preview(&self.wall)
                .map_err(|e| format!("scene: {e}"))?;
            self.record_build(built);
        }
        if reselect {
            if let Some(node) = self.selection.first().copied() {
                self.select(node)?;
            }
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
    if let CoreReplay::NotConnected { missing } = file.scenario.core_replay() {
        return Err(format!(
            "{} 시나리오는 core replay가 아직 실행할 수 없습니다: {missing}",
            file.scenario.name()
        ));
    }
    let model = copy_model(&fixture.built.model)?;
    let nodes = model.node_count() as u64;
    let mut pipeline = Pipeline::new(model, fixture.root, file.viewport, file.scenario)?;
    let dims = TraceDimensions {
        kind: file.scenario.kind(),
        document: DocumentSizeBucket::of(nodes),
        visible: VisibleNodeBucket::of(fixture.visible_nodes as u64),
        cache: file.cache,
    };
    let mut ids = InteractionTraceIds::new();
    let mut traces = Vec::with_capacity(file.events.len() - file.measure_from);
    for (index, event) in file.events.iter().enumerate() {
        if index == file.measure_from {
            pipeline.measuring = true;
            pipeline.cold_pending = file.cache == CacheState::Cold;
        }
        let trace = pipeline.event(&mut ids, dims, event.at_nanos, &event.input)?;
        if index >= file.measure_from {
            traces.push(trace);
        }
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
    let cache_observation = pipeline.observation;
    Ok(ReplayRun {
        scene_build_p99_nanos,
        graph_compile_p99_nanos,
        scenario: file.scenario,
        traces,
        setup_events: file.measure_from,
        cache: file.cache,
        cache_consistent: check_cache_setup(file.cache, &cache_observation).is_ok(),
        cache_observation,
        scene_hash: pipeline.scene.snapshot().semantic_hash().0,
        document_revision: pipeline.state.revision(),
        scene_builds: pipeline.scene_traces.len(),
        graph_compiles: pipeline.graph_traces.len(),
    })
}
