# 작업 보고서: XVI.4.1 T0\~T5 성능 예산 config

- WBS: XVI.4.1
- 작업명: `config/performance-budgets.yaml`에 T0\~T5 단계 목표·상한, p50/p95/p99, GPU p99, queue depth 기록
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 구현 판정 통과, 선행 미충족(계획 1.2 규칙 4): 선행 XVI.1.3(`[~]`)이 `[x]`가 아닙니다(XVI.1.2 사슬). 이전 기록: 완료 (`[x]`)
- 결정: ADR-0084 결정 10
- 선행: XVI.1.3(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `config/performance-budgets.yaml` — `interaction:` 절(보호 경로, ADR-0084).
- `crates/diagnostics/src/budget.rs`(신규).
- `docs/adr/ADR-0084-scene-incremental-and-interaction-performance.md`(신규).
- `docs/task-reports/XVI.4.1.md`(신규).

## 구현 요약

- `interaction:` 절에 아키텍처 §6.2\~§6.4 값을 정수 µs로 옮깁니다. 단계 목표·상한 9행, 합계, p50·p95·p99, 통제 가능 max, GPU p99, present 대기열 깊이, 스레드 점유, hover 목표·상한입니다.
- reader는 다른 절을 건너뛰고 이 절만 엄격히 검사합니다. 거부하는 경우는 다음과 같습니다.
  - 누락·중복·알 수 없는 key, 10진 정수 아님
  - 단계 합 ≠ 합계, 목표 > 상한, p50 ≤ p95 ≤ p99 ≤ max 위반
- T 구간 대응: T0→T1 입력+Hit Test, T1→T2 Command, T2→T3 Binding+레이아웃, T3→T4 Scene+Render Graph, T4→T5 GPU 제출. 스케줄링 여유는 합계에만 들어갑니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| `cargo xtask validate-adr --base d2bf97b3` | lane 결과 보고에 기록 |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `config_matches_architecture_section_six`: 아키텍처 문서 §6.2 9행·합계, §6.3 p50·p95·p99·max·GPU·대기열·점유, §6.4 hover·이동 계약을 config와 자동 비교합니다. 값이 다르면 실패합니다.
- `spans_group_stage_rows`, `rejects_malformed_sections`.

## 성능

- 해당 없음.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- storage·autosave reader와 같은 엄격 형식을 이 crate 안에서 따로 구현했습니다. diagnostics는 storage에 의존할 수 없기 때문입니다.

## 2026-11-23 interaction-trace 후속: 시나리오 최소 sample 수 (ADR-0096 결정 6)

- `config/performance-budgets.yaml` `interaction:` 절에 `scenario_min_samples: 1000`을 더했습니다(계획 XVI.2.5의 1,000).
  보호 경로 변경이라 ADR-0096에 근거를 적었습니다. 단계 목표·상한 등 아키텍처 6장 수치는 바꾸지 않았습니다.
- reader(`budget.rs`)는 새 key를 필수로 읽고 0을 거부합니다. release gate(XVI.4.4)가 이 값을 씁니다.

| 테스트 | 확인 |
|---|---|
| `config_matches_architecture_section_six` | 6장 표와 config 값 비교 통과(변경 없음) |
| `rejects_malformed_sections` | `scenario_min_samples: 0` 거부, 파일 값 1,000 읽음 |

### 근거 커밋

- `0de2d78c` Feat: T0\~T5 상호작용 예산을 performance-budgets에 기록하고 reader 추가 (XVI.4.1, ADR-0084 신규)
- 이 커밋(`Task(XVI.4.1)`): 시나리오 최소 sample 수 항목.
