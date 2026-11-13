# 작업 보고서: IX.6.6 SingleSelectionGroup 계약

- WBS: IX.6.6
- 작업명: SingleSelectionGroup contract로 List+Button 합성 Radio UI 표현, 별도 RadioButton node 없음
- 상태: 완료 (`[x]`)
- 선행: IX.6.2(이 lane에서 `[x]`), IX.6.5(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/control/selection_group.rs`(신규) — `SINGLE_SELECTION_GROUP` 종류(0x0005), field 상수,
  `single_selection_group_contract`, `validate_single_selection_group`, runtime `validate_single_selection`,
  `SelectionStateError`.
- `crates/document/src/control/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.6.6.md`(신규), 계획서(IX.6.6 `[ ]`→`[x]`).

## 구현 요약

- **합성**: 그룹 Frame은 `list`(1, Single 선택 List 계약을 가진 Frame, 필수)와 `selected`(2, 선택 identity String
  상태 변수)만 갖습니다. Radio 하나하나는 List item인 Button 계약 Frame입니다. 새 node 종류는 없습니다.
- **검사**(`validate_single_selection_group`, 연결 계약은 contract 표에서 조회):
  - list node가 List 계약이 아니거나 선택 방식이 Single이 아니면 `InvalidLinkedContract` 오류.
  - List collection의 item(자식) 중 Button 계약이 없는 것이 있으면 `InvalidLinkedContract` 오류.
  - list 누락은 진단, selected가 상태 변수가 아니면 `NotWritable`, `SelectionChange` 미처리는 진단.
- **runtime 상태**: `validate_single_selection`은 선택 identity가 2개 이상이면 `MultipleSelected`로 거부합니다
  (0개·1개 허용).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 301 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 선택 `[a, b]` → `MultipleSelected` 거부, 1개·0개 통과(`more_than_one_selected_runtime_state_is_rejected`) | 통과(거부) |
| NodeKind에 radio 종류 없음, Button 아닌 item·Multiple List·List 아닌 node 연결 오류(`radio_is_list_plus_button_without_a_new_node_kind`) | 통과 |
| Single List + 상태 selected → 0건, list 누락 진단(`complete_group_has_no_issues`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 선택 상태를 List `selection`·Button `pressed`와 자동 동기화하는 것은 상태 머신 transition(IX.5)으로 작성하며
  자동 생성하지 않습니다.
