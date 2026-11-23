# 작업 보고서: XVI.4.4 release gate

- WBS: XVI.4.4
- 작업명: release gate는 모든 required scenario가 p50≤5, p95≤7, p99≤9, controllable max≤10ms를 통과해야 성공
- 상태: 완료 (`[x]`)
- 결정: ADR-0084 결정 11
- 선행: XVI.4.2(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/analyzer.rs` — `release_gate`, `GateViolation`.
- `tools/xtask/src/replay.rs` — `replay run --gate`(위반 시 종료 65).
- `docs/task-reports/XVI.4.4.md`(신규).

## 구현 요약

- required scenario마다 기준 4개를 config 값으로 검사합니다. 경계값(같음)은 통과입니다. sample 없는 required scenario는 실패이고, 필수가 아닌 scenario는 판정하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| `cargo run --release -p xtask -- replay run pan.replay drag.replay resize.replay --gate` | 결과 JSON 기록. 부하에 따라 gate 통과(종료 0) 또는 Drag p99 초과(종료 65) |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `gate_threshold_boundaries`: 경계값 통과, 기준마다 1ns 초과 실패(지표·scenario index), 비필수 무시, 빈 필수 실패.

## 성능

- `replay run --gate`는 개발 장비에서 부하에 따라 통과하거나(종료 0) Drag p99 초과로 실패했습니다(종료 65). 기준 장비 판정이 아닙니다.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- GPU p99(4ms)·present 대기열 깊이 판정은 GPU 경로가 없어 gate에 넣지 않았습니다. config에만 기록합니다.

## 2026-11-23 interaction-trace 후속: gate 보강 (ADR-0096 결정 6)

- required scenario는 다음도 지켜야 통과합니다.
  - 판정 sample이 `scenario_min_samples`(1,000) 이상(`too_few_samples`).
  - 선언한 cache setup과 실제 hit·miss가 일치(`cache_setup`, XVI.2.6).
- `cargo xtask replay run --gate`는 §38.1 시나리오 13개 각각의 warm·cold 결과가 모두 있어야 통과합니다. 실행에 없는
  조합은 sample 0개 결과로 넣어 `no_samples`로 실패합니다(빼지 않음).
- sample 수 위반은 `value_ns`·`limit_ns`에 ns 대신 개수를 씁니다. 사람이 읽는 메시지는 "sample 999개 < 1000개"입니다.

| 테스트 | 확인 |
|---|---|
| `gate_threshold_boundaries` | 기존 경계(5·7·9·10ms) + sample 1,000 통과·999 실패 + cache 불일치 실패 |
| `result_schema_is_identical_across_runs`(xtask) | 2개만 실행하면 없는 조합 24개가 `no_samples`, 작은 실행은 `too_few_samples` |

### 측정

- XVI.2.5 보고서의 release 측정(20개 재생 파일, `--gate`)을 보십시오. Text Edit·Token Propagation·Component Update는
  엔진 경로가 이어지지 않아 sample 0개로 실패합니다.

### 근거 커밋

- `e1b2454f` Feat: 성능 sample analyzer·제외 코드·release gate 추가 (XVI.4.2\~XVI.4.4)
- `c6caa47c` Task(XVI.2.5) 작업 커밋(gate 보강 포함)
- 이 커밋(`Task(XVI.4.4)`): 보고서 기록.
