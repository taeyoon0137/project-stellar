# 작업 보고서: XIV.14.4 시스템 Git branch merge conflict → Semantic Conflict 해결 → git add 통합 실행

## WBS

- WBS: XIV.14.4
- 작업명: branch merge conflict를 시스템 Git으로 만든 뒤 Semantic Conflict 해결·git add까지 통합 실행
- 상태: 완료 (`[x]`) 후보 — 2026-11-24 library-wire, 아래 "작업 (2026-11-24, library-wire)" 절: 텍스트 원본 프로젝트 시나리오가 판정 기준을 통과합니다. 원장 판정은 통합 때 정합니다. 이전 기록: 부분 완료 (`[~]`) — generation 저장 프로젝트로 판정 기준을 통과합니다. 하지만 앱 기본 원본은 텍스트 원본이고(ADR-0088), 텍스트 원본 View 파일의 branch conflict는 ops conflict 세션이 의미 병합하지 않아 unmerged가 남습니다(아래 "남은 위험"). 제품 기본 경로에서 판정 기준을 충족하지 못하므로 남은 범위가 있습니다. 원장 판정은 통합 때 `generate-ledger`로 정합니다. 보고 당시 기록: 완료 (`[x]`) 후보(검토에서 정정).
- 선행: XIV.12.5
- 결정: [ADR-0120](../adr/ADR-0120-recovery-probe-git-scenarios-terminal-env.md) 결정 5, ADR-0080, ADR-0111
- lane: `recovery-git-terminal`

## 변경 파일

- `tests/integration/tests/git_branch_conflict_resolve.rs`(신규)

코드 변경은 시험뿐입니다(제품 코드 변경 없음).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --test git_branch_conflict_resolve` | 0 — 1 passed |
| `cargo clippy -p stellar-integration-tests --test git_branch_conflict_resolve -- -D warnings` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬, 시스템 Git, XIV.14.1 fixture harness(로컬 bare 원격, 네트워크 없음, hermetic
  설정, 고정 author·시각).

## 테스트

| 테스트 | 결과 |
|---|---|
| `system_git_branch_conflict_is_resolved_semantically_and_added_keeping_merge_state` | 통과 — base commit·push → peer clone `theirs`(아이콘 이름 + 구분선 숨김) push, work `main`(아이콘 이름 + 카드 크기) → `git fetch` → 시스템 Git `git merge --no-edit origin/theirs` 실패(conflict) → unmerged = `design/current` 하나, 진행 작업 = Merge → conflict 세션: 단위 generation 하나, raw·제외 0, semantic conflict는 아이콘 이름 하나 → 미해결 적용은 거부되고 unmerged 그대로 → theirs 선택 → `apply_session`: 남은 unmerged 0, Merge 진행 중, `design/current` add → `git ls-files -u` 빈 출력, MERGE_HEAD = theirs commit, HEAD = merge 전 ours commit, `git status` "All conflicts fixed but you are still merging", 작업 트리 = index → 아이콘 = theirs, 카드 400×200(ours), 구분선 숨김(theirs), validator 오류 0 |

## 성능

- 해당 없음.

## 차이

- 해결 UI 대신 `stellar merge-driver resolve`와 같은 ops conflict 세션 API로 해결합니다(사용자 방향 2026-11-23).

## 남은 위험

- 앱 기본 원본은 텍스트입니다(ADR-0088). 두 branch가 앱 창에서 같은 View를 편집하면 `design/pages/**` 텍스트 원본 파일이
  conflict로 남고, ops conflict 세션이 그 경로를 의미 병합하지 않는 Stellar 경로로 제외해 unmerged가 남습니다(XVI.11.3
  보고서 실측, ADR-0081 단점). 텍스트 원본 codec을 conflict 세션과 merge driver dispatch에 더하는 일은 ops 후속이며, 그때
  같은 시나리오를 텍스트 원본 프로젝트로 더해야 합니다.

## 작업 (2026-11-24, library-wire): 텍스트 원본 프로젝트 시나리오

- 결정: [ADR-0127](../adr/ADR-0127-library-product-paths-token-companion.md) 결정 5. 위 "남은 위험"의 조건(텍스트 원본 codec의 conflict
  세션 연결)은 ADR-0113 결정 5로 이미 제품에 있습니다. 같은 시나리오를 텍스트 원본 프로젝트로 더했습니다. 제품 코드 변경은 없습니다.
- 변경 파일: `tests/integration/tests/git_branch_conflict_resolve.rs`, `tests/integration/tests/git_common/mod.rs`(`write_text_project`).

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --test git_branch_conflict_resolve` | 0 — 2 passed |

| 검사 | 결과 |
|---|---|
| `text_source_branch_conflict_is_resolved_semantically_and_added_keeping_merge_state`: 텍스트 원본만 있는 프로젝트에서 base commit·push → peer `theirs`(아이콘 이름 + 구분선 숨김) → work `main`(아이콘 이름 + 카드 크기) → merge driver 없는 시스템 Git `git merge` 실패 → unmerged는 `design/pages/**` View 원본뿐, 진행 작업 = Merge → conflict 세션: 단위 = 텍스트 원본 하나, raw·제외 0, semantic conflict는 아이콘 이름 하나 → 미해결 적용 거부·unmerged 그대로 → theirs 선택 → `apply_session`: 남은 unmerged 0, Merge 진행 중, conflict 경로 모두 add → `git ls-files -u` 빈 출력, MERGE_HEAD = theirs, HEAD = ours, `git status` "still merging", 작업 트리 = index, conflict marker 없음 → 제품 reader로 다시 읽은 의미: 아이콘 = theirs, 카드 400×200(ours), 구분선 숨김(theirs), validator 오류 0 | 통과 |

- 남은 위험: 텍스트 원본과 generation cache를 함께 commit한 프로젝트(앱 저장 결과)에서 둘이 함께 conflict나는 경우는 이 시험이 다루지
  않습니다. 해결 UI는 UI 재설계 때 합니다.
