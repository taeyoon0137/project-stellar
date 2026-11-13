# 작업 보고서: VIII.6.3 binding cycle detector

- WBS: VIII.6.3
- 작업명: binding cycle detector — Property A↔B, token↔property cycle을 모두 탐지
- 상태: 완료 (`[x]`)
- 선행: VIII.6.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/graph.rs` — `BindingCycle`, `DependencyGraph::find_cycle`·`ensure_acyclic`·
  `cycle_if_dependencies`와 테스트 3개.
- `docs/task-reports/VIII.6.3.md`(신규), 계획서(VIII.6.3 `[ ]`→`[x]`).

## 구현 요약

- **검출**: 통합 graph(VIII.6.2) 전체에 반복(비재귀) 3색 DFS. node·간선을 번호(= node 정렬) 순으로 방문해 같은
  graph는 같은 경로(`[a, …, a]`, 자기 참조 `[a, a]`)를 보고합니다. node 종류와 무관하게 같은 방식이라 토큰·속성·
  Component Property·환경 입력이 섞인 cycle도 찾습니다(환경·Component Property는 읽기만 하는 source라 cycle에 들지
  않습니다).
- **거부**: `ensure_acyclic()`이 cycle을 오류로 돌려줍니다(적용·저장 전 거부용).
- **변경 전 예측**: `cycle_if_dependencies(node, new_deps)`가 바꾸기 전 graph에서 새 dependency가 `node`로 돌아오는지
  반복 BFS로 확인해 생길 cycle 경로를 줍니다(binding command VIII.6.6에서 사용).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (28 unit passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 첫 실행 테스트 코드 lint 3종(`cloned_ref_to_slice_refs` 등, `manual_is_multiple_of`) → 테스트 helper 수정 후 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **Property A↔B**(`property_and_token_property_cycles_are_detected`): `[A, B, A]` 오류 | 통과 |
| **token↔property**: 토큰 T가 속성 A를 읽고 A의 binding이 T를 읽으면 `[T, A, T]`; 자기 참조 속성 `[A, A]` | 통과 |
| **mixed cycle fixture 실패**(`mixed_cycle_fixture_fails_and_acyclic_passes`): p1→t1→t2→p2→p1(환경·CP는 cycle 밖)에서 `[t1, t2, p2, p1, t1]`; VIII.6.2 fixture는 비순환; 변경 전 예측 3종(속성↔속성, 토큰↔속성 생성, 무관 변경 없음) | 통과 |
| 50,000단계 토큰·속성 교대 chain과 닫힌 cycle을 재귀 없이 처리(`long_mixed_chain_is_checked_without_recursion`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. (토큰이 속성을 참조하는 것은 토큰 모델이 참조 종류를 제한하지 않아(VIII.5.1) 실제로 만들 수 있는 구조이며,
  이 detector가 그 cycle을 잡습니다.)

## 남은 위험 / 한계

- graph 전체를 매번 검사합니다. 편집 단위 검사는 `cycle_if_dependencies`를 쓰되 graph 자체는 전체 재구성입니다.
