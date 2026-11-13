# 작업 보고서: XII.1.2 UI→DocumentThread bounded command queue와 입력 우선순위

- WBS: XII.1.2
- 작업명: UI→DocumentThread command queue를 bounded channel로 구현하고 입력 이벤트에 priority 부여
- 상태: 완료 (`[x]`)
- 선행: XII.1.1 (이 lane에서 완료: `c490316`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/runtime/src/command_queue.rs`(신규) — `CommandQueue<T>`, `CommandPriority`,
  `CoalesceKey`, `InputEventClass`, `SubmitOptions`, `QueueCapacity`, `Pushed`, `Rejected`,
  `RejectReason`, `QueueConfigError`, 단위 테스트.
- `crates/runtime/src/document_thread.rs` — FIFO 수신함을 `CommandQueue`로 교체,
  `DocumentSender::try_submit`·`SubmitOutcome`·`Submitted`, `SubmitError::QueueFull`,
  `JobError::Superseded`/`Evicted`, `DocumentThread::spawn_with`, UI thread blocking 대기 거부,
  XII.1.2 테스트 모듈(`queue_tests`) 추가. XII.1.1 테스트는 바꾸지 않았습니다.
- `crates/runtime/src/lib.rs` — `command_queue` 모듈 등록.
- `crates/runtime/tests/command_queue_overflow.rs`(신규) — fixture runner.
- `crates/runtime/tests/fixtures/command_queue_overflow_v1.txt`(신규) — overflow 정책 fixture 12 case.
- `docs/adr/ADR-0012-app-runtime-crate.md` — 수신함 결정(4번)·대안·성능 절 갱신.
- `docs/task-reports/XII.1.2.md`(신규), 계획서(XII.1.2 `[ ]`→`[x]`).

## 구현 요약

- **bounded lane**: 수신함은 `Input` → `Command` → `Background` 세 lane이며 lane마다 독립 용량을
  가집니다(0 거부). document thread는 항상 가장 높은 우선순위 lane의 가장 오래된 job을 꺼내고,
  lane 안은 FIFO입니다. `submit`은 `Command` lane, `try_submit`은 우선순위·병합 key를 지정합니다.
- **입력 priority**: 셸 입력은 모두 `Input`입니다. `InputEventClass`가 공통 `InputEvent` 종류를
  접수 옵션으로 바꿉니다 — pointer move(장치별)·resize는 절대 상태라 병합 대상, wheel은 상대 delta라
  병합 금지, pointer down/up/cancel·key·text composition·focus는 이산 이벤트로 병합 금지.
- **overflow 정책**: ① 닫힘 → `Closed` 거부. ② 같은 lane의 **마지막** 항목과 병합 key가 같으면
  대체(`Coalesced`, 용량 불필요, 이전 ticket `Superseded`). ③ 자리가 있으면 `Enqueued`.
  ④ 가득 참: `Input`·`Command`는 즉시 `QueueFull`(기존 항목 보존, 거부 항목은 호출자에게 반환),
  `Background`는 가장 오래된 항목을 밀어냄(`EnqueuedEvicting`, 이전 ticket `Evicted`).
- **UI 무대기**: 접수·닫기·`try_pop`은 O(1) 임계 구역만 잡고 기다리지 않습니다. 대체·밀려난 job의
  폐기와 ticket 통지는 lock 밖에서 합니다. `ThreadRole::Ui` thread의 `wait`/`wait_timeout`은
  `BlockingNotAllowed`로 거부하고 UI는 `try_take`만 씁니다. 기다리는 연산은 document thread의
  `pop_blocking` 하나뿐입니다.
- **기본 용량**: input 1024, command 1024, background 256(초기값, XII.3.5 측정으로 조정).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime` | unit 24 passed, integration 2 passed, doctest 7 passed, 0 failed |
| `cargo clippy -p stellar-runtime --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-runtime -- --check` | 통과 |
| `cargo test -q -p stellar-runtime --lib --tests` 30회 반복 | 실패 0/30 |
| `cargo run -q -p xtask -- validate-crate-graph` / `validate-dependencies` | 종료 0 / 종료 0 |
| `git diff --check` | 종료 0 |

| 검사 | 결과 |
|---|---|
| **queue overflow 정책 fixture**: 12 case·56 push — 우선순위 순서, input·command 가득 참 거부(항목 반환·기존 보존), background 최고령 밀어냄, lane 독립, 인접 같은 장치 pointer move만 병합, 병합은 용량 불필요, resize 병합·wheel 비병합, lane별 병합, 병합 우선 후 밀어냄, 닫힌 queue 거부·잔여 drain(`overflow_policy_fixture_passes`) | 통과 |
| 틀린 기대값이면 runner가 실패(`fixture_runner_detects_a_wrong_expectation`) | 통과 |
| **blocking UI call 없음**: document thread를 job 실행 중으로 멈춘 상태에서 `ThreadRole::Ui` thread가 lane별 6건씩 18건 접수 → 정책대로 즉시 판정(4 접수 + 2 `QueueFull`/`EnqueuedEvicting`)하고 gate 해제 전에 완료, UI의 `wait`/`wait_timeout` 거부·`try_take` 즉시 `None`, 해제 후 실행 순서 input→command→background(`ui_submissions_never_wait_for_a_busy_document_thread`) | 통과 |
| 바쁜 동안 pointer move 100건 → 1건만 실행·나머지 `Superseded`(`pointer_moves_coalesce_to_the_latest_while_busy`) | 통과 |
| move·down·move·wheel·wheel·resize·resize·up 순서 보존, 인접 같은 key만 병합(`discrete_input_is_never_merged_or_reordered`) | 통과 |
| `submit`은 command lane(입력 뒤·background 앞), 종료된 thread에 UI 접수 즉시 `Stopped` | 통과 |
| queue 단위: 용량 0 거부, 입력 분류→옵션, 거부 항목 반환, 닫힌 빈 queue `pop_blocking` 즉시 `None`, 다른 thread push로 깨어남 | 통과 |

## 차이(작업 지시와 다른 점)

- "bounded channel"은 표준 라이브러리 `sync_channel` 대신 우선순위별 bounded lane을 가진
  `Mutex`+`Condvar` queue로 구현했습니다(`sync_channel::send`는 가득 차면 UI를 멈추고 우선순위·병합이
  없음 — ADR-0012 대안 절). 외부 의존성은 추가하지 않았습니다.
- 셸 `InputEvent`(`stellar-ffi`) → `InputEventClass` 변환은 ffi 쪽 책임으로 두었습니다(runtime은 ffi를
  역참조하지 않음, ADR-0012). 이 lane은 `crates/ffi`를 바꾸지 않았습니다.
- lane 사이의 접수 순서는 보존하지 않습니다(입력 우선 처리의 결과). 순서가 중요한 두 job은 같은
  우선순위로 보내야 합니다.

## 남은 위험 / 한계

- 우선순위는 엄격(strict)해서 입력이 계속 들어오면 command·background lane이 굶을 수 있습니다.
  pointer move·resize 병합으로 완화되지만 aging 정책은 없습니다. 실제 입력 부하 측정(XII.3.5) 후
  필요하면 추가합니다.
- UI 쪽 접수는 O(1) 임계 구역의 `Mutex`를 잡습니다. 용량·job 실행은 기다리지 않지만 lock-free는
  아닙니다. 다른 생산자·소비자도 O(1) 동안만 lock을 잡으므로 경합 대기는 짧습니다.
- `ThreadRole::Ui` bind는 셸 경계(ffi)가 UI thread에서 해야 합니다. 아직 ffi 연결이 없어 실제 앱
  UI thread에는 bind되지 않았습니다.
- 기본 lane 용량은 측정 근거 없는 초기값입니다.
- `Input`·`Command` lane이 가득 차 거부되면 해당 입력은 처리되지 않습니다. 셸이 재시도하거나
  사용자에게 알릴 정책은 셸 통합(XII.3) 소관입니다.
