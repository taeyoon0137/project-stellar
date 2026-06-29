# 작업 보고서: IV.4.5 git add --all + identity 검사

- WBS: IV.4.5
- 작업명: `git add --all` 후 user.name/email 검사, 누락 시 파일 보존·커밋 중단 상태
- 상태: 완료 (`[x]`)
- 선행: IV.4.4(git init)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 git, 로컬 검증; identity 테스트는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/repo.rs` (`git_add_all`, `check_user_identity`, `stage_and_check_identity`, `IdentityStatus`, `CommitReadiness`, 테스트)
- `crates/git-porcelain/src/runner.rs` (`GitRunner::with_env_overrides` 추가 + 테스트)
- `docs/task-reports/IV.4.5.md`(신규), 계획서(IV.4.5 `[ ]`→`[x]`)

## 구현 요약

- `git_add_all`: `git add --all`(argv)로 모든 변경을 stage.
- `check_user_identity`: `git config user.name`/`user.email`(유효 config) 존재를 검사
  (`IdentityStatus::Present` 또는 `Missing{name_missing, email_missing}`). 키 없음(코드 1)은
  오류가 아니라 누락으로 봅니다.
- `stage_and_check_identity`: add 후 identity 검사. 누락이면 **파일을 그대로 두고**(롤백 없이)
  `CommitReadiness::HaltedMissingIdentity`를 반환합니다. 커밋(IV.4.6)은 `Ready`일 때만 진행.
- 보조 변경: `GitRunner::with_env_overrides`를 추가해 자식에 명시적 env(`GIT_CONFIG_GLOBAL`
  등)를 설정할 수 있게 했습니다(`env_clear` 보안 유지). identity 테스트를 머신 global config와
  무관하게(hermetic) 수행하는 데 사용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (43 unit) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| fixture | 결과 |
|---|---|
| **identity 있음**(로컬 config) → `Ready`, 파일 보존 | 통과 |
| **identity 없음**(hermetic, 전역/시스템 config 비움) → `HaltedMissingIdentity`, 파일 보존 | 통과 |
| `with_env_overrides`가 자식 env에 적용됨 | 통과 |

## 차이(작업 지시와 다른 점)

- identity 검사를 머신 config에 독립적으로 검증하기 위해 `GitRunner`에 명시적 env override를
  추가했습니다(IV.3.3 runner의 추가 기능). `env_clear` 기반 보안은 유지됩니다.
- identity 검사는 유효 config(local+global+system)를 봅니다. 전역 identity가 있는 사용자는
  로컬 설정 없이도 `Present`로 판정됩니다(실사용에 부합).

## 남은 위험 / 한계

- 실제 최초 커밋과 메시지 검증은 IV.4.6, 단계별 rollback 정책은 IV.4.7에서 다룹니다.
