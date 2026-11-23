# 작업 보고서: XXIV.3.3 performance replay CLI

- WBS: XXIV.3.3
- 작업명: performance replay CLI를 reference runner에서 실행할 수 있게 함
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 구현 판정 통과, 선행 미충족(계획 1.2 규칙 4): 선행 XVI.2.2(`[~]`)가 `[x]`가 아닙니다. 기준 장비 실행 기록도 아직 없습니다. 이전 기록: 완료 (`[x]`)
- 결정: ADR-0084 결정 9·11
- 선행: XVI.2.2(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `tools/xtask/src/replay.rs` — `run_cli`(`record`·`run`).
- `tools/xtask/src/commands.rs` — 명령 registry에 `replay` 등록.
- `tools/xtask/src/replay/tests.rs` — CLI·schema 테스트.
- `docs/task-reports/XXIV.3.3.md`(신규).

## 구현 요약

- `cargo xtask replay record <pan|drag|resize> --out FILE [--seed N] [--size N]`는 표준 scenario 재생 파일을 씁니다.
- `cargo xtask replay run FILE... [--out JSON] [--gate] [환경 인자]`는 core replay와 결과 JSON을 냅니다.
- 환경은 인자가 없으면 `git rev-parse HEAD`·`sysctl`·`sw_vers`에서 읽습니다. 그래서 기준 장비에서 같은 명령이 같은 schema의 결과를 냅니다. scenario별 scene build·render graph p99는 표준 오류에 진단으로 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p xtask --lib replay` | 7 passed, 0 failed |
| `cargo xtask help` | `replay` 표시 |
| `cargo run --release -p xtask -- replay run pan.replay drag.replay resize.replay --gate` | 결과 JSON 기록. 부하에 따라 gate 통과(종료 0) 또는 Drag p99 초과(종료 65) |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `result_schema_is_identical_across_runs`: 두 번 실행한 결과 JSON의 key·고정 문자열 구조가 같음(gate 위반 목록 앞까지).
- `cli_records_and_runs_replay_files`: record → run → 결과 파일. 결과에 경로 없음, 잘못된 명령 64, fixture 명세 변조 시 hash 불일치 70.

## 성능

- XVI.2.2의 수치를 참조합니다.

## 차이(작업 지시와 다른 점)

- 기준 장비(M1 16GB)에서 실제로 실행하지는 않았습니다. CLI와 schema 동일성만 개발 장비에서 확인했습니다.

## 남은 위험 / 한계

- 기준 장비 실행 기록이 아직 없습니다.
