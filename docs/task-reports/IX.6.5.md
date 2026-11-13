# 작업 보고서: IX.6.5 List 계약

- WBS: IX.6.5
- 작업명: List contract에 collection, item identity, selection mode, navigation 의미 정의
- 상태: 완료 (`[x]`)
- 선행: IX.6.1(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/control/list.rs`(신규) — `LIST` 종류(0x0004), field 상수, contract enum 타입 ID
  (`ITEM_IDENTITY_ENUM`·`SELECTION_MODE_ENUM`·`NAVIGATION_ENUM`), `ItemIdentity`·`SelectionMode`·`Navigation`,
  `list_contract`, `validate_list`, `selection_mode`, runtime `validate_item_identities`·`node_identities`,
  `ListItemError`.
- `crates/document/src/control/mod.rs` — 모듈 등록, `FieldChecker::enum_constant`.
- `docs/task-reports/IX.6.5.md`(신규), 계획서(IX.6.5 `[ ]`→`[x]`).

## 구현 요약

- **의미 field**: `collection`(1, item container node, 필수), `itemIdentity`(2, NodeId·DataKey, 기본 NodeId),
  `selectionMode`(3, None·Single·Multiple, 기본 None), `navigation`(4, Vertical·Horizontal·Grid, 기본 Vertical),
  `selection`(5, 선택 identity를 담는 String 상태 변수).
- 열거 값은 core-types `EnumValue`(enum 타입 ID + variant)로 저장합니다. 다른 enum 타입이거나 범위 밖 variant는
  `TypeMismatch` 오류입니다.
- **item identity**: NodeId 방식은 collection 자식 StableId(구조상 유일). DataKey 방식은 runtime item 목록의 key이며
  `validate_item_identities`가 첫 중복을 `DuplicateItemIdentity { identity, first, duplicate }` 오류로 거부합니다.
- **검사**(`validate_list`): collection 누락은 진단, Frame 밖·없는 node는 오류, selection이 상태 변수가 아니면
  `NotWritable`. 상호작용은 focus 가능·`Key` 처리, 선택 방식이 None이 아니면 `SelectionChange` 처리(진단).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 298 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| item `[사과, 배, 사과]` → `DuplicateItemIdentity{사과, 0, 2}` 오류, 앞 두 개는 통과, NodeId 방식 identity 유일(`duplicate_item_identity_fixture_is_an_error`) | 통과(오류) |
| collection 누락·범위 밖 identity·다른 enum 타입 navigation·Property selection·SelectionChange 미처리 → 5건 정확(`invalid_list_fields_are_reported`) | 통과 |
| Single 선택 + DataKey + 상태 selection → 0건(`complete_list_has_no_issues`) | 통과 |

## 차이(작업 지시와 다른 점)

- core-types에 목록 값 타입이 없어 collection을 데이터 값이 아니라 item container node로 정의했습니다. 데이터
  바인딩 목록(반복 생성)은 binding 계층 확장 때 같은 field에 연결합니다.

## 남은 위험 / 한계

- contract enum 타입 ID(0x000C_0001~3)는 이 모듈이 정한 값입니다. 전역 enum 타입 registry가 생기면 그 registry에
  등록해야 합니다.
