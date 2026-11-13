# 작업 보고서: XXVI.1.3 pull 전략 명시 선택

- WBS: XXVI.1.3
- 작업명: pull strategy를 사용자가 merge/rebase/ff-only 중 선택하게 하고 시스템 config 기본을 임의 변경하지 않음
- 상태: 완료 (`[x]`)
- 선행: XXVI.1.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 bare 저장소만 사용; 네트워크 미사용).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/pull.rs` (신규: `pull`, `pull_args`, `PullStrategy`, `PullRequest`, `PullOutcome`, `PullError`, 테스트)
- `crates/git-porcelain/src/advanced/mod.rs` (`pull` 모듈)
- `docs/task-reports/XXVI.1.3.md`(신규), 계획서(XXVI.1.3 `[ ]`→`[x]`)

## 구현 요약

- 전략은 명령 인자로만 전달합니다: Merge = `--no-rebase --ff --no-edit`, Rebase = `--rebase`,
  FastForwardOnly = `--ff-only`. 사용자 config(`pull.rebase`·`pull.ff`)를 쓰거나 기대지 않습니다.
- `git pull --no-recurse-submodules <전략> -- <remote> <branch>`; 등록된 remote와 안전한 branch 이름만 받습니다
  (위반 시 Git 미실행). 취소 가능(`GitCancelToken`).
- fast-forward 불가·conflict는 Git 결과(자격 증명을 가린 stderr)를 그대로 돌려주고 자동 continue·abort·reset을 하지
  않습니다. 결과는 pull 전후 HEAD입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 207 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **strategy fixture**(local config에 반대 설정 `pull.ff=only`·`pull.rebase=true`): ff-only fast-forward 성공; 갈라진 이력 ff-only → Git 거부·HEAD 불변; Merge → merge commit(부모 2); Rebase → 선형(부모 1, `HEAD~1` == `origin/main`); 세 저장소 `.git/config` bytes 불변(`strategy_fixture_and_config_unchanged`) | 통과 |
| option 형태 branch 이름 → spawn 0, 미등록 remote 거부(`invalid_requests_do_not_pull`) | 통과 |
| 전략별 argv 고정(`strategy_args_are_explicit`) | 통과 |

## 차이(작업 지시와 다른 점)

- UI 없이 core API만 제공합니다(lane 지시). Merge 전략은 가능하면 fast-forward(`--ff`)합니다.

## 남은 위험 / 한계

- `rebase.autoStash` 등 다른 사용자 config는 Git이 그대로 적용합니다(Stellar가 덮어쓰지 않음).
- rebase·merge conflict 해결과 continue는 XIV.12·XXVI.1.4(선행 미완) 범위입니다.
