# 작업 보고서: XIV.9.5 기존 remote로 push(force API 없음)

- WBS: XIV.9.5
- 작업명: Push는 사용자가 선택한 existing remote/refspec을 시스템 Git으로 실행, force push UI 미제공
- 상태: 완료 (`[x]`)
- 선행: XIV.9.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 bare 저장소만 사용; 네트워크 미사용).

## 변경 파일 목록

- `crates/git-porcelain/src/remote/push.rs` (신규: `push`, `PushRequest`, `PushRefspec`, `RefspecProblem`, `PushOutcome`, `PushedRef`, `RefUpdateFlag`, `PushError`, `parse_push_porcelain`, 테스트)
- `crates/git-porcelain/src/remote/mod.rs` (`push` 모듈)
- `docs/task-reports/XIV.9.5.md`(신규), 계획서(XIV.9.5 `[ ]`→`[x]`)

## 구현 요약

- `PushRequest{remote, refspec, set_upstream}`에는 force·force-with-lease·mirror·delete 옵션이 **없습니다**.
  `PushRefspec`은 `refs/heads/…`·`refs/tags/…` 전체 이름 쌍만 받고 `+`(force), 빈 src(삭제), `*`, `:`·`..`·
  `@{`·제어문자 등 ref 규칙 위반을 거부합니다.
- remote는 `git remote -v` 목록에 있는 이름만 허용합니다(URL 직접 push·미등록 이름 → `UnknownRemote`, push 미실행).
- 실행: `git push --porcelain [--set-upstream] -- <remote> <src:dst>`. pre-push hook은 Git이 실행하며 우회하지
  않습니다. `--porcelain` 출력의 ref 줄(`<flag>\t<from>:<to>\t<summary>`)만 해석하고 `To <url>`·안내 줄은
  보존하지 않습니다.
- non-fast-forward 등 거부는 `PushError::Rejected{refs, stderr_redacted}`로 Git 결과를 그대로 돌려줍니다.
  stderr는 자격 증명을 가린 텍스트만 남깁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 179 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **normal push fixture**: 로컬 bare `origin`으로 `main` push(`--set-upstream`) → `New`, 원격 `main` == 로컬 HEAD, upstream `origin/main`; 다음 commit push → `FastForward`; 재push → `UpToDate`(`normal_push_fixture`) | 통과 |
| **force option API 없음**: `+src` → `Force`, 빈 src → `Delete`, wildcard·비전체 이름·`refs/remotes/`·`.lock`·공백 등 11종 → `InvalidRef`(`refspec_rejects_force_delete_and_wildcards`) | 통과 |
| 미등록 이름·URL 직접 지정 → `UnknownRemote`, 원격 ref 0(`unknown_remote_and_url_are_not_pushed`) | 통과 |
| 이력 분기 후 push → `Rejected`(flag `!`), 원격 `main` 그대로(`non_fast_forward_is_rejected_as_git_result`) | 통과 |
| porcelain 해석(5종 flag, URL 미보존, 형식 오류, 정보 줄 무시) | 통과 |

## 차이(작업 지시와 다른 점)

- 원격 ref 삭제(`:dst`)와 wildcard refspec도 거부합니다. 파괴적이거나 범위가 불명확한 push를 초기 API에서
  제공하지 않기 위해서입니다.

## 남은 위험 / 한계

- 실제 GitHub HTTPS/SSH push(인증, 서버 측 hook·보호 branch 거부 문구)는 네트워크 금지로 검증하지 않았습니다.
- push 진행률(progress) 이벤트는 제공하지 않습니다(FFI event queue는 XIV.13.3).
