# 작업 보고서: XXX.7.3 ReferenceTarget Context tag와 binding dependency graph

- WBS: XXX.7.3
- 작업명: `ReferenceTarget::Context`(IX.12.1 예약 tag)와 ContextKey·ContextProvision 공통 graph node.
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXX.7.2이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]` 후보)
- 결정: ADR-0099 결정 3
- 선행: XXX.7.2, VIII.6.2

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/expressions/src/{ast,codec}.rs`: `ReferenceTarget::Context(StableId)`, tag `0x05`, codec 읽기·쓰기.
- `crates/bindings/src/graph.rs`: `DependencyNode::ContextKey`·`ContextProvision`, `DependencyKind`·`Producer` variant,
  `to_target`이 provision node에서 `None`.
- `crates/bindings/src/scheduler.rs`: producer별 Context 경로표, `from_specs_with_context`, `reference_node`.
- `crates/bindings/src/provenance.rs`: 읽은 node를 경로표로 해석.
- `crates/bindings/src/{context,context_scheduler,context_tests}.rs`(신규), `crates/bindings/src/orphan.rs`(Context 참조의
  `unresolved`는 `None`).
- `crates/document/src/css/mod.rs`: IX.12.1 예약 비교 시험의 exhaustive match 갱신.

## 구현 요약

- 표현식의 `Context(K)`는 소비 지점마다 다른 값이라, 해석 결과를 producer별 경로표(key → provision 값 node 또는 key default
  node)로 받습니다. 의존 간선·평가·타입 조회·provenance가 모두 같은 경로표를 씁니다.
- key default(`Producer::ContextDefault`)와 provision 값(`Producer::ContextProvision`, 인스턴스 chain 포함 식별)이 공통 node입니다.
- 기존 tag 0x01\~0x04와 AST·binding bytes는 바뀌지 않았습니다(기존 golden 시험 통과).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-expressions` | 통과(기존 golden corpus 포함) |
| `cargo test -p stellar-bindings` | 통과 |

시험:

- `codec::ast_tests::context_reference_uses_reserved_tag_and_round_trips`: payload `0x05 + key 16 bytes`.
- `context::tests::context_nodes_are_common_graph_nodes_and_consumers_follow_routes`: dependency 종류, 같은 key 재읽기 바깥 값,
  경로표 없는 graph의 default 연결.
- `context::tests::mutual_context_reads_are_rejected_as_a_cycle`: K1↔K2.

## 차이(작업 지시와 다른 점)

- `DependencyNode::to_target`의 반환이 `Option`이 되었습니다(provision 값 node는 표현식이 직접 가리키지 않음).

## 남은 위험 / 한계

- 정의 tree node를 target으로 하는 binding은 binding graph에서 인스턴스마다 다른 값을 갖지 못합니다(기존 graph는 속성 경로당
  값 하나). 해석기·provision node는 인스턴스 chain별이지만 소비 binding은 binding당 site 하나입니다.
- 문서 해석 결과를 `ContextGraph`로 옮기는 변환은 지금 종단 시험(`tests/integration/tests/context_common`)에만 있습니다.
  제품 경로(runtime 평가)에 binding graph가 아직 없기 때문입니다.
