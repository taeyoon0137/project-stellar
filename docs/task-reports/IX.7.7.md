# 작업 보고서: IX.7.7 Variant 구조 override의 인스턴스 캡슐화 연결

- WBS: IX.7.7
- 작업명: variant structure override도 ComponentInstance 내부 캡슐화를 침범하지 못하게 validator 연결
- 상태: 완료 (`[x]`)
- 선행: IX.7.3(`[x]`), IX.2.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/view_variant/encapsulation.rs`(신규) — `validate_variant_encapsulation`,
  `VariantEncapsulationError`.
- `crates/document/src/view_variant/mod.rs` — 모듈 등록.
- `crates/document/src/component/slot_override.rs` — `validate_record_slot_overrides` 가시성을 `pub(super)`에서
  `pub(crate)`로(Variant 검사가 같은 투영 규칙을 재사용).
- `docs/task-reports/IX.7.7.md`(신규), 계획서(IX.7.7 `[ ]`→`[x]`).

## 구현 요약

`validate_variant_encapsulation(set, base, view_root, library)`는 IX.7.3 `VariantOverrideSet`에 인스턴스 캡슐화
validator를 연결합니다(참조 무결성은 기존 `validate_references`의 몫).

1. **property patch**: IX.4.2 `classify_property_path`로 분류해 정의 tree node root 경로(인스턴스 내부 직접 접근),
   nested override 경로, 컴포넌트 참조·slot override key 경로를 거부합니다. 인스턴스 override 경로는 IX.2.3
   `check_override`(공개·선언·값 타입)를 통과해야 합니다.
2. **구조 patch**: Add의 부모, Move의 대상·부모, Delete·Replace의 대상이 컴포넌트 정의 tree node(scene에 없고
   library가 소유)면 `DefinitionInternalStructure`로 거부합니다.
3. **적용 후 slot 투영**: 구조 patch와 slot change를 적용한 최종 구조를 만들고, 살아 있는 모든 ComponentInstance의
   소유 node(최종 부모 = 인스턴스)가 IX.3.2 투영 규칙을 지키는지 검사합니다. 미해결 컴포넌트 인스턴스는 건너뜁니다.

일반 레이어 편집(일반 node 속성·구조 변경, 인스턴스 node 자체의 삭제)과 공개 property override는 허용됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 313 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| nested Button node 속성 patch, 이어 붙인 nested override 경로, nested node 삭제·이동·그 아래 추가, 인스턴스 아래 비투영 node 추가, 컴포넌트 참조 key patch, 타입이 틀린 공개 property patch 전부 거부(`nested_direct_override_fixtures_fail`) | 통과(거부) |
| 공개 property override + 일반 레이어 속성·구조 변경, 인스턴스 node 삭제 허용(`property_drilling_and_general_layer_edits_are_allowed`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- drilling binding(IX.4.1·IX.4.3~IX.4.6, 선행 미완으로 차단)이 생기면 Variant가 drilling된 상위 Property를 바꾸는
  경로도 같은 분류를 거칩니다(별도 연결 필요 없음).
