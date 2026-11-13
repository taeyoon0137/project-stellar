# 작업 보고서: XV.2.6 resolver 결과 결정적 정렬

- WBS: XV.2.6
- 작업명: resolver 결과를 LibraryId, commit, dependency path로 deterministic sort
- 상태: 완료 (`[x]`)
- 선행: XV.2.2(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/graph.rs` — `DependencyGraph::canonicalize_paths`, `sorted_entries`,
  `ResolvedEntry`, 테스트.
- `crates/dependencies/src/resolver.rs` — resolve 종료 시 `canonicalize_paths` 호출.
- `docs/task-reports/XV.2.6.md`(신규), 계획서(XV.2.6 `[ ]`→`[x]`).

## 구현 요약

- **발견한 비결정성**: dedup된 node의 하위 edge는 처음 도달한 경로를 물려받으므로, 정렬만으로는
  선언 순서가 바뀌면 경로 값 자체가 달라집니다(예: c가 a·b 양쪽에서 공유될 때 c → d의 경로가
  `a > c > d` 또는 `b > c > d`).
- **canonical 경로**: edge 집합(부모, 이름, 자식)은 각 node를 한 번만 전개하므로 선언 순서와
  무관합니다. 이 집합에서 node마다 root 최소 경로(길이 → 사전식)를 우선순위 큐로 구하고, 모든 edge
  경로를 `canonical(부모) + 선언 이름`으로 다시 씁니다. resolver가 종료 시 적용합니다.
- **정렬 출력**: `sorted_entries()`는 edge 하나당 `ResolvedEntry`(node key, canonical 경로, 선언
  선택자·repository, schema version, 전이 의존 node 목록)를 LibraryId → commit → 경로 순으로 정렬해
  반환합니다. lockfile resolved entry의 원천입니다(XV.4.1).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 30 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| root·library 선언 순서를 모두 뒤집은 fixture(공유 하위 node + 다중 버전): edge 기록 순서는 다르지만 `sorted_entries`·다중 버전 경고 동일, 정렬 결과 6건 기대값과 일치, d 경로 `a > c > d`(`sorted_output_is_independent_of_declaration_order`) | 통과 |
| 최소 경로 규칙: 깊은 경로보다 짧은 직접 경로 `q > r` 선택(`canonical_path_prefers_shorter_then_lexicographic`) | 통과 |
| 기존 diamond·순환·다중 버전 테스트 유지 | 통과 |

## 차이(작업 지시와 다른 점)

- "lock output"은 lockfile 텍스트가 아니라 lock entry의 원천인 `sorted_entries()`로 비교했습니다.
  lock 형식 변환은 XV.4.1, canonical YAML 텍스트 기록은 project writer 범위입니다.
- 정렬만으로는 결정성이 보장되지 않아 edge 경로 canonical화를 추가했습니다(XV.2.2 edge 경로 의미를
  "canonical 부모 경로 + 이름"으로 확정).

## 남은 위험 / 한계

- lock에는 edge(부모·이름)마다 한 경로만 기록합니다. 공유 node 아래의 모든 root 경로 조합을 펼쳐
  기록하지 않으므로(조합 폭발 방지) `b > c > d` 같은 대체 경로는 `b > c` entry와 `c > d` 관계로
  추론해야 합니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 사슬에 `[~]` 행(XV.1.1 등)이 있어 함께 `[~]`로 둡니다. 이 행 자체의 검증은 dependencies crate 안에서 통과했습니다.
