# 작업 보고서: XIV.3.4 StableId + PropertyPath 속성 diff

- WBS: XIV.3.4
- 작업명: property diff는 `StableId + PropertyPath`와 canonical TypedValue를 비교하고 float epsilon diff를 쓰지 않음
- 상태: 완료 (`[x]`)
- 선행: III.2.2 `[x]`, XIV.3.2 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/diff.rs` — 같은 정체성 객체의 속성 merge-join(`changed_properties`).
- `docs/task-reports/XIV.3.4.md`(신규), 계획서(XIV.3.4 `[ ]`→`[x]`).

## 구현 요약

- **단위**: 같은 StableId·같은 정체성 객체의 속성 map(`PropertyPath` 오름차순)을 merge-join해
  값이 다른 경로마다 변경 하나를 냅니다. 한쪽에만 있는 경로는 `None` ↔ 값입니다.
- **비교**: `SemanticValue::canonical_eq` — canonical bytes 동일과 같은 결과(XIV.3.1 테스트로
  고정). float은 `FiniteF32` bit 비교이고 TypedValue는 `to_bytes()` 비교입니다. epsilon을 쓰지
  않으며 `-0.0`은 생성 단계 정규화로 `0.0`과 같습니다.
- **분류**: 변경 kind는 XIV.3.2 규칙(`SemanticChange::property`)을 따릅니다 — asset 참조는
  AssetRefChange, token은 ChangeToken, 컴포넌트 interface는 ComponentInterfaceChange.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | 26 passed, 0 failed |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| Frame size 320.0 → 다음 표현 가능 f32(차이 < 1e-4): SetProperty 1개, old/new 정확(`bit_different_finite_float_is_detected`) | 통과 |
| 같은 객체 clipping·background 독립 경로 변경, `None`↔값(`property_paths_are_compared_independently`) | 통과 |
| transform `-0.0` 입력은 변경 아님(`negative_zero_is_not_a_change`) | 통과 |
| asset 교체 → AssetRefChange, 이름 → SetProperty, token 값 → ChangeToken(`property_kinds_follow_classification`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Path·Text payload는 한 속성(canonical bytes)이라 그 안의 point 하나·글자 하나 변경도 속성
  변경 하나로 보입니다(XIV.3.1 결정 — style run offset과 point id가 묶여 있음).
