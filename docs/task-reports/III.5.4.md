# 작업 보고서: III.5.4 OrderKey Semantic Merge fixture

- WBS: III.5.4
- 작업명: 독립 삽입이 충돌 없이 합쳐지는 OrderKey merge fixture data + 스키마 검증
- 상태: 완료 (`[x]`)
- 선행: III.5.2(OrderKey 알고리즘)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/order_key_merge_fixtures.rs` (신규: fixture 타입·data·스키마 검증·테스트)
- `crates/core-types/src/lib.rs` (`pub mod order_key_merge_fixtures;`)
- `docs/task-reports/III.5.4.md`(신규), 계획서(III.5.4 `[ ]`→`[x]`)

## 구현 요약

- 독립 삽입 merge **fixture data**와 **스키마(구조) 검증**만 제공합니다. Semantic Merge
  알고리즘은 구현하지 않습니다(후속 Phase).
- `OrderKeyMergeFixture { base, branch_a_inserts, branch_b_inserts, expected_merged }`와
  `validate()`(스키마 검증)를 정의: 두 브랜치 모두 삽입 존재, ID 유일, 키 정규형,
  `expected_merged`가 전체 ID 순열이며 결정론적 `(key, id)` 오름차순과 일치.
- fixture 2종:
  - `same_gap_deterministic_key_tie_break`: 두 브랜치가 같은 이웃 사이에 독립 삽입 →
    `between`이 결정론적이라 **동일 키**, 충돌은 `StableId` tie-break으로 해소.
  - `disjoint_front_and_back_inserts`: 한 브랜치는 앞, 다른 브랜치는 뒤에 삽입 → 서로 다른
    키, 충돌 없음.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-core-types` (114 unit + 8 doctest) | 0 |
| `cargo fmt -p stellar-core-types -- --check` | 0 |
| `cargo clippy -p stellar-core-types --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| 모든 fixture 스키마 검증 통과 | 통과 |
| 동일 키 충돌이 ID 오름차순으로 해소되는 fixture 구조 | 통과 |
| 잘못된 fixture(빈 브랜치·기대 순서 역전·비순열) 검증 실패 | 통과 |

## 차이(작업 지시와 다른 점)

- **배치**: fixture는 `OrderKey`·`StableId`에 의존하지만, 의존성 그래프상 `semantic-merge`는
  `semantic-diff`에만 의존(xtask `crate_graph` 검증)하여 `core-types`를 직접 쓸 수 없습니다.
  그래프 변경은 더 큰 결정이므로, fixture를 `OrderKey`가 있는 `core-types`에 재사용 가능한
  `pub` 모듈로 두었습니다. merge 구현 시 `semantic-merge`가 소비합니다.
- **스키마 검증**: 외부 스키마 파일 대신 Rust 타입을 스키마로 보고 `validate()`로 구조
  일관성을 검사합니다(core-types는 std 전용이라 외부 직렬화 의존성 없음). merge 알고리즘은
  실행하지 않고 fixture data의 내부 일관성만 확인합니다.

## 남은 위험 / 한계

- fixture data와 구조 검증만 존재합니다. 실제 Semantic Merge 알고리즘은 후속 Phase(XIV)에서
  구현하며, 그때 본 fixture를 회귀 기준으로 사용합니다.
- 배치를 `core-types`로 둔 것은 의존성 그래프 제약에 따른 잠정 선택입니다. 향후
  `semantic-merge`가 `core-types`를 의존하게 되면 fixture를 그쪽으로 이전할 수 있습니다.
