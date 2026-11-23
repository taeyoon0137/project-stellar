# 작업 보고서: XXX.7.9 provision 값 override

- WBS: XXX.7.9
- 작업명: View Variant와 Target Profile override가 provision 값을 PropertyPath로 덮어쓰도록 resolver 연결.
- 상태: 완료 (`[x]` 후보)
- 결정: ADR-0099 결정 9, ADR-0025 결정 3.6
- 선행: XXX.7.4, IX.7.6, IX.10.7

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/document/src/context/overrides.rs`(신규): 공통값, variant 승자 값·충돌 추출, override를 넣은 set.
- `crates/ops/src/context.rs`(신규), `crates/ops/src/lib.rs`(module 등록): `resolve_context_overrides`.

## 구현 요약

- provision 값 경로 `(provision StableId, [0xEC10])`. 리터럴 provision이 공통값이고, 우선순위는 `resolve_for_profile`의 기존 규칙을
  그대로 씁니다(새 우선순위 코드 없음). 결과 값을 provision 값 식(리터럴)으로 넣은 set으로 해석기를 돌립니다.
- 같은 구체성 Variant 충돌은 variant resolver의 충돌을 provision 단위 진단으로 돌려주고 Variant 값을 적용하지 않습니다.
- key 타입과 다른 override 값은 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib context::` | 통과(2개) |

시험:

- `context::tests::provision_overrides_follow_common_profile_variant_priority`: 공통값 → profile → variant → 더 구체적인 variant →
  profile × variant, Card 안 같은 key 재읽기가 override 값을 씀.
- `context::tests::same_specificity_variant_conflict_is_reported_and_not_applied`: 같은 구체성 충돌 진단, 타입 불일치 거부.

## 차이(작업 지시와 다른 점)

- 비리터럴 provision 값에도 override는 적용되지만 공통값(Base)은 리터럴 provision만 가집니다(비리터럴 값은 평가 시점 값이라
  공통값 표에 두지 않음).

## 남은 위험 / 한계

- View Variant·profile override 원본을 문서에서 읽어 이 함수에 넘기는 제품 경로(runtime·export)는 없습니다.
