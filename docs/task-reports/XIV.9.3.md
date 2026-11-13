# 작업 보고서: XIV.9.3 최초 연결 전 `git ls-remote` refs 확인

- WBS: XIV.9.3
- 작업명: 새 GitHub remote 연결 전에 `git ls-remote`로 refs 존재 여부를 확인하고, refs가 하나라도 있으면 최초 업로드 연결을 거부하고 Clone 안내 상태 반환
- 상태: 완료 (`[x]`)
- 선행: XIV.9.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 bare 저장소만 사용; 네트워크 미사용).

## 변경 파일 목록

- `crates/git-porcelain/src/remote/connect.rs` (신규: `check_initial_github_connection`, `probe_remote_refs`, `count_ls_remote_refs`, `InitialConnectionCheck`, `RemoteRefsState`, `ConnectError`, 테스트)
- `crates/git-porcelain/src/remote/mod.rs` (`connect` 모듈, 자격 증명을 가린 `RemoteError`, `redacted_failure`)
- `crates/git-porcelain/src/test_support.rs` (`github_redirect_runner`: GitHub URL → 로컬 bare `insteadOf` + `GIT_ALLOW_PROTOCOL=file`, `init_bare`)
- `docs/task-reports/XIV.9.3.md`(신규), 계획서(XIV.9.3 `[ ]`→`[x]`)

## 구현 요약

- `check_initial_github_connection`: URL을 먼저 XIV.9.2 canonical GitHub 형식으로 확인합니다(아니면 Git
  실행 0). 그다음 `git ls-remote --end-of-options <url>`의 ref 줄을 세어 0이면
  `ReadyForFirstUpload`, 하나라도 있으면 `UseCloneInstead{ref_count}`(최초 연결 거부·Clone 안내)를 반환합니다.
  원격을 만들거나 바꾸지 않고 GitHub API를 호출하지 않습니다.
- `-`로 시작하는 URL과 빈 URL은 option 주입 방지를 위해 실행 전 거부합니다.
- Git 실패는 `RemoteError::Failed{exit_code, stderr_redacted}`로, stderr의 `scheme://userinfo@`를 가린
  텍스트만 보존합니다(원시 bytes 미보존).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 171 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **empty/non-empty bare repo fixture**: 로컬 경로 probe → 빈 bare `Empty`, commit을 push한 bare `HasRefs`; GitHub URL(HTTPS·SCP, `insteadOf`로 같은 로컬 bare) → `ReadyForFirstUpload` / `UseCloneInstead`; 검사 후 빈 저장소 그대로; 없는 저장소 → `RemoteError::Failed`(`empty_and_non_empty_bare_repo_fixture`) | 통과 |
| non-GitHub·자격 증명 포함 GitHub URL·option 형태 URL → spawn 0 | 통과 |
| `ls-remote` 출력 ref 수 해석·형식 오류 4종 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 테스트의 GitHub URL은 `url.<로컬 경로>/.insteadOf`와 `GIT_ALLOW_PROTOCOL=file`로 로컬 bare
  저장소에 연결해 네트워크를 쓰지 않았습니다.

## 남은 위험 / 한계

- 실제 GitHub HTTPS/SSH 접속(인증 prompt, 비공개 저장소 권한 오류)은 네트워크 금지로 검증하지 않았습니다.
  인증은 Git에 위임됩니다(XIV.9.6).
- 검사와 등록(XIV.9.4) 사이에 원격에 ref가 생기는 경쟁은 막을 수 없습니다. push(XIV.9.5)는 force 없이
  실행하므로 기존 이력을 덮어쓰지 않습니다.
