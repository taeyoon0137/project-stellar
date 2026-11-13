# 작업 보고서: XIV.9.4 빈 GitHub remote 등록

- WBS: XIV.9.4
- 작업명: empty remote만 `git remote add <name> <url>`로 등록, 기본 이름 `origin`, 이미 있으면 호출자가 다른 이름 제공
- 상태: 완료 (`[x]`)
- 선행: XIV.9.3(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 bare 저장소만 사용; 네트워크 미사용).

## 변경 파일 목록

- `crates/git-porcelain/src/remote/connect.rs` (`connect_empty_github_remote`, `ConnectRemoteRequest`, `ConnectedRemote`, `DEFAULT_REMOTE_NAME`, `is_valid_remote_name`, `ConnectError` 변형 추가, 테스트)
- `crates/git-porcelain/src/remote/mod.rs` (`From<GitOpError> for RemoteError`: 자격 증명 가린 변환)
- `docs/task-reports/XIV.9.4.md`(신규), 계획서(XIV.9.4 `[ ]`→`[x]`)

## 구현 요약

- 순서: (1) canonical GitHub URL 확인(아니면 Git 실행 0) → (2) remote 이름 규칙(영숫자·`.`·`_`·`-`,
  `-`·`.` 시작 불가, `..`·`.lock` 끝 불가) 확인(위반 시 Git 실행 0) → (3) `git remote -v`로 같은 이름이 있으면
  **덮어쓰지 않고** `RemoteNameTaken{name}` → (4) `ls-remote`로 ref가 있으면 `RemoteHasHistory`(Clone 안내,
  미등록) → (5) `git remote add -- <name> <url>`.
- 이름을 생략하면 `origin`입니다. 이미 있으면 호출자가 다른 이름을 제공해야 하며 Stellar가 임의로 이름을
  바꾸거나 기존 remote를 수정하지 않습니다.
- 실패 단계 이후는 실행하지 않으며 Git config를 바꾸지 않습니다. 오류의 stderr는 자격 증명을 가린 텍스트만
  보존합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 174 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **origin exists fixture**: 기존 `origin` 있음 + 이름 생략 → `RemoteNameTaken{origin}`, `.git/config` bytes 동일, origin URL 그대로(`origin_exists_fixture_is_not_overwritten`) | 통과 |
| 이력 있는 원격 → `RemoteHasHistory`, config 불변 | 통과 |
| 잘못된 이름(`-u`) → Git 실행 0 | 통과 |
| 다른 이름(`github`) 제공 → 빈 원격 등록, origin 유지 | 통과 |
| origin이 비어 있으면 기본 이름 `origin`으로 등록, config에 사용자가 준 URL 원문(`default_name_is_origin_when_free`) | 통과 |
| remote 이름 규칙 표 | 통과 |

## 차이(작업 지시와 다른 점)

- remote 이름은 Git ref 규칙보다 좁은 부분집합만 허용합니다(UI 입력 안전성). 최종 검증은 `git remote add`가
  합니다.

## 남은 위험 / 한계

- `ls-remote` 검사와 `remote add` 사이 원격 상태 변화는 막을 수 없습니다(push는 force 없이 실행).
- GitHub 외 remote 등록은 외부 터미널 사용(§25.3)이므로 이 API에서 제공하지 않습니다.
