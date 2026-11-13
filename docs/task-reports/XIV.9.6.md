# 작업 보고서: XIV.9.6 자격 증명 Git 위임

- WBS: XIV.9.6
- 작업명: credential helper/SSH agent prompt를 Git에 위임하고 Stellar는 token/key를 저장하지 않음, askpass bridge 입력값은 메모리에서 즉시 폐기
- 상태: 완료 (`[x]`)
- 선행: XIV.9.5(`[x]`, 이 lane), II.3.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 bare 저장소만 사용; 네트워크 미사용).

## 변경 파일 목록

- `crates/git-porcelain/src/remote/credentials.rs` (신규: `CredentialDelegation`, `DELEGATED_ENV`, `AskpassSecret`, 테스트)
- `crates/git-porcelain/src/remote/mod.rs` (`credentials` 모듈)
- `crates/git-porcelain/src/runner.rs` (`GitRunner::with_allowed_env`: allowlist 이름 추가)
- `docs/task-reports/XIV.9.6.md`(신규), 계획서(XIV.9.6 `[ ]`→`[x]`)

## 구현 요약

- `CredentialDelegation::apply`는 remote 실행기에 credential helper·ssh·agent가 쓰는 환경 이름
  (`PATH`·`HOME`·`USER`·`LOGNAME`·`TMPDIR`·`SSH_AUTH_SOCK`, 값은 사용자 환경 그대로)과
  `GIT_TERMINAL_PROMPT=0`만 더합니다. `credential.helper` 등 Git config를 쓰지 않고 token 환경 변수를
  만들지 않습니다. 인증 방식 선택·저장은 시스템 Git(Keychain helper·SSH agent)이 합니다.
- askpass bridge가 필요하면 `GIT_ASKPASS`·`SSH_ASKPASS`·`SSH_ASKPASS_REQUIRE=force`로 bridge 실행 파일만
  지정합니다. bridge가 다루는 입력은 `AskpassSecret`: `Debug`는 `<redacted>`, `write_once_to`로 한 번 쓰고
  volatile write로 즉시 0으로 덮어쓰며 drop 시에도 덮어씁니다.
- 자격 증명이 든 URL은 연결 전에 거부(XIV.9.2)하고, remote 오류는 stderr를 자격 증명을 가린 텍스트로만
  보존합니다(XIV.9.1·9.3·9.5).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 183 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **credential log redaction·storage scan**: marker token이 든 GitHub URL 연결 → `CredentialsInUrl`(Git 미실행), 오류 `Debug`/`Display`에 marker 없음; 정상 연결·push 흐름 실행; 사용자가 직접 넣은 자격 증명 URL remote로 push 실패 시 오류 표시에 marker 없음; 작업 저장소(`.git` 포함)·원격 디렉터리 전체 파일 bytes에 marker 0건(`credential_redaction_and_storage_scan`) | 통과 |
| 위임 환경: override는 `GIT_TERMINAL_PROMPT=0`뿐, token·config 주입 변수 없음, bridge 지정 시 askpass 3종(`delegation_env_has_no_secrets_or_config_writes`) | 통과 |
| 실행기 적용·spawn 카운터 공유(`delegation_applies_to_runner_and_shares_counter`) | 통과 |
| `AskpassSecret` 한 번 쓰기·같은 할당 0 덮어쓰기·`Debug` redaction(`askpass_secret_is_written_once_wiped_and_never_formatted`) | 통과 |

## 차이(작업 지시와 다른 점)

- askpass bridge 실행 파일 자체(앱이 제공하는 prompt UI)는 macOS 셸 통합 단계의 일입니다. 이 작업은 core의
  환경 정책과 비밀값 버퍼만 구현했습니다.

## 남은 위험 / 한계

- 실제 HTTPS credential helper·SSH agent prompt는 네트워크 금지로 실행하지 않았습니다(로컬 전송에서는
  askpass가 호출되지 않음을 storage scan으로만 확인).
- `AskpassSecret`은 자기 버퍼만 덮어씁니다. 호출자가 만든 다른 사본(UI 문자열 등)은 호출자가 관리해야 합니다.
- Git 자체가 stderr에 출력하는 문구의 redaction은 `scheme://userinfo@` 형태에 한정됩니다.
