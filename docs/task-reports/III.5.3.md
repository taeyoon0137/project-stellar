# 작업 보고서: III.5.3 OrderKey 부모 단위 재균형

- WBS: III.5.3
- 작업명: 임계 길이 초과 시 부모 자식 키 균등 재배치(시각 순서 보존, 결정론)
- 상태: 완료 (`[x]`)
- 선행: III.5.2(OrderKey 알고리즘)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/order_key.rs` (`OrderKey::evenly_spaced`, `rebalance` 함수, 재균형 테스트)
- `docs/task-reports/III.5.3.md`(신규), 계획서(III.5.3 `[ ]`→`[x]`)

## 구현 요약

- `OrderKey::evenly_spaced(count)`: `(0,1)` 구간에 균등 분포한 `count`개 키를 오름차순 생성.
  키 `i`는 소수 `(i+1)/(count+1)`의 base-256 전개를 정규화한 값으로, 결정론적·strictly
  increasing이며 짧습니다(난수 미사용). 자릿수 `d`는 `256^d > count+1`을 만족하는 최소값이라
  현실적 자식 수에서 항상 임계 길이 이하입니다.
- `rebalance(children)`: 자식들을 현재 시각 순서(현재 키, 동일 키는 `StableId` tie-break)로
  정렬한 뒤 균등 키를 그 순서대로 재배치합니다. **시각 순서(= `StableId` 순서)를 보존**하고,
  결정론적이라 같은 입력에 두 번 실행해도 동일 결과를 냅니다. 각 자식의 `(StableId, 새 키)`를
  시각 순서대로 반환합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-core-types` (109 unit + 8 doctest) | 0 |
| `cargo fmt -p stellar-core-types -- --check` | 0 |
| `cargo clippy -p stellar-core-types --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| `evenly_spaced` strictly increasing·정규형·임계 길이 이하(count 1·2·5·255·256·1000) | 통과 |
| 재균형 전후 StableId 순서 동일 + 키 길이 회복(20·18 byte → 짧은 키) | 통과 |
| 재균형 결정론(같은 입력 2회 동일) | 통과 |
| 재균형 결과를 다시 재균형해도 동일 결과(두 번 실행 동일) | 통과 |
| 빈·단일 자식 재균형 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. `needs_rebalance`(III.5.2 제공)가 임계 길이 초과를 신호하고, 본 작업의 `rebalance`가
  부모 단위로 키를 회복합니다.

## 남은 위험 / 한계

- `rebalance`는 `(OrderKey, StableId)` 목록을 입력으로 받는 core 함수입니다. 실제 부모 노드의
  자식 컬렉션과의 연결(언제 호출할지, dirty 처리)은 document/scene 계층(후속 Phase)에서
  `needs_rebalance` 신호를 보고 수행합니다.
- `evenly_spaced`의 자릿수는 `count`에 로그적으로 증가합니다. 비현실적으로 큰 `count`(수억
  이상)에서는 키가 임계 길이에 근접할 수 있으나, 단일 부모 자식 수 범위에서는 문제없습니다.
