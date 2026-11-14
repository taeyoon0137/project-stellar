//! worker result gate 단위 테스트(판정 표·hash·집계).

use super::*;

fn rev(value: u64) -> DocumentRevision {
    DocumentRevision(value)
}

fn source(revision: u64, input: &str) -> WorkerSource {
    CapturedInput::capture(rev(revision), input.to_string()).source()
}

fn result(key: u32, revision: u64, input: &str) -> WorkerResult<u32, String> {
    WorkerResult::new(
        key,
        source(revision, input),
        WorkerClass::TextShaping,
        input.to_string(),
    )
}

fn reason<T>(admission: Admission<u32, T>) -> Option<&'static str> {
    match admission {
        Admission::Apply { .. } => None,
        Admission::Stale { reason, .. } => Some(reason.name()),
    }
}

#[test]
fn input_hash_is_fnv1a_128_and_parts_round_trip() {
    // FNV-1a 128 공개 test vector.
    assert_eq!(
        WorkerInputHash::of_bytes(b""),
        WorkerInputHash::from_parts(0x6c62_272e_07bb_0142, 0x62b8_2175_6295_c58d)
    );
    assert_eq!(
        WorkerInputHash::of_bytes(b"a"),
        WorkerInputHash::from_parts(0xd228_cb69_6f1a_8caf, 0x7891_2b70_4e4a_8964)
    );
    let hash = WorkerInputHash::from_parts(0x0123, 0x4567);
    assert_eq!(hash.to_parts(), (0x0123, 0x4567));
    assert_eq!(
        "abc".to_string().input_hash(),
        b"abc".to_vec().input_hash(),
        "String과 bytes는 같은 canonical bytes면 같은 hash"
    );
    assert_ne!(
        "abc".to_string().input_hash(),
        "abd".to_string().input_hash()
    );
    assert_eq!(
        WorkerInputHash::of_bytes(b"").to_string(),
        "6c62272e07bb014262b821756295c58d"
    );
}

#[test]
fn gate_applies_only_results_matching_the_current_input_window() {
    let mut gate = WorkerResultGate::new();
    // 기대 없음.
    assert_eq!(reason(gate.admit(result(1, 1, "a"))), Some("not-expected"));

    // r1에서 입력 a: 새 입력.
    assert_eq!(gate.expect(1, source(1, "a")), Ok(Expectation::NewInput));
    // 다른 node 편집으로 r3까지 왔지만 입력은 같음: 기존 job 유지.
    assert_eq!(
        gate.expect(1, source(3, "a")),
        Ok(Expectation::SameInput { applied: false })
    );
    // r4에서 캡처했는데 r4를 기대하지 않았음.
    assert_eq!(
        reason(gate.admit(result(1, 4, "a"))),
        Some("ahead-of-expectation")
    );
    // 다른 입력의 결과.
    assert_eq!(reason(gate.admit(result(1, 2, "b"))), Some("input-changed"));
    // r1에서 캡처한 결과는 r1..=r3 구간 안이라 적용.
    match gate.admit(result(1, 1, "a")) {
        Admission::Apply { key, source, value } => {
            assert_eq!(key, 1);
            assert_eq!(source.revision, rev(1));
            assert_eq!(value, "a");
        }
        stale => panic!("적용해야 합니다: {stale:?}"),
    }
    // 같은 입력의 중복 결과.
    assert_eq!(
        reason(gate.admit(result(1, 2, "a"))),
        Some("already-applied")
    );
    assert_eq!(
        gate.expect(1, source(5, "a")),
        Ok(Expectation::SameInput { applied: true })
    );

    // 입력이 a → b → a로 돌아오면 since가 새로 정해집니다.
    assert_eq!(gate.expect(1, source(6, "b")), Ok(Expectation::NewInput));
    assert_eq!(gate.expect(1, source(7, "a")), Ok(Expectation::NewInput));
    assert_eq!(
        reason(gate.admit(result(1, 5, "a"))),
        Some("before-current-input")
    );
    assert_eq!(reason(gate.admit(result(1, 6, "b"))), Some("input-changed"));
    assert!(matches!(
        gate.admit(result(1, 7, "a")),
        Admission::Apply { .. }
    ));

    // 역행 거부, 상태는 그대로.
    assert_eq!(
        gate.expect(1, source(6, "c")),
        Err(GateError::RevisionRegressed {
            latest: rev(7),
            requested: rev(6)
        })
    );
    assert_eq!(gate.expected(&1), Some(source(7, "a")));

    // 잊은 key.
    assert!(gate.forget(&1));
    assert!(!gate.forget(&1));
    assert!(gate.is_empty());
    assert_eq!(reason(gate.admit(result(1, 7, "a"))), Some("not-expected"));

    let stats = gate.stats();
    assert_eq!(stats.applied, 2);
    assert_eq!(stats.stale, 7);
    assert_eq!(stats.stale_by_reason, [2, 2, 1, 1, 1]);
}

#[test]
fn keys_are_judged_independently() {
    let mut gate = WorkerResultGate::new();
    gate.expect("title", source(1, "x")).unwrap();
    gate.expect("body", source(1, "y")).unwrap();
    // body만 바뀜.
    gate.expect("body", source(2, "z")).unwrap();
    gate.expect("title", source(2, "x")).unwrap();
    assert_eq!(gate.len(), 2);
    let title = WorkerResult::new("title", source(1, "x"), WorkerClass::TextShaping, ());
    let body = WorkerResult::new("body", source(1, "y"), WorkerClass::TextShaping, ());
    assert!(matches!(gate.admit(title), Admission::Apply { .. }));
    assert!(matches!(
        gate.admit(body),
        Admission::Stale {
            reason: StaleReason::InputChanged { .. },
            ..
        }
    ));
}

#[test]
fn recv_timeout_is_refused_on_role_bound_threads() {
    let (_tx, rx) = worker_result_channel::<u32, ()>();
    let refused = std::thread::spawn(move || {
        let _guard =
            stellar_renderer_core::thread_role::bind_thread_role(ThreadRole::Document).unwrap();
        rx.recv_timeout(Duration::from_millis(1))
            .map(|r| r.is_some())
    })
    .join()
    .unwrap();
    assert_eq!(
        refused,
        Err(ResultWaitError::BlockingNotAllowed {
            role: ThreadRole::Document
        })
    );
}
