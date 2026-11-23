# 작업 보고서: XXX.3.5 reduced motion 대체 동작

- WBS: XXX.3.5
- 작업명: 환경 key `reduced_motion`이 켜지면 MotionSpec 대체 동작 사용.
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXX.3.3이 `[x]`가 아닙니다. 선행이 끝나면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`)
- 결정: ADR-0094 결정 6.5
- 선행: XXX.3.3, VIII.7.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/tests/preview_reduced_motion.rs`(신규). 동작 코드는 XXX.3.3 commit의 `session.rs`.

## 구현 요약

- `SessionConfig::reduced_motion`과 `set_reduced_motion`이 환경 입력을 바꾸고, 그 key를 읽는 binding만 재평가합니다. `instant` spec은 끝값으로 즉시, `play` spec은 그대로 재생합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |

시험:

- `preview_reduced_motion.rs::reduced_motion_uses_fallback`: off는 보간 시작, on+instant는 즉시 끝값·진행 motion 0, on+play는 재생, 세션 중 켜기.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 없음.
