# 작업 보고서: II.1.6 cargo xtask check-core

- WBS: II.1.6
- 작업명: 전체 workspace fmt→clippy→test→doc-test 고정 순서 실행
- 상태: 완료 (`[x]`)
- 선행: II.1.2 (`[~]`), I.4.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (verify 열에 CI 태그 없음).

## 변경 파일 목록

- `tools/xtask/src/check_core.rs` (신규)
- `tools/xtask/src/commands.rs` (check-core 명령 연결, COMMANDS=12)
- `tools/xtask/src/lib.rs` (`pub mod check_core` 추가)
- `docs/task-reports/II.1.6.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: II.1.6 `[ ]` → `[x]`)

## 구현 요약

- 고정 순서: fmt → clippy → test → doc-test. 첫 실패에서 중단.
- 명령별 로그를 `target/xtask-logs/<step>.log`에 저장(target은 gitignore).
- 실패 코드: 검사 실패 65, 실행 불가 70. Phase I xtask-only 검사와 혼합하지 않음.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 고정 순서(fmt,clippy,test,doc-test) | 단위 테스트 통과 |
| 첫 실패 중단(clippy 실패 시 test·doc-test 미실행) | 단위 테스트 통과 |
| `cargo xtask check-core` 실제 | 종료 0, 4개 로그 생성 |
| `cargo fmt/clippy --locked/test --workspace` | fmt 0, clippy 0, test 98 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 실제 하위 명령 실패는 cargo 하위 프로세스 결과로 처리하며, "첫 실패 중단·정확한 실패 단계"는 주입형 runner(`run_steps`) 단위 테스트로 검증했습니다.
