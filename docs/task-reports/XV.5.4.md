# 작업 보고서: XV.5.4 library 전이 resource ref를 같은 graph instance 안에서 해석

- WBS: XV.5.4
- 작업명: library의 transitive resource ref를 동일 dependency graph instance 안에서 resolve
- 상태: 완료 (`[x]`)
- 선행: XV.2.2(`[x]`), XV.5.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/ref_resolution.rs`(신규) — `RefScope`, `DependencyScopes`(`from_graph`·
  `from_lock`), `ScopeError`, `LibraryGraphInstance`, `ResolvedTarget`, `RefResolutionError`.
- `crates/dependencies/src/library_snapshot.rs` — `ExportedComponent`·`ExportedToken`에 `references`
  (library scope 외부 참조) 추가, `SnapshotLimits::max_references_per_resource`(기본 65,536)와
  `SnapshotError::TooManyReferences`, 상한 테스트.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/adr/ADR-0027-library-snapshot-resource-key.md` — 결정 8(참조 scope) 추가.
- `docs/task-reports/XV.5.4.md`(신규), 계획서(XV.5.4 `[ ]`→`[x]`).

## 구현 요약

- **scope**: 사용 프로젝트 문서 참조는 `RefScope::Project`, library resource 안 참조는
  `RefScope::Library(node)`입니다. `(scope, 선언 이름) → node` 표로 해석하므로 같은 참조 텍스트도 scope에
  따라 다른 버전을 가리킵니다.
- **scope 표**: resolver 그래프 edge(부모, 이름, 자식)에서 만들거나, lock entry의 canonical dependency
  path(XV.2.6)에서 마지막 이름을 뺀 경로의 entry를 부모로 찾아 만듭니다. 부모 entry가 없거나 같은
  경로·이름이 다른 node를 가리키면 `ScopeError`입니다.
- **해석**: 선언 이름 → node → LibraryId 확인 → snapshot → resource 종류 확인 순입니다. 실패는
  `UnknownDependency`(전이 library 직접 참조 포함), `LibraryMismatch`, `SnapshotNotLoaded`,
  `MissingResource`, `WrongKind`로 구분합니다.
- **전이 closure**: `resolve_transitive`는 각 resource의 `references`를 그 resource의 node scope에서
  해석합니다. 명시적 stack과 방문 집합을 써서 깊은 chain에서도 재귀하지 않고, 결과는 key 순·중복 제거입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-dependencies` | 종료 코드 0 |
| `cargo test -p stellar-dependencies` | 81 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| root→a→c v1, root→b→c v2(실제 resolver 그래프): a 전이 closure는 c v1 component·token, b는 c v2, 같은 참조 텍스트도 scope별 다른 버전, 사용 프로젝트의 c 직접 참조 거부(`v1_parent_reaches_v1_child_and_v2_parent_reaches_v2_child`) | 통과 |
| lock에서 만든 scope 표 = 그래프 scope 표(`lock_scopes_match_graph_scopes`) | 통과 |
| 부모 entry가 빠진 lock 거부(`lock_without_parent_entry_is_rejected`) | 통과 |
| 미선언 이름·library 불일치·없는 resource·다른 종류·snapshot 미로드 구조화 오류(`resolution_failures_are_structured`) | 통과 |
| 깊이 200 chain closure(중복 참조 1회), key 순 정렬(`transitive_closure_handles_shared_and_deep_chains_iteratively`) | 통과 |

## 차이(작업 지시와 다른 점)

- library resource의 외부 참조 목록은 export record의 `references`로 받습니다. 실제 저장 데이터에서 참조를
  모으는 일은 XV.5.1과 같이 `LibraryExportSource` 구현의 몫입니다.

## 남은 위험 / 한계

- 전이 closure는 첫 해석 실패에서 멈춥니다. 일부 library가 없을 때의 부분 해석·진단은 XV.5.5가 다룹니다.

## 리뷰 수정(9A 리뷰 P5, 2026-11-14)

- 문제: `DependencyScopes::from_graph`는 같은 scope의 같은 선언 이름이 서로 다른 node를 가리키면
  `BTreeMap::insert`로 조용히 덮어썼고, `from_lock`은 같은 입력을 `ScopeError::Conflict`로 거부했습니다. 두
  생성자가 잘못된 입력에서 다르게 동작했습니다.
- 재현: `conflicting_graph_edges_are_rejected_like_lock` — root edge와 같은 이름으로 다른 node를 가리키는
  edge를 추가한 그래프가 수정 전 `Ok`로 만들어져 실패했습니다.
- 수정: `from_graph`가 `Result<DependencyScopes, ScopeError>`를 돌려주고 충돌을 `Conflict`로 거부합니다.
  같은 node를 가리키는 중복 edge는 허용합니다.
- 검증: 위 테스트 통과, 기존 `lock_scopes_match_graph_scopes`(그래프·lock scope 표 동일) 등 통과.
