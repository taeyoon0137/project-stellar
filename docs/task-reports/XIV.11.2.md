# 작업 보고서: XIV.11.2 conflict 세 stage virtual project snapshot

- WBS: XIV.11.2
- 작업명: conflict 관련 Stellar project 파일의 세 stage를 각각 virtual project snapshot으로 materialize, conflict 없는 path는 current index 상태와 결합
- 상태: 부분 완료 (`[~]`)
- 선행: XIV.2.6(`[x]`, 이 lane), XIV.11.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/conflict_snapshot.rs` (신규: `materialize_conflict_snapshots`, `side_entries`, `ConflictSnapshots`, `ConflictSide`, `ConflictSnapshotError`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`conflict_snapshot` 모듈)
- `docs/task-reports/XIV.11.2.md`(신규), 계획서(XIV.11.2 `[ ]`→`[~]`)

## 구현 요약

- index stage 0(conflict 없는 경로, Git이 이미 병합해 stage한 결과)을 세 side 모두에 넣고, Stellar
  프로젝트 파일(XIV.4.4 분류)인 conflict 경로는 각 side의 stage blob(base=1, ours=2, theirs=3)을 넣습니다.
  그 side에 stage가 없으면(삭제·미추가) 넣지 않습니다.
- non-Stellar conflict 경로는 snapshot에 넣지 않고 `raw_conflicts`로 돌려줍니다(raw Git conflict).
- 각 side는 XIV.2.6 materializer로 별도 임시 디렉터리(`.git` 없음)에 만듭니다. 작업 트리·index는 읽기만 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 191 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **base/ours/theirs project open**(fixture loader): 실제 merge conflict(`design/current.stellarref` 양쪽 수정, tokens chunk 수정/삭제, `README.md` 양쪽 수정) → 세 디렉터리를 fixture loader로 열면 pointer가 각각 base/ours/theirs, tokens는 base·ours 내용이고 theirs에는 없음, 매니페스트 공통, README는 어느 snapshot에도 없음(raw conflict 목록에만), `.git` 없음(`base_ours_theirs_projects_open`) | 통과 |
| conflict 없는 공통 chunk와 깨끗이 병합된 추가 chunk가 세 snapshot 모두 index stage 0 내용 | 통과 |
| materialize 전후 작업 트리 bytes·`ls-files --stage` 동일 | 통과 |

## 차이(작업 지시와 다른 점)

- conflict 없는 경로는 index stage 0만 결합합니다(작업 트리의 미stage 변경은 넣지 않음). merge 중 Git이
  병합 결과를 index에 기록하므로 index가 conflict 해결의 기준이라는 아키텍처 §26.3 원칙을 따랐습니다.

## 남은 위험 / 한계

- **`[~]` 사유**: "project open"은 테스트 fixture loader(매니페스트·pointer·chunk 읽기)로만 확인했습니다.
  실제 프로젝트 열기 loader(V.4.4, storage crate)로 세 snapshot을 여는 통합 검증은 crate 경계상 project crate
  통합 단계에서 필요합니다.
- generation pointer와 chunk가 서로 다른 stage에서 와 섞이면 loader 검증이 실패할 수 있습니다(그 경우는
  semantic merge 단계가 판단).
