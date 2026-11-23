# 작업 보고서: XVI.11.3 두 branch conflict→Semantic Conflict 해결→git add E2E

## WBS

- WBS: XVI.11.3
- 작업명: 두 branch conflict→Semantic Conflict 해결→git add E2E
- 상태: 부분 완료 (`[~]`) — 2026-11-24 통합 판정: 이 행의 구현은 끝났지만 선행 XIV.14.4이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`) — 2026-11-24 `text-source-everywhere` lane: 앱이 만든 프로젝트(텍스트 원본이 Git 원본)의 두 branch conflict가 conflict 세션의 텍스트 원본 단위로 의미 병합되고 `git add`까지 unmerged 0, MERGE_HEAD 유지입니다(아래 후속). 처음 보고 당시: 부분 완료 (`[~]`) — 텍스트 원본 View 파일 conflict를 conflict 세션이 의미 병합하지 않았습니다. 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XIV.14.4
- 결정: [ADR-0111](../adr/ADR-0111-headless-release-e2e-scenarios.md)
- lane: `headless-e2e`

## 변경 파일

- `tests/integration/tests/e2e_branch_conflict_resolve.rs`: 시나리오

코드 변경은 테스트뿐입니다(제품 코드 변경 없음).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --locked` | 0 — 전체 통합 테스트 통과(e2e 6개 파일 포함) |
| `cargo clippy -p stellar-integration-tests --tests --locked` | 0 — 경고 0 |
| `cargo xtask validate-adr --base 682c7b0c` | lane 결과 보고에 적음 |
| `STELLAR_CHECK_BASE=682c7b0c cargo xtask check-core` | lane 결과 보고에 적음 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행, debug build, 시스템 Git. 앱 UI 없이 앱 runtime(`AppRuntime`)과 headless ops만 씁니다.
  테스트 HOME·전역 Git 설정은 테스트 프로세스 임시 폴더로 격리합니다(ADR-0111 결정 2).

## 테스트

| 테스트 | 결과 |
|---|---|
| `two_branch_conflict_is_resolved_semantically_and_added` | 통과 — 앱 생성 프로젝트 + 컴포넌트 base commit → feature(width 1280·이름·컴포넌트 200×40)와 main(width 1024·배경색·컴포넌트 240×40)을 `stellar apply`·`component-source resize`로 편집·commit → `git merge` conflict(`design/current`·컴포넌트 원본) → conflict 세션 단위 2개, raw·제외 0, semantic conflict 2개(width·컴포넌트 크기)만, 미해결 적용은 쓰지 않음 → theirs·ours 해결 → `apply_session`: unmerged 0, MERGE_HEAD = feature, HEAD = merge 전 main, commit 없음 → width 1280·이름(theirs)·배경색(ours)·컴포넌트 240, `validate` 오류 0, staged 재검증 통과, 작업 트리 = index |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음).

## 차이

- 시나리오는 CLI로 편집한(텍스트 원본이 없는) 프로젝트를 씁니다. 앱 창으로 두 branch를 편집하면 `design/pages/<page>/<view>.stellar.yaml`이 conflict로 남고 conflict 세션이 그 경로를 "의미 병합하지 않는 Stellar 저장 경로"로 제외해 unmerged가 남습니다(임시 실측, ADR-0111 결정 5).

## 남은 위험

- 텍스트 원본(View·문서·token) codec을 conflict 세션과 merge driver dispatch에 더해야 앱 기본 프로젝트에서 행 기준을 만족합니다(ADR-0081 단점).
- 선행 XIV.14.4는 원장에 `[ ]`입니다. 이 시나리오가 그 행의 증거로 쓰일 수 있는지는 통합 때 판정합니다.

## 후속: 텍스트 원본 conflict 단위 (2026-11-24, ADR-0113 결정 5)

### 변경 파일

- `crates/ops/src/merge_text.rs`(신규): 세 쪽 원본 파일 집합 전체를 같은 codec으로 읽어 문서는 엔진으로, 문서 밖 의미는 단위별 3-way로 병합
- `crates/ops/src/candidate/session.rs`: unmerged 텍스트 원본 경로 전체를 한 단위(`UnitKind::TextSource`)로 열고, 텍스트 원본 프로젝트의 cache pointer(`design/current`) conflict는 그 단위 결과로 cache를 다시 씀(소유 밖 dispatch, ADR-0113)
- `crates/ops/src/candidate/install.rs`: driver 등록이 텍스트 원본 pattern 줄을 더함(소유 밖 한 함수)
- `tools/stellar-cli/src/merge_driver.rs`: driver가 텍스트 원본 경로를 작업 트리 문맥으로 병합
- `tests/integration/tests/e2e_branch_conflict_resolve.rs`: 앱이 만든 프로젝트의 기대값을 텍스트 원본 단위로 바꿈(소유 밖, 시나리오는 그대로)
- `tools/stellar-cli/tests/text_source_git.rs`(신규): CLI 시나리오

### 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --locked --test e2e_branch_conflict_resolve`(샌드박스 해제) | 0 — 1 passed |
| `cargo test -p stellar-cli --locked --test text_source_git`(샌드박스 해제) | 0 — 3 passed |

### 시험

| 시험 | 확인 |
|---|---|
| `two_branch_conflict_is_resolved_semantically_and_added` | unmerged 3(View 원본·`design/current`·컴포넌트 원본) → 단위 2개(텍스트 원본·컴포넌트), raw·제외 0, semantic conflict 2개(width·컴포넌트 크기), width 객체의 raw 경로 = View 원본 파일 → theirs·ours 해결 → unmerged 0, MERGE_HEAD 유지, HEAD 그대로, commit 없음, 이름(theirs)·배경색(ours) 자동 병합, `validate` 오류 0, staged 재검증 통과, 작업 트리 = index |
| `adjacent_line_edits_in_one_view_file_merge_through_the_driver_and_resolve_clears_the_cache_conflict` | 같은 node의 이웃한 두 줄(`locked`·`display_name`)을 두 branch가 바꾸면 Git 텍스트 merge(`git merge-file`)는 충돌, driver는 View 파일을 의미 병합하고 남은 unmerged는 `design/current`뿐 → `merge-driver resolve --apply`가 원본 결과로 cache를 다시 써 unmerged 0, 이름·잠금 모두 반영 |
| `a_real_text_source_conflict_is_listed_and_resolved_to_theirs` | 같은 이름을 두 branch가 다르게 바꾸면 텍스트 원본 단위 conflict 1개(쓰기 불가) → `--resolve 0=theirs --apply` → unmerged 0, 이름 = theirs, 작업 트리 = index |

### 남은 위험(후속)

- driver가 의미 병합하지 못한 텍스트 원본 파일은 Git 텍스트 merge 결과 대신 conflict로 남습니다(conflict 세션이 세 stage로 다시 병합).
- chunk cache가 아직 Git에 있어 두 branch가 모두 저장했으면 `design/current`가 언제나 conflict이고 `merge-driver resolve --apply`가 필요합니다(ADR-0113 결정 7).
