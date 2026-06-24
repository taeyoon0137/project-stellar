# 작업 보고서: I.4.3 sync-plan --check

- WBS: I.4.3
- 작업명: `cargo xtask sync-plan --check` 구현
- 상태: 완료 (`[x]`)
- 선행: I.2.10, I.2.12, I.4.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/sync.rs` (신규)
- `tools/xtask/src/commands.rs` (sync-plan 명령 연결, COMMANDS=11)
- `tools/xtask/src/lib.rs` (`pub mod sync` 추가)
- `docs/task-reports/I.4.3.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.4.3 `[ ]` → `[x]`)

## 구현 요약

- `sync_plan(plan_markdown, ledger)`: 누락(`missing-in-ledger`), 추가(`extra-in-ledger`), 중복(`duplicate-id`), `title-mismatch`, `depends-on-mismatch` 보고. 파일 미수정.
- status는 비교 대상 아님(작업 진행에 따라 변하고 ledger는 주기적 재생성).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 일치 계획/ledger | drift 0 |
| missing / extra / title / depends_on / duplicate fixture | 각 코드 |
| 실제 계획서 vs 실제 ledger | drift 0, `cargo xtask sync-plan --check` 종료 0 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 95 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
