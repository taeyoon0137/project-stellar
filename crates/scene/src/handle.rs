//! render thread용 읽기 전용 snapshot handle (XI.1.5).
//!
//! document thread가 만든 [`SceneSnapshot`]은 [`SnapshotHandle`](`Arc<SceneSnapshot>`)로 render
//! thread에 넘어갑니다. 읽기 규칙은 다음과 같습니다(아키텍처 §9.3).
//!
//! - **잠금 없음**: snapshot은 생성 후 바뀌지 않고 내부 가변성(`Cell`·`Mutex` 등)이 없습니다.
//!   handle을 가진 thread는 `&SceneSnapshot`으로 바로 읽으며 어떤 lock도 잡지 않습니다.
//! - **가변 참조 없음**: handle은 `Deref`만 구현하고 `DerefMut`·`get_mut`을 제공하지 않습니다.
//!   scene API에는 `&mut self` 메서드가 없고, 갱신은 항상 새 snapshot을 만듭니다(XI.1.4).
//! - **수명은 참조 카운트**: 마지막 handle이 drop될 때 snapshot이 회수됩니다. 이전 snapshot을
//!   읽는 중인 render thread가 있으면 document thread가 새 snapshot을 만들어도 이전 값이 유효합니다.
//!   [`SnapshotLifetime`]으로 회수 여부를 관찰할 수 있습니다.
//!
//! 최신 handle을 render thread에 전달하는 lock-free handoff는 RenderThread(XII.1.3)가 구현합니다.
//!
//! handle로는 가변 참조를 얻을 수 없습니다(`DerefMut` 없음, 컴파일 실패 E0596).
//!
//! ```compile_fail,E0596
//! use stellar_scene::handle::SnapshotHandle;
//! use stellar_scene::SceneSnapshot;
//!
//! fn mutate(handle: &mut SnapshotHandle) {
//!     let _snapshot: &mut SceneSnapshot = &mut *handle;
//! }
//! ```
//!
//! 같은 코드에서 읽기 전용 참조는 허용됩니다.
//!
//! ```
//! use stellar_scene::handle::SnapshotHandle;
//! use stellar_scene::SceneSnapshot;
//!
//! fn read(handle: &mut SnapshotHandle) -> u64 {
//!     let snapshot: &SceneSnapshot = &*handle;
//!     snapshot.node_count()
//! }
//! ```

use std::ops::Deref;
use std::sync::{Arc, Weak};

use stellar_core_types::revision::SceneRevision;

use crate::snapshot::SceneSnapshot;

/// 공유 읽기 전용 snapshot handle. 복제는 참조 카운트 증가뿐입니다.
#[derive(Debug, Clone)]
pub struct SnapshotHandle(Arc<SceneSnapshot>);

impl SnapshotHandle {
    /// snapshot을 handle로 감쌉니다. 이후 snapshot은 handle을 통해서만 읽습니다.
    pub fn new(snapshot: SceneSnapshot) -> SnapshotHandle {
        SnapshotHandle(Arc::new(snapshot))
    }

    /// snapshot 읽기.
    pub fn snapshot(&self) -> &SceneSnapshot {
        &self.0
    }

    /// scene revision(세대 번호).
    pub fn scene_revision(&self) -> SceneRevision {
        self.0.scene_revision()
    }

    /// 같은 snapshot을 가리키면 참입니다.
    pub fn ptr_eq(&self, other: &SnapshotHandle) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// snapshot 회수 여부를 관찰하는 약한 참조. snapshot 수명을 늘리지 않습니다.
    pub fn lifetime(&self) -> SnapshotLifetime {
        SnapshotLifetime(Arc::downgrade(&self.0))
    }
}

impl Deref for SnapshotHandle {
    type Target = SceneSnapshot;

    fn deref(&self) -> &SceneSnapshot {
        &self.0
    }
}

/// snapshot 회수 관찰자(약한 참조).
#[derive(Debug, Clone)]
pub struct SnapshotLifetime(Weak<SceneSnapshot>);

impl SnapshotLifetime {
    /// 아직 handle이 하나라도 남아 있으면 참입니다.
    pub fn is_alive(&self) -> bool {
        self.0.strong_count() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    use stellar_core_types::revision::DocumentRevision;

    use crate::fixtures::{self, FixtureConfig, SplitMix64};
    use crate::node::{SceneNode, ZOrder};
    use crate::revision::SceneRevisionMap;
    use crate::snapshot::SceneSemanticHash;
    use crate::stack::{PaintLayer, PaintStack, PaintStackRef};

    #[test]
    fn handle_is_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<SnapshotHandle>();
        assert_send_sync::<SnapshotLifetime>();
    }

    /// 무작위 leaf 경로(fixture는 모든 leaf가 같은 깊이).
    fn random_leaf_path(rng: &mut SplitMix64, snapshot: &SceneSnapshot) -> Vec<usize> {
        let mut path = vec![rng.below(snapshot.root_count() as u64) as usize];
        let mut node = snapshot.root(path[0]).expect("root");
        while node.child_count() > 0 {
            let index = rng.below(node.child_count() as u64) as usize;
            path.push(index);
            node = node.child(index).expect("child");
        }
        path
    }

    fn node_at<'a>(snapshot: &'a SceneSnapshot, path: &[usize]) -> &'a Arc<SceneNode> {
        let mut node = snapshot.root(path[0]).expect("root");
        for &index in &path[1..] {
            node = node.child(index).expect("child");
        }
        node
    }

    /// reader가 snapshot 전체를 lock 없이 읽고 내부 일관성을 검사합니다.
    fn read_fully(snapshot: &SceneSnapshot) -> (SceneSemanticHash, u64) {
        let mut previous: Option<ZOrder> = None;
        let mut count = 0u64;
        for node in snapshot.iter_preorder() {
            if let Some(previous) = previous {
                assert!(node.z() > previous, "z-order 불변식이 깨진 snapshot을 읽음");
            }
            previous = Some(node.z());
            // paint·clip 참조를 실제로 따라가 읽습니다.
            count += node.paint().get().layers.len() as u64 + node.clip().depth() as u64;
        }
        assert_eq!(
            snapshot.iter_preorder().count() as u64,
            snapshot.node_count()
        );
        (snapshot.semantic_hash(), count)
    }

    #[test]
    fn concurrent_readers_see_consistent_snapshots_without_locks() {
        // 검증 기준(XI.1.5): thread sanitizer와 동등한 동시성 테스트.
        // document thread가 구조 공유 snapshot을 연속 생성하는 동안 reader 4개가 동시에
        // 같은 snapshot(공유 node의 참조 카운트를 함께 건드림)을 lock 없이 끝까지 읽습니다.
        const READERS: usize = 4;
        const REVISIONS: u64 = 200;
        let base = fixtures::generate(FixtureConfig {
            seed: 0x15A,
            roots: 4,
            children: 6,
            depth: 3,
        });

        let mut senders = Vec::new();
        let mut readers = Vec::new();
        for reader in 0..READERS {
            let (tx, rx) = mpsc::channel::<(SnapshotHandle, SceneSemanticHash)>();
            senders.push(tx);
            readers.push(thread::spawn(move || {
                let mut retained: Vec<(SnapshotHandle, SceneSemanticHash)> = Vec::new();
                let mut reads = 0u64;
                for (handle, expected) in rx {
                    let (hash, _) = read_fully(&handle);
                    assert_eq!(hash, expected, "reader {reader}: 내용 불일치");
                    reads += 1;
                    // 일부 오래된 handle을 붙잡아 두었다가 나중에 다시 읽습니다.
                    if handle.scene_revision().0 % 25 == reader as u64 {
                        retained.push((handle, expected));
                    }
                }
                // producer가 모두 drop한 뒤에도 붙잡은 snapshot은 유효합니다(참조 카운트 수명).
                for (handle, expected) in &retained {
                    assert_eq!(read_fully(handle).0, *expected);
                }
                (reads, retained.len())
            }));
        }

        let mut map = SceneRevisionMap::new();
        let mut current = map
            .generate(DocumentRevision(1), base.roots().cloned().collect())
            .expect("base");
        let mut rng = SplitMix64::new(0xC0FFEE);
        let mut lifetimes = Vec::new();
        for revision in 1..=REVISIONS {
            let handle = SnapshotHandle::new(current.clone());
            lifetimes.push(handle.lifetime());
            let expected = handle.semantic_hash();
            for tx in &senders {
                tx.send((handle.clone(), expected)).expect("reader alive");
            }
            // 다음 세대: 무작위 leaf 하나의 paint만 바꿔 나머지를 공유합니다.
            let path = random_leaf_path(&mut rng, &current);
            let mut spec = node_at(&current, &path).spec();
            spec.paint = PaintStackRef::new(PaintStack {
                layers: vec![PaintLayer::Fill(fixtures::solid(
                    (revision % 7) as f32 / 7.0,
                    0.5,
                    0.5,
                    1.0,
                ))],
                ..PaintStack::empty()
            });
            current = map
                .derive(
                    &current,
                    DocumentRevision(1 + revision),
                    &path,
                    Arc::new(SceneNode::leaf(spec)),
                )
                .expect("derive");
        }
        drop(senders);
        drop(current);

        let mut total_reads = 0;
        let mut total_retained = 0;
        for reader in readers {
            let (reads, retained) = reader.join().expect("reader panic 없음");
            total_reads += reads;
            total_retained += retained;
        }
        assert_eq!(total_reads, REVISIONS * READERS as u64);
        assert!(total_retained > 0);
        // 모든 handle이 drop되면 모든 세대가 회수됩니다(누수 없음).
        assert!(lifetimes.iter().all(|lifetime| !lifetime.is_alive()));
    }

    #[test]
    fn old_snapshot_stays_valid_while_held() {
        let base = fixtures::small();
        let held = SnapshotHandle::new(base.clone());
        let lifetime = held.lifetime();
        let hash = held.semantic_hash();
        let replacement = held.root(0).expect("root").child(0).expect("child");
        let next = base
            .replace_node(
                DocumentRevision(2),
                stellar_core_types::revision::SceneRevision(2),
                &[0, 0],
                Arc::clone(replacement),
            )
            .expect("same node");
        drop(base);
        drop(next);
        assert!(lifetime.is_alive());
        assert_eq!(held.semantic_hash(), hash);
        let copy = held.clone();
        assert!(copy.ptr_eq(&held));
        drop(held);
        assert!(lifetime.is_alive());
        drop(copy);
        assert!(!lifetime.is_alive());
    }
}
