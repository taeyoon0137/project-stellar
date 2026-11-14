# 작업 보고서: XII.1.1 DocumentThread event loop와 문서 변경 thread 제한

- WBS: XII.1.1
- 작업명: `DocumentThread` event loop를 구현하고 모든 mutable document 접근을 이 thread로 제한
- 상태: 완료 (`[x]`)
- 선행: Phase XI, Phase VII (phase 단위 선행 — 사용자 지시(2026-11-14)로 완화, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/runtime/Cargo.toml`(신규) — 새 crate `stellar-runtime`(의존: core-types, document,
  renderer-core. 외부 의존성 없음).
- `crates/runtime/src/lib.rs`(신규) — crate 문서, `document_thread` 모듈 등록.
- `crates/runtime/src/document_thread.rs`(신규) — `DocumentThread`, `DocumentSender`,
  `DocumentContext`, `DocumentMut`, `JobTicket`, `DocumentLoopStats`, 오류 타입(`SubmitError`,
  `TicketError`, `JobError`, `DocumentThreadError`), `on_document_thread`, 단위 테스트·doctest.
- `Cargo.toml` — workspace member에 `crates/runtime` 추가. `Cargo.lock` — cargo 자동 갱신.
- `tools/xtask/src/crate_graph.rs` — `stellar-runtime` 허용 edge 추가, `stellar-ffi` →
  `stellar-runtime` 허용, 테스트 2건 추가.
- `docs/adr/ADR-0013-app-runtime-crate.md`(신규) — crate 경계와 thread 모델 결정.
- `docs/task-reports/XII.1.1.md`(신규), 계획서(XII.1.1 `[ ]`→`[x]`).

## 구현 요약

- **event loop**: `DocumentThread::spawn`이 `stellar-document` 이름의 thread를 만들고, thread 안에서
  `ThreadRole::Document`(XI.7.1)를 bind한 뒤 `DocumentContext`(열린 문서 집합 + 문서별
  `DocumentRevision`)를 생성합니다. 수신함(FIFO)에서 job을 하나씩 꺼내 순차 실행합니다. 수신함의
  임계 구역은 push/pop 한 번이고 job 실행·drop은 lock 밖에서 합니다.
- **변경 경로 제한**: 다른 thread는 `DocumentSender::submit`으로
  `FnOnce(&mut DocumentContext) -> T + Send + 'static` job만 보내고, 결과는 소유 값으로
  `JobTicket`에 돌아옵니다. `DocumentContext`·`DocumentMut`는 `!Send`·`!Sync`이고 생성자가
  비공개이며, 공개 API는 `&mut Document`를 내보내지 않습니다. 변경은 `DocumentMut`의
  `insert_node`/`remove_node`/`set_parent`로만 하며, 성공한 변경마다 문서 revision을 1 올립니다.
- **ticket**: `try_take`(대기 없음), `wait_timeout`, `wait`. 결과는 한 번만 꺼낼 수 있고, job이
  실행되지 못하면 `JobError::Lost`입니다. document thread에서의 blocking 대기는
  `BlockingNotAllowed`로 거부합니다(자기 대기 deadlock 방지).
- **종료**: `shutdown`/drop은 접수 닫기 → 이미 접수된 job 실행 → join 순서입니다. document thread
  자신이 호출하면 `ShutdownFromDocumentThread`를 반환하고 join하지 않습니다. job panic(test
  profile) 시 수신함을 닫고 남은 ticket을 `Lost`로 알리며 `shutdown`은 `Panicked`를 반환합니다.
- **crate 정책**: ADR-0013에 따라 `stellar-runtime` 허용 edge(core-types, document, commands,
  history, project, storage, assets, renderer-core)와 `stellar-ffi` → `stellar-runtime`을
  추가했습니다. ffi는 runtime을 우회해 document·project에 직접 의존할 수 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime` | unit 11 passed, doctest 7 passed(compile_fail 6), 0 failed |
| `cargo clippy -p stellar-runtime --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-runtime -- --check` | 통과 |
| `cargo test -p xtask` | 117 passed, 0 failed(crate_graph 5건 포함) |
| `cargo clippy -p xtask --all-targets -- -D warnings` / `cargo fmt -p xtask -- --check` | 경고 0 / 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | 종료 0 |
| `cargo run -q -p xtask -- validate-dependencies` | 종료 0 |
| `git diff --check` | 종료 0 |

| 검사 | 결과 |
|---|---|
| **다른 thread 직접 mutate compile 실패**: job 안 scoped thread로 context 전달, `DocumentMut`를 다른 thread로 이동, context를 `'static`으로 유출, `DocumentContext`/`DocumentMut`의 `Send` 요구, 외부 `DocumentContext::new()` — compile_fail doctest 6건. 임시 example로 각 실패 원인이 `*const ()` `Send` 불충족(E0277)·closure 밖 유출(E0521)·비공개 함수(E0624)임을 확인 | 통과 |
| 같은 구조의 정상 사용 doctest(job 안 open·`document_mut`·revision 읽기) | 통과 |
| **API test**: job은 document thread(id·이름·`ThreadRole::Document`)에서만 실행되고 호출 thread와 다름(`jobs_run_only_on_the_document_thread`) | 통과 |
| 4개 thread × 250 job 동시 접수 시 1,000 변경이 revision 1..=1000을 하나씩 받고, sender별 접수 순서 유지(`concurrent_senders_mutate_only_through_the_single_writer`) | 통과 |
| revision은 성공한 변경만 증가, 중복 open 유지, close 후 handle 무효·재open 시 0(`revision_counts_successful_mutations_and_follows_lifecycle`) | 통과 |
| 종료 시 접수된 job drain 후 신규 접수 `Stopped`(`shutdown_drains_accepted_jobs_then_rejects_new_ones`), drop 시 stop+join(`dropping_the_thread_stops_and_joins`) | 통과 |
| document thread에서 `wait`/`wait_timeout` 거부, `try_take`는 대기 없이 `None`(`blocking_wait_is_refused_on_the_document_thread`) | 통과 |
| 자기 자신 shutdown 거부(`shutdown_from_the_document_thread_is_refused`), job panic 시 `Lost`·`Stopped`·`Panicked`(`job_panic_stops_the_thread_and_reports_lost`) | 통과 |
| crate 정책: runtime edge·ffi→runtime 통과, runtime→ffi·document→runtime·project→runtime·ffi→document·ffi→project 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 "Phase XI, Phase VII"는 phase 단위 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다. 이
  작업에 필요한 코드(문서 소유자 `Document`, 열린 문서 집합 `OpenDocumentSet`, thread 역할
  `ThreadRole`)는 base에 있습니다. command/undo(Phase VII)는 아직 골격이라 job은 범용 closure이며,
  command 적용 경로는 후속 작업에서 이 job 경로 위에 얹습니다.
- 계획에 없던 crate `stellar-runtime`을 새로 만들고 crate 정책을 바꿨습니다(ADR-0013).
- 수신함은 이 작업에서 무제한 FIFO입니다. 용량 제한·우선순위는 XII.1.2에서 추가합니다.

## 남은 위험 / 한계

- `docs/architecture/crate-dependency-graph.md`(사람용 표)는 이 lane의 변경 허용 경로 밖이라
  갱신하지 않았습니다. 강제 기준인 `crate_graph.rs`와 ADR-0013가 우선이며, 문서 표는 후속 문서
  변경으로 맞춰야 합니다.
- `stellar-ffi`는 아직 runtime을 연결하지 않았습니다(이 lane은 `crates/ffi` 비변경). 셸에서
  document thread를 쓰려면 ffi 쪽 연결 작업이 필요합니다.
- `DocumentMut`는 현재 문서 API(`insert_node`/`remove_node`/`set_parent`)만 forwarding합니다.
  payload 변경 API가 늘면 함께 확장해야 합니다.
- release profile은 `panic = "abort"`라 job panic 시 프로세스가 종료됩니다(II.1.3 정책). job 안
  panic 경로를 만들지 않는 것은 각 command 구현의 책임입니다.

## 후속 수정(보안 리뷰 FR-1 반영): 종료가 멈춘 document thread를 무기한 기다리지 않음

- **문제**: `AppRuntime` drop(셸 `stellar_runtime_destroy`)은 document thread를 시간 제한 없이 join했습니다.
  job이 응답 없는 파일 시스템 호출(FIFO로 바뀐 매니페스트, 응답 없는 네트워크·FUSE mount)에서 멈추면 셸이
  main thread에서 부르는 destroy와 앱 종료가 영구히 멈췄습니다.
- **수정**: `DocumentThread::shutdown_timeout(timeout)`을 추가했습니다. thread 종료 신호(Condvar)를 최대
  `timeout`까지 기다리고, 넘으면 아직 시작하지 않은 job을 버리고(ticket `Lost`) thread를 분리한 뒤
  `DocumentThreadError::ShutdownTimedOut`을 반환합니다. 실행 중인 job은 끊을 수 없어 그 job이 끝나면 thread가
  스스로 끝납니다. `AppRuntime`은 drop에서 이 경로를 최대 `AppRuntime::SHUTDOWN_TIMEOUT`(2초)로 쓰고,
  `AppRuntime::shutdown_timeout`도 엽니다. 끝까지 기다리는 `shutdown`과 `DocumentThread` drop은 그대로입니다.
  ADR-0013 결정 5와 ADR-0014 XII.6 결정 8에 보완을 적었습니다. 멈춤의 트리거였던 FIFO 매니페스트 열기는
  IV.5.3 후속 수정에서 막았습니다.
- **검증**: `dropping_the_runtime_does_not_wait_forever_for_a_stalled_document_thread`(수정 전 10초 안에 drop이
  끝나지 않아 실패, 수정 후 대기 상한 200ms 뒤 반환하고 뒤에 접수된 job은 실행하지 않음),
  `shutdown_timeout_reports_a_stalled_document_thread`, `shutdown_timeout_detaches_a_stalled_thread_and_discards_pending_jobs`,
  `shutdown_timeout_joins_a_thread_that_finishes_in_time`.
