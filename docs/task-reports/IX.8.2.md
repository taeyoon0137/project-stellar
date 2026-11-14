# 작업 보고서: IX.8.2 시각 tree와 별도인 접근성 parent/child 관계

- WBS: IX.8.2
- 작업명: `AccessibilityTree` — 시각 계층과 다른 접근성 parent/child 관계 허용, source node 정확히 하나, orphan source·cycle 거부
- 상태: 완료 (`[x]`)
- 선행: IX.8.1 (이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/accessibility/src/tree.rs`(신규) — `AccessibilityTree`, `SourceIndex`, `TreeError`, `TreeIssue`, binary 형식.
- `crates/accessibility/src/lib.rs` — `tree` 모듈 선언.
- `docs/task-reports/IX.8.2.md`(신규), 계획서(IX.8.2 `[ ]`→`[x]`).

## 구현 요약

- **별도 관계**: 접근성 tree는 자체 root 목록과 자식 목록을 가지며 시각 계층과 무관하게 node를 묶을 수 있습니다
  (fixture: 시각 `1 > (2, 3 > 4)`를 접근성 `1 > (2, 4)`로 표현).
- **source 정확히 하나**: node는 source StableId를 key로 저장해 node 하나가 source 하나만 갖고, 한 source에서
  node 둘을 만들 수 없습니다(`insert` → `TreeError::DuplicateSource`). `validate(&dyn SourceIndex)`는 source가
  시각 tree에 없으면 `OrphanSource`를 냅니다.
- **forest 검사**(`validate_structure`): 없는 root·자식(`MissingNode`), 없는 부모(`MissingParent`), 중복 root·자식,
  부모 둘 이상(`MultipleParents`), 부모가 있는 root, cycle(강결합 요소마다 대표 cycle 하나, 경로 `[a, b, a]`.
  아래 "리뷰 후 수정" 참조), root에서 도달 불가(보고한 cycle 경로의 node는 cycle로만 보고). 모두 반복 순회이고 StableId·목록 순으로 방문해 결정론적입니다.
- **binary 형식**: `version ++ roots ++ (node ++ children)*`(node source 오름차순). 디코더는 형식(잘림·정렬·잉여
  bytes·재인코딩 일치)만 검사하고 관계 유효성은 `validate`가 맡습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-accessibility` | 13 passed, 0 failed |
| `cargo fmt -p stellar-accessibility -- --check` | 통과 |
| `cargo clippy -p stellar-accessibility --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 시각 계층과 다른 parent/child 관계가 유효(`separate_parent_child_relation_is_valid`) | 통과 |
| 같은 source로 두 번째 node 삽입 거부(`one_source_cannot_produce_two_nodes`) | 통과 |
| orphan source fixture 실패(`orphan_source_fixture_fails`) | 통과 |
| cycle fixture 실패(back edge `[2, 3, 2]`, root 없는 self cycle `[5, 5]`)(`cycle_fixture_fails`) | 통과 |
| 중복 root·없는 root/자식/부모·중복 자식·부모 있는 root·도달 불가 보고와 결정론(`structural_violations_are_reported`) | 통과 |
| tree round-trip, 잘린 prefix·잉여 bytes·source 정렬 위반 거부(`tree_round_trips_and_rejects_malformed_bytes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 시각 tree는 이 crate가 document에 의존할 수 없으므로(crate 그래프) `SourceIndex` trait으로 조회합니다.

## 남은 위험 / 한계

- 접근성 node가 없는 부모의 자식 목록은 검사에서 `MissingParent`로 보고하지만 binary 형식에는 기록하지
  않습니다(자식 목록은 부모 node에 속함). 유효한 tree는 영향이 없습니다.

## 리뷰 후 수정(9A 리뷰, 2026-11-14)

- 지적(A11Y-2): `find_cycles`가 back edge마다 DFS stack 경로를 복사해, 형식만 맞는 작은 입력(`from_bytes`)에서도
  보고 크기·시간이 back edge 수 × DFS 깊이에 비례했습니다. node 2,000개 chain의 모든 node가 node 0을 자식으로 갖는
  입력에서 보고 StableId가 2,005,003개였습니다.
- 수정: cycle 검사를 강결합 요소 분해(반복 Tarjan)로 바꾸고, 요소마다 대표 cycle 하나(요소에서 StableId가 가장
  작은 node에서 시작하는 최단 cycle, 자식 목록 순 BFS)만 보고합니다. 시간·메모리·보고 크기가 node·간선 수에
  비례합니다. 기존 fixture의 경로(`[2, 3, 2]`, `[5, 5]`)는 그대로입니다.
- 의미 변화: 한 강결합 요소 안에 cycle이 여럿이어도 하나만 보고합니다. `Unreachable`을 생략하는 node는 보고한
  cycle 경로에 있는 node입니다(전과 같은 규칙).
- 완료 근거(orphan source·cycle fixture 실패)는 바뀌지 않으므로 상태 `[x]`를 유지합니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-accessibility` | 33 passed, 0 failed |
| `cargo clippy -p stellar-accessibility --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-accessibility -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| back edge 2,000개 입력의 보고 크기가 node 수에 비례하고 대표 cycle이 하나, 대표 경로 밖 요소 node는 도달 불가로 보고(`cycle_report_stays_linear_on_many_back_edges`) | 수정 전 실패(보고 StableId 2,005,003개), 수정 후 통과 |
