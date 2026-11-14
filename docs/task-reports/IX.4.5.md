# 작업 보고서: IX.4.5 승인된 DrillingPlan의 단일 transaction 적용

- WBS: IX.4.5
- 작업명: 승인된 DrillingPlan을 하나의 CommandTransaction으로 적용, 중간 실패 시 property 하나도 남기지 않음
- 상태: 완료 (`[x]`)
- 선행: IX.4.4(이 lane에서 `[x]`), VII.1.6(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/drilling_apply.rs`(신규) — `DrillingCommand`(Add/Remove, inverse 전용 Restore),
  `ComponentCommand`(Property·Drilling closed enum), `ComponentTransaction`(all-or-nothing, inverse 역순 롤백),
  `TransactionError`, `ComponentCommandError`, `apply_drilling_plan`, `AppliedDrilling`, `DrillingApplyError`.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.4.5.md`(신규), 계획서(IX.4.5 `[ ]`→`[x]`).

## 구현 요약

- **transaction**: `ComponentTransaction::apply`는 command를 순서대로 적용하며 inverse를 모읍니다. 하나라도 실패하면
  이미 적용한 command의 inverse를 역순으로 적용해 되돌리고 `TransactionError { index, error, rollback }`을 반환합니다
  (`rollback`은 롤백 자체 실패 시에만 `Some`). 성공 시 inverse 역순 transaction(undo)을 돌려줍니다. 상태 전체 복제
  없이 inverse로 롤백합니다.
- **계획 적용**: 적용 직전에 같은 요청으로 계획을 다시 계산해 승인한 계획과 다르면 `StalePlan`(아무것도 하지 않음).
  새 property(제안 이름·타입·default 리터럴/null AST, 공개)를 모두 Create한 뒤 link를 Add합니다(IX.4.1 의미 검증).
  새 StableId는 호출자 id 원천에서 발급합니다. 결과는 노출 property, 만든 property·link, undo transaction입니다.
- **fault injection**: 내부 `apply_with_probe`가 command마다 probe를 호출해 실패를 주입할 수 있습니다(공개 API는 항상
  성공 probe).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 336 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 1·2·10단계 계획 적용: property·link 수, 노출 property 이름·default·공개, 모든 link 검증 통과, library 저장 검증 통과, 재계획 no-op, 원래 계획 재적용은 `StalePlan`, undo 한 번에 bytes 정확 복원, redo 재현(`approved_plan_applies_as_one_transaction_and_undoes_exactly`) | 통과 |
| 3단계 계획(command 6개)의 모든 위치(0~5)에 실패 주입 → 매번 library·drilling bytes 적용 전과 동일, 어느 컴포넌트에도 새 property 없음, 롤백 오류 없음(`fault_injection_at_every_step_rolls_back_everything`) | 통과 |
| 실제 오류(두 번째 Create의 id 충돌)도 첫 Create까지 롤백(`real_mid_transaction_failure_also_rolls_back`) | 통과 |
| DrillingCommand Add/Remove/Restore apply·inverse, 검증 실패 Add 거부(`drilling_commands_apply_and_invert`) | 통과 |

## 차이(작업 지시와 다른 점)

- `stellar-commands` `CommandTransaction`(VII.1.6)은 scene `DocumentModel` 대상이고 `crates/commands`는 이 lane의 허용
  경로가 아니라, 같은 all-or-nothing 계약의 `ComponentTransaction`을 document 컴포넌트 계층에 구현했습니다. command
  계층 등록·history 연동은 후속입니다.

## 남은 위험 / 한계

- 롤백 중 inverse가 실패하는 경우(정상 경로에서는 발생하지 않음)는 `TransactionError::rollback`으로 알리지만 상태 복구를
  보장하지 않습니다.

## 후속 수정 (리뷰 반영)

### inverse 전용 `DrillingCommand::Restore`의 공개 생성 차단

`Fix: DrillingCommand::Restore를 Remove inverse로만 만들 수 있게 제한` 커밋에서 고쳤습니다.

- **문제**: `Restore { binding, orphan }`은 inverse 전용이라 검사 없이 binding을 넣고 orphan 표시를 붙이는데, 공개 enum
  variant라 crate 밖에서 임의 binding·orphan 정보로 만들 수 있었습니다. `Add`가 강제하는 IX.4.1 의미 검증과 crate
  내부 전용 `mark_orphan`을 우회하는 공개 경로였습니다.
- **변경**: variant를 `Restore(RemovedBinding)`으로 바꿨습니다. `RemovedBinding`(지운 binding + 그때의 orphan 정보)은
  필드가 비공개이고 `Remove` 적용 결과로만 만들어집니다. 읽기 accessor(`binding`, `orphan`)는 공개입니다. `Restore`
  적용 시 검증을 추가하지 않은 이유: 롤백·undo 경로에서 `Restore`가 실패하면 상태 복구가 깨집니다. 모든 library
  변경이 drilling 모음 검증을 유지한다는 보장이 이 계층에는 없습니다(아래 `RestoreRecord` 참고). 그래서 지우기 전
  live binding이 되살릴 때도 의미 검증을 통과한다고 가정할 수 없습니다.
- **검증**: `DrillingCommand` 문서에 crate 밖 위조를 시도하는 `compile_fail` doctest를 먼저 추가했고, 수정 전에는
  "Test compiled successfully, but it's marked `compile_fail`"로 실패했습니다. 수정 후에는 새 형태로 위조를
  시도하는 doctest가 컴파일되지 않습니다. 원인이 비공개 필드(E0451)인지는 표시를 잠시 빼고 실행해 확인했습니다. 읽기용
  doctest와 `drilling_commands_apply_and_invert`(Remove inverse가 지운 binding·orphan 정보를 그대로 담음)는 통과합니다.

| 명령(후속 수정) | 결과 |
|---|---|
| `cargo test -p stellar-document --doc`(수정 전) | 1 failed(위조 snippet이 컴파일됨) |
| `cargo test -p stellar-document` | unit 353 passed, 통합 7+1+1, doc 4 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

### `CommandTransaction` 미연동 (사실 기록, 코드 변경 없음)

- 이 행은 "하나의 CommandTransaction으로 적용"을 요구합니다. 실제 적용 단위는 document 컴포넌트 계층의
  `ComponentTransaction`입니다. `stellar-commands`의 `CommandTransaction`(VII.1.6)은 `DocumentModel`(scene) 대상
  closed enum만 담고, `DocumentModel`은 `ComponentLibrary`·`DrillingSet`을 소유하지 않습니다. 그래서 drilling 적용은
  앱 history(VII.4.1 `HistoryEntry`)에 들어가지 않고 앱 Undo/Redo로 되돌릴 수 없습니다.
- 행의 검증 열("fault injection rollback")은 실제 테스트로 충족했고, 전부 적용 또는 전부 롤백이라는 계약도
  `ComponentTransaction`이 지킵니다. 따라서 상태 `[x]`는 유지합니다. 연동(문서 모델의 component library 소유 +
  `stellar-commands` closed enum 등록 + history 연결)을 맡는 WBS 행은 계획서에 아직 없습니다. 계획 변경으로 행을
  추가해야 합니다.
- 관련 위험: `PropertyCommand::RestoreRecord`(IX.4.6, inverse 전용)도 공개 variant입니다. 같은 id·이름 검사와 revive
  검사는 하지만, 임의 record로 타입·공개 여부를 바꿔도 끊긴 binding을 orphan으로 표시하지 않습니다. 이번 리뷰 지적
  범위 밖이라 고치지 않았습니다.
