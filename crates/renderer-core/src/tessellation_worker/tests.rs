//! tessellation worker pool 테스트 (XI.6.3).

use super::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Condvar;

use stellar_scene::geometry::PathCommand;
use stellar_scene::Vec2;

use crate::stroke::ReferenceStrokeTessellator;
use crate::test_scene::empty_handle;
use crate::thread_role::current_thread_role;

const WAIT: Duration = Duration::from_secs(10);

/// 테스트가 열 때까지 worker를 붙잡는 문.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn open(&self) {
        if let Ok(mut open) = self.open.lock() {
            *open = true;
        }
        self.changed.notify_all();
    }

    fn wait(&self) {
        let Ok(mut open) = self.open.lock() else {
            return;
        };
        while !*open {
            open = match self.changed.wait(open) {
                Ok(guard) => guard,
                Err(_) => return,
            };
        }
    }
}

/// gate를 지난 뒤 subpath anchor fan으로 채우고, 실행 thread 역할을 검사하는 fill tessellator.
struct GatedFan {
    gate: Arc<Gate>,
    calls: AtomicUsize,
    non_worker_calls: AtomicUsize,
}

impl GatedFan {
    fn new(gate: Arc<Gate>) -> GatedFan {
        GatedFan {
            gate,
            calls: AtomicUsize::new(0),
            non_worker_calls: AtomicUsize::new(0),
        }
    }
}

impl FillTessellator for GatedFan {
    fn tessellate_fill(
        &self,
        path: &PathGeometry,
        _style: &FillStyle,
    ) -> Result<FillMesh, TessellationError> {
        if current_thread_role() != Some(ThreadRole::Worker) {
            self.non_worker_calls.fetch_add(1, Ordering::SeqCst);
        }
        self.gate.wait();
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut vertices = Vec::new();
        for command in path.commands() {
            if let PathCommand::MoveTo(v) | PathCommand::LineTo(v) = command {
                vertices.push(*v);
            }
        }
        if vertices.len() < 3 {
            return Err(TessellationError::Failed {
                reason: "삼각형을 만들 수 없음",
            });
        }
        let indices = (1..vertices.len() as u32 - 1)
            .flat_map(|i| [0, i, i + 1])
            .collect();
        FillMesh::new(vertices, indices)
    }
}

fn square(size: f32) -> PathGeometry {
    let p = |x: f32, y: f32| Vec2::new(x, y).unwrap();
    PathGeometry::new(vec![
        PathCommand::MoveTo(p(0.0, 0.0)),
        PathCommand::LineTo(p(size, 0.0)),
        PathCommand::LineTo(p(size, size)),
        PathCommand::LineTo(p(0.0, size)),
        PathCommand::Close,
    ])
    .unwrap()
}

fn fill(size: f32) -> TessellationInput {
    TessellationInput::Fill {
        path: square(size),
        style: FillStyle::default(),
    }
}

struct Fixture {
    gate: Arc<Gate>,
    fan: Arc<GatedFan>,
    latest: LatestSceneRevision,
    async_tess: AsyncTessellation,
    cache: TessellationCache,
}

fn fixture(workers: usize, capacity: usize, max_in_flight: usize) -> Fixture {
    let gate = Arc::new(Gate::default());
    let fan = Arc::new(GatedFan::new(Arc::clone(&gate)));
    let pool = TessellationWorkerPool::new(
        workers,
        capacity,
        Tessellators {
            fill: Arc::clone(&fan) as Arc<dyn FillTessellator>,
            stroke: Arc::new(ReferenceStrokeTessellator),
        },
    )
    .unwrap();
    let latest = LatestSceneRevision::new();
    let async_tess = AsyncTessellation::new(pool, latest.clone(), max_in_flight);
    Fixture {
        gate,
        fan,
        latest,
        async_tess,
        cache: TessellationCache::new(),
    }
}

impl Fixture {
    fn publish(&self, revision: u64) {
        self.latest.publish(empty_handle(revision).scene_revision());
    }

    /// 진행 중 job을 모두 처리할 때까지 기다립니다.
    fn drain(&mut self) -> Vec<CollectedResult> {
        let mut out = Vec::new();
        while self.async_tess.in_flight() > 0 {
            let result = self
                .async_tess
                .collect_one_blocking(&mut self.cache, WAIT)
                .expect("worker 결과 대기 시간 초과");
            out.push(result);
        }
        out
    }
}

#[test]
fn stale_revision_results_are_never_applied() {
    // 검증 기준(XI.6.3): stale result 적용 0.
    let mut fx = fixture(2, 8, 8);
    fx.publish(1);
    let a = fill(1.0);
    let b = fill(2.0);
    let c = fill(3.0);
    for input in [a.clone(), c.clone()] {
        assert!(matches!(
            fx.async_tess.request(&mut fx.cache, 1, input),
            RequestOutcome::Pending
        ));
    }
    // 같은 key를 같은 revision에서 다시 요청하면 진행 중 job과 합칩니다.
    assert!(matches!(
        fx.async_tess.request(&mut fx.cache, 1, a.clone()),
        RequestOutcome::Pending
    ));
    assert_eq!(fx.async_tess.stats().coalesced, 1);

    // document가 revision 2로 넘어갔고, revision 2는 b와 c만 필요합니다(a는 더 이상 없음).
    fx.publish(2);
    fx.async_tess.request(&mut fx.cache, 2, b.clone());
    fx.async_tess.request(&mut fx.cache, 2, c.clone());
    assert_eq!(fx.async_tess.in_flight_revision(&c.key()), Some(2));
    assert_eq!(fx.async_tess.in_flight_revision(&a.key()), Some(1));

    fx.gate.open();
    let results = fx.drain();
    assert_eq!(results.len(), 3);
    for result in &results {
        match result {
            CollectedResult::Applied { key, revision } => {
                assert!(*key == b.key() || *key == c.key());
                assert_eq!(*revision, 2);
            }
            CollectedResult::DiscardedStale {
                key,
                revision,
                latest,
            } => {
                assert_eq!(*key, a.key());
                assert_eq!((*revision, *latest), (1, 2));
            }
            CollectedResult::Failed { .. } => panic!("실패 결과 없음"),
        }
    }
    // stale 결과(a)는 cache에 없고, 최신 revision이 요청한 b·c만 있습니다.
    assert!(fx.cache.lookup(&a.key()).is_none());
    assert!(fx.cache.lookup(&b.key()).is_some());
    assert!(fx.cache.lookup(&c.key()).is_some());
    let stats = fx.async_tess.stats();
    assert_eq!((stats.applied, stats.discarded_stale), (2, 1));
    // 모든 tessellation은 worker thread에서 실행되었습니다.
    assert_eq!(fx.fan.non_worker_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fx.fan.calls.load(Ordering::SeqCst), 3);

    // cache에 들어간 결과는 다음 요청에서 바로 Ready입니다. 버린 a는 다시 제출됩니다.
    assert!(matches!(
        fx.async_tess.request(&mut fx.cache, 2, b.clone()),
        RequestOutcome::Ready(_)
    ));
    assert!(matches!(
        fx.async_tess.request(&mut fx.cache, 2, a.clone()),
        RequestOutcome::Pending
    ));
    let again = fx.drain();
    assert!(matches!(
        again[..],
        [CollectedResult::Applied { revision: 2, .. }]
    ));
}

#[test]
fn failures_are_not_cached_and_limits_defer() {
    let mut fx = fixture(1, 4, 2);
    fx.publish(1);
    // 꼭짓점 2개짜리 path는 GatedFan이 실패시킵니다.
    let p = |x: f32, y: f32| Vec2::new(x, y).unwrap();
    let degenerate = TessellationInput::Fill {
        path: PathGeometry::new(vec![
            PathCommand::MoveTo(p(0.0, 0.0)),
            PathCommand::LineTo(p(1.0, 0.0)),
        ])
        .unwrap(),
        style: FillStyle::default(),
    };
    assert!(matches!(
        fx.async_tess.request(&mut fx.cache, 1, degenerate.clone()),
        RequestOutcome::Pending
    ));
    fx.async_tess.request(&mut fx.cache, 1, fill(1.0));
    // 진행 중 상한 2에 걸려 미룹니다(render thread는 막히지 않음).
    assert!(matches!(
        fx.async_tess.request(&mut fx.cache, 1, fill(2.0)),
        RequestOutcome::Deferred
    ));
    assert_eq!(fx.async_tess.stats().deferred, 1);
    fx.gate.open();
    let results = fx.drain();
    assert!(results
        .iter()
        .any(|r| matches!(r, CollectedResult::Failed { key, .. } if *key == degenerate.key())));
    assert!(fx.cache.lookup(&degenerate.key()).is_none());
    assert_eq!(fx.async_tess.stats().failed, 1);
}

#[test]
fn stroke_jobs_run_on_workers_and_match_inline_result() {
    let mut fx = fixture(2, 4, 4);
    fx.gate.open();
    let handle = empty_handle(5);
    fx.latest.publish(handle.scene_revision());
    let style = StrokeStyle::new(2.0).unwrap();
    let input = TessellationInput::Stroke {
        path: square(10.0),
        style,
    };
    assert!(matches!(
        fx.async_tess
            .request_for(&mut fx.cache, &handle, input.clone()),
        RequestOutcome::Pending
    ));
    let results = fx.drain();
    assert!(matches!(
        results[..],
        [CollectedResult::Applied { revision: 5, .. }]
    ));
    let RequestOutcome::Ready(mesh) = fx.async_tess.request(&mut fx.cache, 5, input) else {
        panic!("cache에 있어야 합니다");
    };
    let inline = crate::stroke::tessellate_stroke(&square(10.0), &style).unwrap();
    assert_eq!(*mesh, inline);
}

/// 결정적 xorshift64.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

#[test]
fn interleaved_revisions_never_apply_stale_results() {
    // 빠르게 바뀌는 200 revision, worker 3개, 결과 도착 순서 비결정: 적용된 결과는 항상 판정 시점
    // 최신 revision이 요청한 key입니다.
    let mut fx = fixture(3, 16, 16);
    fx.gate.open();
    let universe: Vec<TessellationInput> = (1..=12).map(|i| fill(i as f32)).collect();
    let mut rng = Rng(0x0006_0003);
    let mut wanted_by_revision: HashMap<u64, HashSet<TessellationKey>> = HashMap::new();
    let mut applied = 0u64;
    let mut discarded = 0u64;
    for revision in 1..=200u64 {
        fx.publish(revision);
        let wanted = wanted_by_revision.entry(revision).or_default();
        for _ in 0..1 + rng.below(4) {
            let input = &universe[rng.below(universe.len() as u64) as usize];
            wanted.insert(input.key());
            fx.async_tess
                .request(&mut fx.cache, revision, input.clone());
        }
        // 다음 revision 전에 처리할지 무작위로 정합니다(처리 전 revision이 넘어가면 stale).
        let results = if rng.below(3) == 0 {
            fx.drain()
        } else {
            fx.async_tess.collect(&mut fx.cache)
        };
        for result in results {
            match result {
                CollectedResult::Applied { key, revision: r } => {
                    applied += 1;
                    assert_eq!(r, revision, "적용 결과는 최신 revision 요청이어야 합니다");
                    assert!(wanted_by_revision[&revision].contains(&key));
                }
                CollectedResult::DiscardedStale {
                    revision: r,
                    latest,
                    ..
                } => {
                    discarded += 1;
                    assert!(r < latest);
                    assert_eq!(latest, revision);
                }
                CollectedResult::Failed { .. } => panic!("실패 없음"),
            }
        }
    }
    let results = fx.drain();
    for result in &results {
        if let CollectedResult::Applied { revision, .. } = result {
            assert_eq!(*revision, 200);
        }
    }
    let stats = fx.async_tess.stats();
    assert_eq!(stats.applied + stats.discarded_stale, stats.submitted);
    assert!(applied > 0);
    let _ = discarded;
    assert_eq!(fx.fan.non_worker_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn pool_validation_and_shutdown() {
    let tessellators = || Tessellators {
        fill: Arc::new(GatedFan::new(Arc::new(Gate::default()))) as Arc<dyn FillTessellator>,
        stroke: Arc::new(ReferenceStrokeTessellator),
    };
    assert_eq!(
        TessellationWorkerPool::new(0, 1, tessellators()).unwrap_err(),
        WorkerPoolError::InvalidWorkerCount { requested: 0 }
    );
    assert_eq!(
        TessellationWorkerPool::new(MAX_TESSELLATION_WORKERS + 1, 1, tessellators()).unwrap_err(),
        WorkerPoolError::InvalidWorkerCount {
            requested: MAX_TESSELLATION_WORKERS + 1
        }
    );
    assert_eq!(
        TessellationWorkerPool::new(1, 0, tessellators()).unwrap_err(),
        WorkerPoolError::InvalidCapacity
    );
    // 대기 중 worker가 있어도 drop은 queue를 닫고 join합니다(멈추지 않음).
    let pool = TessellationWorkerPool::new(4, 2, tessellators()).unwrap();
    assert_eq!(pool.worker_count(), 4);
    assert!(pool.try_recv().is_none());
    drop(pool);
}
