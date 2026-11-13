# 작업 보고서: VIII.6.2 통합 binding dependency graph

- WBS: VIII.6.2
- 작업명: binding dependency graph가 token, property, component property, environment input을 공통 node로 표현
- 상태: 완료 (`[x]`)
- 선행: VIII.6.1(`[x]`, 이 lane), VIII.5.3(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/graph.rs`(신규) — `DependencyNode`, `DependencyKind`, `Producer`, `NodeIndex`, `ProducerRecord`,
  `DependencyGraph`(`build`, `from_records`), `GraphBuildError`와 테스트.
- `crates/bindings/src/binding.rs` — `BindingSet`(ID·target 색인, 속성 하나에 binding 하나, 같은 target·owner 교체),
  `BindingSetError`와 테스트 1개.
- `crates/bindings/src/lib.rs` — `graph` 모듈 등록.
- `docs/task-reports/VIII.6.2.md`(신규), 계획서(VIII.6.2 `[ ]`→`[x]`).

## 구현 요약

- **공통 node**: `DependencyNode = Token | Property(PropertyPath) | ComponentProperty(StableId) | Environment(key)`.
  표현식 참조 대상(`ReferenceTarget`)과 1:1로 왕복 변환합니다. 간선 `a → b` = a가 b를 읽음.
- **producer**: 토큰 node는 토큰 정의, 속성 node는 그 속성을 target으로 하는 binding이 값을 만듭니다. binding 없는 속성,
  Component Property, 환경 입력은 외부에서 값이 주어지는 source node입니다. 한 node에 producer가 둘이면
  `from_records`가 거부합니다(토큰 ID·binding target 유일성 덕분에 `build`에서는 생기지 않음).
- **결정론·규모**: node는 정렬 순서로 번호(`NodeIndex`)를 받고 인접 목록(정방향·역방향)은 번호 오름차순 `Vec`입니다.
  입력 순서와 무관하게 같은 graph가 되며, index 기반이라 VIII.9.1 대규모 graph에서도 순회 비용이 작습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 첫 실행 1 실패(테스트 기대 간선 수 8을 잘못 셈, 실제 7) → 기대값 수정 후 종료 코드 0 (25 unit passed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **dependency 종류별 fixture**(`every_dependency_kind_is_a_common_node`): 토큰→토큰·환경, 속성→토큰, 속성→속성(binding 있는 속성·source 속성), 속성→Component Property·환경; 환경 입력 역방향 dependents; producer 유무; node 종류 순서·간선 7개 | 통과 |
| 입력 순서를 뒤집어도 같은 graph, 인접 목록 오름차순, producer 중복 거부, node↔참조 대상 왕복(`graph_is_deterministic_and_rejects_duplicate_producers`) | 통과 |
| BindingSet: ID·target 중복 거부, target 조회, target 변경 교체 거부, 교체·제거(`binding_set_keeps_one_binding_per_target`) | 통과 |

## 차이(작업 지시와 다른 점)

- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).
- binding 모음(`BindingSet`)은 행에 명시되지 않았지만 graph 입력으로 필요해 같은 커밋에 추가했습니다.

## 남은 위험 / 한계

- Component Property 값의 producer(instance override·default, IX.1.x)는 Phase IX 소관이라 현재는 source node입니다.
- graph는 변경마다 전체를 다시 만듭니다(증분 갱신은 후속).
