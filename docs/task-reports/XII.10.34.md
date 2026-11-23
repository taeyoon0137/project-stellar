# 작업 보고서: XII.10.34 ComponentInstance 캔버스 편집

## WBS

- WBS: XII.10.34
- 작업명: 인스턴스 안 깊은 선택 읽기 전용 윤곽, 공개 Property 연결 값 override, Reset·Detach·정의로 이동 코어
- 상태: 부분 완료 (`[~]`) — 깊은 선택 윤곽, 허용·거부 판정, override Set·속성별·전체 Reset, 문맥 동작 판정은
  끝났습니다. 2026-11-24 text-instance-2 작업으로 Slot 끌어 놓기(문서 규칙과 command), Detach 실행 command, Object
  동작 command ID를 끝냈습니다. 메뉴 막대·문맥 메뉴·`ShortcutPolicy`(셸)가 남았습니다.
- 선행: IX.2.4, IX.2.6, IX.3.3, IX.4.2, XII.10.4, XII.6.5
- 결정: ADR-0118 결정 4·5, ADR-0129 결정 3·4·5(ADR-0118 결정 4.5·4.6 대체)

## 변경 파일

2026-11-24 text-instance-2 작업:

- `crates/document/src/validate.rs`(수정) — 인스턴스 자식 = 투영된 Slot content 규칙. `can_own_slot_content`,
  `instance_projection_issues`, 위반 `UnprojectedInstanceChild`·`InvalidSlotContent`. `can_have_children`은 자유 자식
  판정으로 그대로 둡니다.
- `crates/commands/src/detach.rs`(신규), `crates/commands/src/detach_tests.rs`(신규) — 인스턴스 구조 command payload
  `InstanceRestructure`(검증·역·적용·codec, 적용 뒤 문서 전체 검증), Detach 계획 `plan_detach`(IX.2.6 함수를
  인스턴스 subtree 사본에 실행).
- 등록: `crates/commands/src/{kind,command,codec,lib}.rs` — `detach-instance`(0x0094)·`place-slot-content`(0x0095),
  command codec 버전 7. `crates/commands/src/{test_support,property_tests,fuzz}.rs` — 생성기, VII.6.1 왕복 등록, ADR
  종류 fuzz 수.
- `crates/runtime/src/canvas/instance.rs`(수정) — `plan_slot_drop`·`plan_slot_take_out`·`plan_detach_instance`,
  `SlotContent`, 거부 이유(Slot·Detach), `InstanceAction`(Object 동작 command ID)과 `InstanceActions::enabled`.
- `crates/runtime/src/canvas/instance_tests.rs`(수정) — Slot 넣기·순서 바꾸기·꺼내기·default 복귀, 규칙 거부, Detach
  history 1건과 undo, command ID fixture.
- `docs/adr/ADR-0129-text-instance-editing-followups.md`(신규).
- `schemas/command-transaction.schema.json`(생성 파일 재생성) — canonical 입력 종류 목록에 두 종류.
- `crates/commands/src/convert.rs`(소유 밖, 수정) — `restore-node-records`가 바뀐 종류·payload에 붙지 않는 명시 layout
  longhand를 남기는 복원을 거부합니다. 새 command 종류 등록으로 random 순서 시험(`random_sequences`)의 생성 순서가
  바뀌며 드러난 기존 결함입니다(Fragment로 되돌린 node에 `grid-column-start`가 남음).

2026-11-23 canvas-text-instance 작업:

- `crates/runtime/src/canvas/instance.rs`(신규) — `pick_inside`(커서 아래 가장 위 정의 node와 world 윤곽),
  `PropertyLink`, `plan_inner_edit`(공개 Property 연결 값만 override Set), `plan_reset`, `plan_reset_all`,
  `Refused`(이유와 Detach 가능 여부), `instance_actions`(속성별·전체 Reset, Detach, 정의로 이동).
- `crates/runtime/src/canvas/instance_tests.rs`(신규) — fixture.
- `crates/runtime/src/canvas.rs`(수정) — `instance` mod 등록.
- `docs/task-reports/XII.10.34.md`(신규).

## 명령과 종료 코드

2026-11-24 text-instance-2 작업:

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-document --lib validate` | 0 |
| `cargo test -p stellar-commands --lib detach` | 0 (3 passed) |
| `cargo test -p stellar-commands --lib` | 0 (243 passed, `detach_instance_round_trip`·`place_slot_content_round_trip`·fuzz 포함) |
| `cargo test -p stellar-runtime --lib canvas::instance` | 0 (7 passed) |
| `cargo clippy -p stellar-document -p stellar-commands -p stellar-runtime -p stellar-ffi -p stellar-ops --all-targets --locked -- -D warnings` | 0 |

2026-11-23 canvas-text-instance 작업:

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::instance` | 0 (3 passed) |
| `cargo clippy -p stellar-text -p stellar-commands -p stellar-runtime --all-targets --locked -- -D warnings` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 없음.

## 테스트

- 깊은 선택(`deep_pick_returns_a_read_only_outline_of_the_topmost_definition_node`): (500,300)에 놓인 인스턴스 안에서
  자식 정의 node 902의 world 윤곽 (510,310)–(550,350)과 root 901을 고릅니다. 인스턴스 밖이나 인스턴스가 아닌 node는
  없음이고, 문서는 바뀌지 않습니다.
- override·reset fixture(`linked_public_value_becomes_an_override_and_resets_undo_once_each`): Button label Text의
  연결 값 편집은 label override Set 하나(history 1건)입니다. 속성별 Reset 뒤 문서는 처음과 같습니다(희소 저장).
  전체 Reset도 동작마다 1건이고, undo로 정확히 되돌아갑니다.
- 거부 fixture(`unlinked_private_nested_and_locked_edits_are_refused_with_detach_path`): 연결 없음, 값 타입 불일치,
  nested 닫기 Button 직접 편집, 비공개 Property, 잠긴 인스턴스를 거부합니다. 잠금 외에는 Detach 경로를 함께 냅니다.
- 남은 경고 0.

## 성능

- 해당 없음. 깊은 선택은 정의 tree node 수에 선형입니다.

## 차이

- override Set·Reset은 새 command 종류 없이 인스턴스 payload 교체 command(`swap-instance`) 하나로 씁니다. command
  registry를 바꾸지 않기 위해서입니다(ADR-0118 결정 5).
- 정의 node 속성과 Property의 연결은 binding 저장소가 넘기는 `PropertyLink` 목록입니다. binding은 문서 모델에 저장되지
  않습니다.
- Slot 끌어 놓기는 만들지 않았습니다. `validate::can_have_children`이 ComponentInstance를 container로 보지 않아
  `InsertNode`·`MoveNode`와 문서 validator가 인스턴스 자식을 거부합니다. 시도한 계획 함수가 `MoveNode` 검증에서
  `InvalidParent`로 실패하는 것을 확인하고 코드를 지웠습니다. 그 규칙은 이 작업 소유 밖(document crate)이고 문서
  의미 변경입니다.
- 문맥 메뉴·메뉴 막대 Object 메뉴 항목, `docs/design/ui/tools_menus.md` 표, `ShortcutPolicy`는 셸 작업이라 하지
  않았습니다(사용자 방향 2026-11-23). 그래서 `ShortcutPolicy.audit()`은 실행하지 않았습니다.

## 테스트(2026-11-24 text-instance-2)

- 문서 규칙(`instance_children_must_be_projected_slot_content`): 투영된 자식·빈 override는 유효, 투영되지 않은 자식은
  `UnprojectedInstanceChild`, 같은 자식 두 번 투영은 `InvalidSlotContent`입니다. 자식이 아닌 node·없는 node를 가리키는
  옛 형식 참조는 문서 위반이 아닙니다(이 규칙 전 저장 문서 호환, ADR-0129 결정 3). 자유 자식 판정은 인스턴스를 넣지
  않습니다.
- Slot fixture(`slot_drop_and_take_out_are_one_entry_each_and_undo_exactly`): Frame 11을 Modal body 맨 앞에 넣기,
  같은 인스턴스 안 순서 바꾸기, root로 꺼내기, 마지막 content를 default 복귀로 꺼내기가 각각 history 1건입니다. 매번
  문서 validator 통과, 4번 Undo로 처음과 같습니다.
- 거부 fixture(`slot_drop_refuses_rule_violations_without_changing_the_document`): 인스턴스 조상, 없는 Slot, 허용하지
  않는 종류(Rectangle), 부모가 인스턴스가 아닌 새 content, Slot content가 아닌 node 꺼내기.
- Detach fixture:
  - commands `detach_replaces_the_instance_with_a_frame_copy_and_moves_slot_content`: 인스턴스 id·부모·이름 유지,
    root transform 이동 0, 명시 width 320px → Frame 크기, 정의 node 복제와 body 복제 아래로 content 이동, 같은 입력
    같은 계획, 역 command 정확, codec 왕복.
  - runtime `detach_instance_is_one_entry_and_undoes_once`: history 1건, Undo 한 번에 처음과 같음.
  - 미해결 정의·비인스턴스 거부.
- command ID(`instance_action_command_ids_are_unique_and_follow_availability`): 4개 ID 유일, 이름 왕복, 사용 가능 여부가
  `instance_actions`를 따릅니다.
- 등록: `every_phase_vii_command_is_registered`, 왕복 property(detach 120건 이상, slot 60건 이상),
  `adr_0079_kinds_survive_mutation_fuzz`(16종).

## 차이(2026-11-24)

- Detach는 Property override 값을 복제 node에 반영하지 않습니다. IX.2.6 함수와 같은 동작이고, 정의 node 속성과
  Property의 binding이 문서에 없기 때문입니다.
- 메뉴 막대 Object 메뉴·문맥 메뉴·`ShortcutPolicy`·`docs/design/ui/tools_menus.md` 표는 셸 작업이라 하지 않았습니다.
  코어 command ID(ADR-0129 결정 5)만 정했습니다. 그래서 `ShortcutPolicy.audit()`은 실행하지 않았습니다.
- Detach·Slot 끌어 놓기의 전용 CLI·MCP 명령은 없습니다. canonical command 입력으로 적용할 수 있습니다.

## 남은 위험

- override 편집의 history 이름은 "인스턴스 교체"로 보입니다.
- 인스턴스 자식이 있는 문서를 scene 그리기·semantic diff·codegen 경로로 확인하지 않았습니다(ADR-0129
  Consequences).
- 새 command 번호 0x0094·0x0095와 codec 버전 7은 registry 전역 변경입니다. 동시에 다른 작업이 같은 번호를 쓰면 통합 때
  맞춰야 합니다.
- nested instance 정의 node는 scene 투영이 아직 그리지 않아 깊은 선택으로도 고를 수 없습니다.
