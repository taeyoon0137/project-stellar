# 작업 보고서: IX.2.3 공개 Property만 override 가능한 validator

- WBS: IX.2.3
- 작업명: 인스턴스에서 공개 Property만 override 가능하게 validator 구현, private·nested property 직접 접근 거부
- 상태: 완료 (`[x]`)
- 선행: IX.1.2(이 lane에서 `[x]`), IX.2.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/override_rules.rs`(신규) — `PropertyOverrideError`, `check_override`,
  `check_payload_overrides`, `validate_scene_property_overrides`, `validate_definition_property_overrides`,
  테스트 공용 Button·Card fixture.
- `crates/document/src/component/definition.rs` — `ComponentDefinitionError::PropertyOverride`, library 디코딩이
  정의 tree 안 nested 인스턴스 override를 검사.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.2.3.md`(신규), 계획서(IX.2.3 `[ ]`→`[x]`).

## 구현 요약

- **규칙**(key 오름차순, 첫 위반 반환):
  1. 경로가 있는 `OverrideKey`(nested instance chain을 거치는 직접 접근) → `NestedAccess`. 공개 property여도
     거부합니다 — nested 값은 상위 정의의 Property와 명시적 drilling으로만 바꿉니다(기술 아키텍처 14.3).
  2. 정의에 선언·record가 없는 property → `UnknownProperty`.
  3. private property → `PrivateProperty`.
  4. 값 타입이 선언 타입과 다름 → `ValueTypeMismatch { expected, found }`.
- **적용 위치**: scene 인스턴스(`validate_scene_property_overrides`)와 정의 tree 안 nested 인스턴스
  (`validate_definition_property_overrides`). library 디코딩은 후자를 모든 컴포넌트에 실행하며, 참조
  컴포넌트가 library에 없는 nested 인스턴스는 slot 투영 검사와 같은 규칙으로 건너뜁니다(III.2.4). scene 검사는
  미해결 참조를 `UnresolvedComponent`로 구분해 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 258 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Card 인스턴스가 정의 안 nested Button의 공개 label을 경로로 직접 override → `NestedAccess`(`direct_nested_access_fixture_fails`) | 통과(거부) |
| 다른 컴포넌트 property·타입 불일치 거부, 정의 안 nested 인스턴스의 private override 거부·public override 허용(`private_unknown_and_mistyped_overrides_fail`) | 통과 |
| 공개 direct override·빈 override 통과(`public_direct_override_is_accepted`) | 통과 |
| 정의 안 nested 경로 key를 가진 library 디코딩 거부, 유효 library round-trip, 미해결 컴포넌트 구분(`library_decode_rejects_invalid_definition_overrides`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- scene 전체(모든 인스턴스)를 순회하는 문서 수준 연결은 component 계층이 document validator(VI.10.1)에
  합류할 때 연결합니다. 현재는 인스턴스·컴포넌트 단위 API입니다.
