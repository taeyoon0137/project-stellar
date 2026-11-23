# 작업 보고서: IX.11.22 합성 PropertyKey의 longhand key 분할

- WBS: IX.11.22
- 작업명: Frame `size`·`corner_radius`, modifier Shadow offset 합성 key를 longhand key로 나누고 옛 key를 묶음 key로 남김
- 상태: 부분 완료 (`[~]`) — registry·command·묶음 key 진단·decode 동등성은 구현·검증했습니다. semantic snapshot의
  Shadow offset 경로 교체와 3-way merge 실행 test는 소유 파일 밖(`crates/semantic-diff/src/build.rs`·`records.rs`,
  `crates/semantic-merge/`)이라 하지 않았습니다.
- 결정: ADR-0075 결정 1

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 테스트).

## 변경 파일 목록

- `crates/document/src/frame_metadata.rs` — `FramePropertyMetadata.longhands`·`is_bundle`, SIZE→`width`·`height`,
  CORNER_RADIUS→4 모서리 묶음 표시, 묶음 대응 test.
- `crates/document/src/modifier_metadata.rs` — `SHADOW_OFFSET_X`(0x1094)·`SHADOW_OFFSET_Y`(0x1095),
  `ModifierPropertyType::Coordinate`, 0x1092를 묶음 key로 표시, `properties_of`가 묶음 key를 빼고 payload 필드
  대응을 longhand로 바꿈, test.
- `crates/document/src/css/mod.rs`·`css/bundle.rs`(신규) — 묶음 key 목록·`bundle_longhands`·`bundle-key-reference`
  진단, fixture test. `crates/document/src/lib.rs` 등록 한 줄.
- `crates/commands/src/property.rs` — 새 편집 key 목록이 registry 묶음 표시로 묶음 key를 뺌.
- `crates/commands/src/modifier_ops.rs`(소유 밖, 최소 변경) — offset x·y longhand 읽기·쓰기(다른 축 유지).
- `crates/commands/src/test_support.rs`(소유 밖, 컴파일 보정 한 줄) — 새 값 타입 대응.
- `crates/commands/src/property_tests.rs` — 옛 offset 묶음 bytes golden decode·apply가 longhand 두 command와 같음,
  한 축 편집과 undo, 새 편집 목록에 묶음 key 없음.
- `crates/semantic-diff/src/keys.rs` — 새 key 등록, Frame 모서리 편집이 서로 다른 snapshot 경로만 바꾸는 test.
- `docs/adr/ADR-0075-css-property-registry.md`(신규).

## 구현 요약

- 묶음 key는 registry 항목의 `longhands`가 비어 있지 않은 항목입니다. 번호는 재사용하지 않고 이전
  `SetProperty`·`SetModifierProperty` bytes와 journal을 그대로 decode·실행합니다. `CommandKind`·codec은 바꾸지
  않았습니다.
- 새 편집 목록(`node_property_keys`, `properties_of`)은 묶음 key를 내지 않습니다. 속성 패널 묶음 편집
  (`border-radius`)은 이미 longhand 4개 command였습니다.
- 저장 참조가 묶음 key를 가리키면 `bundle_key_reference`가 경로를 바꾸지 않고 longhand 목록과 함께 진단합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib` | 372 passed, 0 failed |
| `cargo test -p stellar-commands --lib` | 126 passed, 0 failed, 4 ignored |
| `cargo test -p stellar-semantic-diff` | 82 passed |
| `cargo clippy -p stellar-document -p stellar-commands -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| registry key 중복 0, 묶음 key→longhand 대응(`bundle_keys_map_to_registered_longhands`, `bundle_keys_split_into_registered_longhands_in_the_same_band`) | 통과 |
| 옛 Frame size·모서리 묶음 bytes의 decode·apply = longhand command 묶음(기존 test) | 통과 |
| 옛 Shadow offset 묶음 bytes(0x1092) golden decode·apply = x·y longhand 두 command | 통과 |
| 묶음 key 참조 진단 fixture(ValueBinding target·Variant patch·modifier 경로) | 통과 |
| 서로 다른 모서리 편집이 서로 다른 snapshot 경로만 바꿈 | 통과 |
| storage golden 불변 | payload·chunk encoder를 바꾸지 않음(코드 검토). storage crate 테스트는 lane 최종 check-core에 포함 |

## 차이(작업 지시와 다른 점)

- semantic snapshot은 Frame에서 이미 longhand key를 씁니다. Shadow offset은 `build.rs`가 아직 0x1092 묶음 key
  경로에 두 값을 넣습니다. 이 파일은 semantic-merge lane 소유라 바꾸지 않았습니다.
- 묶음 key 참조 진단은 순수 함수로 제공합니다. ValueBinding(`crates/bindings`)·Variant(`crates/document/src/view_variant`,
  `component/variant_override.rs`) validator가 이 함수를 부르게 하는 연결은 각 소유 범위입니다.

## 남은 위험 / 한계

- 후속: semantic-merge lane이 `build.rs`·`records.rs`에서 Shadow offset을 `SHADOW_OFFSET_X`·`SHADOW_OFFSET_Y`
  두 경로로 투영(옛 snapshot의 0x1092 읽기 유지)하고, 같은 Frame의 서로 다른 모서리를 양쪽에서 바꾼 3-way merge
  conflict 0 test를 추가합니다. bindings·variant validator가 `bundle_key_reference`를 호출합니다.
