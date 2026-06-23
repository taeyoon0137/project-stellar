# 작업 보고서: I.2.6 단일 레코드 검증

- WBS: I.2.6
- 작업명: `validate_record` — 단일 레코드 필수값·nullable·형식 규칙
- 상태: 완료 (`[x]`)
- 선행: I.2.5 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/task_status/validate_record.rs` (신규)
- `tools/xtask/src/task_status/mod.rs` (모듈 추가)
- `docs/task-reports/I.2.6.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.6 `[ ]` → `[x]`)

## 구현 요약

- `validate_record(&TaskStatusRecord) -> Vec<RecordError>` (빈 Vec = 통과).
- 검사·코드: `empty-title`, `empty-required-environments`, `duplicate-environment`, `invalid-report-path`(절대경로·`..`·빈 segment 거부), `invalid-commit-sha`(7~40자 16진수).
- 다른 레코드·상태별 증거·graph는 검사하지 않음.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 정상 레코드 | 오류 0 |
| table-driven: 각 필드 오류 1개 fixture | 해당 코드 반환 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 26 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
