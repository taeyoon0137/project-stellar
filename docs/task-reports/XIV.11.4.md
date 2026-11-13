# 작업 보고서: XIV.11.4 non-Stellar conflict는 raw Git conflict

- WBS: XIV.11.4
- 작업명: Stellar 파일이 아닌 conflict는 raw Git conflict로 표시하고 Semantic Merge로 해석하지 않음
- 상태: 완료 (`[x]`)
- 선행: XIV.11.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/conflict.rs` (`partition_conflicts`, `read_conflict_partition`, `ConflictPartition`, `RawConflict`, `unmerged_xy`, 테스트)
- `docs/task-reports/XIV.11.4.md`(신규), 계획서(XIV.11.4 `[ ]`→`[x]`)

## 구현 요약

- unmerged 경로를 XIV.4.4 분류로 나눠 Stellar 프로젝트 파일은 `semantic`(Semantic Merge 입력), 그 밖은
  `raw`(`RawConflict`)로 돌려줍니다. raw 항목은 semantic 쪽 목록에 들어가지 않습니다.
- `RawConflict`는 경로, Git unmerged `XY`(stage 존재로 결정: `UU`·`AA`·`DU`·`UD`·`AU`·`UA`·`DD`, `git status`와
  같은 표기), stage 원본(외부 도구로 해결할 때 참고)을 보존합니다.
- XIV.11.2 snapshot materialize도 같은 분류로 non-Stellar conflict를 snapshot에서 제외하고 raw로 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 193 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **README conflict raw 유지**: 실제 merge에서 `README.md` 양쪽 수정·`docs/notes.md` 삭제/수정·`design/current.stellarref` 양쪽 수정 → semantic 목록은 design 파일만, raw는 `README.md` `UU`, `docs/notes.md` `DU`; raw `XY`가 `git status` unmerged 종류와 일치, stage 원본 보존(`readme_conflict_stays_raw_and_is_not_semantic_input`) | 통과 |
| stage 존재 → `XY` 표 7종, 합성 분할(`unmerged_xy_matches_stage_presence`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Semantic Merge 엔진(XIV.10)은 아직 없으므로, "해석하지 않음"은 입력 목록 분리(API 경계)로 보장했습니다.
