# 작업 보고서: XI.6.3 tessellation worker pool과 stale scene revision 결과 폐기

- WBS: XI.6.3
- 작업명: tessellation 작업을 worker pool에서 실행하고 최신 scene이 요청하지 않은 오래된 revision 결과를 버림
- 상태: 완료 (`[x]`)
- 선행: X.7.5 (`[x]`), XI.6.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/tessellation_worker.rs`(신규) — `TessellationInput`, `Tessellators`,
  `TessellationJob`·`TessellationOutput`, `TessellationWorkerPool`(`WorkerPoolError`, `SubmitError`),
  `AsyncTessellation`(`RequestOutcome`, `CollectedResult`, `AsyncTessellationStats`).
- `crates/renderer-core/src/tessellation_worker/tests.rs`(신규) — gate로 순서를 고정한 stale 시나리오와 다중
  worker 교차 시뮬레이션.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.6.3.md`(신규), 계획서(XI.6.3 `[ ]`→`[x]`).

## 구현 요약

- **worker pool**: `std::thread` worker(1..=16)가 시작 시 `ThreadRole::Worker`를 bind하고 공유 job queue
  (상한 있는 `sync_channel`)에서 job을 받아 fill/stroke tessellator를 실행한 뒤 결과 channel로 보냅니다.
  `try_submit`은 막히지 않고 `Full`/`Disconnected`(job 반환)를 돌려줍니다. drop은 queue를 닫고 모든
  worker를 join합니다.
- **입력 hash + 세대**(X.7.5와 같은 규칙): job 식별은 `TessellationKey`(path canonical bytes + 정규화한
  style)입니다. `AsyncTessellation::request(cache, revision, input)`은 cache hit면 `Ready`, 같은 key가 진행
  중이면 필요 revision만 올리고 합치며(`coalesced`), 진행 중 상한·queue가 차면 `Deferred`, 아니면 제출합니다.
- **stale 폐기**: `collect`(막힘 없음)·`collect_one_blocking`은 결과의 필요 revision이
  `LatestSceneRevision`의 최신 세대보다 작으면 cache에 넣지 않고 `DiscardedStale`로 버립니다. 실패 결과도
  cache에 넣지 않습니다(다음 요청에서 재시도). 버린 key는 이후 요청에서 다시 제출됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-renderer-core` | 종료 코드 0 |
| `cargo test -p stellar-renderer-core` | 113 unit passed(신규 5) + 3 doctest passed, 0 failed |
| `cargo test -q -p stellar-renderer-core tessellation_worker` 30회 반복 | 30회 모두 5 passed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| revision 1이 a·c 요청 → revision 2로 넘어가 b·c 요청 → worker 해제: a는 `DiscardedStale(1 < 2)`이고 cache에 없음, b·c만 적용(revision 2), 모든 tessellation이 Worker thread에서 실행, 버린 a 재요청 시 재제출·적용(`stale_revision_results_are_never_applied`) | 통과 |
| 실패 결과 미저장·`failed` 집계, 진행 중 상한에서 `Deferred`(`failures_are_not_cached_and_limits_defer`) | 통과 |
| stroke job이 worker에서 실행되고 inline 결과와 같은 mesh, snapshot revision 요청(`stroke_jobs_run_on_workers_and_match_inline_result`) | 통과 |
| 200 revision × 무작위 key 요청, worker 3개 비결정 도착 순서: 모든 적용 결과가 판정 시점 최신 revision의 요청 key, 모든 폐기는 revision < latest, `applied + discarded = submitted`(`interleaved_revisions_never_apply_stale_results`) | 통과 |
| worker 수·queue 용량 검증, 대기 worker가 있는 pool drop 종료(`pool_validation_and_shutdown`) | 통과 |

## 차이(작업 지시와 다른 점)

- runtime crate의 thread 모델(XII.1)과 아직 연결하지 않았습니다. renderer-core 안의 독립 pool이며, worker 작업
  분류·우선순위(XII.1.4)가 이 pool을 class로 흡수할 예정입니다.
- "stale"은 결과의 필요 revision이 최신 세대보다 작은 경우로 정의했습니다. 같은 key를 최신 revision이 다시
  요청했으면(입력 hash가 같아 mesh가 같음) 그 결과는 최신 요청의 결과로 적용합니다.

## 남은 위험 / 한계

- tessellator가 panic하면(test profile) 그 key는 진행 중으로 남아 다시 제출되지 않습니다. 계약상 tessellator는
  panic하지 않으며 release는 abort입니다.
- 결과 channel은 상한이 없지만 진행 중 job 수(`max_in_flight`)가 결과 수를 제한합니다.
- 다중 worker 교차 테스트의 stale 발생 횟수는 thread 스케줄에 따라 달라집니다. 폐기 동작 자체는 gate로 순서를
  고정한 테스트가 결정적으로 검증합니다.
