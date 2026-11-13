//! frame scheduler 테스트 (XI.5.5).

use super::*;
use crate::backend::mock::MockBackend;
use crate::backend::tests::{device_config, surface_graph};
use crate::backend::{
    BackendCall, BackendErrorKind, CheckedBackend, FrameEncodeRequest, ImportBinding, ImportTarget,
};
use crate::render_graph::{CompiledRenderGraph, ResourceId};
use crate::test_scene::empty_handle;

const IMPORTS: [ImportBinding; 1] = [ImportBinding {
    resource: ResourceId(0),
    target: ImportTarget::Surface,
}];

type Backend = CheckedBackend<MockBackend>;

fn backend() -> Backend {
    let mut backend = CheckedBackend::new(MockBackend::new());
    backend.init(&device_config(64, 64)).unwrap();
    backend
}

/// surface graph로 encode하는 closure 본문.
fn encode_with(
    graph: &CompiledRenderGraph,
) -> impl Fn(&mut Backend, &SnapshotHandle, FrameId) -> Result<EncodeOutcome, BackendError> + '_ {
    move |backend, _snapshot, frame| {
        backend.encode(&FrameEncodeRequest {
            frame,
            graph,
            passes: &[],
            imports: &IMPORTS,
        })
    }
}

/// document thread 모사: 세대를 먼저 알리고 handle을 넘깁니다(XI.1.6 순서).
fn publish(latest: &LatestSceneRevision, revision: u64) -> SnapshotHandle {
    let handle = empty_handle(revision);
    latest.publish(handle.scene_revision());
    handle
}

#[test]
fn slow_gpu_keeps_present_queue_depth_at_most_one() {
    // 검증 기준(XI.5.5): slow GPU simulation에서 queue depth ≤ 1.
    let graph = surface_graph();
    let encode = encode_with(&graph);
    let latest = LatestSceneRevision::new();
    let mut scheduler = FrameScheduler::new(latest.clone());
    let mut backend = backend();
    let mut presented_revisions: Vec<u64> = Vec::new();
    let mut submitted_revisions: Vec<(SubmissionId, u64)> = Vec::new();
    let mut max_gpu_backlog = 0;

    for tick in 1..=300u64 {
        // document thread는 tick마다 새 snapshot을 만들고(가끔 2개 연속), render thread는 tick마다 돕니다.
        scheduler.offer(publish(&latest, tick * 2 - 1));
        if tick % 7 == 0 {
            scheduler.offer(publish(&latest, tick * 2));
        }
        let outcome = scheduler.tick(&mut backend, &encode).unwrap();
        if let TickOutcome::Submitted {
            submission,
            scene_revision,
            ..
        } = outcome
        {
            submitted_revisions.push((submission, scene_revision));
        }
        assert!(scheduler.present_queue_depth() <= MAX_PRESENT_QUEUE_DEPTH);
        max_gpu_backlog = max_gpu_backlog.max(backend.inner().in_flight());
        // 느린 GPU: 4 tick마다 frame 하나만 끝내고 present합니다.
        if tick % 4 == 0 {
            if let Some(done) = backend.inner_mut().complete_next() {
                let revision = submitted_revisions
                    .iter()
                    .find(|(submission, _)| *submission == done)
                    .map(|(_, revision)| *revision)
                    .unwrap();
                presented_revisions.push(revision);
            }
        }
    }
    let stats = scheduler.stats();
    assert!(stats.max_queue_depth <= MAX_PRESENT_QUEUE_DEPTH);
    assert_eq!(stats.max_queue_depth, 1);
    // GPU에 쌓인 frame도 1개를 넘지 않습니다(present queue가 막아 줌).
    assert!(max_gpu_backlog <= 1);
    assert!(stats.throttled > 0);
    // 느린 GPU 때문에 대부분의 snapshot은 그려지지 않고 최신 것으로 대체됩니다.
    assert!(stats.dropped_stale() > stats.submitted);
    assert_eq!(
        stats.offered,
        stats.submitted + stats.dropped_stale() + u64::from(scheduler.pending_revision().is_some())
    );
    // 화면에 나간 revision은 엄격히 증가합니다(오래된 frame이 새 frame 뒤에 나가지 않음).
    assert!(presented_revisions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(presented_revisions.len() > 50);

    // GPU가 따라잡으면 마지막 snapshot이 결국 그려집니다.
    backend.inner_mut().complete_all();
    let final_tick = scheduler.tick(&mut backend, &encode).unwrap();
    let last = latest.latest().unwrap().0;
    match final_tick {
        TickOutcome::Submitted { scene_revision, .. } => assert_eq!(scene_revision, last),
        TickOutcome::Idle => assert_eq!(scheduler.last_submitted_revision(), Some(last)),
        other => panic!("예상하지 못한 결과: {other:?}"),
    }
}

#[test]
fn newer_snapshot_replaces_pending_and_late_snapshots_are_dropped() {
    let latest = LatestSceneRevision::new();
    let mut scheduler = FrameScheduler::new(latest.clone());
    assert!(scheduler.offer(publish(&latest, 1)).is_none());
    // 새 snapshot이 아직 그리지 않은 snapshot을 밀어냅니다.
    let replaced = scheduler.offer(publish(&latest, 2)).unwrap();
    assert_eq!(replaced.scene_revision().0, 1);
    assert_eq!(scheduler.pending_revision(), Some(2));
    // 늦게 도착한 더 오래된 snapshot은 받지 않습니다.
    let late = scheduler.offer(empty_handle(1)).unwrap();
    assert_eq!(late.scene_revision().0, 1);
    assert_eq!(scheduler.pending_revision(), Some(2));
    assert_eq!(scheduler.stats().dropped_before_encode, 2);

    let graph = surface_graph();
    let encode = encode_with(&graph);
    let mut backend = backend();
    assert!(matches!(
        scheduler.tick(&mut backend, &encode).unwrap(),
        TickOutcome::Submitted {
            scene_revision: 2,
            ..
        }
    ));
    // 이미 제출한 revision 이하는 받지 않습니다.
    assert!(scheduler.offer(empty_handle(2)).is_some());
    assert_eq!(
        scheduler.tick(&mut backend, &encode).unwrap(),
        TickOutcome::Idle
    );
}

#[test]
fn stale_snapshots_are_dropped_before_and_after_encode() {
    let graph = surface_graph();
    let latest = LatestSceneRevision::new();
    let mut scheduler = FrameScheduler::new(latest.clone());
    let mut backend = backend();

    // encode 전: pending(3)보다 새로운 세대(4)가 이미 알려짐(handle은 아직 도착 전).
    scheduler.offer(publish(&latest, 3));
    latest.publish(empty_handle(4).scene_revision());
    let calls_before = backend.inner().calls.len();
    assert_eq!(
        scheduler.tick(&mut backend, encode_with(&graph)).unwrap(),
        TickOutcome::DroppedStale {
            stale: 3,
            latest: 4,
            after_encode: false
        }
    );
    // poll 외에는 backend를 부르지 않았습니다(encode 안 함).
    assert_eq!(backend.inner().calls[calls_before..], [BackendCall::Poll]);

    // encode 후: encode하는 동안 document thread가 세대 6을 알림 → 제출하지 않고 버립니다.
    scheduler.offer(publish(&latest, 5));
    let racing = latest.clone();
    let outcome = scheduler
        .tick(&mut backend, |backend: &mut Backend, snapshot, frame| {
            let result = encode_with(&graph)(backend, snapshot, frame);
            racing.publish(empty_handle(6).scene_revision());
            result
        })
        .unwrap();
    assert_eq!(
        outcome,
        TickOutcome::DroppedStale {
            stale: 5,
            latest: 6,
            after_encode: true
        }
    );
    assert_eq!(backend.inner().discarded.len(), 1);
    assert_eq!(backend.inner().in_flight(), 0);
    assert_eq!(scheduler.present_queue_depth(), 0);
    assert_eq!(
        backend.inner().calls[backend.inner().calls.len() - 2..],
        [BackendCall::Encode, BackendCall::Discard]
    );
    let stats = scheduler.stats();
    assert_eq!(stats.dropped_before_encode, 1);
    assert_eq!(stats.dropped_after_encode, 1);
    assert_eq!(stats.submitted, 0);

    // 최신(6)이 오면 그립니다.
    scheduler.offer(empty_handle(6));
    assert!(matches!(
        scheduler.tick(&mut backend, encode_with(&graph)).unwrap(),
        TickOutcome::Submitted {
            scene_revision: 6,
            ..
        }
    ));
}

#[test]
fn skips_and_errors_keep_the_snapshot_for_retry() {
    let graph = surface_graph();
    let latest = LatestSceneRevision::new();
    let mut scheduler = FrameScheduler::new(latest.clone());
    let mut backend = backend();
    scheduler.offer(publish(&latest, 1));

    backend.inner_mut().no_drawable = true;
    assert_eq!(
        scheduler.tick(&mut backend, encode_with(&graph)).unwrap(),
        TickOutcome::Skipped(SkipReason::NoDrawable)
    );
    assert_eq!(scheduler.pending_revision(), Some(1));
    backend.inner_mut().no_drawable = false;

    backend.inner_mut().fail_next = Some((BackendCall::Encode, BackendErrorKind::Timeout));
    assert_eq!(
        scheduler
            .tick(&mut backend, encode_with(&graph))
            .unwrap_err()
            .kind(),
        BackendErrorKind::Timeout
    );
    assert_eq!(scheduler.pending_revision(), Some(1));

    backend.inner_mut().fail_next =
        Some((BackendCall::SchedulePresent, BackendErrorKind::SurfaceLost));
    assert_eq!(
        scheduler
            .tick(&mut backend, encode_with(&graph))
            .unwrap_err()
            .kind(),
        BackendErrorKind::SurfaceLost
    );
    // present 실패 frame은 버려지고(GPU 미제출) snapshot은 남습니다.
    assert_eq!(backend.inner().discarded.len(), 1);
    assert_eq!(scheduler.pending_revision(), Some(1));

    backend.inner_mut().fail_next = Some((BackendCall::Submit, BackendErrorKind::DeviceLost));
    assert_eq!(
        scheduler
            .tick(&mut backend, encode_with(&graph))
            .unwrap_err()
            .kind(),
        BackendErrorKind::DeviceLost
    );
    assert_eq!(scheduler.pending_revision(), Some(1));
    // device 재생성 후 이어서 그립니다.
    backend.init(&device_config(64, 64)).unwrap();
    scheduler.reset_present_queue();
    assert!(matches!(
        scheduler.tick(&mut backend, encode_with(&graph)).unwrap(),
        TickOutcome::Submitted {
            scene_revision: 1,
            ..
        }
    ));
    assert_eq!(scheduler.stats().skipped, 1);

    // present queue가 차면 다음 snapshot은 기다립니다.
    scheduler.offer(publish(&latest, 2));
    assert_eq!(
        scheduler.tick(&mut backend, encode_with(&graph)).unwrap(),
        TickOutcome::Throttled { queue_depth: 1 }
    );
    backend.inner_mut().complete_all();
    assert!(matches!(
        scheduler.tick(&mut backend, encode_with(&graph)).unwrap(),
        TickOutcome::Submitted {
            scene_revision: 2,
            ..
        }
    ));
}
