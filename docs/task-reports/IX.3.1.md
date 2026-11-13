# 작업 보고서: IX.3.1 ComponentSlot

- WBS: IX.3.1
- 작업명: `ComponentSlot` — StableId, host FrameRef, allowed node kinds, cardinality, default content refs
- 상태: 완료 (`[x]`)
- 선행: IX.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/slot.rs`(신규) — `ComponentSlot`, `SlotCardinality`, `SlotError`,
  `ComponentLibrary::{add_slot, validate_slots}`.
- `crates/document/src/component/definition.rs` — `ComponentEntry`에 slot record 저장·조회
  (`slot`, `slots`, `slot_for_host`), library 직렬화에 slot record 포함, `ComponentDefinitionError::Slot`.
- `crates/document/src/component/mod.rs` — `pub mod slot;` 등록.
- `docs/task-reports/IX.3.1.md`(신규), 계획서(IX.3.1 `[ ]`→`[x]`).

## 구현 요약

- **ComponentSlot**: id(`ComponentSlotId`), host(`NodeRef`, Frame 검증), 허용 node 종류 집합,
  `SlotCardinality { min, max: Option }`(min > max 거부), default content refs(시각 순서).
- **host 검증**: `ComponentSlot::validate(tree)`는 host가 소유 컴포넌트 정의 tree 안에 없으면
  `HostOutsideDefinition`(다른 컴포넌트 node·scene node 모두), Frame이 아니면 `HostNotFrame`입니다.
- **default content 규칙**: host는 slot 전용 컨테이너로, host의 정의 tree 자식 전체가 default
  content입니다. 목록은 그 자식을 시각 순서대로 정확히 나열해야 하며(밖 node·누락 자식·순서
  불일치 거부), 종류·중복·개수 규칙(`check_content`, override에도 재사용)을 지켜야 합니다.
- **library 등록**: `add_slot`은 정의 Slot 목록 선언 여부, 중복 record, tree 검증, host 공유 금지를
  검사하고 실패 시 상태 불변입니다. `validate_slots`는 선언된 slot의 record 누락을 보고합니다.
  library binary에 slot record를 포함해 round-trip합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 164 passed, `component_expansion_scale` 1 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 다른 컴포넌트 Frame·scene node를 host로 지정하면 `HostOutsideDefinition`, library 불변, 정의 안 host는 등록(`host_outside_definition_is_rejected`) | 통과 |
| 비Frame host, host 밖 default content, 누락 자식, 순서 불일치, 금지 종류, 개수 위반, 빈 허용 종류, 잘못된 cardinality 거부(`host_and_default_content_rules`) | 통과 |
| 미선언 slot, 중복 record, host 공유, record 누락, 없는 컴포넌트(`library_slot_registration_rules`) | 통과 |
| slot·library(slot 포함) round-trip, 절단·trailing 거부(`slot_and_library_round_trip`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실행 순서: lane 목록 순서(IX.2.5·IX.2.6 다음)보다 앞당겨 IX.2.2 직후에 수행했습니다. IX.2.5
  (Slot만 변경 허용)와 IX.2.6(detach 시 slot content 반영)이 Slot host 정의를 필요로 하기 때문입니다.
  선행 규칙(IX.1.1)은 그대로 충족합니다.
- slot record는 `ComponentDefinition`의 Slot id 목록(IX.1.1 interface)과 분리해 library entry에
  저장합니다. IX.1.1의 정의 타입·직렬화 형식은 바꾸지 않았습니다.

## 남은 위험 / 한계

- "host = slot 전용 컨테이너" 규칙(고정 자식과 slot 영역 혼합 불가)은 설계 선택입니다. 혼합이
  필요해지면 별도 host Frame을 두는 방식으로 표현합니다.
- 정의 tree 편집 후 slot 재검증(default content 동기화)은 정의 편집 command(Phase VII 연동) 몫입니다.
