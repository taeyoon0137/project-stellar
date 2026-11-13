# 작업 보고서: VIII.5.3 토큰 dependency graph와 저장 전 cycle 거부

- WBS: VIII.5.3
- 작업명: token dependency graph를 구축하고 cycle을 저장 전 거부
- 상태: 완료 (`[x]`)
- 선행: VIII.5.1(`[x]`, 이 lane), VIII.2.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/token_graph.rs`(신규) — `TokenDependencyGraph`, `TokenCycle`, `TokenValidationError`,
  `validate_for_save`와 테스트.
- `crates/bindings/src/lib.rs` — `token_graph` 모듈 등록.
- `docs/task-reports/VIII.5.3.md`(신규), 계획서(VIII.5.3 `[ ]`→`[x]`).

## 구현 요약

- **graph**: registry의 모든 토큰이 node, default expression의 토큰 참조가 간선(`A → B` = A가 B에 의존)입니다.
  역방향 색인(dependents)을 함께 둡니다. registry에 없는 대상으로 가는 간선은 넣지 않고 VIII.5.2
  `reference_issues`가 따로 보고합니다.
- **cycle 검출**: 반복(비재귀) 3색 DFS, 토큰·간선 ID 순 방문 → 같은 registry는 삽입 순서와 무관하게 같은 경로
  (`[a, …, a]`, 자기 참조 `[a, a]`).
- **저장 전 거부**: `validate_for_save(registry, others)`가 cycle → 참조 문제 → default expression type 오류
  순으로 모아 하나라도 있으면 `Err`를 돌려줍니다.
- **보조 API**: `topological_order`(Kahn, 같은 단계 ID 순; cycle이면 그 cycle 오류), `transitive_dependents`,
  `cycle_if_references`(참조를 바꾸기 전에 새 cycle 경로를 계산, command 검증용, 반복 BFS).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (11 unit passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 첫 실행 `map_entry` 1건 → `Entry::Vacant`로 수정 후 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **A→B→A fixture 실패**(`two_token_cycle_is_rejected_before_save`): 경로 `[A, B, A]`, 위상 정렬도 같은 cycle 오류, `validate_for_save` 첫 오류 = cycle | 통과 |
| 자기 참조 `[a, a]`, 간접 1→2→3→1 경로가 삽입 순서와 무관하게 동일(`self_and_indirect_cycles_are_detected_deterministically`) | 통과 |
| diamond 위상 순서·transitive dependents·변경 전 cycle 예측(`1 → 4 → 2 → 1`, 자기 참조)·무cycle 변경(`acyclic_graph_orders_dependencies_first`) | 통과 |
| dangling 참조·type 오류 보고 순서(`save_validation_reports_references_and_types`) | 통과 |
| 20,000단계 chain·전체 cycle을 재귀 없이 처리(`long_chain_is_handled_without_recursion`) | 통과 |

## 차이(작업 지시와 다른 점)

- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).
- "저장 전"은 저장 경로(storage) 연결 없이 검증 함수로 제공합니다. 실제 저장 호출부 연결은 토큰 저장 형식이 정해지는
  후속 작업 소관입니다.

## 남은 위험 / 한계

- 토큰↔속성처럼 종류가 섞인 cycle은 통합 binding graph(VIII.6.3)에서 검사합니다. 이 graph는 토큰 간 간선만 봅니다.
