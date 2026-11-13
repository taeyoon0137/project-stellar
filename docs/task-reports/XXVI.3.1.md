# 작업 보고서: XXVI.3.1 `git worktree list --porcelain` parser

- WBS: XXVI.3.1
- 작업명: `git worktree list --porcelain` parser 구현
- 상태: 완료 (`[x]`)
- 선행: XIV.1.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/worktree.rs` (신규: `list_worktrees`, `parse_worktree_list`, `WorktreeEntry`, `Annotation`, 테스트)
- `crates/git-porcelain/src/advanced/mod.rs` (`worktree` 모듈)
- `docs/task-reports/XXVI.3.1.md`(신규), 계획서(XXVI.3.1 `[ ]`→`[x]`)

## 구현 요약

- `git worktree list --porcelain -z`를 사용합니다(경로에 개행이 있어도 안전한 NUL 형식). 속성 `worktree`·`HEAD`·
  `branch`·`detached`·`bare`·`locked [이유]`·`prunable [이유]`를 `WorktreeEntry`로 해석하고, 첫 레코드를 주
  worktree로 표시합니다. 경로는 원시 바이트, 모르는 속성은 무시하되 `unknown`에 보존합니다.
- 레코드 끝 빈 줄 누락, `worktree` 줄 없음·중복, 잘못된 HEAD는 해석 오류입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 218 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **main/linked/locked/prunable fixture**(실제 Git): 공백 경로 주 worktree, `feature` branch linked, 한글 이유로 lock한 detached, 디렉터리를 지운 prunable → 4개, 주 worktree 경로·branch, linked branch, locked 이유, prunable 표시 정확; linked 안에서 읽어도 같은 목록(`main_linked_locked_prunable_fixture`) | 통과 |
| 합성 출력(공백 경로·이유 있음/없음 lock·prunable·미지 속성·bare·빈 출력) | 통과 |
| 형식 오류 5종 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- `-z`(Git 2.36+)를 씁니다. 그보다 오래된 Git은 이 명령이 실패하며 Git 오류를 그대로 돌려줍니다.

## 남은 위험 / 한계

- lock·prunable 이유 문구는 Git이 쓰는 텍스트(locale 의존 가능)이며 표시 전용입니다.
