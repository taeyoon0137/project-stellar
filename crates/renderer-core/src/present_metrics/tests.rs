//! present queue 지표 테스트 (XI.11.5).

use super::*;
use stellar_scene::freshness::LatestSceneRevision;
use stellar_scene::handle::SnapshotHandle;

use crate::backend::mock::MockBackend;
use crate::backend::tests::{device_config, surface_graph};
use crate::backend::{
    BackendError, CheckedBackend, EncodeOutcome, FrameEncodeRequest, FrameId, ImportBinding,
    ImportTarget, RendererBackend,
};
use crate::frame_scheduler::TickOutcome;
use crate::render_graph::{CompiledRenderGraph, ResourceId};
use crate::test_scene::empty_handle;

type Backend = CheckedBackend<MockBackend>;

const IMPORTS: [ImportBinding; 1] = [ImportBinding {
    resource: ResourceId(0),
    target: ImportTarget::Surface,
}];

fn encode(
    backend: &mut Backend,
    graph: &CompiledRenderGraph,
    frame: FrameId,
) -> Result<EncodeOutcome, BackendError> {
    backend.encode(&FrameEncodeRequest {
        frame,
        graph,
        passes: &[],
        imports: &IMPORTS,
    })
}

fn publish(latest: &LatestSceneRevision, revision: u64) -> SnapshotHandle {
    let handle = empty_handle(revision);
    latest.publish(handle.scene_revision());
    handle
}

/// slow GPU benchmark 모사: 300 tick, GPU는 3 tick마다 frame 하나를 끝내고, 일부 tick에서는 encode 중 새 세대가
/// 알려집니다.
fn run_slow_gpu_benchmark() -> PresentMetrics {
    let graph = surface_graph();
    let latest = LatestSceneRevision::new();
    let mut scheduler = FrameScheduler::new(latest.clone());
    let mut backend = CheckedBackend::new(MockBackend::new());
    backend.init(&device_config(64, 64)).unwrap();
    let mut recorder = PresentRecorder::new();
    let mut revision = 0u64;
    for tick in 1..=300u64 {
        revision += 1;
        scheduler.offer(publish(&latest, revision));
        if tick % 5 == 0 {
            revision += 1;
            scheduler.offer(publish(&latest, revision));
        }
        let racing = tick % 11 == 0;
        let next = revision + 1;
        let outcome = scheduler
            .tick(&mut backend, |backend: &mut Backend, _snapshot, frame| {
                let result = encode(backend, &graph, frame);
                if racing {
                    // encode 중 document thread가 다음 세대를 알림 → 제출 직전 폐기.
                    latest.publish(empty_handle(next).scene_revision());
                }
                result
            })
            .unwrap();
        if racing {
            revision = next;
            if let TickOutcome::DroppedStale { after_encode, .. } = outcome {
                assert!(after_encode);
            }
        }
        recorder.record(&scheduler);
        if tick % 3 == 0 {
            backend.inner_mut().complete_next();
        }
    }
    recorder.finish(&scheduler)
}

#[test]
fn benchmark_result_includes_queue_depth_and_dropped_frames() {
    // 검증 기준(XI.11.5): queue depth ≤ 1 assertion.
    let metrics = run_slow_gpu_benchmark();
    metrics.assert_queue_depth().unwrap();
    metrics.check_accounting().unwrap();
    assert_eq!(metrics.limit, 1);
    assert_eq!(metrics.max_depth, 1);
    assert_eq!(metrics.ticks, 300);
    assert_eq!(metrics.depth_histogram.iter().sum::<u64>(), 300);
    assert_eq!(metrics.depth_histogram[DEPTH_BUCKETS - 1], 0);
    // 느린 GPU라 대부분의 snapshot이 버려지고 encode 뒤 폐기도 있습니다.
    assert!(metrics.dropped_stale_frames() > metrics.stats.submitted);
    assert!(metrics.stats.dropped_before_encode > 0);
    assert!(metrics.stats.dropped_after_encode > 0);
    assert!(metrics.stats.throttled > 0);

    // 결과 JSON에 지표가 고정 key 순서로 들어갑니다(결정적 모사라 값까지 고정).
    let json = metrics.to_json();
    assert_eq!(
        json,
        concat!(
            "{\"present_queue\":{\"limit\":1,\"max_depth\":1,\"ticks\":300,\"offered\":360,",
            "\"submitted\":100,\"dropped_stale_frames\":259,\"dropped_before_encode\":250,",
            "\"dropped_after_encode\":9,\"throttled_ticks\":191,\"skipped_ticks\":0,",
            "\"pending_at_end\":1,\"depth_histogram\":[9,291,0]}}"
        )
    );
    assert!(json.starts_with("{\"present_queue\":{\"limit\":1,\"max_depth\":1,\"ticks\":300,"));
    for key in [
        "\"offered\":",
        "\"submitted\":",
        "\"dropped_stale_frames\":",
        "\"dropped_before_encode\":",
        "\"dropped_after_encode\":",
        "\"throttled_ticks\":",
        "\"skipped_ticks\":",
        "\"pending_at_end\":",
        "\"depth_histogram\":[",
    ] {
        assert!(json.contains(key), "{key} 없음: {json}");
    }
    assert!(json.ends_with("]}}"));
    assert_eq!(json, run_slow_gpu_benchmark().to_json());
    let expected = format!(
        "\"dropped_stale_frames\":{}",
        metrics.stats.dropped_before_encode + metrics.stats.dropped_after_encode
    );
    assert!(json.contains(&expected));
}

#[test]
fn queue_depth_violation_fails_the_assertion() {
    let mut metrics = run_slow_gpu_benchmark();
    // 깊이 2 표본이 들어온 결과는 assertion 실패입니다.
    let mut recorder = PresentRecorder::new();
    recorder.record_depth(0);
    recorder.record_depth(2);
    assert_eq!(recorder.ticks(), 2);
    metrics.max_depth = 2;
    metrics.depth_histogram = [1, 0, 1];
    assert_eq!(
        metrics.assert_queue_depth(),
        Err(PresentMetricsError::QueueDepthExceeded {
            observed: 2,
            limit: 1
        })
    );
    // 분포의 초과 칸만 있어도 실패입니다.
    metrics.max_depth = 1;
    assert!(metrics.assert_queue_depth().is_err());
    // 집계 불일치도 잡습니다.
    let mut broken = run_slow_gpu_benchmark();
    broken.stats.offered += 1;
    assert!(matches!(
        broken.check_accounting(),
        Err(PresentMetricsError::AccountingMismatch { .. })
    ));
}
