# 작업 보고서: XXX.7.6 영향받는 소비 binding만 topological 재평가

- WBS: XXX.7.6
- 작업명: Context 해석을 바꾸는 변경에서 영향받는 소비 binding만 binding scheduler로 재평가.
- 상태: 완료 (`[x]` 후보) — 아래 한계(binding당 site 하나)가 있습니다.
- 결정: ADR-0099 결정 6
- 선행: XXX.7.4, VIII.6.4, IX.7.3

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/document/src/context/{invalidate,invalidate_tests}.rs`(신규): `ContextChange`, footprint, `affected_consumers`.
- `crates/bindings/src/context_scheduler.rs`(신규, scheduler 하위 module): `RouteChange`, `apply_context_change`.
- `tests/integration/tests/{context_common/mod.rs,context_reevaluation.rs}`(신규), `tests/integration/Cargo.toml`(bindings
  dev-dependency 한 줄), `Cargo.lock`.

## 구현 요약

- footprint = 지나간 (key, node), chain의 instance node, 쓴 provision, default로 끝난 key, 지나간 Slot(provision 값 식 읽기까지
  전이). 무효화 조건마다 footprint와 겹치는 소비만 고릅니다.
- binding scheduler는 바뀐 key·provision producer와 고른 소비의 경로표만 바꾸고 graph 간선을 다시 만든 뒤, 값은 유지한 채 바뀐
  producer에서 위상 순서로 영향받는 producer만 평가합니다. cycle이 생기면 상태를 바꾸지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib context::` | 통과 |
| `cargo test -p stellar-bindings --lib context` | 통과 |
| `cargo test -p stellar-integration-tests --test context_reevaluation` | 통과(3개) |

시험:

- `context::invalidate_tests::each_invalidation_condition_selects_only_consumers_whose_resolution_it_can_change`: 12개 조건별 영향 집합.
- `context_reevaluation`: provision 추가·삭제·값 변경, default 변경, reparent, slot 콘텐츠 삭제, Slot 공개 목록 변경, View Variant
  구조 patch 적용·해제, Fragment 묶기·풀기, instance ComponentRef 교체마다 증분 값 = 전체 재평가 값, 영향 밖 소비 평가 0,
  같은 입력 같은 평가 순서.
- `context::tests::rerouting_reevaluates_only_affected_consumers_in_topological_order`(bindings).

## 차이(작업 지시와 다른 점)

- View Variant 구조 patch와 Fragment 묶기는 그 결과와 같은 모델 변경(node 이동·삽입)으로 fixture를 만들었습니다. variant
  resolver의 구조 patch를 scene에 적용하는 경로는 아직 없습니다(IX.7 쪽 몫).

## 남은 위험 / 한계

- 정의 node를 target으로 하는 binding의 인스턴스별 재평가는 binding graph가 binding당 값 하나라 할 수 없습니다(XXX.7.3 한계).
- graph 간선 재구성 비용은 graph 크기에 비례합니다. 평가 수는 영향 범위에 비례합니다.
