# 작업 보고서: XIV.5.3 선택 변경의 필수 dependency 반환(자동 stage 없음)

- WBS: XIV.5.3
- 작업명: 선택 변경만으로 snapshot validator가 실패하면 필수 dependency를 `required_additions`로 반환하고 자동 stage하지 않음
- 상태: 완료 (`[x]`)
- 선행: XIV.5.2(`[x]`, 이 lane), VI.10.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/validate.rs`(신규) — `SnapshotViolation`, `validate_snapshot`, `introduced_violations`.
- `crates/semantic-diff/src/stage.rs` — `SelectedOnlyOutcome`, `StagePlan`, `StageError`, `plan_stage`, `plan_selection`(crate 내부), staging fixture.
- `crates/semantic-diff/src/lib.rs` — `pub mod validate;`.
- `docs/task-reports/XIV.5.3.md`(신규), 계획서(XIV.5.3 `[ ]`→`[x]`).

## 구현 요약

- **snapshot validator**: parent 존재(owner가 binding record일 수 있는 expression node·Variant 제외), parent
  cycle(반복 walk), 같은 parent·sequence(modifier stack 별도)의 OrderKey 유일, 알려진 참조 key(View root,
  문서 페이지·페이지 View·Slot·local token 목록, clip·mask·boolean 대상, image asset, 컴포넌트 참조, token
  expression, AST child, slot default/override content)의 대상 존재(NIL 제외), Page·View·Slot 소속 목록,
  부모 없는 node는 View root·컴포넌트 root, instance override의 Property·Slot이 interface에 있음(nested 경로는
  전체 interface)을 정렬된 위반 집합으로 냅니다. `introduced_violations`는 기준 snapshot에 이미 있던 위반을
  빼 변경이 새로 만든 위반만 남깁니다.
- **계획**: `plan_stage(index, working, request)`는 Index → WorkingTree diff로 dependency graph(XIV.5.2)를 만들어
  선택의 closure를 구하고(`StageSelection`), 선택만 Index에 적용한 결과를 validator로 검사해
  `SelectedOnlyOutcome`(Valid·Invalid(새 위반)·NotApplicable(적용 불가))로 함께 돌려줍니다. 필수 dependency는
  `required_additions()`로 반환만 하며 candidate를 만들지 않습니다(자동 stage 없음). 오래된 ID는
  `UnknownChange`, unstage 요청은 `WrongOperation`으로 거부합니다. snapshot은 읽기만 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | unit 69 passed, fuzz 통합 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **invalid partial fixture**: 새 자식 21만 선택 → `MissingParent(21→20)`, required = 문서 페이지 목록·페이지 4·View 5·frame 20; override 311만 선택 → `UnknownInterfaceMember(311)`, required = interface 변경; 독립 변경(frame 11 숨김) → Valid·승인 불필요; 계획 전후 Index 동일(`invalid_partial_selection_returns_required_dependencies_without_staging`) | 통과 |
| 없는 ID → `UnknownChange`, unstage 요청 → `WrongOperation`(`stale_or_wrong_requests_are_rejected`) | 통과 |
| 교체의 추가만 선택 → required = 삭제, `NotApplicable(ObjectExists)`(`replacement_half_is_not_applicable_alone`) | 통과 |
| validator: 7종 위반 위치 정확, 기준 위반 제외(`each_invariant_reports_its_location`), override interface 검사·nested 허용(`instance_overrides_must_name_interface_members`), AST child 대상·NIL 제외·owner record 허용(`expression_children_and_nil_references`), HEAD fixture·random 30 문서는 snapshot 밖 참조 외 위반 0(`valid_documents_have_no_violations`) | 통과 |

## 차이(작업 지시와 다른 점)

- `required_additions`는 validator 결과가 아니라 dependency graph closure입니다. graph가 validator보다 넓은
  참조(binding payload 안 token 등)를 보므로, validator가 통과해도 필수 변경이 있을 수 있습니다. validator 결과는
  `selected_only`로 함께 보고합니다.
- "snapshot validator"는 semantic snapshot 무결성 validator로 구현했습니다. 문서 validator(VI.10.1) 실행은
  candidate 단계(XIV.5.4)에서 합니다.

## 남은 위험 / 한계

- validator는 expression payload 안의 참조(token·component Property)를 해석하지 못합니다(형식 비의존). 그 무결성은
  dependency graph의 보수적 scan이 담당합니다.
