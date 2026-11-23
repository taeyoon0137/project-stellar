# 작업 보고서: XXIII.2.2 Candidate 출처 provenance

- WBS: XXIII.2.2
- 작업명: object/property별 A, B, auto, custom source provenance를 C에 기록
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XXIII.2.1(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XXIII.2.1
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/candidate.rs` — `UnitKey`, `Source`, `unit_state`, `initial_provenance`, `provenance`·`provenance_entries`.
- `crates/semantic-merge/tests/candidate_resolution.rs`(신규).

## 구현 요약

- 단위는 `(StableId, 대상)`(객체 존재·위치·속성 경로)입니다. base → C 변경의 단위마다 C 상태를 A·B 상태와 비교해 A·B 한쪽과만
  같으면 그쪽, 양쪽과 같거나(같은 변경) 둘 다 아니면(엔진 계산: key 재배정·정의 목록 병합) `Auto`입니다.
- conflict 단위는 base를 유지하므로 `Base`, 기록이 없는 단위는 base 그대로입니다. `Custom`은 해결 command(XXIII.2.3)가 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 전체 통과(candidate_resolution 1) |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| A만 바꾼 이름 = A, B만 = B, 같은 값으로 양쪽 = Auto, conflict = Base, 무변경 = Base, 기록 목록 순서(`provenance_records_a_b_auto_and_base`) | 통과 |

## 차이(작업 지시와 다른 점)

- 출처에 `Base`를 더했습니다. conflict로 base를 유지하거나 사용자가 base를 고른 단위를 A·B·auto와 구분하기 위해서입니다.

## 남은 위험 / 한계

- 객체 전체를 바꾼 단위(객체 대상 해결)는 그 객체의 속성·위치 출처 기록을 지우고 객체 단위 하나로 기록합니다.
