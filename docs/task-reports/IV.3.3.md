# 작업 보고서: IV.3.3 보안 GitRunner

- WBS: IV.3.3
- 작업명: argv·cwd·env allowlist·timeout·byte capture를 쓰는 보안 `GitRunner`
- 상태: 완료 (`[x]`)
- 선행: IV.3.1(Git 탐색)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; 보안/타임아웃 테스트는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/runner.rs` (신규: `GitRunner`, `GitOutput`, `GitRunError`, `argv`, 보안 테스트)
- `crates/git-porcelain/src/lib.rs` (`pub mod runner;`)
- `docs/task-reports/IV.3.3.md`(신규), 계획서(IV.3.3 `[ ]`→`[x]`)

## 구현 요약

- `GitRunner::run(args, cwd)`는 `Command::new(git).args(args)`로 **argv 배열**을 전달하고
  **shell을 거치지 않습니다**. cwd를 명시하고, `env_clear` 후 **allowlist 변수만** 전달합니다.
- **timeout**: 별도 스레드로 stdout/stderr를 동시에 drain(파이프 버퍼 차단 방지)하면서
  `try_wait` 폴링으로 마감 시한을 검사하고, 초과 시 자식을 kill 합니다.
- **byte capture**: stdout/stderr를 원시 `Vec<u8>`로 캡처합니다(문자열 가정 없음).
- 실패 분류: `Spawn{kind}`(실행 불가), `Timeout`, `Wait{kind}`. 비영 종료는 `GitOutput`으로
  반환하며 자동 성공으로 위장하지 않습니다.

## 적대적/보안 검증(ultracode)

- **shell metacharacter 미해석**: `; touch`, `&& touch`, `$(touch ...)`, `| touch`,
  `` `touch ...` ``, `> file`을 인자로 넘겨도 어떤 주입 파일도 생성되지 않음을 확인(argv라
  리터럴 인자).
- **env allowlist**: 허용 변수만 자식에 노출되고 비허용(비밀) 변수는 노출되지 않음을
  `/usr/bin/env` 출력으로 확인.
- **timeout**: 10초 `sleep`을 100ms timeout으로 종료(2초 내 반환)함을 확인.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (17 unit) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| stdout 캡처·성공 | 통과 |
| **shell metacharacter 미해석(주입 파일 0)** | 통과 |
| **env allowlist 필터(비밀 변수 미노출)** | 통과 |
| **timeout으로 장기 실행 자식 kill** | 통과 |
| 비영 종료 보존(자동 성공 위장 없음) | 통과 |
| cwd 반영, 실행 불가 → `Spawn` | 통과 |

## 차이(작업 지시와 다른 점)

- timeout은 외부 crate 없이 std(스레드 drain + `try_wait` 폴링 + `kill`)로 구현했습니다.
- 보안/타임아웃/env 테스트는 `/bin/echo`·`/usr/bin/env`·`/bin/sleep`·`/usr/bin/false` 등 표준
  unix 실행파일로 러너 메커니즘을 검증합니다(`cfg(unix)`).

## 남은 위험 / 한계

- `try_wait` 폴링 간격은 5ms입니다. 매우 짧은 timeout에서는 최대 ~5ms 오차가 있을 수 있습니다.
- 비영 종료를 보존하는 `GitCommandResult` 타입화는 IV.3.4에서 정식 정의합니다(현재 `GitOutput`이
  코드·stdout·stderr를 보존).
