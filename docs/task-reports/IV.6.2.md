# 작업 보고서: IV.6.2 git clone + temp rename

- WBS: IV.6.2
- 작업명: 임시 폴더 clone 후 성공 시 최종 폴더 rename(실패 시 최종 폴더 부재)
- 상태: 완료 (`[x]`)
- 선행: IV.6.1(clone request)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 git, 로컬 검증; clone 테스트는 `cfg(unix)` + 로컬 repo).

## 변경 파일 목록

- `crates/git-porcelain/src/repo.rs` (`git_clone` 추가)
- `crates/project/src/clone.rs` (`clone_project`, `CloneError`, clone 테스트)
- `docs/task-reports/IV.6.2.md`(신규), 계획서(IV.6.2 `[ ]`→`[x]`)

## 구현 요약

- `git_clone(runner, url, dest)`: `git clone -- <url> <dest>`(argv, shell 없음). `--`로 URL이
  플래그로 해석되지 않게 합니다.
- `clone_project(request, runner)`: 부모 아래 **임시 폴더**로 clone하고, **성공하면 최종 폴더로
  rename**합니다. **실패·취소 시** 임시 폴더를 제거하므로 **최종 폴더는 존재하지 않습니다.**
  최종 경로가 이미 있으면 `TargetExists`로 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (전체) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| **로컬 repo clone 성공 → 최종 폴더 rename**(.git·파일 존재, 임시 폴더 잔여 없음) | 통과 |
| **clone 실패(없는 source) → 최종 폴더 부재**, 임시 폴더 정리(부모 비어 있음) | 통과 |

## 차이(작업 지시와 다른 점)

- 테스트는 네트워크 없이 **로컬 git repo**를 source로 clone합니다. 임시 폴더는 부모 아래 고유
  이름으로 두어 rename이 동일 파일시스템에서 일어나게 합니다.
- clone 시 `GIT_TERMINAL_PROMPT=0`으로 인증 프롬프트 hang을 방지합니다(hermetic runner).

## 남은 위험 / 한계

- clone 결과의 `stellar.yaml` 부재 거부와 `ProjectProbe` 통과 요구는 IV.6.3~IV.6.4에서
  추가합니다.
