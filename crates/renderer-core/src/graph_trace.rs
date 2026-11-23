//! RenderGraph compile trace (XI.4.6).
//!
//! [`compile_frame_traced`]는 frame compile(XI.4.2~XI.4.4)과 batching(XI.4.5)을 한 번에 실행하고, 그
//! CPU 시간과 pass·draw·batch·layer 수를 [`RenderGraphCompileTrace`]로 [`TraceSink`]에 남깁니다.
//! 아키텍처 §6.1의 T4(Scene Snapshot·Render Graph 갱신 완료) 측정에 쓰는 시작·끝 시각도 담습니다.
//!
//! **비민감 기록만**: trace에는 정적 event 이름과 숫자(시각·개수·scene revision)만 있습니다. node 이름·
//! text·경로·asset 같은 프로젝트 내용은 담을 수 없도록 field가 모두 정수입니다(II.3.2 telemetry 규칙과
//! 같음). diagnostics crate로 보내는 변환은 runtime이 [`RenderGraphCompileTrace::fields`]로 합니다
//! (renderer-core는 crate 의존 정책상 diagnostics를 직접 쓰지 않음).
//!
//! 시각은 [`TraceClock`]으로 주입합니다. 제품은 [`MonotonicClock`]을, 테스트는 고정 시계를 씁니다.

use std::fmt::Write as _;
use std::time::Instant;

use crate::batching::{batch_count, batch_frame, PassBatches, TargetInfo};
use crate::pass_compiler::{CompileInput, FrameCompileError, FrameCompiler, FramePlan};

/// 단조 증가 시계(ns).
pub trait TraceClock {
    /// 임의 기준점으로부터의 ns.
    fn now_nanos(&self) -> u64;
}

/// `Instant` 기반 단조 시계. 생성 시점이 0입니다.
#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    origin: Instant,
}

impl MonotonicClock {
    /// 지금을 기준점으로 하는 시계.
    pub fn new() -> MonotonicClock {
        MonotonicClock {
            origin: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> MonotonicClock {
        MonotonicClock::new()
    }
}

impl TraceClock for MonotonicClock {
    fn now_nanos(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// RenderGraph compile 한 번의 trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderGraphCompileTrace {
    /// compile한 scene revision.
    pub scene_revision: u64,
    /// compile 시작 시각(ns, [`TraceClock`] 기준).
    pub start_nanos: u64,
    /// compile·batching 완료 시각(T4 후보).
    pub end_nanos: u64,
    /// CPU 시간(`end - start`).
    pub cpu_nanos: u64,
    /// graph pass 수.
    pub pass_count: u32,
    /// draw 수.
    pub draw_count: u32,
    /// batch 수.
    pub batch_count: u32,
    /// offscreen layer 수.
    pub layer_count: u32,
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

impl RenderGraphCompileTrace {
    /// 정적 event 이름.
    pub const EVENT: &'static str = "render_graph.compile";

    /// (정적 field 이름, 값) 목록. 순서는 고정입니다.
    pub fn fields(&self) -> [(&'static str, u64); 8] {
        [
            ("scene_revision", self.scene_revision),
            ("start_ns", self.start_nanos),
            ("end_ns", self.end_nanos),
            ("cpu_ns", self.cpu_nanos),
            ("passes", u64::from(self.pass_count)),
            ("draws", u64::from(self.draw_count)),
            ("batches", u64::from(self.batch_count)),
            ("layers", u64::from(self.layer_count)),
        ]
    }

    /// 고정 형식 한 줄(`<event> name=value ...`). fixture·로컬 진단 파일용입니다.
    pub fn to_line(&self) -> String {
        let mut line = String::from(Self::EVENT);
        for (name, value) in self.fields() {
            // String에 쓰기는 실패하지 않습니다.
            let _ = write!(line, " {name}={value}");
        }
        line
    }
}

/// trace 수신자.
pub trait TraceSink {
    /// trace 하나를 기록합니다.
    fn record(&mut self, trace: &RenderGraphCompileTrace);
}

impl TraceSink for Vec<RenderGraphCompileTrace> {
    fn record(&mut self, trace: &RenderGraphCompileTrace) {
        self.push(*trace);
    }
}

/// trace와 함께 compile된 frame.
#[derive(Debug, Clone)]
pub struct CompiledFrame {
    /// frame plan.
    pub plan: FramePlan,
    /// pass별 batch(graph 실행 순서).
    pub batches: Vec<PassBatches>,
    /// 기록한 trace.
    pub trace: RenderGraphCompileTrace,
}

/// frame을 compile·batch하고 trace를 기록합니다. 실패하면 trace를 기록하지 않습니다.
///
/// 이전 frame을 기억하지 않습니다. frame마다 부르는 쪽은 [`compile_frame_traced_with`]를 씁니다.
pub fn compile_frame_traced(
    input: &CompileInput<'_>,
    surface: TargetInfo,
    clock: &dyn TraceClock,
    sink: &mut dyn TraceSink,
) -> Result<CompiledFrame, FrameCompileError> {
    compile_frame_traced_with(&mut FrameCompiler::one_shot(), input, surface, clock, sink)
}

/// [`FrameCompiler`]로 바뀐 scene 부분만 다시 compile하고 batch·trace합니다(ADR-0090). plan과 trace 값은
/// [`compile_frame_traced`]와 같고 CPU 시간만 다릅니다.
pub fn compile_frame_traced_with(
    compiler: &mut FrameCompiler,
    input: &CompileInput<'_>,
    surface: TargetInfo,
    clock: &dyn TraceClock,
    sink: &mut dyn TraceSink,
) -> Result<CompiledFrame, FrameCompileError> {
    let start_nanos = clock.now_nanos();
    let plan = compiler.compile(input)?;
    let batches = batch_frame(&plan, surface);
    let end_nanos = clock.now_nanos();
    let trace = RenderGraphCompileTrace {
        scene_revision: input.snapshot.scene_revision().0,
        start_nanos,
        end_nanos,
        cpu_nanos: end_nanos.saturating_sub(start_nanos),
        pass_count: saturating_u32(plan.graph.order().len()),
        draw_count: saturating_u32(plan.draw_count()),
        batch_count: saturating_u32(batch_count(&batches)),
        layer_count: saturating_u32(plan.layers.len()),
    };
    sink.record(&trace);
    Ok(CompiledFrame {
        plan,
        batches,
        trace,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::pass_compiler::tests::{input, viewport_with};
    use crate::pass_compiler::EffectBoundaries;
    use crate::render_graph::{SampleCount, TextureFormat};
    use crate::test_scene::{group, opaque, rect, snapshot_with, ContentFx, IDENTITY};

    /// 호출마다 정해진 값을 차례로 돌려주는 시계.
    struct ScriptedClock {
        ticks: Vec<u64>,
        next: Cell<usize>,
    }

    impl TraceClock for ScriptedClock {
        fn now_nanos(&self) -> u64 {
            let index = self.next.get();
            self.next.set(index + 1);
            self.ticks.get(index).copied().unwrap_or(u64::MAX)
        }
    }

    const SURFACE: TargetInfo = TargetInfo {
        format: TextureFormat::Bgra8Unorm,
        samples: SampleCount::X4,
    };

    #[test]
    fn trace_fixture_matches_golden() {
        // 검증 기준(XI.4.6): trace fixture 통과.
        let scene = snapshot_with(
            7,
            42,
            &[
                rect(1, [0.0, 0.0, 8.0, 8.0], opaque(1.0, 0.0, 0.0)),
                rect(2, [8.0, 0.0, 8.0, 8.0], opaque(0.0, 1.0, 0.0)),
                rect(3, [0.0, 8.0, 4.0, 4.0], [0.0, 0.0, 1.0, 0.5]),
                rect(4, [4.0, 8.0, 4.0, 4.0], [0.0, 0.0, 1.0, 0.5]),
                group(
                    5,
                    [0.0, 0.0, 16.0, 16.0],
                    vec![
                        rect(6, [10.0, 10.0, 4.0, 4.0], opaque(1.0, 1.0, 0.0)),
                        rect(7, [11.0, 11.0, 4.0, 4.0], opaque(1.0, 0.0, 1.0)),
                    ],
                )
                .with_opacity(0.5),
                rect(8, [0.0, 12.0, 2.0, 2.0], opaque(0.0, 0.0, 0.0)).with_content(
                    ContentFx::Text {
                        key: 8,
                        bounds: [0.0, 12.0, 2.0, 2.0],
                    },
                ),
            ],
        );
        let viewport = viewport_with(16.0, 16.0, IDENTITY);
        let clock = ScriptedClock {
            ticks: vec![1_000, 1_750],
            next: Cell::new(0),
        };
        let mut sink: Vec<RenderGraphCompileTrace> = Vec::new();
        let frame = compile_frame_traced(
            &input(&scene, &viewport, EffectBoundaries::Minimal),
            SURFACE,
            &clock,
            &mut sink,
        )
        .unwrap();
        // pass: layer(5) → opaque(1,2) → transparent(3,4, layer 5 합성, text 8).
        // batch: layer 1, opaque 1, transparent [3,4] [5 합성] [8 text] = 3.
        let expected = RenderGraphCompileTrace {
            scene_revision: 42,
            start_nanos: 1_000,
            end_nanos: 1_750,
            cpu_nanos: 750,
            pass_count: 3,
            draw_count: 2 + 2 + 4,
            batch_count: 1 + 1 + 3,
            layer_count: 1,
        };
        assert_eq!(frame.trace, expected);
        assert_eq!(sink, vec![expected]);
        assert_eq!(
            expected.to_line(),
            "render_graph.compile scene_revision=42 start_ns=1000 end_ns=1750 cpu_ns=750 \
             passes=3 draws=8 batches=5 layers=1"
        );
        // trace 값은 반환한 plan·batch와 일치합니다.
        assert_eq!(frame.plan.draw_count(), 8);
        assert_eq!(batch_count(&frame.batches), 5);
        assert_eq!(frame.plan.graph.order().len(), 3);
    }

    #[test]
    fn trace_fields_are_static_names_and_numbers() {
        let trace = RenderGraphCompileTrace {
            scene_revision: 1,
            start_nanos: 10,
            end_nanos: 5,
            cpu_nanos: 0,
            pass_count: 1,
            draw_count: 0,
            batch_count: 0,
            layer_count: 0,
        };
        let names: Vec<&'static str> = trace.fields().iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            vec![
                "scene_revision",
                "start_ns",
                "end_ns",
                "cpu_ns",
                "passes",
                "draws",
                "batches",
                "layers"
            ]
        );
        assert_eq!(RenderGraphCompileTrace::EVENT, "render_graph.compile");
        // 시계가 뒤로 가도 CPU 시간은 넘치지 않습니다.
        let backwards = ScriptedClock {
            ticks: vec![500, 100],
            next: Cell::new(0),
        };
        let scene = snapshot_with(1, 1, &[]);
        let viewport = viewport_with(16.0, 16.0, IDENTITY);
        let mut sink: Vec<RenderGraphCompileTrace> = Vec::new();
        let frame = compile_frame_traced(
            &input(&scene, &viewport, EffectBoundaries::Minimal),
            SURFACE,
            &backwards,
            &mut sink,
        )
        .unwrap();
        assert_eq!(frame.trace.cpu_nanos, 0);
        // 빈 frame: clear pass 하나, draw·batch 없음.
        assert_eq!(
            (
                frame.trace.pass_count,
                frame.trace.draw_count,
                frame.trace.batch_count
            ),
            (1, 0, 0)
        );
    }

    #[test]
    fn monotonic_clock_never_goes_backwards() {
        let clock = MonotonicClock::default();
        let first = clock.now_nanos();
        let second = clock.now_nanos();
        assert!(second >= first);
    }
}
