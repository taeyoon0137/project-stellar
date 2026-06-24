# 작업 보고서: I.2.16 PR 템플릿과 validator

- WBS: I.2.16
- 작업명: `.github/pull_request_template.md` + `pr_body` validator
- 상태: 완료 (`[x]`)
- 선행: I.2.15 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `.github/pull_request_template.md` (신규)
- `tools/xtask/src/pr_body.rs` (신규)
- `tools/xtask/src/lib.rs` (`pub mod pr_body` 추가)
- `docs/task-reports/I.2.16.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.16 `[ ]` → `[x]`)

## 구현 요약

- 템플릿 섹션: WBS, 변경 경로, 명령과 환경 증거, 영향(파일 포맷·성능·보안), 보고서.
- `validate_pr_body(body) -> Vec<PrBodyError>`: `missing-wbs`, `missing-evidence`(환경 코드), `missing-report`(docs/task-reports 링크).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 정상 body 2개 | 통과 |
| WBS 누락 / 증거 누락 / 보고서 누락 | 각 코드 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 61 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
