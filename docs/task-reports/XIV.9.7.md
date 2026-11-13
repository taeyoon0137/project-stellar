# 작업 보고서: XIV.9.7 push 후 remote tracking 갱신

- WBS: XIV.9.7
- 작업명: push 성공 후 remote tracking status 갱신, 실패 시 local commit/index 미변경
- 상태: 완료 (`[x]`)
- 선행: XIV.9.5(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 bare 저장소만 사용; 네트워크 미사용).

## 변경 파일 목록

- `crates/git-porcelain/src/remote/push.rs` (`push_and_refresh`, `PushReport`, 테스트)
- `docs/task-reports/XIV.9.7.md`(신규), 계획서(XIV.9.7 `[ ]`→`[x]`)

## 구현 요약

- `push_and_refresh`: push(XIV.9.5)가 성공하면 `read_repository_status`(XIV.2.2)로 upstream ahead/behind를
  다시 읽어 `PushReport{outcome, status}`로 돌려줍니다(Git이 push 시 갱신한 `refs/remotes/<remote>/…` 반영).
- 실패하면 오류만 돌려주고 그 밖의 Git 명령(fetch·pull·reset·rollback)을 실행하지 않습니다. push는 local
  commit·index·작업 트리를 바꾸지 않으므로 거부된 push 뒤 local 상태가 그대로입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 185 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| 최초 push(`--set-upstream`) 후 `Tracking{origin/main, 0, 0}`; 새 commit 후 ahead 1 → push 후 ahead 0(`successful_push_refreshes_tracking_status`) | 통과 |
| **rejected push fixture local state 동일**: 원격 이력 분기 + local commit·staged·unstaged 변경 상태에서 push 거부 → HEAD, 모든 ref(`for-each-ref`), index(`ls-files --stage -z`), status 출력, 작업 트리 bytes 모두 이전과 동일(`rejected_push_keeps_local_state_identical`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 원격이 push 일부 ref만 받아들인 경우(여러 refspec)는 현재 API가 refspec 1개만 받으므로 다루지 않습니다.
- tracking 갱신은 Git push가 remote-tracking ref를 갱신하는 동작에 의존합니다(fetch는 하지 않음).
