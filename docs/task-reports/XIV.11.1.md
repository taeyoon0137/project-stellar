# 작업 보고서: XIV.11.1 unmerged index stage 수집

- WBS: XIV.11.1
- 작업명: `git ls-files -u -z`를 읽어 stage 1(base), 2(ours), 3(theirs) blob과 mode를 path별로 수집
- 상태: 완료 (`[x]`)
- 선행: XIV.2.5(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/conflict.rs` (신규: `read_unmerged_paths`, `group_unmerged`, `UnmergedPath`, `StageBlob`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`conflict` 모듈)
- `crates/git-porcelain/src/test_support.rs` (`commit_file`이 상위 디렉터리를 만들도록 보강)
- `docs/task-reports/XIV.11.1.md`(신규), 계획서(XIV.11.1 `[ ]`→`[x]`)

## 구현 요약

- `git ls-files -u -z --full-name -- :/`(cwd와 무관하게 root 기준 전체)를 XIV.2.5 해석기로 읽고, 경로별로
  stage 1·2·3을 `UnmergedPath{base, ours, theirs}`(`StageBlob{mode, oid}`)에 모읍니다. 없는 stage는 `None`
  (추가/추가의 base, 삭제/수정의 삭제 쪽 등).
- 같은 경로·stage 중복이나 stage 0이 섞이면 해석 오류입니다. index를 읽기만 하며 trust gate를 거칩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 190 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **text/binary conflict fixture**: 텍스트·바이너리 양쪽 수정 merge → 두 경로 모두 stage 1·2·3, 각 blob이 Git `:<stage>:<path>` 해석과 일치, 하위 cwd 결과 동일(`text_and_binary_conflict_fixture`) | 통과 |
| **rename conflict fixture**: 같은 파일을 양쪽에서 다른 이름으로 rename → unmerged 항목 수집, 모든 경로·stage blob이 Git 해석과 일치(`rename_conflict_fixture`) | 통과 |
| 삭제/수정 충돌 → ours `None`, base·theirs 있음(`delete_modify_conflict_has_missing_side`) | 통과 |
| 합성 stage 묶음·누락 stage, 중복·stage 0 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- rename 충돌에서 Git(ort)이 어떤 경로에 어떤 stage를 기록하는지는 Git 버전 동작을 그대로 따릅니다. 테스트는
  고정 배치 대신 Git 자신의 `:<stage>:<path>` 해석과 일치하는지로 검증했습니다.
