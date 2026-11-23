# 작업 보고서: XXII.1.4 코드 생성용 capability matrix 확장

## WBS

- WBS: XXII.1.4
- 작업명: IX.11.19 capability matrix와 IX.11.18 modifier stack 선형화 계획을 코드 생성용으로 확장(Apple·Android
  profile 상태와 lowering 규칙)
- 상태: 완료 보고(`reported_done`). 선행 IX.11.18·IX.11.19가 `[~]`라 원장 판정은 통합 때 정합니다.
- 결정: ADR-0106 결정 3, ADR-0025 결정 1.3·4.2
- 선행: XXII.1.2, IX.11.6, IX.11.18, IX.11.19

## 변경 파일

- `crates/codegen/src/matrix/css.rs`(신규): 닫힌 규칙 집합 `Rule`(24개, 규칙별 짝 진단), registry 항목별 Apple·Android
  표 `CSS_LOWERINGS`(120항목), 규칙 적용 함수 `apply`.
- `crates/codegen/src/matrix/mod.rs`: `css_entry`의 Apple·Android 분기, 표 golden 텍스트 `css_matrix_text`.
- `crates/codegen/src/matrix/rule_tests.rs`(신규), `crates/codegen/tests/fixtures/{matrix_rules,matrix_css}.txt`(신규).
- `crates/ops/src/codegen.rs`(신규): 문서 registry·layout longhand → `CssPropertyUse`, 완전성 검사.
- `crates/ops/src/lib.rs`: module 등록.
- `crates/codegen/tests/fixtures/{ir_ios,ir_android}.snapshot`, `mappings_{ios,android}.txt`: Apple·Android 판정 반영.

## 구현 요약

- Web 항목은 `css_matrix::web_capability`를 그대로 부르고 다시 정의하지 않습니다.
- Apple·Android 상태는 CSS 매핑 상태에서 유도하지 않습니다. 표의 규칙이 정하고, 같은 항목은 CSS 매핑 상태가 무엇이든
  같은 결과입니다(test로 확인).
- 1:1이 아닌 항목은 규칙과 진단을 한 쌍으로 둡니다.
  - modifier 적용 순서: `modifier-chain`(Native, `modifier-chain-order`). SwiftUI modifier·Compose `Modifier` chain은 선언
    순서로 감싸므로 stack 순서를 그대로 지킵니다. Web의 고정 순서 선형화는 IX.11.18 계획을 그대로 씁니다.
  - 블렌드·opacity·blur·shadow 합성 색공간: `srgb-composite`(Approximated).
  - 자유 배치·inset: `absolute-offset`(Approximated).
  - 기본값 차이(stack spacing·alignment·box-sizing): `explicit-default`(Native, 항상 명시 값).
  - 그 밖에 테두리 overlay, grid, flex item, mask, 줄 높이, aspect-ratio, transform, path 굽기, 래스터화, 출력 없음.
- Native가 아닌 항목은 모두 진단이 붙은 규칙을 씁니다(test로 확인).

## 명령과 결과

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen` | 통과(61 + 2) |
| `cargo test -p stellar-ops --lib codegen` | 통과(3) |
| `cargo clippy -p stellar-codegen -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

검증 기준 대응:

- 완전성 검사: `ops::codegen::tests::every_registry_item_has_apple_and_android_status_and_rule` — 문서 registry 80항목과
  layout longhand를 합친 120항목 × Apple·Android 모두 상태·규칙이 있음. 표 항목이 registry에 없는 이름을 쓰지 않음
  (`rule_table_names_only_existing_registry_items`).
- Web 항목 상태가 IX.11.19 matrix와 같음: `web_status_equals_the_ix_11_19_matrix_for_every_registry_item`,
  `matrix::rule_tests::web_entries_are_the_ix_11_19_matrix_and_platform_entries_come_from_rules`.
- 규칙별 입력·출력·진단 fixture: `matrix::rule_tests::every_rule_has_input_output_and_diagnostic_fixture`
  (`matrix_rules.txt`, 규칙 24개 모두), `rule_application_examples`.

## 남은 위험

- Apple·Android 출력 API template은 lowering 규칙의 대상 표기입니다. 실제 SwiftUI·Compose source 생성과 compile 검사는
  XXII.3.2·XXII.4.2 몫입니다.
- 상태 판정(예: `font-stretch` Android Unsupported)은 기준 SDK 문서 확인 없이 API 존재 여부로 정했습니다. adapter
  compile fixture가 생기면 다시 대조해야 합니다.
