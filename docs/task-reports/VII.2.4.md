# 작업 보고서: VII.2.4 SetProperty

- WBS: VII.2.4
- 작업명: `SetProperty` — metadata registry로 type과 invalidation 검사
- 상태: 완료 (`[x]`)
- 선행: VII.1.4, VI.3.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/property.rs`(신규) — `PropertyValueType`, `PropertyValue`(tag 직렬화), `PropertyInfo`,
  `node_property_info`(registry 조회), `SetProperty`(validate/invert/apply/codec), 테스트 3건.
- `crates/commands/src/command.rs` — `Command::SetProperty`, `CommandError::{UnknownProperty, PropertyTypeMismatch,
  PropertyNotSettable}`.
- `crates/commands/src/kind.rs`, `crates/commands/src/codec.rs` — `SetProperty = 0x0010`, f32 LE writer/reader(비유한
  거부), VII.2.1에서 남겨 둔 임시 `allow(dead_code)` 제거.
- `crates/commands/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VII.2.4.md`(신규), 계획서(VII.2.4 `[ ]`→`[x]`).

## 구현 요약

- **registry**: `node_property_info(kind, key)`가 설정 가능한 속성의 값 타입과 무효화 class를 반환합니다.
  - 공통 필드(모든 종류): `VISIBLE`·`LOCKED`(Bool), `DISPLAY_NAME`(Text). 무효화는 VII.1.4의 공통 필드 규칙.
  - Frame: VI.3.5 `FRAME_PROPERTIES`의 7개 속성. 값 타입은 `FramePropertyType`을 1:1 대응, 무효화는 registry의
    layout/paint를 `InvalidationSet`으로 옮긴 값.
  - `PARENT`·`ORDER_KEY`는 `PropertyNotSettable`(MoveNode 전용). 그 밖은 `UnknownProperty`.
- **검증**: registry에 없는 key·node 종류에 없는 key 거부, 값 타입 불일치 `PropertyTypeMismatch { expected, found }`,
  clip 대상이 문서에 있으면 Path여야 함.
- **apply**: 레코드를 읽어 해당 필드를 바꾼 뒤 `replace_record` 한 경로(core patch)로 씁니다 — 이름·Frame payload는 O(1)
  primitive, 표시·잠금은 subtree 재삽입 경로. change set은 `(node, key)` 속성과 registry 무효화입니다.
- **inverse**: 현재 값으로의 `SetProperty`.
- **값 직렬화**: tag `1..=9` + 값. f32는 LE bit 그대로이며 디코딩은 비유한 값과 각 타입 생성자 검증(음수 size·radius 등)을
  거칩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 42 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Frame 7개 + 공통 3개 속성 각각 설정 → 값·경로·registry 무효화 일치, 자손 유지, inverse 후 snapshot 동일(`every_registered_frame_property_round_trips_with_registry_invalidation`) | 통과 |
| 미등록 key, Text node에 Frame key, 타입 불일치(Size←Bool, Visible←Text), PARENT 설정, clip 종류 불일치, 없는 node 거부·문서 불변(`wrong_property_key_or_type_is_rejected`) | 통과 |
| 값 타입 9종 + 변형 5종 codec round-trip, kind 0x0010, 잘못된 값 tag 거부(`set_property_codec_round_trips_every_value_type`) | 통과 |

## 차이(작업 지시와 다른 점)

- Shape·Path·Text·Image payload는 document crate에 속성 metadata registry가 아직 없어 이번 registry에 포함하지 않았습니다
  (해당 key는 `UnknownProperty`). registry가 생기면 `node_property_info`에 같은 방식으로 추가합니다.

## 남은 위험 / 한계

- 표시·잠금 변경은 `DocumentModel` 제약으로 subtree 재삽입 비용(O(subtree × N))이 듭니다.
