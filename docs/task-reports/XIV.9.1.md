# 작업 보고서: XIV.9.1 기존 remote 목록

- WBS: XIV.9.1
- 작업명: existing remote 목록을 `git remote -v`의 안정 parse 방식으로 읽고 fetch/push URL 구분, URL은 민감값
- 상태: 완료 (`[x]`)
- 선행: XIV.1.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`, 네트워크 미사용).

## 변경 파일 목록

- `crates/git-porcelain/src/remote/mod.rs` (신규: `list_remotes`, `parse_remote_verbose`, `RemoteEntry`, `RemoteUrl`, `redact_credentials`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`remote` 모듈)
- `docs/task-reports/XIV.9.1.md`(신규), 계획서(XIV.9.1 `[ ]`→`[x]`)

## 구현 요약

- `git remote -v` 줄 형식 `<name>\t<url> (fetch|push)`을 첫 TAB(remote 이름은 공백·TAB 불가)과 고정
  접미사로만 나눠 URL(공백 포함 경로 등)을 그대로 보존합니다. remote별 fetch URL 1개와 push URL
  목록(여러 개 가능)을 구분하고 첫 등장 순서를 유지합니다. 형식 오류·fetch URL 중복은 해석 오류이며,
  오류 원시값에는 URL을 넣지 않습니다.
- `RemoteUrl`은 민감값입니다: `Debug`는 `RemoteUrl(<redacted>)`만 출력하고, 표시용 `redacted()`는
  `scheme://user:token@` userinfo를 `***`로 가립니다. 실제 값은 Git 인자 전달용 `expose()`로만 꺼냅니다.
- `redact_credentials`는 Git stderr처럼 URL이 섞인 텍스트의 모든 `scheme://userinfo@`를 가립니다(scp
  형식 `git@host:` 사용자 이름은 비밀이 아니므로 유지). 후속 remote 연산 오류 표시에 씁니다.
- 실행은 trust gate를 거칩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 165 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **multiple remote fixture**(실제 Git): `origin`(fetch HTTPS, push URL 2개: SSH·공백 경로) + `backup`(push URL 미설정) → 이름·fetch·push 정확, push 미설정은 Git 표시대로 fetch URL(`multiple_remote_fixture_from_real_git`) | 통과 |
| 합성 출력 2 remote·여러 push URL·공백 경로·빈 출력(`multiple_remotes_fetch_and_push_urls_are_distinguished`) | 통과 |
| 형식 오류 6종 거부, 오류에 URL 미포함 | 통과 |
| `Debug`·표시에서 token 미노출, 텍스트 내 여러 URL redaction | 통과 |
| untrusted gate → spawn 증가 0 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `git remote -v`는 `url.<base>.insteadOf` 적용 후 URL을 보여 줍니다(Git이 실제로 쓰는 URL). config 원문이
  필요하면 별도 조회가 필요합니다.
- 줄 바꿈이 들어간 URL은 Git이 한 줄로 출력하지 못하므로 지원하지 않습니다(해석 오류).
