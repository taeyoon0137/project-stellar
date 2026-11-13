# 작업 보고서: XXIX.1.4 ledger 허용 상태에서 `cancelled` 제거

- WBS: XXIX.1.4
- 작업명: `TaskState`의 cancelled variant, collection 검증 허용 목록, evidence 검증 분기를 삭제하고 `cancelled` record를 unknown status로 거부
- 상태: 완료 (`[x]`)
- 선행: 없음

## 필수 실행 환경

- `DEV-MAC` (순수 Rust xtask, 로컬 검증).

## 변경 파일 목록

- `tools/xtask/src/task_status/model.rs` (수정) — `TaskState::Cancelled`와 `cancelled` 토큰 삭제(`ALL` 6→5), 폐기 상태를 두지 않는다는 doc comment, round-trip test에 `cancelled` 거부 확인 추가.
- `tools/xtask/src/task_status/validate_collection.rs` (수정) — 모듈 문서의 허용 상태 목록에서 `cancelled` 삭제, cancelled record fixture test 추가.
- `tools/xtask/src/task_status/validate_evidence.rs` (수정) — 모듈 문서와 상태별 분기에서 `Cancelled` 삭제.
- `docs/task-reports/XXIX.1.4.md`(신규), 계획서(XXIX.1.4 `[ ]`→`[x]`).

## 구현 요약

- 제품 소유자 결정(2026-11-14, 계획서 1.1 상태 표기)에 따라 폐기 상태를 없앴습니다. 목적이 사라진 행은 계획서와 ledger에서 삭제합니다.
- decoder(`decode.rs`)는 `TaskState::from_token`이 `None`을 반환하면 이미 `unknown-status`로 거부하므로 decoder 코드는 바꾸지 않았습니다. variant와 토큰을 지우는 것만으로 `status: cancelled` record가 unknown status로 거부됩니다.
- 완료 행 I.2.1·I.2.7의 계획서 문구는 작업 지시대로 고치지 않았습니다.
- 현재 ledger(`planning/task-status.yaml`)에는 `status: cancelled` record가 0개입니다(`grep -c "status: cancelled"` 결과 0).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| 구현 전 `cargo test -p xtask task_status` (새 fixture test만 추가한 상태) | 2 failed(`task_state_token_round_trip`, `cancelled_record_rejected_as_unknown_status`), 45 passed — 요구사항 실패 확인 |
| `cargo test -p xtask` | lib 158 passed, 0 failed(통합 test 2+2 passed), 종료 코드 0 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0, 종료 코드 0 |
| `cargo fmt -p xtask -- --check` | 통과, 종료 코드 0 |
| `cargo xtask validate-task-status` (현재 ledger) | 종료 코드 0 |
| `cargo xtask sync-plan` | 종료 코드 0 |
| `cargo xtask validate-task-status <scratch>/cancelled-ledger.yaml` (`status: cancelled` record 하나) | 종료 코드 65, `decode/unknown-status` |
| 같은 fixture를 이 변경 전 xtask(`380caa5`, scratch 복사본 빌드)로 실행 | 종료 코드 0(cancelled 허용) |

| 검사 | 결과 |
|---|---|
| cancelled record fixture 거부(`cancelled_record_rejected_as_unknown_status`, `task_state_token_round_trip`) | 통과 |
| 현재 ledger 통과(`validate_versions_and_status_on_real_files`, `cargo xtask validate-task-status`) | 통과 |
| xtask fmt·clippy·test | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다. 이 작업은 ledger(`planning/task-status.yaml`)를 재생성하지 않았습니다.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 완료 행 I.2.7의 계획서 문구와 ledger title에는 허용 상태 목록에 `cancelled`가 그대로 남아 있습니다(작업 지시상 수정하지 않음). 현재 동작은 이 보고서와 계획서 1.1 상태 표기가 기준입니다.
