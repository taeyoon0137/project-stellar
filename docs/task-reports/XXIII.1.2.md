# 작업 보고서: XXIII.1.2 공통 이력 base와 two-way 모드

- WBS: XXIII.1.2
- 작업명: 두 source의 공통 Git history가 있으면 시스템 Git `merge-base`로 base를 구하고, 없으면 two-way
  comparison mode로 명시
- 상태: 완료 (`[x]`)
- 선행: XXIII.1.1 (이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/composer/base.rs`(신규) — `ComparisonMode`(`ThreeWay`·`CrissCross`·
  `TwoWay`), `TwoWayReason`, `parse_merge_bases`, `comparison_mode_for_commits`, `comparison_mode`.
- `crates/git-porcelain/src/composer/mod.rs` — `base` 모듈 등록·재노출.
- `docs/task-reports/XXIII.1.2.md`(신규), 계획서(XXIII.1.2 `[ ]`→`[x]`).

## 구현 요약

- `comparison_mode(git, cwd, a, b)`는 두 `ResolvedSource`의 이력 기준 commit(branch·tag·commit·HEAD는 그
  commit, Index·WorkingTree는 그 worktree HEAD)으로 `git merge-base --all --end-of-options <a> <b>`를
  실행합니다. commit graph를 직접 순회하지 않습니다.
  - 종료 코드 0, base 1개 → `ThreeWay { base }`.
  - 종료 코드 0, base 여러 개(criss-cross) → `CrissCross { bases }`(정렬·중복 제거). 단일 base를 임의로
    고르지 않고 명시합니다.
  - 종료 코드 1·출력 없음(공통 조상 없음) → `TwoWay { reason: NoCommonHistory }`.
  - 한쪽이라도 commit이 없음(unborn HEAD의 index·작업 트리) → Git 실행 없이
    `TwoWay { reason: MissingHistory }`.
  - 그 밖 종료 코드는 `GitOpError::Git`(Git 결과 보존). 출력 줄이 object ID가 아니면 해석 오류입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 277 passed + doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **related fixture**(XIV.14.1 표준 scenario): main·feature → 분기점 initial, feature·tag v1 → initial, 조상 initial·HEAD → initial, 같은 commit(HEAD·v1) → main, WorkingTree·feature → initial(작업 트리는 HEAD 기준), Index·WorkingTree → main | 통과 |
| **unrelated fixture**: orphan branch와 main·feature → `TwoWay { NoCommonHistory }` | 통과 |
| criss-cross(서로 병합한 두 branch) → `CrissCross { bases: [left 1, right 1] }` | 통과 |
| 한쪽·양쪽 commit 없음 → `TwoWay { MissingHistory }`, spawn 0 | 통과 |
| 없는 commit ID → Git 실패 보존, 출력 해석(정렬·중복 제거, 잘못된 줄 거부) | 통과 |

## 차이(작업 지시와 다른 점)

- 작업 지시는 base 유무만 다룹니다. criss-cross처럼 merge base가 여럿인 경우를 `ThreeWay`로 숨기지 않고
  `CrissCross`로 따로 돌려줍니다(단일 base 선택은 SemanticMergeEngine·UI 정책에서 결정).

## 남은 위험 / 한계

- criss-cross base를 하나로 합치는 방식(Git recursive의 가상 base 등)은 정하지 않았습니다. XXIII.2
  (Candidate C) 단계에서 정책이 필요합니다.
- shallow clone처럼 이력이 잘린 저장소에서는 Git이 공통 조상을 찾지 못해 `TwoWay`가 될 수 있습니다.
