# 작업 보고서: I.2.15 보고서 validator

- WBS: I.2.15
- 작업명: `task_report/validate.rs` — 완료/차단 보고서 validator
- 상태: 완료 (`[x]`)
- 선행: I.2.13, I.2.14 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/task_report/validate.rs` (신규)
- `tools/xtask/src/task_report/mod.rs` (신규)
- `tools/xtask/src/lib.rs` (`pub mod task_report` 추가)
- `docs/task-reports/I.2.15.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.15 `[ ]` → `[x]`)

## 구현 요약

- `validate_report(content, kind, expected_id) -> Vec<ReportError>`.
- 완료/차단별 필수 heading 각 1회(`missing-heading`/`duplicate-heading`), WBS 일치(`missing-wbs`), 완료 보고서의 환경 증거 코드 존재(`missing-environment`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 정상 완료/차단 보고서 | 오류 0 |
| heading 제거 | `missing-heading` |
| WBS 제거 | `missing-wbs` |
| 환경 증거 제거 | `missing-environment` |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 57 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 명령 종료 코드·artifact locator 형식은 heading 존재 검사로 포괄하고, 핵심 실패 케이스(heading·WBS·환경 증거)를 코드로 분리했습니다.
