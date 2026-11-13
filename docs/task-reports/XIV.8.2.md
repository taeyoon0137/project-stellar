# 작업 보고서: XIV.8.2 commit message stdin 전달

- WBS: XIV.8.2
- 작업명: commit message를 temp file 또는 stdin으로 시스템 Git에 전달하고 shell escaping에 의존하지 않음
- 상태: 완료 (`[x]`)
- 선행: IV.3.3(GitRunner)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 실제 git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/commit.rs` (신규: `commit_with_message`, `COMMIT_FROM_STDIN_ARGS`, `CommitError`, 테스트)
- `crates/git-porcelain/src/runner.rs` (`GitRunner::run_with_input`: stdin 바이트 전달, 별도 writer 스레드, 입력 없을 때는 기존처럼 stdin null; stdin 전달 테스트)
- `crates/git-porcelain/src/trust.rs` (`GatedGitRunner::run_with_input`: untrusted면 spawn 0)
- `crates/git-porcelain/src/lib.rs` (`pub mod commit;`)
- `docs/task-reports/XIV.8.2.md`(신규), 계획서(XIV.8.2 `[ ]`→`[x]`)

## 구현 요약

- `commit_with_message(gate, worktree, message)`는 `git commit --file=- --cleanup=verbatim`을
  실행하고 message 바이트를 stdin으로 전달합니다. message는 argv에 들어가지 않으므로 quote·escape가
  필요 없고, shell을 거치지 않습니다(`GitRunner`는 원래 shell을 쓰지 않음).
- `--cleanup=verbatim`을 명시해 사용자 config(`commit.cleanup`)와 무관하게 message를 바꾸지
  않습니다. `#`로 시작하는 줄, 들여쓰기, 끝 개행 유무가 그대로 저장됩니다. hooks는 Git이 그대로
  실행하며 `--no-verify` 같은 우회를 쓰지 않습니다.
- NUL 문자가 든 message는 Git commit message로 표현할 수 없으므로 Git 실행 전에
  `CommitError::MessageContainsNul`로 거부합니다(spawn 0). Git 비영 종료는 `CommitError::Git`으로
  결과를 보존해 반환합니다.
- `GitRunner::run_with_input`은 stdin을 pipe로 열고 별도 스레드에서 쓴 뒤 닫습니다. 자식이 출력하며
  읽는 경우의 교착을 막고, 취소·timeout 시 tree 종료(XIV.1.4) 후 writer 스레드도 회수합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 83 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check -p stellar-project --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| **multiline/unicode message fixture**(한글·일본어·이모지·결합 문자, 빈 줄, `#` 줄, `--amend`로 시작하는 줄, 들여쓰기·따옴표) → `git cat-file commit HEAD`의 message 바이트가 입력과 정확히 일치 | 통과 |
| 끝 개행 없는 unicode 한 줄 message → 저장 바이트 정확히 일치(개행 추가 없음) | 통과 |
| shell metacharacter(`$(...)`, backtick, `;`, `&&`, `|`) message → 그대로 저장, marker 파일 생성 0 | 통과 |
| `run_with_input`이 metacharacter 포함 바이트를 `/bin/cat` stdin에 그대로 전달, 파일 생성 0 | 통과 |
| staged 변경 없는 commit → `CommitError::Git`(비영 종료 결과 보존) | 통과 |
| NUL 포함 message → `MessageContainsNul`, spawn 0 / untrusted → `ProjectNotTrusted`, spawn 0 | 통과 |

## 차이(작업 지시와 다른 점)

- temp file 대신 stdin(`--file=-`)을 선택했습니다. 임시 파일 생성·정리·권한 문제가 없고 message가
  디스크에 남지 않기 때문입니다.
- cleanup 모드를 `verbatim`으로 고정했습니다. Git 기본값(`whitespace`/`strip`)은 사용자 입력을
  바꿀 수 있어 "입력한 그대로 커밋" 계약과 맞지 않습니다.

## 남은 위험 / 한계

- 기존 IV.4.6 최초 커밋(`repo::git_commit`)은 고정 message를 argv(`-m`)로 전달하며 이 작업에서
  바꾸지 않았습니다(고정 ASCII 문자열이라 escaping 위험 없음).
- identity 누락 구조화(XIV.8.3), commit 전 staged snapshot 재검증(XIV.8.1), commit 후 재조회
  (XIV.8.4), hook 정책(XIV.8.5)은 후속 작업입니다.
