# 작업 보고서: XII.1.3 RenderThread와 최신 SceneSnapshot lock-free handoff

- WBS: XII.1.3
- 작업명: RenderThread를 구현하고 최신 SceneSnapshot을 lock-free handoff로 수신
- 상태: 완료 (`[x]`)
- 선행: XI.1.5 (`[x]`), XII.1.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/runtime/src/render_thread.rs`(신규) — `RenderThread<S>`, `RenderPublisher<S>`,
  `RenderFrame<S>`, `FrameFreshness`, `FrameRenderer`, `FrameOutcome`, `PublishOutcome`,
  `PublishError`, `RenderLoopStats`, `RenderThreadError`, 내부 `LatestSlot<T>`, 단위 테스트·doctest.
- `crates/runtime/src/lib.rs` — `render_thread` 모듈 등록.
- `docs/task-reports/XII.1.3.md`(신규), 계획서(XII.1.3 `[ ]`→`[x]`).

## 구현 요약

- **handoff**: document thread(생산자)와 render thread(소비자) 사이에 한 칸짜리 mailbox
  (`LatestSlot`, `AtomicPtr` 하나)를 둡니다. publish는 atomic swap 한 번으로 새 frame을 넣고
  `Thread::unpark`로 render thread를 깨웁니다. 아직 렌더되지 않은 이전 frame은 swap으로 돌려받아
  렌더 없이 버립니다(latest wins, `PublishOutcome::ReplacedPending`). render thread는 swap 한 번으로
  mailbox를 비워 frame을 꺼내며, 꺼낸 frame은 render thread만 소유하므로 읽을 때 lock이 없습니다.
  lock·condvar·용량 대기·렌더 완료 대기가 없어 publish는 기다리지 않습니다(wait-free).
- **메모리 상한**: mailbox에는 최대 한 frame만 대기합니다. render thread가 느려도 대기 snapshot은
  늘지 않고, 대체된 snapshot은 publish 안에서 바로 회수됩니다.
- **세대 검사**: publish는 frame의 `SceneRevision`을 공유 atomic 최신값에 먼저 알립니다. 0과 역행
  세대는 거부합니다(`ZeroRevision`, `StaleRevision`). render thread는 렌더 전에, renderer는 GPU 제출
  직전에 `FrameFreshness::is_stale()`로 더 새 세대를 확인하고 폐기합니다(XI.1.6과 같은 규칙).
- **단일 생산자**: `RenderPublisher`는 `Clone`이 아니고 `publish(&mut self)`라 생산자는 하나입니다
  (compile_fail doctest). 그래서 단조 검사와 mailbox 교체 사이에 다른 생산자가 끼어들지 않습니다.
  frame snapshot은 `&S`로만 노출합니다(compile_fail doctest).
- **thread 역할**: render thread는 이름 `stellar-render`, `ThreadRole::Render`로 bind합니다(XI.7.1).
- **종료**: `shutdown`은 정지 표시 → unpark → join입니다. 대기 frame은 렌더하지 않고 버립니다. render
  loop가 panic을 포함한 어떤 경로로 끝나도 정지를 표시해, 이후 publish는 기다리지 않고 `Stopped`를
  받으며 snapshot을 쌓아 두지 않습니다. render thread 안에서의 자기 종료는 거부합니다.
- **snapshot 타입**: crate 의존 정책상 runtime은 `stellar-scene`에 직접 의존하지 않으므로 frame 내용은
  generic `S: Send + 'static`입니다. 앱에서는 `stellar_scene::handle::SnapshotHandle`
  (`Arc<SceneSnapshot>`, `Send + Sync`, XI.1.5)을 넣습니다. 세대 번호는 core-types의
  `SceneRevision`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime` | unit 35 passed, integration 2 passed, doctest 11 passed, 0 failed |
| `cargo clippy -p stellar-runtime --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-runtime -- --check` | 통과 |
| `cargo test -q -p stellar-runtime --lib` 50회 반복 | 실패 0/50 |
| `cargo test -q --release -p stellar-runtime --lib render_thread` | 10 passed |
| `git diff --check` | 종료 0 |

| 검사 | 결과 |
|---|---|
| **document thread가 render submit을 기다리지 않음**: render thread가 첫 frame "GPU 제출"(gate)에서 멈춘 동안 DocumentThread job이 세대 2..=1000을 publish → gate 해제 전에 job 완료, 첫 publish `Delivered`·나머지 998건 `ReplacedPending`, 대체 snapshot 998개 즉시 회수, 해제 후 제출 순서 `[1, 1000]`(최신만), 종료 후 1000개 전부 회수(`document_thread_does_not_wait_for_render_submit`) | 통과 |
| 동시성: 생산자 thread가 10,000 frame을 쉬지 않고 넘기는 동안 render thread가 lock 없이 읽음 — 받은 snapshot 내용 무결, 제출 세대 엄격 증가, 마지막 세대 제출, 전부 회수(`concurrent_handoff_delivers_consistent_monotonic_frames`) | 통과 |
| 제출 직전 더 새 세대 확인 시 `Discarded`, 다음 frame 제출(`renderer_discards_a_frame_that_became_stale_before_submit`) | 통과 |
| 세대 0·역행 거부(snapshot은 호출 안에서 회수), 같은 세대 재전달 허용(`publish_rejects_zero_and_regressed_revisions`) | 통과 |
| 종료 시 대기 frame 미렌더·회수, 종료 후 publish `Stopped`(`shutdown_drops_the_pending_frame_and_rejects_later_publishes`) | 통과 |
| renderer panic 후 publish는 기다리지 않고 `Stopped`, 쌓인 snapshot 없음, shutdown `Panicked`(`renderer_panic_never_blocks_the_publisher`) | 통과 |
| render thread 이름·`ThreadRole::Render`·thread id(`frames_run_on_the_render_thread`) | 통과 |
| mailbox 단위: 최신 값만 유지, 대체·drop 시 회수(`slot_keeps_only_the_latest_value_and_frees_the_rest`) | 통과 |
| compile_fail doctest 3건: publisher 복제 불가, 공유 참조 publish 불가, frame snapshot 가변 참조 불가 | 통과 |

## 차이(작업 지시와 다른 점)

- frame 내용 타입을 `SceneSnapshot`으로 고정하지 않고 generic `S`로 두었습니다. `stellar-runtime →
  stellar-scene` edge가 crate 정책(`tools/xtask/src/crate_graph.rs`)에 없고, `stellar-renderer-core`도
  scene 타입을 재노출하지 않기 때문입니다. 정책 파일은 이 lane의 변경 범위 밖이라 바꾸지 않았습니다.
  테스트는 `SnapshotHandle`과 같은 성질(불변, `Arc` 공유, `Send + Sync`)의 대역 타입으로 했습니다.
- `unsafe`는 `LatestSlot`의 `Box::into_raw`/`Box::from_raw` 두 곳과 `Sync` 구현 하나뿐이며 각 위치에
  SAFETY 근거를 적었습니다. ADR-0013 보안 절의 "runtime에 unsafe 없음" 기록은 이 lane의 허용 경로 밖이라
  갱신하지 않았습니다(후속 문서 정리 필요).

## 남은 위험 / 한계

- thread sanitizer·Miri는 실행하지 않았습니다(고정 toolchain 1.96.0 stable에 nightly 없음). 대신
  동시성 테스트와 50회 반복 실행으로 확인했습니다. mailbox는 swap만 쓰고 CAS·재사용 pointer 비교가
  없어 ABA 문제는 구조적으로 없습니다.
- 대체된 이전 frame은 publish를 호출한 document thread에서 회수합니다. 큰 subtree가 이전 frame에만
  있으면 그 해제 비용이 document thread에 걸립니다(구조 공유로 보통 바뀐 subtree만 해제).
- 실제 renderer(Metal backend)와 scene builder 연결, document thread가 publisher를 소유하는 배선은
  XII.4.x 소관입니다. 지금은 runtime 단위 테스트에서 job이 publisher를 소유하는 방식으로만 검증했습니다.
- present 대기열 깊이·frame pacing(XI.5.5)은 다루지 않습니다.
