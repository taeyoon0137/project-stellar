# 작업 보고서: IX.2.1 ComponentInstance payload

- WBS: IX.2.1
- 작업명: ComponentInstance node payload — ComponentRef, sparse PropertyOverride map, SlotOverride map
- 상태: 완료 (`[x]`)
- 선행: IX.1.1, VI.2.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/instance.rs`(신규) — `ComponentInstancePayload`, `OverrideKey`,
  `SlotOverride`, `InstancePayloadError`, 속성 key `keys::{COMPONENT_REF, PROPERTY_OVERRIDES,
  SLOT_OVERRIDES}`(0x0A01~0x0A03).
- `crates/document/src/component/mod.rs` — `pub mod instance;` 등록.
- `docs/task-reports/IX.2.1.md`(신규), 계획서(IX.2.1 `[ ]`→`[x]`).

## 구현 요약

- **payload**: `ComponentInstancePayload`는 `ComponentRef`, `BTreeMap<OverrideKey, TypedValue>`
  (PropertyOverride), `BTreeMap<ComponentSlotId, SlotOverride>`(SlotOverride)를 갖습니다. 인스턴스가
  바꾼 항목만 저장하고, override를 지우면 map entry가 사라져 override 없는 payload와 같은 bytes가
  됩니다(희소 저장).
- **OverrideKey**: Component Property StableId + nested instance 경로. 빈 경로가 직접 override이고,
  경로가 있는 key는 nested 직접 접근을 **표현만** 합니다(거부는 IX.2.3·IX.4.2 validator 몫).
- **SlotOverride**: slot에 투영할 인스턴스 소유 node의 순서 목록(중복 거부). 빈 목록은 slot을
  명시적으로 비운 상태이고, entry 부재는 default content입니다.
- **직렬화**: canonical LE, map은 key 오름차순. 디코딩은 오름차순 위반·중복·잘못된 `TypedValue`·
  trailing bytes를 거부하고, `decode_for`는 ComponentInstance가 아닌 종류를 거부합니다.
- **참조 index**: `sync_references`가 인스턴스 → 컴포넌트 정의 edge를 `ReferenceIndex`(VI.2.5)에
  등록해 정의 삭제 전 inbound 인스턴스를 조회할 수 있게 합니다.
- 새 속성 key는 0x0Axx 범위만 사용했습니다(기존 0x01xx frame, 0x0601 image와 충돌 없음 —
  `grep "PropertyKey(0x"`로 확인).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 154 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 여러 값 타입·nested key·slot override payload round-trip, 재인코딩 byte-identical(`payload_round_trips`) | 통과 |
| 설정 순서와 무관한 동일 bytes(`encoding_is_independent_of_insertion_order`) | 통과 |
| 희소 저장: 빈 payload 24 bytes, 설정 후 삭제 시 원래 bytes(`storage_is_sparse`) | 통과 |
| 절단·trailing·종류 불일치·slot content 중복·key 비오름차순·잘못된 값 거부(`malformed_payloads_are_rejected`) | 통과 |
| ReferenceIndex 인스턴스→정의 edge(`instance_references_component_in_index`) | 통과 |

## 차이(작업 지시와 다른 점)

- override 값은 core-types `TypedValue`만 담습니다. 표현식 값(Phase VIII)은 document crate의 의존
  대상이 아니라 제외했습니다.

## 남은 위험 / 한계

- override key의 Property가 정의에 선언됐는지, 공개인지, 값 타입이 맞는지는 ComponentProperty
  (IX.1.2, VIII.2.5 선행 미완)와 validator(IX.2.3)가 생기면 검사합니다.
- slot content가 실제로 인스턴스 소유 node인지는 IX.3.2 validator가 검사합니다.
