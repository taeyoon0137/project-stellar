# 작업 보고서: XIV.10.9 merge 결과·conflict 결정론적 정렬

- WBS: XIV.10.9
- 작업명: merge 결과와 conflict 순서를 deterministic sort
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록: 구현·검증과 선행 판정은 이미 `[x]` 조건을 충족했고 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 `[~]`에 머물렀습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XIV.10.8(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XIV.10.8
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/merge.rs` — 공개 `canonical_conflicts`(정렬·중복 제거), `merge`가 사용.
- `crates/semantic-merge/tests/determinism.rs`(신규).

## 구현 요약

- conflict 순서는 `(StableId, 대상)`(객체 < 위치 < 속성 경로), 같은 키는 kind code 순이며 같은 `(StableId, 대상)`은 code가
  작은 하나만 남깁니다. 이전 순서의 앞 키였던 소유 문서를 뺐습니다(lane 결정: `(StableId, PropertyPath)` 순).
- 병합 snapshot은 StableId map, 변경 목록은 semantic-diff canonical 순서라 입력 순서와 무관합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | lib 28, dedicated 3, determinism 3, result_validation 4 passed |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 세 입력 객체 삽입 순서를 39가지로 바꿔도 `MergeResult` canonical bytes 동일(conflict 4개 이상 포함) | 통과 |
| conflict가 `(StableId, 대상)` 엄격 오름차순(중복 없음) | 통과 |
| 뒤집고 중복시킨 목록, 회전한 목록에 `canonical_conflicts` 적용 시 같은 목록 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- conflict 순서 키가 바뀌어(문서 우선 → StableId 우선) 같은 입력의 conflict 목록 순서가 이전 엔진과 다를 수 있습니다. 출력
  schema는 같습니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIV.10.9)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `cbc81019` Feat: merge conflict를 StableId·대상 순으로 결정론적 정렬
