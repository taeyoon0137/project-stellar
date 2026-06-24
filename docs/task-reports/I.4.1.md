# 작업 보고서: I.4.1 명령 registry

- WBS: I.4.1
- 작업명: `help` router를 command registry로 확장 + validate-* 명령 연결
- 상태: 완료 (`[x]`)
- 선행: I.2.12, I.2.15, I.2.16, I.2.17, I.3.3, I.3.5 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/commands.rs` (신규, dispatch + 핸들러)
- `tools/xtask/src/main.rs` (commands::run에 위임)
- `tools/xtask/src/lib.rs` (`pub mod commands` 추가)
- `docs/task-reports/I.4.1.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.4.1 `[ ]` → `[x]`)

## 구현 요약

- 명령: help, generate-ledger, validate-task-status, validate-task-report, validate-pr-body, validate-commit-message, validate-versions, validate-dependencies, validate-adr.
- 종료 코드: 성공 0, 입력 오류 64, 정책 위반 65, 내부 오류 70.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| help에 각 명령 1회 | 통과 |
| validate-commit-message 정상/위반/인자없음 | 0 / 65 / 64 |
| validate-adr 보호경로/문서 | 65 / 0 |
| 알 수 없는 명령·빈 인자 | 64 |
| 실제 validate-versions / validate-task-status / validate-dependencies | 종료 0 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 86 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점 / 후속

- 기존 task report(I.1.x~I.3.x)는 I.2.13 템플릿 도입 이전 형식이라 `validate-task-report`의 캐노니컬 heading과 다릅니다. I.4.2(verify-task)는 report 존재를 기준으로 확인하며, 보고서 형식 소급 정렬은 별도 후속으로 둡니다.
