//! lease 조정자 test (XIX.4.5, XIX.4.6, XIX.4.7).

use super::*;
use crate::command::test_fixtures::{definition, frame, sid};
use stellar_document::model::DocumentModel;

const CHILD_DIR_ENV: &str = "STELLAR_TEST_LEASE_CHILD_DIR";
const CHILD_MODE_ENV: &str = "STELLAR_TEST_LEASE_CHILD_MODE";
const KEY: &str = "0123456789abcdef0123456789abcdef";

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "stellar-lease-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// root(10) ─ 11 ─ 13 ─ 14, root ─ 12.
fn model() -> DocumentModel {
    let mut model = DocumentModel::new(definition());
    for (id, parent, key) in [
        (10, None, 0x10),
        (11, Some(10), 0x10),
        (12, Some(10), 0x20),
        (13, Some(11), 0x10),
        (14, Some(13), 0x10),
    ] {
        model.insert_node(frame(id, parent, key)).expect("node");
    }
    model
}

fn lease(id: u64, roots: &[u64]) -> Lease {
    let model = model();
    Lease {
        id: LeaseId(sid(id)),
        subtrees: roots
            .iter()
            .map(|root| SubtreeLock::capture(&model, sid(*root)).unwrap())
            .collect(),
        externals: Vec::new(),
    }
}

fn gesture(id: u64, nodes: &[u64]) -> Gesture {
    let model = model();
    Gesture {
        id: sid(id),
        nodes: nodes
            .iter()
            .map(|node| SubtreeLock::capture(&model, sid(*node)).unwrap())
            .collect(),
    }
}

#[test]
fn windows_of_one_process_share_leases() {
    // 검증 기준(XIX.4.5): 같은 project의 두 창이 같은 lease를 봅니다.
    let temp = TempDir::new("windows");
    let first = ProjectLockCoordinator::open(&temp.0, KEY).unwrap();
    let second = ProjectLockCoordinator::open(&temp.0, KEY).unwrap();
    let held = first.try_acquire(lease(900, &[13])).unwrap();
    let seen = second.leases().unwrap();
    assert_eq!(seen.get(LeaseId(sid(900))), Some(held.lease()));
    assert!(matches!(
        second.try_acquire(lease(901, &[14])),
        Err(AcquireError::Conflict(Conflict::Subtree { holder, .. })) if holder == LeaseId(sid(900))
    ));
    assert!(matches!(
        second.try_acquire(lease(900, &[12])),
        Err(AcquireError::DuplicateId(_))
    ));
    let sibling = second.try_acquire(lease(901, &[12])).unwrap();
    held.release().unwrap();
    assert_eq!(first.leases().unwrap().len(), 1);
    let again = second.try_acquire(lease(902, &[14])).unwrap();
    drop(again);
    drop(sibling);
    assert!(first.leases().unwrap().is_empty());
}

#[test]
fn different_project_keys_do_not_share() {
    // 다른 worktree는 다른 project identity key를 쓰므로 lease를 공유하지 않습니다.
    let temp = TempDir::new("keys");
    let main = ProjectLockCoordinator::open(&temp.0, KEY).unwrap();
    let worktree = ProjectLockCoordinator::open(&temp.0, "fedcba9876543210").unwrap();
    let _held = main.try_acquire(lease(900, &[13])).unwrap();
    assert!(worktree.leases().unwrap().is_empty());
    let _other = worktree.try_acquire(lease(901, &[13])).unwrap();
    for bad in [
        "",
        "ABC",
        "../x",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0",
    ] {
        assert_eq!(
            ProjectLockCoordinator::open(&temp.0, bad),
            Err(CoordinatorError::InvalidKey),
            "{bad:?}"
        );
    }
}

#[test]
fn lease_waits_for_gesture_end() {
    // 검증 기준(XIX.4.7): drag 중인 node는 gesture가 끝난 뒤에만 lease를 획득합니다.
    let temp = TempDir::new("gesture");
    let coordinator = ProjectLockCoordinator::open(&temp.0, KEY).unwrap();
    let drag = coordinator.begin_gesture(gesture(700, &[14])).unwrap();
    for roots in [&[14][..], &[13], &[10]] {
        assert_eq!(
            coordinator.try_acquire(lease(900, roots)).map(|_| ()),
            Err(AcquireError::Conflict(Conflict::Gesture {
                gesture: sid(700),
                requested: sid(roots[0])
            })),
            "{roots:?}"
        );
    }
    // 겹치지 않는 subtree는 gesture 중에도 잠급니다.
    let sibling = coordinator.try_acquire(lease(901, &[12])).unwrap();
    drag.end().unwrap();
    let held = coordinator.try_acquire(lease(900, &[13])).unwrap();
    // lease 안 node에서 gesture를 시작하는 것은 거부합니다.
    assert!(matches!(
        coordinator.begin_gesture(gesture(701, &[14])),
        Err(AcquireError::Conflict(Conflict::Subtree { .. }))
    ));
    drop((held, sibling));
}

#[test]
fn stale_entry_from_ended_owner_is_removed() {
    let temp = TempDir::new("stale");
    let coordinator = ProjectLockCoordinator::open(&temp.0, KEY).unwrap();
    // 잠금 없이 남은 파일(쥔 프로세스가 끝난 경우와 같음).
    let path = coordinator.dir().join(format!("lease-{}.txt", sid(900)));
    std::fs::write(
        &path,
        encode("lease", sid(900), &lease(900, &[13]).subtrees, &[]),
    )
    .unwrap();
    assert!(coordinator.leases().unwrap().is_empty());
    assert!(!path.exists());
    let _held = coordinator.try_acquire(lease(901, &[13])).unwrap();
}

#[test]
fn entry_text_round_trips() {
    let mut original = lease(900, &[13, 12]);
    original.externals.push(ExternalResource {
        kind: ExternalKind::Asset,
        id: sid(42),
    });
    let text = encode(
        "lease",
        original.id.0,
        &original.subtrees,
        &original.externals,
    );
    match decode(&text) {
        Some(Decoded::Lease(lease)) => assert_eq!(lease, original),
        _ => panic!("lease"),
    }
    let g = gesture(700, &[14]);
    match decode(&encode("gesture", g.id, &g.nodes, &[])) {
        Some(Decoded::Gesture(decoded)) => assert_eq!(decoded, g),
        _ => panic!("gesture"),
    }
    for bad in [
        "",
        "stellar-agent-lease 2\nlease x\n",
        &text.replace("subtree", "subtrees"),
    ] {
        assert!(decode(bad).is_none(), "{bad:?}");
    }
}

/// 다른 프로세스가 쥔 lease를 봅니다. 자식 프로세스는 이 test binary의 [`child_process_holds_lease`]입니다.
#[test]
fn other_process_lease_is_shared_and_freed_when_it_exits() {
    // 검증 기준(XIX.4.5): 다른 프로세스와도 lease를 공유하고, 그 프로세스가 비정상 종료해도 남지 않습니다.
    for mode in ["release", "crash"] {
        let temp = TempDir::new(&format!("process-{mode}"));
        let ready = temp.0.join("ready");
        let release = temp.0.join("release");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "lock::coordinator::tests::child_process_holds_lease",
                "--ignored",
                "--test-threads=1",
            ])
            .env(CHILD_DIR_ENV, &temp.0)
            .env(CHILD_MODE_ENV, mode)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !ready.exists() {
            assert!(std::time::Instant::now() < deadline, "자식 준비 시간 초과");
            if let Some(status) = child.try_wait().unwrap() {
                panic!("자식이 먼저 끝남: {status}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let coordinator = ProjectLockCoordinator::open(&temp.0, KEY).unwrap();
        assert!(
            coordinator
                .leases()
                .unwrap()
                .get(LeaseId(sid(950)))
                .is_some(),
            "{mode}"
        );
        assert!(matches!(
            coordinator.try_acquire(lease(900, &[14])),
            Err(AcquireError::Conflict(Conflict::Subtree { holder, .. })) if holder == LeaseId(sid(950))
        ));
        std::fs::write(&release, b"").unwrap();
        let status = child.wait().unwrap();
        assert_eq!(
            status.code(),
            Some(if mode == "crash" { 3 } else { 0 }),
            "{mode}"
        );
        assert!(coordinator.leases().unwrap().is_empty(), "{mode}");
        let _held = coordinator.try_acquire(lease(900, &[14])).unwrap();
    }
}

/// [`other_process_lease_is_shared_and_freed_when_it_exits`]의 자식 프로세스. 환경 변수가 없으면 아무것도 하지
/// 않습니다.
#[test]
#[ignore = "자식 프로세스 전용"]
fn child_process_holds_lease() {
    let Some(dir) = std::env::var_os(CHILD_DIR_ENV).map(PathBuf::from) else {
        return;
    };
    let mode = std::env::var(CHILD_MODE_ENV).unwrap_or_default();
    let coordinator = ProjectLockCoordinator::open(&dir, KEY).unwrap();
    let held = coordinator.try_acquire(lease(950, &[13])).unwrap();
    std::fs::write(dir.join("ready"), b"").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !dir.join("release").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if mode == "crash" {
        // 소멸자를 부르지 않고 끝냅니다(비정상 종료와 같이 파일이 남고 잠금만 OS가 풉니다).
        std::process::exit(3);
    }
    held.release().unwrap();
}
