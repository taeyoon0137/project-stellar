//! 제출 전 오래된 snapshot 감지 (XI.1.6).
//!
//! 오래된 렌더 결과는 세대 번호([`SceneRevision`])로 폐기합니다(아키텍처 §9.3).
//! [`LatestSceneRevision`]은 document thread와 render thread가 공유하는 atomic 최신 세대 번호입니다.
//!
//! - document thread는 snapshot을 render thread에 넘기기 **전에** [`LatestSceneRevision::publish`]로
//!   그 세대를 알립니다. 값은 단조 증가하며(`fetch_max`), 늦게 도착한 더 작은 값은 무시됩니다.
//! - render thread는 GPU 명령을 제출하기 **직전**에 [`LatestSceneRevision::check`]를 호출합니다.
//!   그 사이 더 새로운 세대가 알려졌으면 [`PresentCheck::Discard`]를 받고 제출하지 않습니다.
//!
//! 두 연산 모두 lock 없이 atomic 하나만 씁니다. document thread는 render thread를 기다리지 않습니다.
//! present 대기열 깊이 제한(최대 1)은 frame scheduler(XI.5.5)가 이 판정을 써서 구현합니다.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use stellar_core_types::revision::SceneRevision;

use crate::handle::SnapshotHandle;

/// 제출 직전 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentCheck {
    /// 알려진 최신 세대입니다. 제출(present 후보)해도 됩니다.
    Present {
        /// 후보 세대.
        revision: SceneRevision,
    },
    /// 더 새로운 세대가 알려져 이 결과는 오래되었습니다. 제출하지 않고 폐기합니다.
    Discard {
        /// 폐기할 세대.
        stale: SceneRevision,
        /// 판정 시점의 최신 세대.
        latest: SceneRevision,
    },
}

impl PresentCheck {
    /// 제출해도 되면 참입니다.
    pub const fn is_present(&self) -> bool {
        matches!(self, PresentCheck::Present { .. })
    }
}

/// document thread와 render thread가 공유하는 최신 scene 세대 번호.
///
/// 복제하면 같은 값을 공유합니다. 0은 "아직 없음"입니다(scene revision은 1부터, XI.1.3).
#[derive(Debug, Clone, Default)]
pub struct LatestSceneRevision(Arc<AtomicU64>);

impl LatestSceneRevision {
    /// 아직 알려진 세대가 없는 상태.
    pub fn new() -> LatestSceneRevision {
        LatestSceneRevision(Arc::new(AtomicU64::new(0)))
    }

    /// 새 세대를 알립니다. 반환값은 알림 후의 최신 세대입니다(더 작은 값이면 기존 값 유지).
    pub fn publish(&self, revision: SceneRevision) -> SceneRevision {
        let previous = self.0.fetch_max(revision.0, Ordering::AcqRel);
        SceneRevision(previous.max(revision.0))
    }

    /// 지금까지 알려진 최신 세대.
    pub fn latest(&self) -> Option<SceneRevision> {
        match self.0.load(Ordering::Acquire) {
            0 => None,
            value => Some(SceneRevision(value)),
        }
    }

    /// `candidate` 세대 결과를 제출해도 되는지 판정합니다.
    ///
    /// 알려진 최신 세대보다 작으면 폐기, 같거나 크면(아직 알리기 전 세대 포함) 제출입니다.
    pub fn check(&self, candidate: SceneRevision) -> PresentCheck {
        let latest = self.0.load(Ordering::Acquire);
        if candidate.0 < latest {
            PresentCheck::Discard {
                stale: candidate,
                latest: SceneRevision(latest),
            }
        } else {
            PresentCheck::Present {
                revision: candidate,
            }
        }
    }

    /// handle이 가리키는 snapshot 세대로 [`LatestSceneRevision::check`]를 호출합니다.
    pub fn check_handle(&self, handle: &SnapshotHandle) -> PresentCheck {
        self.check(handle.scene_revision())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    use stellar_core_types::revision::DocumentRevision;

    use crate::fixtures::{self, FixtureConfig};
    use crate::node::SceneNode;
    use crate::revision::SceneRevisionMap;
    use crate::snapshot::SceneSnapshot;
    use crate::stack::{PaintLayer, PaintStack, PaintStackRef};

    /// `count`개 세대를 구조 공유로 만듭니다(매 세대 첫 leaf paint만 변경).
    fn revisions(count: u64) -> Vec<SnapshotHandle> {
        let base = fixtures::generate(FixtureConfig {
            seed: 0x16,
            roots: 2,
            children: 4,
            depth: 2,
        });
        let mut map = SceneRevisionMap::new();
        let mut current: SceneSnapshot = map
            .generate(DocumentRevision(1), base.roots().cloned().collect())
            .expect("base");
        let mut out = vec![SnapshotHandle::new(current.clone())];
        for revision in 2..=count {
            let leaf = current
                .root(0)
                .and_then(|root| root.child(0))
                .and_then(|frame| frame.child(0))
                .expect("leaf");
            let mut spec = leaf.spec();
            spec.paint = PaintStackRef::new(PaintStack {
                layers: vec![PaintLayer::Fill(fixtures::solid(
                    (revision % 10) as f32 / 10.0,
                    0.0,
                    0.0,
                    1.0,
                ))],
                ..PaintStack::empty()
            });
            current = map
                .derive(
                    &current,
                    DocumentRevision(revision),
                    &[0, 0, 0],
                    Arc::new(SceneNode::leaf(spec)),
                )
                .expect("derive");
            out.push(SnapshotHandle::new(current.clone()));
        }
        out
    }

    #[test]
    fn publish_is_monotonic_and_check_compares_against_latest() {
        let latest = LatestSceneRevision::new();
        assert_eq!(latest.latest(), None);
        assert!(latest.check(SceneRevision(1)).is_present());
        assert_eq!(latest.publish(SceneRevision(5)), SceneRevision(5));
        // 늦게 도착한 작은 값은 무시됩니다.
        assert_eq!(latest.publish(SceneRevision(3)), SceneRevision(5));
        assert_eq!(latest.latest(), Some(SceneRevision(5)));
        assert_eq!(
            latest.check(SceneRevision(4)),
            PresentCheck::Discard {
                stale: SceneRevision(4),
                latest: SceneRevision(5)
            }
        );
        assert_eq!(
            latest.check(SceneRevision(5)),
            PresentCheck::Present {
                revision: SceneRevision(5)
            }
        );
        // 복제본은 같은 값을 공유합니다.
        let shared = latest.clone();
        shared.publish(SceneRevision(9));
        assert!(!latest.check(SceneRevision(5)).is_present());
    }

    #[test]
    fn rapid_100_revisions_present_only_latest_scripted() {
        // 검증 기준(XI.1.6): 빠른 100 revision fixture에서 최신 결과만 present 후보.
        // 결정론적 interleaving: renderer가 frame 하나를 encode하는 동안 document thread가
        // 3세대를 더 만듭니다. 제출 직전 check가 오래된 frame을 모두 폐기해야 합니다.
        let handles = revisions(100);
        let latest = LatestSceneRevision::new();
        let mut presented = Vec::new();
        let mut discarded = 0;
        let mut published = 0usize;
        while published < handles.len() {
            // document thread: 새 세대 publish 후 handoff.
            latest.publish(handles[published].scene_revision());
            let in_flight = handles[published].clone();
            published += 1;
            // renderer encode 중 document thread가 최대 3세대를 더 publish.
            for _ in 0..3 {
                if published < handles.len() {
                    latest.publish(handles[published].scene_revision());
                    published += 1;
                }
            }
            match latest.check_handle(&in_flight) {
                PresentCheck::Present { revision } => presented.push(revision),
                PresentCheck::Discard { stale, latest } => {
                    assert!(stale < latest);
                    discarded += 1;
                }
            }
        }
        // 마지막으로 renderer가 최신 handle을 받아 제출합니다.
        let newest = handles.last().expect("100세대");
        assert!(latest.check_handle(newest).is_present());
        presented.push(newest.scene_revision());

        assert_eq!(discarded, 25);
        assert_eq!(presented, vec![SceneRevision(100)]);
    }

    #[test]
    fn rapid_100_revisions_present_only_latest_threaded() {
        // 실제 thread: document thread가 100세대를 쉬지 않고 publish·전달하고, renderer는
        // 받은 것 중 최신만 골라 느리게 encode한 뒤 제출 직전 check합니다.
        let handles = revisions(100);
        let latest = LatestSceneRevision::new();
        let (tx, rx) = mpsc::channel::<SnapshotHandle>();
        let renderer_latest = latest.clone();
        let renderer = thread::spawn(move || {
            let mut presented: Vec<SceneRevision> = Vec::new();
            let mut discarded = 0u32;
            let mut newest: Option<SnapshotHandle> = None;
            loop {
                // 대기 중인 handle을 모두 비우고 가장 새 것만 남깁니다.
                let mut disconnected = false;
                loop {
                    match rx.try_recv() {
                        Ok(handle) => newest = Some(handle),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
                if let Some(frame) = newest.take() {
                    // 느린 encode: snapshot 전체를 읽습니다.
                    let _ = frame.semantic_hash();
                    thread::yield_now();
                    match renderer_latest.check_handle(&frame) {
                        PresentCheck::Present { revision } => {
                            if let Some(last) = presented.last() {
                                assert!(revision > *last, "present 순서 역전");
                            }
                            presented.push(revision);
                        }
                        PresentCheck::Discard { stale, latest } => {
                            assert!(stale < latest);
                            discarded += 1;
                        }
                    }
                } else if disconnected {
                    break;
                } else {
                    thread::yield_now();
                }
            }
            (presented, discarded)
        });
        for handle in &handles {
            latest.publish(handle.scene_revision());
            tx.send(handle.clone()).expect("renderer alive");
        }
        drop(tx);
        let (presented, discarded) = renderer.join().expect("renderer panic 없음");
        // 마지막 제출은 반드시 최신 세대이고, 제출된 모든 세대는 증가 순서입니다.
        assert_eq!(presented.last(), Some(&SceneRevision(100)));
        assert!(presented.windows(2).all(|w| w[0] < w[1]));
        // 판정 시점에 최신이 아니던 세대는 하나도 제출되지 않았습니다(Discard 경로가 검사).
        assert!(presented.len() as u32 + discarded <= 100);
    }
}
