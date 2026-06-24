# 작업 보고서: I.4.2 verify-task

- WBS: I.4.2
- 작업명: `cargo xtask verify-task <ID>` 구현
- 상태: 완료 (`[x]`)
- 선행: I.4.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/task_status/verify.rs` (신규)
- `tools/xtask/src/task_status/mod.rs` (모듈 추가)
- `tools/xtask/src/commands.rs` (verify-task 명령 연결)
- `docs/task-reports/I.4.2.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.4.2 `[ ]` → `[x]`)

## 구현 요약

- `verify_task(records, id, report_exists, workspace_ready)`: 선행 verified 여부, validate_record·validate_evidence, report 존재, II.1.2 이전 조기 workspace build 요구를 검사.
- 코드: `unknown-task`, `missing-dependency`, record/evidence 코드, `missing-report`, `premature-workspace-requirement`.
- 명령 계층: report 존재(파일), workspace_ready(II.1.2 verified 여부) 판정.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| success / missing-dependency / missing-report / missing-CI-WIN-evidence / premature-workspace-requirement | 각 코드 |
| `cargo xtask verify-task I.1.1` (실제 ledger) | 종료 0 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 91 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- product workspace build 결과는 II.1.2 verified 이전에는 요구하지 않으며, 조기 요구는 `premature-workspace-requirement`로 보고합니다.
