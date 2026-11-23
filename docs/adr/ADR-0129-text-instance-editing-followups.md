# ADR-0129: Text·인스턴스 편집 후속(빈 Text 정리, Text 입력 경로 하나, Slot content 문서 규칙, Detach command)

## Status

Accepted (2026-11-24). 계획 XII.10.33·XII.10.34의 코어 후속(Swift·FFI 제외)을 정합니다. ADR-0118 결정 2.5의 끝내기
규칙, 결정 4.5·4.6, 결정 8과 Consequences의 "빈 Text가 남음"·"Slot 끌어 놓기와 Detach 실행이 남음"을 이 ADR이
대체합니다.

## Context

- ADR-0118은 캔버스 Text 편집기(`canvas::text::edit::CanvasTextEditor`)와 인스턴스 편집 판정을 runtime 순수 함수로
  만들었습니다. 다음이 남았습니다.
  - 새로 만든 Text를 입력한 뒤 모두 지우고 끝내면 빈 Text가 남습니다. 빈 Text는 화면에 보이지 않지만 레이어 목록,
    Git 원본, code export에는 빈 요소로 남습니다.
  - 지운 셸의 C ABI 뒤에 옛 Text 입력 경로 `app::text_input`이 따로 있었습니다. 편집 가능 판정, grouping key, 새
    Text 생성(폭 layout record 없음)이 새 편집기와 달랐습니다. 같은 동작의 두 번째 구현입니다.
  - 문서 모델이 ComponentInstance 자식을 허용하지 않아 Slot content를 문서에 둘 수 없었습니다(`validate::
    can_have_children`, 문서 validator `ParentNotContainer`).
  - Detach(IX.2.6) 실행 command와 인스턴스 편집 Object 동작의 command ID가 없었습니다.
- 사용자 방향(2026-11-23): 지금은 엔진·기능만 만듭니다. Swift UI와 셸 FFI는 바꾸지 않습니다.
- 제약:
  - `crates/ffi`의 Text ABI(`stellar_text_*`)는 `AppRuntime`의 `begin_canvas_text`·`mark_canvas_text`·
    `commit_canvas_text`·`cancel_canvas_text`·`replace_canvas_text`·`canvas_text_content` 이름과 인자에 기대고 있습니다.
  - history crate에는 기록한 항목을 redo 갈래 없이 버리는 API가 없습니다.
  - 컴포넌트 library는 문서 모델 밖에 있습니다. command 적용은 문서 모델만 봅니다.

## Decision

### 1. 새로 만든 Text를 비우고 끝내면 지웁니다 (XII.10.33)

1. 편집을 끝낼 때(`CanvasTextEditor::finish`) 이 편집에서 만든 Text의 본문이 비었으면 그 Text를 지우는
   `delete-subtree` 하나를 별도 history 항목으로 기록합니다. 결과는 `EditEnd::Removed`입니다.
2. 선택은 그 삭제가 정리합니다(삭제된 node는 창 선택에서 빠짐). Undo 한 번은 빈 Text와 선택을 되살리고, 이어지는
   Undo들이 입력·생성을 거꾸로 되돌립니다. 모든 Undo가 보이는(레이어 목록에서 보이는) 변화를 하나씩 되돌립니다.
3. 입력 없이 끝난 새 Text는 이전과 같이 아무것도 만들지 않습니다(history 0건).
4. 이미 있던 Text는 비워도 지우지 않습니다. 사용자가 일부러 비운 기존 node이고, 참조(mask·boolean operand 등)를
   깨뜨릴 수 있기 때문입니다.
5. 조합 확정과 빈 Text 정리는 한 번의 끝내기에서 함께 일어나지 않습니다. 확정하는 조합 문자열은 비어 있지 않기
   때문입니다. 그래서 `finish`는 command ID 하나만 받습니다.

### 2. 앱 Text 입력은 코어 편집기 하나를 씁니다 (XII.10.33)

1. 편집기는 무엇을 어떤 command·grouping key로 기록할지만 정합니다. 기록은 `TextEditHost` trait이 합니다.
   - 코어 경로(시험·CLI처럼 history를 직접 쥔 곳): `HistoryHost`(창 history + 작업 문서).
   - 앱 창: `app::text_input`의 창 session host. 기록마다 Undo 메뉴 이름표를 붙입니다(입력 `TYPING`, 편집 중 style
     변경 `CHANGE_PROPERTY`, 빈 Text 정리 `DELETE`).
2. `app::text_input`은 창마다 진행 중 `CanvasTextEditor` 하나를 두고, 비동기 요청을 편집기 호출로 바꾸는 일만 합니다.
   옛 별도 구현(조합 범위 하나를 바로 기록, `GroupingKey::TextInput` 직접 조립, 별도 편집 가능 판정)은 지웠습니다.
3. FFI가 쓰는 `AppRuntime` 메서드 이름·인자는 그대로 둡니다(셸 FFI·`stellar_ffi.h` 무변경). 의미는 코어 편집기를
   따릅니다.
   - 편집 가능 판정은 `commands::lock::edit_permission`과 활성 View 소속입니다. 숨김은 편집을 막지 않습니다
     (ADR-0118 결정 2.8). Text가 단독 선택돼 있을 필요는 없습니다(선택 엔진의 `EditContent`가 편집을 엽니다).
   - grouping key는 입력 종류별 `GroupingKey::text_edit`입니다. 선택을 바꾸는 확정(Replace)과 이어지는 삽입(Insert)은
     다른 항목입니다.
   - 클릭 생성 Text는 `width: max-content` layout record를 가집니다(ADR-0118 결정 1.2). 끌기 결과는
     `TextTarget::Placement`(고정 폭)로 넘깁니다.
   - `commit_canvas_text`는 편집을 끝내지 않습니다. 같은 token으로 입력이 이어지고, 편집은 새
     `end_canvas_text`(Escape 뒤·바깥 클릭·도구 전환)가 끝냅니다. 같은 Text에 다시 `begin_canvas_text`하면 그
     편집기를 이어 씁니다(이 편집에서 만들었는지 유지). 다른 대상이면 남은 편집을 먼저 끝냅니다.
   - 다른 경로(undo, 다른 창)로 문서가 바뀌어도 편집기가 본문을 다시 읽고 이어 갑니다. 대상이 사라지거나 잠기면
     편집기를 버리고 요청이 실패합니다.
4. 새 Rust API(FFI 없음): `delete_canvas_text`(grapheme 단위 Backspace·Delete), `move_canvas_caret`,
   `set_canvas_text_style`, `end_canvas_text`.

### 3. ComponentInstance 자식 = 투영된 Slot content (문서 규칙, XII.10.34)

1. ComponentInstance는 자식을 가질 수 있습니다. 그 자식은 그 인스턴스가 소유한 Slot content이고, 인스턴스 payload의
   slot override 하나에 정확히 한 번 투영돼야 합니다(IX.3.2 규칙을 문서 validator로).
2. 문서 validator 위반 두 가지를 더합니다.
   - `UnprojectedInstanceChild`: 인스턴스 자식이 어느 slot override에도 없음.
   - `InvalidSlotContent`: slot override가 인스턴스 자식이 아닌 node(없는 node 포함)를 가리키거나 같은 node를 두 번
     투영함.
3. 판정 함수는 하나입니다. `validate::instance_projection_issues`(투영 목록, 실제 자식)가 문제 목록을 내고, 문서
   validator와 인스턴스 구조 command가 같은 판정(문서 validator)을 씁니다.
4. `can_have_children(kind)`는 **자유 자식**(삽입·이동·붙여넣기가 아무 node나 둘 수 있는 부모: Frame·Fragment)
   판정으로 그대로 둡니다. 새 `can_own_slot_content(kind)`가 ComponentInstance입니다. validator의 부모 검사는 둘 중
   하나면 통과입니다.
5. Slot 종류·개수·가림 규칙은 컴포넌트 library가 있어야 알 수 있어 문서 validator가 보지 않습니다. 계획 단계(IX.3
   함수 `ComponentSlot::check_content`, `check_hidden_slot_content`)가 봅니다.

### 4. 인스턴스 구조 command 두 종류 (XII.10.34, IX.2.6, IX.3.3)

1. `detach-instance`(0x0094)와 `place-slot-content`(0x0095)를 command registry에 등록합니다. command codec 버전은
   7입니다(두 종류의 첫 버전).
2. 두 종류는 같은 payload `commands::detach::InstanceRestructure`를 씁니다: 대상 node 레코드 교체(id·부모·순서
   고정), 대상 layout record, 만들 node(부모는 대상 또는 앞선 레코드)와 그 modifier stack·layout record, 옮길 node,
   지울 node(자식부터). 적용 순서는 교체 → 만들기 → 옮기기 → 지우기입니다.
3. 역 command는 같은 종류의 `InstanceRestructure`입니다(현재 레코드, 지운 node 복원, 원래 부모·순서, 만든 node
   지우기). history 이름은 종류 이름("인스턴스 분리", "Slot 콘텐츠 배치")입니다.
4. 검증: 적용 전에 대상·id·부모 관계·cycle·View 경계를 봅니다. 적용 뒤에는 결과 문서 전체를 문서 validator로 다시
   검사하고, 이 command가 새로 만든 위반이 있으면 `InvalidResult`로 되돌립니다. 투영 규칙(결정 3)은 이 검사가
   지킵니다.
5. Detach 계획(`commands::detach::plan_detach`)은 IX.2.6 함수 `component::detach::detach_instance`를 인스턴스 subtree
   사본(scene columns·payload bytes)에 실행해 만듭니다. Detach 규칙의 원본은 IX.2.6 함수 하나입니다.
   - 인스턴스 StableId는 그대로이고 복제 node는 새 id를 받습니다(`StableIdGenerator`·`MonotonicClock` 인자).
   - 인스턴스 배치는 인스턴스 Transform modifier가 계속 가집니다. 그래서 복제 root Frame payload transform은 정의의
     선형 부분만 남기고 이동을 0으로 둡니다(scene 투영 `placed_free_frame`과 같은 그림).
   - 인스턴스 layout record의 명시 `width`·`height`가 `<length>`이면 Frame payload 크기로 옮깁니다(Frame 크기는
     payload에 저장, ADR-0086). 그 밖의 크기 값은 지웁니다. 나머지 longhand는 그대로 둡니다.
   - Property override 값은 정의 node 속성과의 binding(VIII.6)이 문서에 없어 복제 node에 반영하지 않습니다(IX.2.6
     함수와 같은 동작).
6. Slot 끌어 놓기 계획은 runtime `canvas::instance`가 컴포넌트 library로 만듭니다.
   - `plan_slot_drop`(인스턴스, slot, 투영 위치, 기존 node 또는 새 subtree): 공개 Slot(컴포넌트가 선언한 Slot)에만
     넣습니다. override가 없던 Slot은 그 content 하나짜리 override가 됩니다. 같은 인스턴스의 다른 Slot에 있던
     content는 그 Slot에서 빠집니다(부모는 그대로).
   - `plan_slot_take_out`(인스턴스, content, 새 부모, 순서, `restore_default`): 마지막 content를 빼면
     `restore_default`일 때 override를 지워 default content로 돌아가고, 아니면 빈 override로 남깁니다.
   - 거부: 인스턴스 잠금·미해결 정의, 없는 Slot, 종류·개수 위반, 가려진 Slot, 옮길 수 없는 content(View root,
     인스턴스 자신·조상, 잠긴 content·부모). 거부하면 문서를 바꾸지 않습니다.

### 5. 인스턴스 편집 Object 동작의 코어 command ID (XII.10.34)

| 동작 | command ID | 사용 가능 |
|---|---|---|
| Reset Overrides | `object.reset_overrides` | 직접 Property override가 있고 인스턴스가 잠기지 않음 |
| 속성별 Reset(문맥 메뉴 하위) | `object.reset_override` | `reset_properties`가 비지 않음 |
| Detach Instance | `object.detach_instance` | 잠기지 않았고 정의가 있음 |
| 정의로 이동 | `object.go_to_definition` | 정의가 있음 |

`canvas::instance::InstanceAction::name`이 원본입니다. 메뉴 막대·문맥 메뉴·단축키 정책은 이 이름을 씁니다
(`ViewToggle::name`과 같은 방식, ADR-0117 결정 1).

## Alternatives

- 빈 Text 정리를 이 편집의 마지막 입력 항목에 합침: 생성부터 정리까지 한 항목이면 순효과가 없는 항목이 남아 Undo가
  아무것도 안 하는 것처럼 보입니다. 채택하지 않았습니다.
- 빈 Text 정리 때 이 편집의 항목을 Undo로 되돌림: redo 갈래가 남아 Redo가 빈 Text를 다시 만듭니다. redo 갈래만
  버리는 history API가 없습니다. 채택하지 않았습니다.
- 기존 Text도 비우면 지움: 참조를 깨뜨리고 사용자가 의도하지 않은 삭제가 됩니다. 채택하지 않았습니다.
- 옛 `app::text_input`을 셸 재연결 때까지 둠(ADR-0118 결정 8): 같은 입력의 두 구현이 서로 다른 규칙을 갖습니다.
  셸 ABI 이름만 지키면 지금 합칠 수 있어 합쳤습니다.
- `can_have_children`에 ComponentInstance를 넣음: `InsertNode`·`MoveNode`·붙여넣기 대상 판정이 이 함수를 쓰므로
  투영 없이 인스턴스 아래에 node를 두는 경로가 열립니다. 자유 자식 판정과 Slot content 판정을 나눴습니다.
- Detach를 기존 command 여러 개의 transaction으로 씀: 인스턴스 아래로의 이동이 `MoveNode` 검증(자유 자식만)에 막히고,
  종류 교체(`restore-node-records`)·삽입·이동이 각각 중간 상태를 검증해 순서가 맞지 않습니다. 한 command로 묶었습니다.
- Detach command가 library를 들고 적용 때 펼침: command 적용은 문서 모델만 봅니다. 계획에서 펼친 결과를 담습니다.

## Consequences

- 텍스트 입력은 앱·코어가 같은 편집기 하나를 씁니다. 새 UI는 `AppRuntime` Text 요청과 `end_canvas_text`에 연결합니다.
- 문서가 인스턴스 Slot content를 저장할 수 있습니다. Git 원본·semantic diff·scene·codegen이 인스턴스 자식을 아직
  따로 다루지 않을 수 있습니다. 이 결정에서 각 경로(특히 scene 그리기)를 인스턴스 자식이 있는 문서로 확인하지
  않았습니다. 각 경로의 확인은 후속입니다.
- 인스턴스 구조 command는 적용·undo·redo마다 문서 크기에 선형인 검증을 두 번 합니다(드문 사용자 동작).
- Detach는 Property override 값을 복제 node에 반영하지 않습니다. binding 저장소가 문서와 함께 있게 되면 계획에
  더해야 합니다.
- 새 command 두 종류는 registry 전역 변경입니다. 같은 시기에 다른 작업이 0x0094·0x0095나 codec 버전 7을 쓰면 통합
  때 번호를 맞춰야 합니다.
- Detach 실행·Slot 끌어 놓기의 CLI·MCP 전용 명령은 없습니다. canonical command 입력(`stellar apply`)으로는 적용할
  수 있습니다.

## Compatibility

- 파일 포맷 변경은 없습니다. 인스턴스 자식은 기존 node 레코드 형식 그대로이고, 이전에 거부하던 구조만 받습니다.
- command codec 버전 6 → 7. 옛 reader는 새 종류를 담은 history journal 복원만 포기합니다(ADR-0079 규칙).
- FFI·`stellar_ffi.h` 변경은 없습니다.
- 공개 Rust API 변경:
  - `CanvasTextEditor`의 기록 메서드가 `(history, doc)` 대신 `&mut impl TextEditHost`를 받습니다.
  - `EditEnd::Removed`가 생겼습니다.
  - `app::text_input::TextTarget::Placement`가 생겼고, `AppRuntime`에 Text 요청 4개가 생겼습니다.
  - `DocumentViolation`에 두 variant가 생겼습니다.
  - `Command`·`CommandKind`에 두 variant가 생겼습니다.
  - `InstanceEditRefusal`에 Slot·Detach 이유가 생겼습니다.

## Performance

- Text 편집은 이전과 같습니다(본문 길이에 선형).
- 문서 validator의 Slot content 검사는 node 수와 slot content 수에 선형입니다.
- 인스턴스 구조 command는 적용마다 문서 전체 검증 2회(O(N))입니다. Detach 계획은 인스턴스 subtree와 정의 tree 크기에
  선형입니다.

## Security

영향 없음.

## Migration

없음. 저장된 문서는 인스턴스 자식을 갖지 않았습니다(이전 validator가 거부).
