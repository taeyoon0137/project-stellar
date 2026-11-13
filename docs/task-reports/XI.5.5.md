# 작업 보고서: XI.5.5 present queue depth ≤ 1 frame scheduler

- WBS: XI.5.5
- 작업명: present queue depth를 최대 1로 유지하는 frame scheduler, 새 snapshot이 오면 제출 전 오래된 frame drop
- 상태: 완료 (`[x]`)
- 선행: XI.5.1 (이 lane에서 `11a70c9`로 `[x]`), XI.1.6 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/frame_scheduler.rs`(신규) — `FrameScheduler`, `TickOutcome`,
  `SchedulerStats`, `MAX_PRESENT_QUEUE_DEPTH`.
- `crates/renderer-core/src/frame_scheduler/tests.rs`(신규) — 느린 GPU simulation과 drop·재시도 테스트.
- `crates/renderer-core/src/backend/mod.rs` — `RendererBackend::discard`(encode한 frame을 제출하지 않고
  버림) 추가, 호출 순서 문서 갱신.
- `crates/renderer-core/src/backend/call_order.rs` — `BackendCall::Discard`·`BackendCall::ALL`,
  `before_discard`/`after_discard`, `CheckedBackend::discard`.
- `crates/renderer-core/src/backend/mock.rs` — mock `discard`와 버린 frame 기록.
- `crates/renderer-core/src/backend/recovery.rs` — error matrix가 `BackendCall::ALL`(14종)을 돌도록 확장.
- `crates/renderer-core/src/backend/tests.rs` — `discard` 호출 순서 테스트.
- `crates/renderer-core/src/test_scene.rs`(신규, `#[cfg(test)]`) — scene 공개 codec bytes로 실제
  `SceneSnapshot`을 만드는 테스트 fixture(renderer-core는 core-types 타입을 이름으로 만들 수 없음).
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.5.5.md`(신규), 계획서(XI.5.5 `[ ]`→`[x]`).

## 구현 요약

- **pending은 최신 하나**: `offer`는 아직 encode하지 않은 이전 snapshot을 새 snapshot으로 바꾸고 밀려난
  것을 돌려줍니다. pending·마지막 제출 revision 이하로 늦게 도착한 snapshot은 받지 않습니다.
- **present queue 깊이 ≤ 1**: present를 예약해 제출했지만 아직 화면에 나가지 않은 submission을 queue로
  추적하고(`observe(GpuProgress)`로 present 진행 반영), 깊이가 1이면 새 frame을 시작하지 않습니다
  (`Throttled`). 기다리는 동안에도 pending은 계속 최신으로 바뀌므로 GPU가 느려도 대기열이 쌓이지 않고,
  다음 frame은 그 시점 최신 snapshot입니다.
- **제출 전 폐기(XI.1.6 연결)**: encode 직전과 제출 직전에 `LatestSceneRevision::check_handle`로 세대를
  확인합니다. 더 새 세대가 알려졌으면 encode하지 않거나, encode한 frame을 새로 추가한
  `RendererBackend::discard`로 GPU에 보내지 않고 버립니다(Metal: command buffer를 commit하지 않고
  drawable 해제). `discard`는 present 예약 후에도 허용되며 frame을 닫습니다.
- **재시도**: backend skip(drawable 없음)·encode·present·submit 오류 시 snapshot을 pending으로 되돌려
  다음 tick에 다시 그립니다. present 예약 실패 frame은 discard로 닫습니다. device 재생성 뒤에는
  `reset_present_queue`로 이전 device의 present 대기를 비웁니다.
- **통계**: 제출 수, encode 전·후 drop 수, skip 수, throttle 수, 최대 queue 깊이를 `SchedulerStats`로
  남깁니다(XI.11.5 benchmark 보고용).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 73 passed, 0 failed; doctest 3 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| 느린 GPU simulation(300 tick, tick마다 새 snapshot·가끔 2개, GPU는 4 tick에 1 frame만 present): 매 tick present queue 깊이 ≤ 1, 최대 깊이 1, GPU backlog ≤ 1, throttle 발생, drop 수 > 제출 수, offer 수 = 제출 + drop + pending, 화면에 나간 revision 엄격 증가, GPU가 따라잡으면 마지막 snapshot 제출(`slow_gpu_keeps_present_queue_depth_at_most_one`) | 통과 |
| 새 snapshot이 pending을 밀어냄, 늦은·이미 제출한 revision 거부(`newer_snapshot_replaces_pending_and_late_snapshots_are_dropped`) | 통과 |
| encode 전 세대 확인으로 encode 생략(backend 호출은 poll뿐), encode 중 새 세대 알림 시 discard하고 미제출(`stale_snapshots_are_dropped_before_and_after_encode`) | 통과 |
| drawable 없음 skip·encode timeout·present surface lost·submit device lost 뒤 snapshot 유지, 재 init 후 제출, queue full이면 throttle(`skips_and_errors_keep_the_snapshot_for_retry`) | 통과 |
| `discard` 호출 순서: 열린 frame 없음·frame 불일치 거부, present 예약 후 discard, 다음 frame 정상 제출(`discard_closes_the_open_frame_without_submitting`) | 통과 |
| error matrix가 `discard`를 포함한 14 호출 × 9 오류 = 126 경우로 확장되어 통과(`mock_backend_error_matrix`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XI.5.1(이 lane)·XI.1.6은 명시 row-ID이고 `[x]`입니다. 완화 적용은 없습니다.
- "제출 전 오래된 frame drop"을 encode한 frame까지 적용하려고 `RendererBackend`에 `discard`를 추가했습니다
  (XI.5.1 interface 확장, 같은 lane, Metal 구현 전).
- scene revision은 renderer-core가 core-types `SceneRevision`을 이름으로 쓸 수 없어 결과·통계에서
  `u64`(`SceneRevision.0`)로 보고합니다. 판정 자체는 scene의 `LatestSceneRevision`이 타입으로 합니다.
- simulation은 단일 thread 결정론 모사입니다. document thread와의 실제 동시 handoff는 RenderThread
  (XII.1.3)에서 검증합니다.

## 남은 위험 / 한계

- present 완료 신호는 backend `poll`의 `presented`에 의존합니다. Metal은 drawable present handler로 이 값을
  채워야 하며(XI.8.5), 늦게 오면 throttle이 길어집니다.
- tick 사이 대기(vsync·GPU 완료 대기) 방식은 RenderThread(XII.1.3)가 정합니다. scheduler는 비대기로만
  동작합니다.
