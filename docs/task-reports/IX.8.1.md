# 작업 보고서: IX.8.1 AccessibilityNode 정의

- WBS: IX.8.1
- 작업명: `AccessibilityNode` — source StableId, role, label binding, value binding, state, action, reading order와 canonical round-trip
- 상태: 완료 (`[x]`)
- 선행: IX.6.1, VIII.6.1 (둘 다 base에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/accessibility/src/lib.rs` — 모듈 선언(`codec`, `node`, 테스트 전용 `test_support`).
- `crates/accessibility/src/codec.rs`(신규) — LE canonical 도우미, 할당 전 개수 상한 검사 `Reader`, `CodecError`.
- `crates/accessibility/src/node.rs`(신규) — `AccessibilityNode`, `AccessibilityRole`, `StateKind`, `ActionKind`,
  `ActionTarget`, `AccessibilityBinding`, binary 형식.
- `crates/accessibility/src/test_support.rs`(신규, `#[cfg(test)]`) — 결정론 StableId fixture.
- `docs/task-reports/IX.8.1.md`(신규), 계획서(IX.8.1 `[ ]`→`[x]`).

## 구현 요약

- **node**: `source`(정확히 하나의 시각 node StableId), `role`, `label`·`value`(`Option<AccessibilityBinding>`),
  `states`(`StateKind` → binding 희소 map), `actions`(`ActionKind` → `ActionTarget` 희소 map),
  `reading_order`(`Option<u32>`, 형제 사이 명시 순서).
- **binding**: 상수(`TypedValue`), 속성 경로(`PropertyPath` — VIII.6.1 `ValueBinding`의 target이 될 수 있는 경로),
  컴포넌트 Property, 상태 변수, node 텍스트 내용, 선택 상태 일치(`SelectionMatches`). 값 계산은 binding
  evaluator 몫이고 이 모델은 출처만 기록합니다.
- **역할·상태·동작**: 닫힌 enum이며 tag는 저장 형식 계약입니다(역할 11종, 상태 9종, 동작 5종).
- **binary 형식**: `version:u16 ++ source ++ role ++ opt(label) ++ opt(value) ++ states ++ actions ++
  opt_u32(reading_order)`. 디코더는 미지 tag·잘림·state/action tag 정렬 위반·잉여 bytes를 거부하고, 개수는
  남은 입력보다 크면 할당 전에 거부하며, 재인코딩 bytes가 입력과 같을 때만 받아들입니다. `PropertyPath`
  canonical bytes(III.2.2)는 core-types에 디코더가 없어 이 crate에서 경계 검사 후 디코딩합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-accessibility` | 7 passed, 0 failed |
| `cargo fmt -p stellar-accessibility -- --check` | 통과 |
| `cargo clippy -p stellar-accessibility --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 모든 field·모든 role/state/action/binding 갈래 round-trip, 같은 node 같은 bytes(`node_round_trips_every_field`) | 통과 |
| 모든 잘린 prefix·잉여 bytes·미지 버전·미지 role tag 거부(`malformed_node_bytes_are_rejected`) | 통과 |
| state tag 정렬 위반 거부(`unsorted_or_duplicate_state_tags_are_rejected`) | 통과 |
| 잘못된 상수 TypedValue·과대 key 개수 PropertyPath 거부(`invalid_constant_and_property_path_payloads_are_rejected`) | 통과 |
| 과대 개수 할당 전 거부, primitive round-trip(`codec::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. crate 그래프(`stellar-accessibility → core-types`만 허용)를 지키기 위해 label/value binding은
  bindings crate의 `ValueBinding` 타입을 직접 쓰지 않고 그 target `PropertyPath`·StableId로 출처를 가리킵니다.

## 남은 위험 / 한계

- binding의 기대 타입(label = String, state = Boolean)은 상수 값만 정적으로 확인할 수 있습니다. 속성·상태
  변수 binding의 타입 검사는 binding evaluator와 연결하는 adapter(문서→접근성, 후속 단계)가 맡습니다.
