# 작업 보고서: XXVI.1.2 fetch와 진행률

- WBS: XXVI.1.2
- 작업명: fetch UI와 progress 연결(core API)
- 상태: 완료 (`[x]`)
- 선행: XXVI.1.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 bare 저장소만 사용; 네트워크 미사용).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/fetch.rs` (신규: `fetch_remote`, `FetchProgress`, `ProgressParser`, `parse_progress_segment`, `FetchOutcome`, `RefUpdate`, `FetchError`, 테스트)
- `crates/git-porcelain/src/advanced/mod.rs` (`fetch` 모듈)
- `crates/git-porcelain/src/runner.rs` (`GitRunner::run_streaming_stderr`: stderr 조각을 도착하는 대로 콜백 전달, 취소·timeout·tree 종료 동일)
- `crates/git-porcelain/src/trust.rs` (`GatedGitRunner::run_streaming_stderr`: untrusted면 spawn 0)
- `docs/task-reports/XXVI.1.2.md`(신규), 계획서(XXVI.1.2 `[ ]`→`[x]`)

## 구현 요약

- 등록된 remote만 `git fetch --progress --no-recurse-submodules -- <remote>`로 가져옵니다(submodule 저장소에서
  Git을 실행하지 않음, `--prune` 미제공).
- 실행기는 stderr drain 스레드가 읽은 조각을 channel로 넘기고 호출 스레드가 대기 루프에서 콜백을 부릅니다.
  완료 결과의 stderr에도 같은 bytes 전체가 남습니다.
- `ProgressParser`는 `\r`·`\n`으로 끝난 segment를 조각 경계와 무관하게 모아 `FetchProgress{remote_side, phase,
  percent, done, total}`로 바꿉니다(Git 문구 그대로의 단계 이름, 진행률이 아닌 줄은 무시).
- 결과는 fetch 전후 `refs/remotes/<remote>/`·`refs/tags/` 비교(`RefUpdate{name, old, new}`)입니다. 실패는
  자격 증명을 가린 stderr로, 취소는 `Cancelled`로 반환하며 성공으로 보고하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 204 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |
| `cargo check -p stellar-project --all-targets`(하위 사용 crate) | exit 0 |

| 검사 | 결과 |
|---|---|
| **local remote fixture**: 로컬 bare(5 commit + tag)를 fetch → 진행률 이벤트 수신(100% 또는 개수 포함), `origin/main`·`v1` 새 ref; 변화 없는 재fetch → 갱신 0; 새 commit push 후 fetch → `origin/main` old→new(`local_remote_fetch_reports_progress_and_ref_updates`) | 통과 |
| 미등록 remote → `UnknownRemote`; 취소된 token → `Cancelled`(`unknown_remote_and_cancelled_fetch_are_not_success`) | 통과 |
| 진행률 segment 해석(로컬·remote·개수형·비진행률 줄), `\r` 조각 경계 처리 | 통과 |
| streaming 실행: 첫 stderr 조각이 프로세스 종료 전에 전달, 전체 bytes 보존(`streaming_stderr_delivers_chunks_before_exit_and_keeps_full_output`) | 통과 |

## 차이(작업 지시와 다른 점)

- UI 없이 core API(진행률 콜백)만 제공합니다(lane 지시).
- Git 2.54는 fetch 시 `refs/remotes/origin/HEAD`도 만들 수 있어, 결과 검증은 해당 remote 접두어 안의 갱신으로 했습니다.

## 남은 위험 / 한계

- 진행률 문구는 Git locale·버전에 따라 달라질 수 있습니다(표시 전용, 해석 실패해도 fetch 결과에 영향 없음).
- 네트워크 원격의 진행률·인증은 검증하지 않았습니다.
