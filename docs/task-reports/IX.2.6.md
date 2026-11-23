# 작업 보고서: IX.2.6 명시적 detach command

- WBS: IX.2.6
- 작업명: virtual subtree를 실제 nodes로 복제하고 ComponentRef를 제거하는 detach command(새 ID remap)
- 상태: 완료 (`[x]`)
- 선행: IX.2.2, III.1.5, Phase VII(phase 단위 선행 — 완화 적용, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/detach.rs`(신규) — `detach_instance`, `undo_detach`, `DetachRecord`,
  `SceneNodeRecord`, `MovedContent`, `DetachError`.
- `crates/document/src/component/mod.rs` — `pub mod detach;` 등록.
- `crates/document/tests/component_detach.rs`(신규) — 시각 의미·undo·remap 검증.
- `docs/task-reports/IX.2.6.md`(신규), 계획서(IX.2.6 `[ ]`→`[x]`).

## 구현 요약

- **검증 후 적용**: 인스턴스 해석과 slot 투영 검증(IX.3.2)을 scene 변경 전에 끝내고, 실패하면 scene을
  바꾸지 않습니다.
- **복제**: 정의 tree를 전위 순회해 복제합니다. 인스턴스 node 자리는 정의 root Frame 복제본이 차지하되
  외부 참조 보존을 위해 **인스턴스 StableId를 유지**하고(부모·순서·이름 유지, 표시 = 인스턴스∧root,
  잠금 = 인스턴스∨root), 나머지 복제 node는 `IdRemapTable::clone_ids`(III.1.5)로 새 StableId를 받습니다.
  nested 인스턴스 payload의 내부 참조(slot content)는 새 id로 재매핑하고, 외부 참조(참조 컴포넌트,
  property override)는 유지합니다.
- **slot**: override된 slot은 default content를 복제하지 않고, 인스턴스 소유 content를 복제된 host 아래로
  실제 reparent합니다(투영 순서대로 새 OrderKey). override 없는 slot은 default content를 복제합니다.
- **ComponentRef 제거**: 인스턴스 자리는 Frame이 되어 payload도 정의 root payload로 바뀝니다. nested
  인스턴스는 한 단계만 풀려 실제 인스턴스 node로 남습니다.
- **undo**: `DetachRecord`(원래 인스턴스 필드·payload, 생성 node, 옮긴 content의 원래 부모·OrderKey,
  remap 표)로 복원합니다. scene이 detach 직후 상태가 아니면 아무것도 바꾸지 않고 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 174 passed, `component_detach` 2, `component_expansion_scale` 1, `component_slot_projection` 1 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| detach 전 가상 subtree와 detach 후 실제 subtree의 (깊이, 종류, 실효 표시, payload/참조 컴포넌트) 전체 일치 — nested 인스턴스·nested slot·slot의 실제 인스턴스·숨김 default content 포함(`detach_preserves_visual_meaning_and_undo_restores`) | 통과 |
| undo 후 scene 의미 snapshot(StableId·종류·부모·OrderKey·표시·잠금·이름·payload) detach 전과 동일, slot 투영 검증 재통과(같은 테스트) | 통과 |
| 복제 node 6개 모두 새 id(정의 id와 불교차), override된 default content 미복제, nested payload 내부 참조 재매핑·외부 참조 유지, 정의 library bytes 불변(같은 테스트) | 통과 |
| 같은 seed·시계면 같은 결과(`detach_is_deterministic_for_same_seed`) | 통과 |
| root 복제본이 인스턴스 자리·id 유지, 표시·잠금·이름 합성(`root_copy_takes_instance_place_and_identity`) | 통과 |
| 비인스턴스·미해결 컴포넌트 거부 시 scene 불변(`invalid_targets_leave_scene_unchanged`), 상태가 바뀐 뒤 undo 거부·불변(`undo_rejects_changed_state`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `Phase VII`은 phase 단위 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다. 3002e6d의
  `stellar-commands`는 골격뿐이라 closed enum command·revision·history는 없습니다. 이 작업은 document 계층
  apply/undo 함수 쌍(`detach_instance`/`undo_detach` + 되돌리기 payload)으로 구현했고, VII.1.2 이후
  command enum에 등록·감싸는 것은 후속입니다.
- 실행 순서: slot 투영(IX.3.2)이 필요해 IX.3.2 뒤에 수행했습니다.
- "시각 의미"는 binding이 없는 현재 기준(구조·종류·실효 표시·payload bytes)으로 비교했습니다.

## 남은 위험 / 한계

- PropertyOverride 값의 node 속성 반영은 binding(Phase VIII) 이후입니다. 그때 detach는 override가 반영된
  최종 값을 복제 node 속성에 굽는(bake) 단계가 필요합니다.
- ReferenceIndex(인스턴스 → 컴포넌트 edge) 정리와 selection 갱신은 command 계층(VII)이 detach 적용 시 함께
  처리해야 합니다.

## 후속 수정 (리뷰 MAJOR 1: nested slot host)

- **결함**: 안쪽 slot host가 바깥 slot의 default content 안에 있고(nested slot host) 바깥·안쪽 slot이 모두
  override되면, 순회가 바깥 host에서 멈춰 안쪽 slot content가 reparent되지 않았습니다. 그 node는 지워진
  인스턴스 핸들을 부모로 가진 채 남았고 `undo_detach`가 복원하지 못했습니다.
- **모델**: nested slot host는 허용합니다. 바깥 slot이 override되면(빈 override 포함) 그 default content 안의
  안쪽 slot은 가려지며, 가려진 slot은 content를 가질 수 없습니다(빈 override만 허용). `ComponentEntry::enclosing_slots`,
  `slot_override::hiding_slot`·`check_hidden_slot_content`로 판정하고, `validate_scene_slot_overrides`·
  `validate_definition_slot_overrides`가 `SlotOverrideError::HiddenSlotContent`로 거부합니다. `add_slot`은
  nested host를 등록 순서와 무관하게 받으며 문서에 가림 규칙을 명시했습니다.
- **detach**: 위 validator로 적용 전에 거부하고, 추가로 인스턴스의 모든 소유 node가 복제된(보이는) host 아래로
  옮겨질 자리를 갖는지 적용 전에 확인합니다(`DetachError::UnplacedContent`, 방어적 검사).
- **undo**: 복제 subtree 아래에 복제 node·옮긴 content 외의 node가 있거나 옮긴 content가 subtree 밖으로 나갔으면
  아무것도 바꾸지 않고 `StateMismatch`로 거부합니다(고아 node 방지).

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | unit 236, `component_detach` 6, `component_expansion_scale` 1, `component_slot_projection` 1, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 안쪽 slot만 override: 안쪽 host 복제본 아래 투영 순서로 이동, 시각 의미 동일, undo 후 snapshot 동일(`nested_slot_host_content_is_reparented_and_undo_restores`) | 통과 |
| 바깥·안쪽 모두 override(가려진 content): validator `HiddenSlotContent`, detach 거부·scene 불변(`hidden_nested_slot_content_is_rejected_before_detach`). 새 검사를 끄면 이 테스트가 실패함을 확인 | 통과 |
| 가려진 slot의 빈 override: detach·undo 정상(`empty_hidden_override_detaches_and_undo_restores`) | 통과 |
| detach 뒤 복제 subtree에 외부 node 추가 시 undo 거부·불변(`undo_rejects_foreign_child_under_detached_subtree`) | 통과 |
| 정의 tree 안 nested 인스턴스의 가려진 content 거부, `hiding_slot` 판정(`hidden_nested_slot_content_is_rejected_in_definitions`) | 통과 |
| nested host 등록 순서 무관·`enclosing_slots` 관계(`nested_slot_host_is_registered_with_enclosing_relation`) | 통과 |

- 남은 한계: slot command·구조 편집 validator의 적용 후 가림 검사는 별도 수정(MAJOR 2)에서 다룹니다.

## 후속 수정 (2026-11-24 instance-integrity: Detach가 Property override 값을 반영, ADR-0133 결정 3)

### 배경

문서 command Detach(`commands::detach::plan_detach`, ADR-0129 결정 4.5)는 Property override 값을 복제 node에 반영하지
않았습니다. 정의 node 속성과 Property의 연결(binding)이 문서에 없기 때문입니다. 그래서 override가 있는 인스턴스를 Detach하면
화면이 바뀌어 이 행의 검증 "detach 후 시각 의미 동일"을 어겼습니다.

### 변경 파일

- `crates/commands/src/detach.rs` — `PropertyLink`(정의 node·속성·Property 연결), `plan_detach`의 `links` 인자, 값 대응
  `registry_value`(단위를 바꾸지 않는 대응만), 값 쓰기 `write_linked_value`, `DetachPlanError::Link`.
- `crates/commands/src/property.rs`(소유 밖, 리팩터) — `SetProperty` 적용의 레코드 쓰기를 `write_property`로 꺼내 Detach와
  같은 쓰기 경로를 씁니다(동작 변경 없음).
- `crates/runtime/src/canvas/instance.rs`(소유 밖, 최소 수정) — `PropertyLink`를 commands 타입의 다시 내보내기로 바꾸고
  `plan_detach_instance`가 `links`를 받습니다. `crates/runtime/src/canvas/instance_tests.rs` 호출 2곳.
- `crates/commands/src/detach_tests.rs` — Card fixture에 공개 String Property `title`을 선언.
- `crates/commands/src/detach_integrity_tests.rs` — Detach 반영 fixture.

### 규칙

- 쓰는 값은 `override_rules::check_override`(공개·선언·타입)를 통과한 직접 override뿐입니다. 없거나 규칙을 어긴 override는
  정의 값 그대로입니다(IX.1.4 Property 값 캐시와 같은 규칙).
- Text 본문(`TEXT_CONTENT`)은 VI.7.5 `edit_text`로, 그 밖의 속성은 `write_property`로 씁니다.
- 쓸 수 없는 연결은 `DetachPlanError::Link`로 거부합니다. 복제되지 않은 정의 node를 가리키는 연결은 건너뜁니다.

### 실행한 명령

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --lib` | 통과(259 passed, 6 ignored) |
| `cargo test -p stellar-runtime --lib canvas::instance` | 통과(8 passed) |
| `cargo clippy -p stellar-commands -p stellar-runtime --all-targets --locked -- -D warnings` | 통과 |

### 테스트

- `detach_writes_linked_property_overrides_into_the_copies`: `title` override "확인 창"이 있는 Card 인스턴스를 Detach하면 복제
  title Text 본문이 "확인 창"이고, 적용·undo가 정확합니다. override가 없거나 연결을 넘기지 않으면 정의 값(빈 본문) 그대로,
  선언되지 않은 Property의 override는 반영하지 않습니다. 문자열 Property를 Frame `width`에 잇는 연결은 `Link`로 거부합니다.
- `registry_values_keep_css_units`: `px` 길이 → 비음수, 음수 길이 거부, `%` → 0..=1, 타입 불일치 거부. 값 타입 `Number`는
  payload registry 값 종류로 단위를 맞춥니다. `px` 길이는 좌표 속성에만, `deg` 각도는 각도 속성에만 쓰고, 개수·종류를 모르는
  속성에는 단위 있는 값을 쓰지 않습니다(검토 보완, ADR-0133 결정 3.4).
