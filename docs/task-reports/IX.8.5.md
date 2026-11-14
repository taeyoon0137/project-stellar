# 작업 보고서: IX.8.5 접근성 진단

- WBS: IX.8.5
- 작업명: label 누락, invalid nesting, focusable·action 불일치, duplicate reading order 진단
- 상태: 완료 (`[x]`)
- 선행: IX.8.4 (이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/accessibility/src/diagnostics.rs`(신규) — `diagnose`, `AccessibilityIssue`, `AccessibilityProblem`,
  `NestingRule`, `Severity`.
- `crates/accessibility/src/lib.rs` — `diagnostics` 모듈 선언.
- `docs/task-reports/IX.8.5.md`(신규), 계획서(IX.8.5 `[ ]`→`[x]`).

## 구현 요약

- **label 누락**(`missing-label`, 오류): `Button`·`Radio`·`TextField`·`Image`·`Dialog`에 사용할 수 있는 label이
  없을 때. `Button`·`Radio`는 자손 `StaticText`·`Image` label로 이름을 계산할 수 있으면 통과합니다. 빈·공백
  문자열과 문자열이 아닌 상수는 label이 아니며, 속성·상태 binding은 정적으로 알 수 없어 통과로 봅니다.
- **invalid nesting**(`invalid-nesting`, 오류): 상호작용 역할(`Button`·`TextField`·`Radio`) 안의 상호작용 역할,
  `List` 밖 `ListItem`, `RadioGroup` 밖 `Radio`. 가장 가까운 의미 조상을 찾을 때 `Generic`은 건너뜁니다.
- **focusable·action**: 상수 `Focusable = true`인데 사용자 동작(`Activate`·`SetValue`·`Select`·`Dismiss`)이 없으면
  `focusable-without-action`(경고, 탐색 container `List`·`RadioGroup`·`Dialog` 제외), `Activate`·`SetValue`가
  있는데 focus 불가(상태 없음 또는 상수 `false`)면 `action-without-focus`(오류, 키보드 접근 불가).
- **duplicate reading order**(`duplicate-reading-order`, 오류): 같은 부모(또는 root 목록)의 형제가 같은 명시 읽기
  순서를 가질 때, 순서 값마다 형제 목록을 담아 한 번 보고합니다.
- **순회**: root부터 반복 preorder, 방문 집합으로 cycle에도 종료합니다. 결과 순서는 root 중복 검사 → node
  preorder(자신의 진단 → 자식 읽기 순서 검사)로 결정론적입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-accessibility` | 28 passed, 0 failed |
| `cargo fmt -p stellar-accessibility -- --check` | 통과 |
| `cargo clippy -p stellar-accessibility --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| label 누락 fixture: 무라벨 Image·Button·TextField, 공백·Boolean 상수 label은 보고, 내용 이름 Button·binding label Dialog는 통과(`missing_label_fixture`) | 정확 |
| invalid nesting fixture: Generic을 건넌 Button 안 Button, Group 아래 ListItem, 의미 조상 없는 Radio, List 아래 ListItem 통과(`invalid_nesting_fixture`) | 정확 |
| focus·action fixture: 동작 없는 focusable Generic(경고), focus 없는 Activate·`Focusable=false` SetValue(오류), binding Focusable·탐색 container·Dialog Dismiss 통과(`focus_and_action_fixture`) | 정확 |
| duplicate reading order fixture: root 목록과 부모별, 순서 값마다 한 번(`duplicate_reading_order_fixture`) | 정확 |
| IX.8.4 생성 결과(radio group + text field) 진단 0건, cycle tree에서도 종료·결정론(`generated_control_trees_are_clean_and_diagnosis_is_deterministic`) | 통과 |

## 차이(작업 지시와 다른 점)

- "focusable action 없음"을 양방향으로 구현했습니다: focus 가능하지만 동작 없음(경고)과, 기술 아키텍처 §18의
  "키보드 접근 불가"에 해당하는 동작은 있지만 focus 불가(오류). `Select`·`Dismiss`는 목록·대화 상자 container가
  focus를 중개하므로 focus 불가 검사에서 제외했습니다.

## 남은 위험 / 한계

- 진단 결과는 이 crate의 `AccessibilityIssue`입니다. 공통 `DiagnosticIndex` 집계(XXV.2.1)는 diagnostics crate와의
  연결이 필요하며 그 작업에서 다룹니다.
- 기술 아키텍처 §18의 "읽기 순서와 시각 순서 불일치", "선택 상태 누락" 진단은 이 행 범위 밖이라 구현하지
  않았습니다.
