# 작업 보고서: VIII.6.4 증분 binding scheduler

- WBS: VIII.6.4
- 작업명: source 변경 시 영향받는 binding만 topological order로 재평가하는 scheduler
- 상태: 완료 (`[x]`)
- 선행: VIII.6.2(`[x]`, 이 lane), VIII.4.6(`[~]`, Windows 비교만 ADR-0006으로 보류된 부분 완료 — 충족으로 봄)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/scheduler.rs`(신규) — `BindingEvaluator`, `SourceInputs`, `StaticSources`, `NodeEvalError`,
  `NodeValue`, `SchedulerError`, `UpdateReport`와 테스트.
- `crates/bindings/src/lib.rs` — `scheduler` 모듈 등록.
- `docs/task-reports/VIII.6.4.md`(신규), 계획서(VIII.6.4 `[ ]`→`[x]`).

## 구현 요약

- **값 저장소**: 통합 graph의 node마다 값(`Result<Option<TypedValue>, NodeEvalError>`)을 둡니다. source node 값은
  `SourceInputs`·`update_sources`로 주어지고, producer(토큰·binding target 속성)는 표현식을 bytecode로 평가합니다
  (VIII.4 평가 한도 적용).
- **생성**: graph 구성 → 비순환 확인(VIII.6.3) → 전체 위상 순위(Kahn, 같은 단계 node 번호 순) 1회 계산 → 컴파일 →
  전체 평가.
- **증분 재평가**(`update_sources`, `update_expression`):
  1. 바뀐 node에서 역방향 BFS로 영향받는 producer(**affected**)와 순회 node 수(**visited**)를 구합니다.
  2. affected를 위상 순위로 정렬해, 읽는 node 중 이번 갱신에서 값이 바뀐 것이 있는 producer만 평가합니다
     (**evaluated**, early cutoff). 방문 기록은 영향 범위 크기의 집합이라 비용이 graph 전체가 아니라 영향 범위에
     비례합니다.
  - 값이 같은 source 변경은 무시하고, dependency가 바뀌는 표현식 교체는 `StructureChanged`(graph 재구성 필요)입니다.
- **오류**: source 값 없음, 컴파일 실패, 평가 실패, 실제로 읽은 node가 오류인 경우 `Dependency`(short-circuit 유지).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (32 unit passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 첫 실행 테스트 `type_complexity` 1건 → `StaticSources` 별칭 도입 후 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **무관 subtree 평가 count 0**(`unrelated_subtree_is_not_evaluated_and_order_is_deterministic`): 두 독립 subtree에서 e1 변경 시 평가 순서 `[t2, p1, p2]`, affected 3·evaluated 3·visited 4, 오른쪽 subtree(p3, p4) 평가 0, 값 갱신 정확 | 통과 |
| **순서 deterministic**: 새 evaluator에서 같은 변경 → `UpdateReport` 동일; 같은 값 재설정은 평가 0 | 통과 |
| 초기 평가 값·위상 순서(모든 간선에서 dependency 순위가 앞)(`initial_evaluation_follows_topological_order`) | 통과 |
| 토큰 표현식 교체 시 dependents만 평가, 같은 값 교체는 early cutoff(affected 4, evaluated 1), 구조 변경·잘못된 대상 거부(`token_expression_change_reevaluates_dependents_with_cutoff`) | 통과 |
| source 타입 없음 → 컴파일 오류·dependency 오류 전파, 모르는 node, cycle graph 생성 거부(`missing_sources_errors_and_cycles`) | 통과 |

## 차이(작업 지시와 다른 점)

- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).
- dependency 구조가 바뀌는 편집(참조 추가·삭제)은 증분 처리하지 않고 재구성을 요구합니다.

## 남은 위험 / 한계

- 생성 시 graph 전체 구성·컴파일·평가 비용은 graph 크기에 비례합니다. 대규모 측정은 VIII.9.2에서 합니다.
- 참조 값 조회가 `DependencyNode` → 번호 BTreeMap 조회를 거치며, 속성 경로 참조는 조회마다 경로를 복제합니다.
