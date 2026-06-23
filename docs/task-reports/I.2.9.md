# 작업 보고서: I.2.9 상태별 증거 규칙 검증

- WBS: I.2.9
- 작업명: `validate_evidence` — 상태별 증거 규칙
- 상태: 완료 (`[x]`)
- 선행: I.2.7 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/task_status/validate_evidence.rs` (신규)
- `tools/xtask/src/task_status/mod.rs` (모듈 추가)
- `docs/task-reports/I.2.9.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.9 `[ ]` → `[x]`)

## 구현 요약

- `verified`: commit·report·verified_at + 모든 required environment evidence 필요(`missing-commit`/`missing-report`/`missing-verified-at`/`missing-environment-evidence`).
- `reported_done`: report 필요(`missing-report`).
- `not_started`: commit·report·evidence 없어야 함(`unexpected-*`).
- in_progress/blocked/cancelled: 추가 제약 없음.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 정상 verified | 오류 0 |
| 환경 1개 누락 | `missing-environment-evidence` |
| report 누락 | `missing-report` |
| not_started + evidence | `unexpected-evidence` |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 42 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- doc 주석의 리스트 뒤 연속 문장은 clippy `doc_lazy_continuation`를 피하려 빈 줄로 분리했습니다.
