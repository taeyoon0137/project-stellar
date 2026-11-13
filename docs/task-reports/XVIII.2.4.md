# 작업 보고서: XVIII.2.4 child exit·signal·cancellation·app close 시 process tree 정리

- WBS: XVIII.2.4
- 작업명: child exit, signal, cancellation, app close 시 process tree 정리 구현
- 상태: 완료 (`[x]`)
- 선행: XVIII.2.1

## 필수 실행 환경

- `DEV-MAC` (macOS 실제 PTY·process tree, 로컬 검증).

## 변경 파일 목록

- `crates/terminal/src/pty/mod.rs` — 세션 전체 정리(비회수 종료 확인, 세션 열거, hangup→SIGKILL 단계), `KILL_WAIT`, `Drop`, master open 일시 오류 재시도, 정리 테스트.
- `crates/terminal/src/pty/sys.rs` — `waitid`·`getsid`·`proc_listpids` FFI, `SigInfo`, `P_PID`·`W*`·`CLD_*`·`PROC_ALL_PIDS` 상수와 고정 테스트.
- `crates/terminal/src/registry.rs` — `poll`(child 종료), `cancel`(취소), `close_all`(창·앱 종료), `Drop`, 실제 PTY 정리·누수 테스트.
- `crates/terminal/src/test_support.rs` — PTY 테스트 공유/배타 lock(tty fd 수 검사용).
- `docs/task-reports/XVIII.2.4.md`(신규), 계획서(XVIII.2.4 `[ ]`→`[x]`).

## 구현 요약

- **정리 대상 = 세션 전체**: leader process group만이 아니라 job control로 다른 group에 있는 job,
  `SIGHUP` 무시 프로세스, leader가 먼저 끝난 뒤 남은 background 프로세스까지 포함합니다.
- **pid 재사용 안전성**: leader 종료를 `waitid(P_PID, WEXITED|WNOHANG|WNOWAIT)`로 **회수 없이**
  확인합니다. 회수 전에는 leader pid가 예약되어 pgid·세션 id(= leader pid)가 다른 프로세스에 재사용될
  수 없으므로, group 신호와 세션 열거는 회수 전에만 합니다. 최종 종료 상태는 회수(`wait`) 결과입니다.
- **세션 열거**: `proc_listpids(PROC_ALL_PIDS)`(상한 2^18개) 중 `getsid(pid) == leader pid`인 것
  (이 프로세스·pid ≤ 1 제외). zombie는 `getsid`가 실패해 제외됩니다.
- **단계**: hangup(`SIGHUP`+`SIGCONT`, leader group과 다른 group 프로세스) → 유예(기본 1.5초) →
  남은 프로세스 `SIGKILL`(정리 중 새로 생긴 것도 반복) → 세션이 비면 leader 회수.
  `SIGKILL` 뒤 `KILL_WAIT`(2초) 안에도 남는 프로세스는 `remaining_processes`로 보고합니다.
- **경로별**: child 스스로 종료·외부 신호 종료 → `TerminalRegistry::poll`(비차단, 세션은
  `Finished{ChildExited}`로 남음). 취소 → `cancel`(`Finished{Cancelled}`). 닫기 → `close`. 창·앱
  종료 → `close_all`과 registry `Drop`(모든 세션에 먼저 hangup을 보낸 뒤 기다려 전체 시간이 세션
  하나의 상한 수준). 정리하지 않은 `PtyProcess`를 drop해도 `terminate`가 돕니다.
- **master open 재시도**: 테스트 반복 중 `posix_openpt`가 errno `-6`(XNU 내부 `EREDRIVEOPEN`)으로 한 번
  실패한 것을 실측했습니다. `EINTR`·`EBUSY`·`EAGAIN`·`-6`만 1 ms 간격 최대 8회 다시 시도합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-terminal` | 56 passed, 0 failed |
| `cargo test -p stellar-terminal` 30회 반복 | 실패 0, 반복 뒤 `pgrep -fl "sleep 300"` 결과 없음 |
| `cargo fmt -p stellar-terminal -- --check` | 통과 |
| `cargo clippy -p stellar-terminal --all-targets -- -D warnings` | 경고 0 |

| orphan process 0 검사(fixture: 같은 group job + `set -m` 다른 group job + `SIGHUP` 무시 job) | 결과 |
|---|---|
| child 종료: exit 0, 세 job 모두 사라짐, `remaining_processes` 0(`child_exit_cleans_up_whole_session`) | 통과 |
| 외부 신호: leader `SIGKILL` → `Signaled(9)`, 세 job 사라짐(`external_signal_to_leader_cleans_up_session`) | 통과 |
| 닫기·취소 경로 terminate: `SIGHUP` 무시 job 때문에 유예 뒤 `SIGKILL`까지 진행(경과 ≥ 유예, 상한 안), 남은 0(`terminate_kills_other_groups_and_sighup_ignoring_jobs`) | 통과 |
| handle drop(앱 종료 경로): tree 정리·EOF(`dropping_unfinished_process_terminates_tree`) | 통과 |
| registry: `poll`로 child 종료 `Finished{Exited(3), ChildExited, 0}`, `cancel` `Finished{Cancelled}`, registry drop으로 남은 2세션 정리(유예+`KILL_WAIT` 안), 모든 pid 사라짐(`registry_cleans_up_on_child_exit_cancel_and_app_close`) | 통과 |
| 20 PTY session create/start/close × 2회: 실행 중 tty fd = 기준 + 40(master+보관 slave), 닫은 뒤 기준과 같음, leader 모두 사라짐(`twenty_pty_sessions_create_and_close_without_leaks`, 배타 lock) | 통과 |
| tty fd 계측 자체(`tty_fd_count_sees_pty_fds`), FFI 상수·`siginfo_t` 104 byte 고정 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 스스로 `setsid()`로 새 세션을 만든 daemon은 터미널 세션 밖이므로 정리 대상이 아닙니다(의도된
  분리).
- 세션 열거와 신호 사이에 그 프로세스가 끝나고 pid가 재사용되는 경쟁은 이론상 남습니다(macOS는 pid를
  순차 할당하므로 수 μs 안의 재사용 가능성은 매우 낮음). leader group 신호는 회수 전이라 이 경쟁이
  없습니다.
- 앱이 비정상 종료(crash·`SIGKILL`)되면 `Drop`이 돌지 않습니다. 이때는 master가 닫히며 커널이
  세션에 hangup을 보내므로 `SIGHUP`을 무시하는 프로세스만 남을 수 있습니다. 복구 snapshot에 터미널을
  넣지 않고 재연결하지 않는 검사는 XVIII.4.6 범위입니다.
- `-6` 오류의 원인 설명은 XNU 공개 헤더의 커널 내부 errno 정의와 실측에 근거한 추정입니다.
