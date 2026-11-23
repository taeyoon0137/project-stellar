# 작업 보고서: XXX.2.2 결정론적 자동 배치

- WBS: XXX.2.2
- 작업명: hint 없는 node 위치를 계산하고 hint node는 고정.
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXX.2.1, XXX.1.5이 `[x]`가 아닙니다. 선행이 끝나면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`)
- 결정: ADR-0094 결정 5, ADR-0025 결정 2.6
- 선행: XXX.2.1, XXX.1.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/src/auto_layout.rs`(신규).
- `crates/prototype/src/lib.rs`: module 등록.
- `crates/prototype/tests/graph_layout.rs`(신규).

## 구현 요약

- 상태 변수 `write` 되먹임 edge를 뺀 가장 긴 경로로 열을 정하고(Kahn, node 순), 열마다 node 순으로 행을 채웁니다. 남는 순환 node는 마지막 열 다음입니다.
- hint node는 hint 좌표에 고정하고 행을 차지하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `graph_layout.rs::auto_layout_is_deterministic_and_keeps_hints`: 같은 입력 같은 좌표, hint node 좌표 불변.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- hint node와 자동 배치 node가 겹치는지는 보지 않습니다(UI 재설계 때 결정).
