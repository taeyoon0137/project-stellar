# 작업 보고서: XXV.2.3 영향받는 진단만 증분 재검사

- WBS: XXV.2.3
- 작업명: document 변경 시 영향받는 diagnostic만 증분 재검사
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXV.2.1이 `[x]`가 아닙니다. 선행이 끝나면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]` 후보). 선행 XXV.2.1 판정을 따릅니다.
- 결정: ADR-0095 결정 4

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 test).

## 변경 파일 목록

- `crates/integrity/src/incremental.rs`: `IntegrityEngine`.
  - 내장 checker 전체 검사, change set 대조 재검사.
  - 외부 생산자 결과 넣기, stale 보고, 문서 제거.
  - checker 실행 계측.
- `crates/integrity/src/tests.rs`: 무관한 편집 재계산 0, 관련 편집은 scope 하나만 재계산, 무작위 command 열에서 증분 결과 =
  전체 재검사.

## 구현 요약

- (문서, 생산자) scope마다 footprint를 남깁니다: 직접 읽은 객체, 관심 속성 key, node 삽입 반응.
- `ChangeSet`(VII.1.4)의 바뀐·삽입·삭제 node와 바뀐 속성 경로를 footprint와 대조합니다.
- 걸린 내장 checker만 다시 실행합니다. 하나도 걸리지 않으면 문서 레코드를 만들지 않습니다.
- 걸린 외부 생산자 scope는 stale로 보고합니다. 같은 scope로 다시 넣으면 풀립니다.
- 잠금·표시 이름 key는 "모든 key" 관심에서도 뺍니다(편집 전용 필드).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integrity` | 통과(17 passed) |

| 검사 | 결과 |
|---|---|
| unrelated edit recompute 0: 무관한 node의 표시 이름·잠금 변경 뒤 checker 실행 0, stale 0, index revision·내용 불변 | 통과(`tests::unrelated_edit_recomputes_nothing_and_related_edit_rechecks_only_its_scope`) |
| 표시 여부 변경은 Target Profile·binding·font·접근성 scope를 stale로 만들고 내장 checker는 0(접근성 tree 밖 node도 표시 여부로 tree에 들어올 수 있음) | 통과(같은 test) |
| 어떤 외부 진단에도 없는 node의 표시 여부 변경도 font·접근성·token scope를 stale로 만들고, 지정 node만 보는 binding scope는 그대로 | 통과(같은 test, 검토 보완) |
| clip 해제는 참조 checker 하나만 다시 실행하고 진단 1개 감소 | 통과(같은 test) |
| 무작위 문서 12개 × 유효 command 최대 40개: 매 단계 증분 index = 새 엔진 전체 검사, 일부 단계는 실행 0 | 통과(`tests::incremental_result_equals_full_recheck_on_random_command_sequences`, 미해결 node 참조가 있는 단계 136개 포함) |

## 차이(작업 지시와 다른 점)

- 접근성·Target Profile·font·binding·library처럼 문서 밖 상태를 읽는 생산자는 엔진이 다시 실행할 수 없습니다. stale 보고로
  대신하고, 호출자(앱 runtime)가 다시 실행해 넣어야 합니다.

## 남은 위험 / 한계

- 참조 checker footprint의 참조 위치 목록은 문서 validator의 참조 검사와 같아야 합니다. 새 참조 속성이 생기면 함께 고쳐야
  합니다. 무작위 비교 test가 어긋남을 잡습니다.
- 앱 문서 thread에서 command 뒤 `apply_changes`를 부르는 연결은 UI 재설계 때 합니다.
