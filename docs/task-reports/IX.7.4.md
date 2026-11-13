# 작업 보고서: IX.7.4 일치 Variant resolver

- WBS: IX.7.4
- 작업명: 현재 변수 assignment와 일치하는 모든 variant를 찾고 specificity 순으로 정렬하는 resolver
- 상태: 완료 (`[x]`)
- 선행: IX.7.2(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/view_variant/resolver.rs`(신규) — `ViewVariantId`, `ViewVariant`(조건 + IX.7.3
  `VariantOverrideSet`), `matching_variants`, 16장 예제 fixture.
- `crates/document/src/view_variant/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.7.4.md`(신규), 계획서(IX.7.4 `[ ]`→`[x]`).

## 구현 요약

- `matching_variants(variants, assignment, variables)`는 조건이 할당(없는 변수는 default)과 맞는 Variant를 모두
  고르고, **구체성 오름차순**(`Equals` term 수), 같은 구체성은 **Variant StableId 오름차순**으로 정렬합니다. 이
  순서가 적용 순서입니다: `Base → 일반 Variant → 더 구체적인 조합 Variant`(기술 아키텍처 16장).
- 정렬 key가 (구체성, id)뿐이라 입력 순서와 무관하게 결과가 같습니다. 같은 구체성끼리의 충돌 판정은 IX.7.5가
  하며, id 순서는 결정적 나열을 위한 것이지 우선순위가 아닙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 309 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| compact+error 할당 → [Any(0), compact(1), error(1), compact+error(2)], regular+error → [Any, error], 입력 역순에도 같은 결과, dark=true면 같은 구체성 안에서 id 순(`general_and_specific_variants_are_ordered`) | 통과 |

## 차이(작업 지시와 다른 점)

- 구체성은 `Equals` term 수로 정했습니다(조건 집합의 포함 관계가 아닌 개수). 같은 개수의 서로 다른 조합은 같은
  구체성이며, 같은 속성을 다르게 지정하면 IX.7.5 충돌입니다.

## 남은 위험 / 한계

- Variant 목록의 저장·편집은 IX.7.8 command가 맡습니다.
