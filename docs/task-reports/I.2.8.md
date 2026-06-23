# 작업 보고서: I.2.8 선행 graph 검증

- WBS: I.2.8
- 작업명: `validate_graph` — depends_on 존재·self·중복 edge·cycle
- 상태: 완료 (`[x]`)
- 선행: I.2.7 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/task_status/validate_graph.rs` (신규)
- `tools/xtask/src/task_status/mod.rs` (모듈 추가)
- `docs/task-reports/I.2.8.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.8 `[ ]` → `[x]`)

## 구현 요약

- 코드: `self-dependency`, `duplicate-edge`, `invalid-dependency`(WbsId 아님; 범위·Phase·Gate 문자열), `missing-dependency`(존재하지 않는 ID), `cycle`(DFS 3-color back edge).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| DAG | 오류 0 |
| missing / self / duplicate-edge / invalid(range·gate) | 각 코드 |
| 2-node / 10-node cycle | `cycle` |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 37 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
