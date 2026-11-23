# 작업 보고서: XXX.3.4 활성 MotionSpec과 대상 binding만 평가

- WBS: XXX.3.4
- 작업명: tick마다 활성 MotionSpec과 그 대상 binding만 평가하도록 binding scheduler에 연결.
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXX.3.3이 `[x]`가 아닙니다. 선행이 끝나면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`)
- 결정: ADR-0094 결정 6.3
- 선행: XXX.3.3, VIII.6.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/tests/preview_scheduler.rs`(신규). 연결 코드는 XXX.3.3 commit의 `session.rs`.

## 구현 요약

- 세션은 `BindingEvaluator::update_sources`에 바뀐 상태 변수만 넘깁니다. 상태 변화가 없는 tick은 binding을 평가하지 않고 진행 중 motion만 보간합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |

시험:

- `preview_scheduler.rs::only_affected_bindings_are_evaluated`: 토글 tick 평가 2(상태와 무관한 knob width 0), 입력 없는 tick 평가 0.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 평가 수는 scheduler `UpdateReport.evaluated`입니다. 보간 비용은 따로 세지 않습니다.
