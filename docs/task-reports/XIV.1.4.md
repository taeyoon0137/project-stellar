# 작업 보고서: XIV.1.4 Git 명령 취소와 프로세스 tree 종료

- WBS: XIV.1.4
- 작업명: Git command cancellation, child process와 자식 프로세스 tree의 플랫폼 안전 종료, partial 결과를 성공으로 처리하지 않음
- 상태: 완료 (`[x]`)
- 선행: IV.3.3(GitRunner)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 프로세스 tree fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/cancel.rs` (신규: `GitCancelToken`, `TERMINATE_GRACE`, 플랫폼별 `terminate_process_tree`, 테스트)
- `crates/git-porcelain/src/runner.rs` (`run_cancellable`, `GitRunError::Cancelled`, unix `process_group(0)`, timeout·취소·wait 오류 시 tree 종료)
- `crates/git-porcelain/src/trust.rs` (`GatedGitRunner::run_cancellable`: untrusted면 spawn 0)
- `crates/git-porcelain/src/test_support.rs` (`hermetic_runner_with_timeout` 추가)
- `crates/git-porcelain/src/lib.rs` (`pub mod cancel;`)
- `docs/task-reports/XIV.1.4.md`(신규), 계획서(XIV.1.4 `[ ]`→`[x]`)

## 구현 요약

- `GitCancelToken`(`Arc<AtomicBool>`, 사본 간 공유)으로 다른 스레드가 취소를 요청합니다.
  `GitRunner::run_cancellable`은 실행 전에 이미 취소되어 있으면 프로세스를 만들지 않고, 실행 중
  취소를 관찰하면 프로세스 tree를 종료한 뒤 `Err(GitRunError::Cancelled)`를 반환합니다. 부분
  stdout/stderr는 버리며 성공 결과로 돌려주지 않습니다. Git이 취소 관찰 전에 스스로 끝났으면 그
  결과를 그대로 반환합니다.
- unix: Git을 새 process group의 leader로 시작(`CommandExt::process_group(0)`)하고, 종료 시
  group 전체에 `SIGTERM`(Git의 lock 파일 정리 기회) → 최대 `TERMINATE_GRACE`(1.5초) 대기 →
  group 전체에 `SIGKILL`을 보냅니다. 신호 전송은 libc `kill(2)` FFI 선언 한 곳의 `unsafe`
  블록이며, `pgid > 1`일 때만 보내 전체 프로세스(-1)·자기 group(0)에는 보내지 않습니다. 새 외부
  dependency는 추가하지 않았습니다.
- Windows: `taskkill /PID <pid> /T /F`를 argv로 실행해 tree를 강제 종료합니다(아래 남은 위험 참고).
- 기존 timeout 경로와 `try_wait` 오류 경로도 같은 tree 종료를 씁니다. 그래서 손자 프로세스가
  stdout 파이프를 잡고 있어도 drain 스레드가 손자 종료(예: 30초)를 기다리지 않습니다.
- `GatedGitRunner::run_cancellable`은 trust gate를 먼저 통과해야 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 69 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check -p stellar-project --all-targets` / `cargo test -p stellar-project` (하위 사용 crate) | exit 0 / 116 passed |

| 검사 | 결과 |
|---|---|
| **long-running fixture**: `git -c alias.longrun=!<script>`가 script → `sleep`, 중첩 `sh` → `sleep` tree(pid 4개 기록)를 만든 뒤 다른 스레드에서 취소 → `Err(Cancelled)`(부분 출력이 있어도 성공 아님), 10초 이내 반환, **pid 4개 모두 잔존 0** | 통과 |
| 같은 fixture + timeout 1초 → `Err(Timeout)`, 10초 이내 반환(손자 대기 없음), pid 4개 잔존 0 | 통과 |
| 대조 실행(임시로 직접 자식만 종료하도록 바꾼 뒤 위 취소 테스트 실행, 확인 후 원복) → 30.10초 지연 후 실패 | 대조군 실패 확인 |
| 실행 전 취소된 token → `Err(Cancelled)`, spawn 0 | 통과 |
| untrusted gate의 `run_cancellable` → `ProjectNotTrusted`, spawn 0 | 통과 |
| 취소되지 않은 `run_cancellable`은 정상 결과 반환 | 통과 |

## 차이(작업 지시와 다른 점)

- 취소 관찰 전에 Git이 스스로 끝난 경우에는 실제 결과를 반환합니다. Git이 이미 저장소를 바꿨는데
  취소로 보고하면 상태를 잘못 안내하게 되기 때문입니다. `Cancelled`를 받은 호출자는 저장소 상태를
  다시 조회해야 한다고 문서화했습니다.

## 남은 위험 / 한계

- Windows tree 종료(`taskkill /T /F`)는 이 환경에서 컴파일·실행하지 않았습니다(Windows target
  미설치, 필수 환경은 `DEV-MAC`). `/F` 강제 종료라 Git이 `index.lock`을 정리하지 못할 수 있습니다.
  Windows 검증과 Job Object 방식 검토는 후속 Windows 검증 단계에서 필요합니다.
- unix에서 자식이 스스로 새 session/process group을 만들면(`setsid` 등) group 신호 범위를 벗어납니다.
- `SIGTERM`이 Git의 lock 파일을 정리하는지는 Git 동작에 의존하며 이 작업에서 별도 fixture로
  검증하지 않았습니다.
