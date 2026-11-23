# ADR-0118: 캔버스 텍스트·인스턴스 편집과 키보드 경로 코어

## Status

Accepted (2026-11-23). 계획 XII.10.33·XII.10.34·XII.10.35·XII.10.41의 코어(Swift·FFI 제외)를 정합니다.

## Context

- 사용자 방향(2026-11-23)은 지금 엔진·기능만 만들고 UI는 나중에 처음부터 다시 설계하는 것입니다. 셸 UI 문서
  (캔버스 명세, 도구·메뉴 명세, 속성 모델 문서의 UI 절)는 ADR-0098로 지워졌습니다. 이 ADR은 계획서 문구와 기존
  ADR(0067, 0074, 0079, 0082, 0086, 0104, 0105)만으로 값을 정합니다.
- 이미 있는 코어 부품:
  - Text 범위 교체 `replace-text`, 범위 style `set-text-range-style`, 입력 종류별 grouping key
    `GroupingKey::text_edit`(XII.10.32).
  - 선택 엔진의 내용 편집 요청 `SessionOutput::EditContent`(ADR-0104 결정 3.4).
  - 조합 미리보기 `TextPreview`와 scene의 공통 text 엔진 그리기(ADR-0067 §6).
  - 인스턴스 교체 `swap-instance`와 보존·탈락 preview(ADR-0079 결정 1·6).
  - override 규칙 `check_override`(IX.2.3), layout record와 `set-layout-property`·`unset-property`(ADR-0086).
  - 키보드 이동·크기 경로 `transform::keyboard`(XII.10.14)와 단위 표(ADR-0104 결정 2).
- 제약:
  - 이 작업은 command registry(`CommandKind`)를 바꾸지 않습니다. 다른 작업과 동시에 도는 전역 mutex 변경이기 때문입니다.
  - 문서 모델은 ComponentInstance가 자식을 갖는 것을 허용하지 않습니다(`validate::can_have_children`, 문서
    validator `ParentNotContainer`, `InsertNode`·`MoveNode` 검증).
  - 정의 node 속성과 Component Property를 잇는 binding(VIII.6)은 문서 모델에 저장되지 않습니다.

## Decision

### 1. Text 만들기와 폭 (XII.10.33)

1. Text 도구 클릭은 자동 폭, 끌기는 고정 폭입니다. 화면 이동이 끌기 시작 거리(`DRAG_THRESHOLD_PT`) 안이면
   클릭입니다(`canvas::text::edit::plan_text_tool`).
2. 자동 폭은 명시 `width: max-content`입니다. 기본값 세트의 Text `width`는 `auto`(ADR-0082)이고, 흐름 안의 `auto`는
   부모 폭을 채웁니다. 그래서 "내용 폭으로 늘어나고 줄바꿈하지 않음"을 CSS 그대로 쓰려면 `max-content`를 명시해야
   합니다.
3. 고정 폭은 `width: <끈 가로 길이>px`(최소 1px)이고, 상자 왼쪽 위가 원점입니다. 높이는 내용으로 정하므로 쓰지
   않습니다.
4. 자동 폭인지는 명시 `width`가 `<length>`·`<percentage>`가 아닌지로 판정합니다(미설정·keyword 포함,
   `text_width_mode`).
5. 상자 변 끌기는 `width: <px>`를 쓰는 `set-layout-property` 하나입니다(`edge_width_command`). 자동 폭이면 고정
   폭으로 바뀝니다.

### 2. Text 편집 모드와 입력기 조합 (XII.10.33)

1. 본문·선택·조합은 공통 text buffer `stellar_text::edit::TextEditBuffer`가 갖습니다. 위치는 문서와 같은 scalar
   offset입니다. 삭제·caret 이동 단위는 확장 grapheme cluster(UAX #29)입니다.
2. 조합은 AppKit `setMarkedText`·`unmarkText`·`insertText` 의미를 따릅니다.
   - 조합 시작 때의 선택이 조합으로 바뀔 범위입니다.
   - 조합 중 문자열은 `TextPreview`(transient)로만 보입니다. scene이 공통 text 엔진으로 그립니다.
   - 빈 marked text는 조합 취소입니다.
   - 조합 중 삭제·caret 이동은 입력기 몫이라 buffer가 처리하지 않습니다.
3. 확정 입력만 기록합니다. 기존 Text는 `replace-text` 하나와 `GroupingKey::text_edit`로 연속 입력을 묶습니다.
4. 새 Text는 첫 확정 때 만듭니다. `InsertNode` 하나에 원점 Transform modifier와 폭 layout record를 담고, 새 선택과
   함께 그 입력 group의 첫 항목으로 기록합니다(`record_grouped_and_select`). Undo 한 번이 생성과 이어진 입력을
   함께 되돌립니다.
5. 입력 없이 끝난 새 Text는 아무것도 만들지 않고 기록하지 않습니다(history 0건).
6. 끝내기 규칙:
   - 조합 중 Escape는 조합만 버리고 편집을 계속합니다. 조합이 없으면 Escape가 편집을 끝냅니다.
   - 바깥 클릭·도구 전환은 남은 조합을 확정하고 끝냅니다.
   - 끝나면 Text node 하나를 선택합니다. 이미 그 Text 하나만 선택돼 있었으면 선택 기록을 만들지 않습니다.
7. 편집 중 다른 경로(undo, 다른 창)로 본문이 바뀌면 편집기가 문서 본문을 다시 읽습니다. caret은 바뀐 구간 끝에
   둡니다. 대상이 사라지면 `Stale`로 편집을 끝내게 합니다.
8. 편집 가능 판정은 `commands::lock::edit_permission`(D8)입니다. 숨김은 편집을 막지 않습니다.

### 3. 편집 중 typography 변경 (XII.10.33 목표)

문자 style(`DocumentTextStyles::runs` 번호) 변경은 CommandTransaction 하나(history 1건, 앞 입력과 묶지 않음)입니다.

- 조합이 있으면 조합 확정 `replace-text`와 그 범위의 `set-text-range-style`을 한 transaction으로 씁니다. 조합
  문자열이 바꿀 범위의 본문과 같으면 style command만 씁니다.
- 선택이 있으면 선택 범위의 `set-text-range-style` 하나입니다.
- caret뿐이거나 아직 만들지 않은 새 Text이면 다음 확정 입력의 transaction에 style command를 함께 넣습니다.
- style 표 항목을 새로 만드는 command는 없습니다. 그래서 편집기는 표에 이미 있는 번호만 씁니다.

### 4. ComponentInstance 캔버스 편집 (XII.10.34)

1. 인스턴스 안 정의 node는 문서 node가 아니어서 선택 대상이 아닙니다. `canvas::instance::pick_inside`는 커서 아래
   가장 위의 정의 node를 찾아 읽기 전용 윤곽(world 네 꼭짓점)으로 돌려줍니다.
   - 계산은 scene의 인스턴스 root 행 world에서 시작합니다. 정의 tree를 scene 투영과 같은 `placed_free_frame`
     규칙으로 내려갑니다.
   - 윤곽은 저장·history에 없습니다.
2. 바꿀 수 있는 값은 공개 Property에 연결된 정의 node 속성뿐입니다.
   - 연결 목록 `PropertyLink`(정의 node, 속성 key, Property)는 binding 저장소가 넘깁니다. 연결은 그 속성의
     binding이 Property 참조 하나인 경우입니다.
   - 편집은 그 Property의 override Set이고 IX.2.3 규칙(`check_override`)을 그대로 씁니다.
3. 거부할 때는 문서를 바꾸지 않습니다. 이유와 Detach 가능 여부를 함께 돌려줍니다(`Refused`). 거부 이유는 다음과
   같습니다.
   - 연결 없음.
   - nested instance 안 값(IX.4.2, drilling Property로만 바꿈).
   - 비공개·미선언 Property, 값 타입 불일치.
   - 잠금, 미해결 정의.
4. 문맥 동작(`instance_actions`)은 속성별 Reset(직접 override가 있는 Property), 전체 Reset(Slot content 유지),
   Detach, 정의로 이동(컴포넌트와 정의 root)의 사용 가능 여부입니다. 잠긴 인스턴스는 Reset·Detach가 불가합니다.
5. Slot 끌어 놓기는 이 작업에서 만들지 않습니다. 문서 모델이 인스턴스 자식을 허용하지 않아 인스턴스 소유 Slot
   content를 문서에 둘 수 없습니다. 그 규칙(`can_have_children`, 문서 validator, `InsertNode`·`MoveNode` 검증)을
   여는 결정이 먼저 필요합니다.
6. Detach 실행 command는 command registry에 없습니다. 이 작업은 Detach 가능 여부만 냅니다.

### 5. 인스턴스 payload 교체 command 하나 (XII.10.34·XII.10.35)

1. `swap-instance`(0x0090)를 "인스턴스 payload(참조, Property override, Slot override) 교체" command로 씁니다. 다음
   동작이 모두 이 command 하나입니다.
   - 교체(swap).
   - override Set·속성별 Reset·전체 Reset.
   - 역은 원래 payload를 담은 같은 command라 undo가 정확합니다.
2. command 종류를 새로 만들지 않습니다(registry 무변경).
3. 교체 코드는 `stellar_commands::swap`으로 옮겼습니다. 기존 경로 `node_ops::{SwapInstance, …}`는 다시 내보내기로
   유지합니다.

### 6. 교체 크기 (XII.10.35)

인스턴스 크기는 인스턴스 layout record의 `width`·`height`(ADR-0086 결정 2)입니다.

- 기본(`SwapSizing::DefinitionDefault`): 명시 `width`·`height`를 `unset-property`로 지웁니다. 인스턴스는 새 정의
  root 크기를 따릅니다.
- 현재 크기 유지(`SwapSizing::KeepCurrent`): 명시 값이 없는 축에 지금 보이는 크기(현재 정의 root payload 크기)를
  `set-layout-property`로 `<length>`를 둡니다. 명시 값은 그대로 둡니다.
- 크기 command는 교체 command 뒤에 같은 transaction으로 씁니다(`InstanceSwapPlan::commands`). 정의 root 크기를
  알 수 없는데 현재 크기 유지가 필요하면 계획을 거부합니다.
- ADR-0079 결정 6의 "현재 크기 유지 거부"를 이 결정으로 대체합니다.

### 7. 키보드만으로 하는 캔버스 편집 (XII.10.41)

1. `canvas::keyboard::CanvasFocus`는 키 초점과 초점 표시를 갖습니다. 키보드로 초점을 받거나 키를 누르면 표시하고,
   캔버스 pointer 입력이 오면 숨깁니다.
2. 키 배정(`keyboard_command`)과 단위는 ADR-0104 결정 2 표입니다.
   - 화살표 1px, Shift 10px 이동.
   - ⌘-화살표 1px, ⌘⇧ 10px 크기 조절.
   - Tab·Shift-Tab·Return·Shift-Return·Escape·⌘A는 선택 엔진에 그대로 넘깁니다(`select_key`).
   - 문맥 메뉴는 Shift-F10, 문맥 메뉴 키, ⌃Return으로 엽니다. 위치는 선택 bounds 가운데이고, 선택이 없으면 화면
     가운데입니다.
3. 이동·크기 키는 `transform::keyboard`(XII.10.14)를 그대로 부릅니다(`transform_key`). 선택이 없을 때 화살표 camera
   이동은 40pt, Shift는 200pt입니다.
4. Insert 메뉴는 화면 가운데에 기본 크기 100×100px(Line은 가로 100px 수평선)로 만듭니다.
   - `plan_insert_at_center`는 부모 local 상자·두 점·Text 자리만 정합니다. 삽입은 기존 Frame·Shape 삽입 경로와
     Text 편집기가 합니다.
   - Text는 화면 가운데에 자동 폭으로 둡니다.
5. 핸들 대체 표(`handle_equivalents`)는 다음과 같습니다. 인스펙터 이름은 CSS registry 또는 layout longhand 이름입니다.

| 핸들 | 키보드·인스펙터 대체 |
|---|---|
| 본체(이동) | 화살표, Shift-화살표, `left`·`top` |
| 크기 모서리·변 | ⌘-화살표, ⌘⇧-화살표, `width`·`height` |
| 회전 | `transform` |
| 반경 | `border-*-radius` 네 longhand |

### 8. 기존 경로

`app::text_input`(ADR-0067 §6)은 지운 셸의 C ABI 뒤에 있는 옛 Text 입력 경로입니다. 새 UI는 이 ADR의
`canvas::text::edit` 경로에 연결하고, 그 연결 때 옛 경로를 지웁니다(ADR-0104 결정 6과 같은 방식).

## Alternatives

- 클릭 Text를 `width` 미설정으로 둠: 기본값 `auto`는 흐름 안에서 부모 폭을 채워 자동 폭이 아닙니다. 채택하지
  않았습니다.
- 고정 폭 Text에 `height`도 둠: 내용이 늘면 넘칩니다. 디자인 도구의 고정 폭 Text는 높이가 내용을 따르므로
  채택하지 않았습니다.
- override 편집용 command 종류를 새로 둠: 같은 payload 교체 의미의 두 번째 command가 생기고, 동시 작업 중
  registry 전역 mutex가 필요합니다. history 이름이 "인스턴스 교체"로 보이는 한계는 Consequences에 적었습니다.
- 인스턴스 크기를 payload 필드로 둠: ADR-0086 결정 2가 ComponentInstance 크기 저장 위치를 layout record로 정했습니다.
- 편집 중 typography 변경을 조합이 끝날 때까지 거부: 입력기마다 확정 시점이 달라 결과가 결정적이지 않습니다.

## Consequences

- 캔버스 Text·인스턴스·키보드 편집 규칙이 runtime 순수 함수와 상태 값으로 모였습니다. 새 UI와 CLI·MCP가 같은 규칙을
  씁니다.
- override Set·Reset의 history 이름은 command 종류 이름("인스턴스 교체")입니다. 동작별 이름은 registry를 바꾸는
  작업에서 정합니다.
- 인스턴스 record 크기(`width`·`height`)를 scene이 아직 그리지 않습니다. layout 결과로 그리는 일(SD.2)이 들어와야
  현재 크기 유지가 화면에 보입니다.
- Slot 끌어 놓기와 Detach 실행은 각각 문서 규칙 결정과 registry 등록이 필요해 남았습니다.
- 새로 만든 Text를 입력한 뒤 모두 지우고 끝내면 빈 Text가 남습니다. 자동 삭제는 별도 history 항목이 되어 undo
  의미가 어색해지므로 하지 않았습니다.
- Text 상자 폭(`width`)을 scene의 text layout이 아직 쓰지 않습니다(`max_width` 없음). 그리기 연결은 layout 결과
  그리기와 함께 합니다.

## Compatibility

- 파일 포맷·FFI·command codec 변경은 없습니다.
- 공개 Rust API가 바뀌었습니다.
  - `InstanceSwapPlan`에 `sizing` 필드가 생겨 `Eq`가 빠졌습니다.
  - `plan_instance_swap`이 `current_size` 인자를 받습니다.
  - 이 API를 쓰는 다른 crate는 없습니다.

## Performance

- 편집기 연산은 본문 길이에 선형입니다(grapheme 경계, scalar 변환).
- 깊은 선택 hit는 정의 tree node 수에 선형입니다.
- 키 배정은 상수 시간입니다.
- 편집 성능 replay(XII.10.43)가 IME 조합 scenario로 판정합니다.

## Security

영향 없음.

## Migration

없음. 옛 `app::text_input` 경로는 셸 재연결 때 이 경로로 바꿉니다.
