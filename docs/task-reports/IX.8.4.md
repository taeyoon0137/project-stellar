# 작업 보고서: IX.8.4 control contract → role/state/action 매핑

- WBS: IX.8.4
- 작업명: Button·TextInput·Modal·List·SingleSelectionGroup 계약을 접근성 role·state·action으로 매핑
- 상태: 완료 (`[x]`)
- 선행: IX.6.7 (base에서 `[x]`), IX.8.3 (이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/accessibility/src/control.rs`(신규) — `SourceControl`, `ControlSemantics`, `SelectionMode`, 계약·항목 문맥 매핑.
- `crates/accessibility/src/source.rs` — `SourceNode::control` field와 `with_control` 빌더.
- `crates/accessibility/src/generate.rs` — 생성 전 control 문맥 계산, flatten 대상 제외, 매핑 적용.
- `crates/accessibility/src/lib.rs` — `control` 모듈 선언.
- `docs/task-reports/IX.8.4.md`(신규), 계획서(IX.8.4 `[ ]`→`[x]`).

## 구현 요약

- **입력**: crate 그래프상 document의 `ControlContract`를 직접 읽을 수 없으므로, 계약 종류별 의미 field를 담은
  중립 `SourceControl`(`ControlSemantics` + focus 가능 여부)을 `SourceNode`에 둡니다. 계약 kind·field id와의
  대응표는 `control.rs` 모듈 문서에 있습니다(field 값: 상수 → `Constant`, 컴포넌트 Property →
  `ComponentProperty`, 상태 변수 → `State`).
- **매핑**: Button → `Button`(activate → `Activate`, focused·disabled·pressed 상태), TextInput → `TextField`
  (value = 편집 상태 변수, `Editable`, `SetValue`, label 없으면 placeholder), Modal → `Dialog`(`Modal`, dismiss →
  `Dismiss`), List → `List`(Multiple이면 `Multiselectable`), SingleSelectionGroup → `RadioGroup`. focus 가능한
  계약은 `Focusable` 상태와 `Focus` 동작을 더합니다. 모르는 계약 종류는 기본 매핑을 유지합니다.
- **항목 문맥**(시각 계층과 다른 접근성 관계, IX.8.2): List collection의 자식은 `ListItem`(선택 방식에 따라
  `Select`, Single이면 `Selected = SelectionMatches`), 의미 없는 collection Frame은 flatten. radio group이
  가리키는 List는 flatten되고 그 Button 항목은 `Radio`(`Checked = SelectionMatches`, `Select`, pressed 제거)가
  되어 radio group 바로 아래에 옵니다. 문맥은 source 전체를 StableId 순으로 한 번 훑어 결정론적으로 계산합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-accessibility` | 23 passed, 0 failed |
| `cargo fmt -p stellar-accessibility -- --check` | 통과 |
| `cargo clippy -p stellar-accessibility --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| Button golden tree(상태 binding 3종·activate·focus)(`button_golden_tree`) | 통과 |
| TextInput golden tree(placeholder label 대체, 작성자 label 우선)(`text_input_golden_tree`) | 통과 |
| Modal golden tree(dialog + 내부 button)(`modal_golden_tree`) | 통과 |
| List golden tree(collection flatten, Single 선택 상태, Multiple·None 선택 방식)(`list_golden_tree`) | 통과 |
| SingleSelectionGroup golden tree(List flatten, Button → Radio, 그룹 selected 없으면 List selection)(`single_selection_group_golden_tree`) | 통과 |
| 모르는 계약 종류는 기본 매핑 유지(`unknown_contract_keeps_base_mapping`) | 통과 |
| 모든 golden tree가 `validate(source)` 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- 계약은 문서 타입이 아니라 중립 사본(`SourceControl`)으로 받습니다. document에 의존하지 않는 crate 그래프
  (`stellar-accessibility → core-types`)를 지키기 위한 선택이며, 매핑 규칙은 IX.6 계약 field를 그대로 따릅니다.
- 계약의 IME 입력·overlay·focus scope root·navigation 방향과 Modal `open`은 접근성 트리 의미가 아니어서
  옮기지 않았습니다.

## 남은 위험 / 한계

- 문서 `ControlContractTable` → `SourceControl` adapter는 아직 없습니다(IX.8.3 보고서와 같은 crate 그래프
  제약). adapter를 추가할 때 대응표의 kind·field id와 일치하는지 검사하는 테스트가 필요합니다.
- 연결 List가 flatten될 때 그 List의 작성자 label은 radio group으로 옮기지 않습니다.
- DataKey 항목 identity의 runtime key 해석은 binding evaluator 몫이며, 이 매핑은 항목 source StableId를 씁니다.

## 리뷰 후 수정(9A 리뷰, 2026-11-14)

- 지적(A11Y-4, minor): SingleSelectionGroup 합성이 연결 List의 선택 방식(Single)을 확인하지 않아 Multiple List도
  조용히 radio로 합성했고, Button이 아닌 항목은 flatten된 List의 항목 문맥(`ListItem`, 선택 상태, `Select`)을 잃고
  `RadioGroup` 아래 `Generic`으로 남았습니다. 이 보고서는 두 경우를 적지 않았습니다.
- 수정: 합성은 IX.6.6 검사 조건(연결 List의 선택 방식이 Single이고 collection 항목이 모두 Button 계약, 항목이
  없어도 됨)을 만족할 때만 적용합니다. 조건을 어긴 그룹(문서 검사의 `InvalidLinkedContract`)은 `RadioGroup` role만
  받고, 연결 List는 flatten되지 않고 일반 List 매핑(선택 방식, List 항목 문맥과 List 자신의 selection)을 그대로
  가집니다. 따라서 의미를 잃는 항목이 없습니다.
- 한계: 조건을 어긴 그룹을 접근성 진단(IX.8.5)이 따로 보고하지는 않습니다. 계약 오류는 문서 계약 검사(IX.6.6)가
  보고합니다.
- 완료 근거(control별 golden tree 통과)는 바뀌지 않으므로 상태 `[x]`를 유지합니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-accessibility` | 36 passed, 0 failed |
| `cargo clippy -p stellar-accessibility --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-accessibility -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Multiple List·Button 아닌 항목이 있는 Single List를 가리키는 그룹은 List 매핑을 유지하고, 항목 없는 Single List 그룹은 합성(`invalid_selection_group_keeps_list_mapping`) | 수정 전 실패(Multiple List를 radio로 합성), 수정 후 통과 |
