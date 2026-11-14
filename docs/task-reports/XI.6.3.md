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

## 후속 수정 (리뷰 반영)

리뷰 지적 두 건을 `Fix: tessellation 결과 판정을 render thread frame revision 기준으로 바꾸고 panic job 정리`
commit으로 고쳤습니다. 위 "구현 요약"의 stale 기준과 "남은 위험"의 panic 항목은 이 절로 대체됩니다.

- **starvation(major)**: 최초 구현은 결과의 필요 revision을 document thread가 publish한 최신 세대
  (`LatestSceneRevision`)와 비교했습니다. render thread는 아직 받지 않은 snapshot의 요청을 알 수 없으므로,
  document가 frame마다 새 revision을 내는 편집 중에는 도착한 결과가 항상 `wanted < latest`로 버려지고 같은
  key가 frame마다 다시 제출되어 cache에 한 번도 들어가지 못했습니다. 리뷰 재현 순서(frame마다 collect →
  request(rev = frame) → publish(frame + 1), worker 1개, 50 frame)를 lane HEAD(`b971205`)에서 probe 테스트로
  돌려 `submitted: 50, applied: 0, discarded_stale: 49`로 실패를 확인했습니다.
- **변경**: 판정 기준을 render thread가 처리 중인 scene revision(현재 frame revision)으로 바꿨습니다.
  `AsyncTessellation::new(pool, max_in_flight)`는 더 이상 `LatestSceneRevision`을 받지 않습니다.
  `begin_frame(revision)`·`begin_frame_for(snapshot)`이 현재 frame revision을 올리고(줄지 않음), `request`도
  넘긴 revision까지 올립니다. 필요 revision이 현재 frame revision보다 작은 결과만 `DiscardedStale`(필드
  `current`)로 버립니다. frame 규약은 `begin_frame` → `request` → `collect`이며 모듈 문서에 적었습니다.
  stale 정의("render thread가 처리 중인 scene이 요청하지 않은 key의 결과는 cache에 넣지 않음")와 검증 기준
  "stale result 적용 0"은 그대로 유지됩니다.
- **panic 정리(minor)**: worker가 `catch_unwind`로 tessellator panic을 잡아 그 job을
  `TessellationError::Failed` 결과로 보내고 계속 실행합니다. 그래서 진행 중 기록이 정리되고 같은 key가
  영구히 `Pending`으로 합쳐지지 않습니다. release는 `panic = "abort"`라 이 경로는 dev·test 안전장치입니다
  (`docs/architecture/panic-policy.md`). lane HEAD에서 probe 테스트로 결과가 오지 않고(`None`) worker가
  끝나는 것을 먼저 확인했습니다.
- **고치지 않은 부분**: 모든 worker가 끝났을 때 `SubmitError::Disconnected`를 `Deferred`와 구분해 보고하는 것은
  넣지 않았습니다. panic을 잡은 뒤에는 pool이 살아 있는 동안 worker가 끝나는 경로(역할 bind 실패, queue lock
  poison, 결과 receiver drop)가 실제로 생기지 않아 재현·검증할 수 없기 때문입니다.

| 명령 / 검사 | 결과 |
|---|---|
| lane HEAD에서 리뷰 재현 probe(`collect → request → publish` 50 frame) | 실패 재현: `submitted: 50, applied: 0, discarded_stale: 49` |
| lane HEAD에서 panic probe(worker 1개, panic job 결과 대기) | 실패 재현: 결과 `None`, `in_flight` 남음 |
| `unchanged_key_is_cached_during_continuous_edits`(frame 시작 collect 순서, 50 frame) | 통과: frame 2부터 `Ready`, `submitted 1 · applied 1 · discarded_stale 0` |
| `rerequested_key_is_applied_at_current_frame_revision`(권장 순서, 결과가 3 frame 뒤 도착) | 통과: `Applied { revision: 3 }`, `submitted 1 · coalesced 2` |
| `key_not_requested_by_current_frame_is_discarded`(요청 없는 frame 2) | 통과: `DiscardedStale { revision: 1, current: 2 }`, cache 없음 |
| `tessellator_panic_reports_failure_and_keeps_worker`(worker 1개) | 통과: panic job 2번 모두 `Failed`·재제출, 이어진 job `Applied` |
| `cargo test -q -p stellar-renderer-core tessellation_worker` 30회 반복 | 30회 모두 9 passed |
