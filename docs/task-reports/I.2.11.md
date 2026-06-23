# 작업 보고서: I.2.11 결정론적 ledger writer

- WBS: I.2.11
- 작업명: `task_status/write.rs` — deterministic ledger writer
- 상태: 완료 (`[x]`)
- 선행: I.2.5 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/task_status/write.rs` (신규)
- `tools/xtask/src/task_status/mod.rs` (모듈 추가)
- `docs/task-reports/I.2.11.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.11 `[ ]` → `[x]`)

## 구현 요약

- `write_ledger(&[TaskStatusRecord]) -> String`. 고정 key 순서(id, title, status, owner, depends_on, required_environments, evidence, commit, report, verified_at), 2-space indent, LF, 끝 newline, task ID(WbsId) 정렬, 주석 없음.
- scalar 값은 double-quote + `\`/`"` escape. None/빈 컬렉션은 생략(round-trip 안정).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 정렬·LF·끝 newline·CR 없음 | 통과 |
| 동일 collection 100회 출력 byte-identical | 통과 |
| parse→write→parse 의미 동일 | 통과 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 49 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
