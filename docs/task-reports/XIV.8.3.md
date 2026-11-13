# 작업 보고서: XIV.8.3 commit identity 누락 구조화

- WBS: XIV.8.3
- 작업명: user.name/email 미설정 오류를 구조화해 UI가 안내할 수 있게 함, Stellar는 전역 config를 자동 수정하지 않음
- 상태: 완료 (`[x]`)
- 선행: IV.4.5(identity 검사)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 실제 git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/commit.rs` (`IdentityKey`, `MissingIdentity`, `CommitError::MissingIdentity`, `check_commit_identity`, `commit_with_message`에 커밋 전 identity 확인 추가, 테스트)
- `docs/task-reports/XIV.8.3.md`(신규), 계획서(XIV.8.3 `[ ]`→`[x]`)

## 구현 요약

- `check_commit_identity(gate, worktree)`는 trust gate를 거쳐 `git config --get user.name`,
  `git config --get user.email`을 읽기만 합니다. 종료 코드 0 + 비어 있지 않은 값이면 있음, 종료
  코드 1(key 없음)이면 누락입니다. 그 밖의 비영 종료(손상된 config 등)는 누락으로 숨기지 않고
  `CommitError::Git`으로 결과를 보존해 반환합니다(IV.4.5 검사보다 엄격).
- 누락이면 `MissingIdentity { name_missing, email_missing }`를 반환합니다. UI는
  `missing_keys()`(`IdentityKey::UserName`/`UserEmail`, `config_key()` = `user.name`/`user.email`)로
  안내 문구를 만들 수 있습니다. `Display`는 누락 key를 포함합니다.
- `commit_with_message`는 NUL 검사 → identity 확인 → 누락 시 `git commit`을 실행하지 않고
  `CommitError::MissingIdentity` 반환 → 있으면 stdin commit(XIV.8.2) 순서로 동작합니다.
- Stellar는 global·local Git config를 쓰지 않습니다(`git config` 쓰기 명령을 실행하는 경로 없음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 88 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check -p stellar-project --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| **missing identity fixture**(global config 임시 파일에 user 없음, local 미설정): `MissingIdentity{name, email}`, `missing_keys` = [user.name, user.email], 메시지에 두 key 포함 | 통과 |
| 같은 fixture: config 읽기 2회만 실행(spawn 2), `git commit` 미실행 → HEAD 없음 | 통과 |
| 같은 fixture: **global config 파일·`.git/config` 바이트가 실행 전후 동일**(자동 수정 없음) | 통과 |
| global에 name만 있음 → `MissingIdentity{name_missing: false, email_missing: true}`, commit 미실행 | 통과 |
| global에 name·email 있음(local 없음) → 확인 통과, 커밋 author가 global identity | 통과 |
| 손상된 global config → `CommitError::Git`(exit 0/1 아님), 누락으로 숨기지 않음 | 통과 |
| untrusted identity 확인 → `ProjectNotTrusted`, spawn 0 | 통과 |
| XIV.8.2 기존 commit 테스트(identity 설정됨) 그대로 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- IV.4.5의 `repo::check_user_identity`(모든 비영 종료를 누락으로 처리, gate 없음)를 재사용하지 않고
  gate를 거치는 엄격한 확인을 commit 모듈에 두었습니다. 기존 함수는 IV.4 생성 흐름용으로 그대로
  둡니다.

## 남은 위험 / 한계

- `GIT_AUTHOR_NAME` 같은 환경 변수 identity는 `GitRunner`가 환경을 비우므로(allowlist 없으면) 반영하지
  않습니다. Git의 hostname 기반 자동 identity 추정도 사용하지 않고 config 설정을 요구합니다.
- UI 안내 화면 연결과 FFI 노출(XIV.13.3)은 후속 작업입니다.
