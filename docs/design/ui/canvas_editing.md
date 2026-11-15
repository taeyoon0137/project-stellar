# 캔버스 편집 UX 명세 (canvas_editing)

- 상태: 초안(2026-11-14), 구현 전 명세
- 범위: macOS 캔버스에서 노드를 선택·변형·생성·편집하는 상호작용과, 각 상호작용이 만드는 Command·history·접근성·성능 계약
- 상위 기준: [macOS 셸 디자인 가이드](../macos-shell-design.md)(이하 "가이드"), 기술 아키텍처 명세(이하 "아키텍처"), 구현 계획(이하 "계획")
- 관련 명세: [도구·메뉴](tools_menus.md), [인스펙터](inspector.md), [사이드바](sidebar.md), 속성 모델 `docs/design/property_model.md`(이하 "속성 모델", 구현은 계획 IX.11)

이 문서는 가이드·아키텍처·계획을 바꾸지 않습니다. 어긋나면 그 문서들이 우선이고 이 문서를 고칩니다. [가이드 0.2] 이 문서가 요구하지만 계획에 행이 없던 동작은 계획 `XII.10`(캔버스 편집 UX)으로 추가했습니다(24장).

---

## 0. 문서 사용법

### 0.1 근거 표기

가이드 0.1절의 표기를 쓰고, 다음 두 가지를 더합니다.

| 표기 | 의미 |
|---|---|
| `[PO 2026-11-14 편집 UX]` | 제품 책임자 방향(2026-11-14): 편집 UX는 전문 UI 디자인 도구의 관례를 참고하되 베끼지 않고, UI 형태는 Stellar 고유의 macOS 네이티브 형태로 하며, 디자인 속성은 CSS 기반으로 편집하고, 이후 노드 기반 프로토타이핑·Property Drilling·Context·코드 변환으로 잇습니다. |
| `[관례]` | 전문 UI 디자인 도구(예: Figma, Sketch 계열) 사용자가 공통으로 기대하는 편집 관례. 개념 수준의 기대치만 뜻하며, 특정 제품의 화면·문구·고유 명칭·세부 동작을 옮기지 않습니다. |

`[관례]`가 붙은 규칙은 사용자의 손에 익은 동작을 존중하기 위한 것이고, 근거로서의 강도는 `[아키텍처]`, `[계획]`, `[HIG]`보다 낮습니다. 둘이 어긋나면 아키텍처·계획·HIG를 따릅니다.

### 0.2 수치

가이드 0.2절과 같이 이 문서에는 이동 단위, 허용 오차, 핸들 크기, 줌 단계 같은 수치를 두지 않습니다. 수치는 계획 XII.10.2의 ADR이 정하고 가이드 12장에 기록합니다. 성능 예산은 아키텍처 §6의 값을 그대로 인용합니다.

### 0.3 용어

| 용어 | 뜻 |
|---|---|
| 편집 범위(Editing Scope) | 지금 사용자가 "들어가 있는" 컨테이너. 클릭은 이 범위의 직계 자식 단위로 선택합니다. WindowViewState에 두고 저장하지 않습니다. |
| 흐름 컨테이너 | 자식을 흐름으로 배치하는 Frame(Stack·Grid 계열, CSS로는 `display: flex`·`display: grid`). 속성 이름은 속성 모델을 따릅니다. |
| 위치 지정 자식 | 흐름에서 빠져 inset으로 놓이는 자식(CSS로는 `position: absolute`)과 자유 배치(Free) 컨테이너의 자식. |
| 편집 의도 | 캔버스 조작이 만드는 "어느 node의 어느 CSS 속성을 어떤 값으로" 묶음. Command로 바뀌기 전 단계이며 인스펙터 편집과 같은 형태입니다. |
| transient overlay | 선택 윤곽, 핸들, 가이드, 측정 표시처럼 렌더러가 그리지만 문서에 없는 표시. [가이드 1.2 원칙 2] [계획 XII.4.4] |

---

## 1. 범위와 원칙

### 1.1 범위

다룹니다: 도구와 수식키, 선택, 변형 핸들, 이동, 스냅·가이드·눈금자·그리드, 정렬·분배, 순서와 묶음, 레이아웃 인지 편집, 생성 도구, 벡터 편집, Boolean 연산, 캔버스 텍스트, 컴포넌트 인스턴스 편집, 줌·팬, 클립보드, 문맥 메뉴, Undo 단위, 키보드 전용·VoiceOver 경로, 성능 예산.

다루지 않습니다: 인스펙터 화면 구성(인스펙터 명세), 메뉴 막대 구성과 단축키 충돌 감사(도구·메뉴 명세), 속성 이름·값 문법·저장 필드 대응(속성 모델), 노드 기반 프로토타이핑 그래프와 Context 공급·소비 UI(PO 방향의 후속 단계), 실시간 공동 편집(아키텍처 §4.3 비목표).

### 1.2 원칙

1. **관례를 따르되 베끼지 않습니다.** 선택·변형·스냅·레이아웃 핸들의 동작은 전문 도구 사용자가 기대하는 결과를 내야 합니다. 화면 모양, 문구, 색, 아이콘은 Stellar가 macOS 시스템 구성요소와 SF Symbols로 정합니다. [PO 2026-11-14 편집 UX] [관례] [가이드 1.2 원칙 1]
2. **셸은 해석하지 않고 전달합니다.** pointer·key·제스처는 공통 `InputEvent`로 코어에 가고, 도구 상태 기계·hit test·스냅·Command 생성은 코어가 합니다. 셸은 커서, 문맥 메뉴, 접근성, IME 연결만 맡습니다. [가이드 1.2 원칙 4] [아키텍처 §8.2, §9.2] [계획 XII.3.1, XII.3.6]
3. **캔버스 조작과 인스펙터 편집은 같은 속성을 바꿉니다.** 핸들 드래그, nudge, padding 핸들은 모두 속성 모델의 CSS longhand(`width`, `padding-top`, `column-gap`, `rotate` 등)에 대한 편집 의도를 만들고, 인스펙터는 같은 속성을 같은 이름으로 보여 줍니다. 캔버스에만 있는 숨은 속성을 만들지 않습니다. [PO 2026-11-14 편집 UX] [속성 모델]
4. **한 제스처는 한 history entry입니다.** pointer down부터 up까지, 연속 키 반복, 스크럽은 하나로 묶고, 취소한 제스처와 hover·camera·도구 전환은 기록하지 않습니다. [아키텍처 §23.3] [계획 VII.4.2, VII.4.4, VII.4.5, XII.3.7]
5. **hover가 클릭 결과를 미리 보여 줍니다.** 수식키를 누른 상태까지 반영해, hover 윤곽은 지금 클릭하면 선택될 node와 언제나 같습니다. 깊이 규칙이 예측 가능해야 한다는 것이 이 규칙의 목적입니다. [관례] [계획 XII.10.7]
6. **모든 동작에 키보드와 VoiceOver 경로가 있습니다.** 핸들로만 할 수 있는 동작을 두지 않습니다. 각 동작은 메뉴 명령, 키 조작, 인스펙터 필드 중 하나 이상으로도 됩니다. [가이드 8.2, 8.3] [HIG Accessibility]
7. **성능 계약이 우선입니다.** hover·선택·핸들 표시는 목표 5ms·상한 8ms, Pan·Zoom·Drag·Resize·Rotate는 Present 예약까지 10ms 이하입니다. 이 예산을 넘는 시각 효과는 넣지 않습니다. [아키텍처 §6.4] [가이드 1.2 원칙 6]

---

## 2. 편집 모델

### 2.1 흐름

```text
NSEvent(셸)
  → InputEvent(병합·좌표 변환)                  XII.3.1, XII.3.2, XII.3.4
  → 도구 상태 기계(CanvasTool, 편집 범위)         XII.3.6, XII.10.1, XII.10.4
  → hit test·핸들 hit·스냅                       XI.3.3~XI.3.5, XII.10.8, XII.10.15
  → 편집 의도(CSS 속성 기준) + transient preview   XII.3.7
  → pointer up: CommandTransaction 확정           VII.1.6, VII.4.2
  → 창별 history entry 하나                        VII.5.1
```

- 드래그 중에는 문서를 바꾸지 않고 preview만 Scene builder에 넣습니다. pointer up에서 CommandTransaction 하나를 확정하고, Escape나 포커스 상실로 취소하면 preview를 버립니다. [계획 XII.3.7, VII.4.2, VII.4.5]
- 흐름 컨테이너 안 조작은 preview 단계에서도 증분 레이아웃을 거쳐 형제의 재배치를 보여 줍니다. [아키텍처 §13] [계획 X.7.1]

### 2.2 이 문서가 쓰는 Command

| Command | 구현 | 쓰는 곳 |
|---|---|---|
| `SetSelection` | VII.3.2 | 선택 전부 |
| 영역 선택 grouping(`AreaSelectionMode`) | VII.3.3 | 마퀴 |
| `SetProperty`(specialization 포함) | VII.2.4, VII.2.5 | 크기·위치·회전·반경·padding·gap·잠금·숨김 |
| `InsertNode`, `DeleteSubtree`, `MoveNode`, `DuplicateSubtree` | VII.2.1, VII.2.2, VII.2.3, VII.2.8 | 생성·삭제·재정렬·재부모화·복제·묶음 |
| `InsertModifier`, `RemoveModifier`, `MoveModifier`, `SetModifierProperty` | VII.2.6 | Boolean, Scale |
| `ConvertShapeToPath`, `ConvertFragmentToFrame` | VII.2.7 | 벡터 편집 진입, 묶음 변환 |
| `CommandTransaction` | VII.1.6 | 여러 Command를 한 entry로 |
| `PropertyOverrideCommand::Set`·`Reset` | IX.2.4 | 인스턴스 override |
| detach | IX.2.6 | 인스턴스 Detach |
| Slot 콘텐츠 삽입·삭제·재정렬 | IX.3.3 | 인스턴스 Slot |
| Path point 편집(가칭 `PathEdit`) | 신규 XII.10.28 | 벡터 편집 |
| Text 범위 편집(가칭 `TextEdit`) | 신규 XII.10.32 | 캔버스 텍스트 |
| 인스턴스 교체(가칭 `SwapInstanceComponent`) | 신규 XII.10.35 | 인스턴스 swap |

속성 편집 의도를 `SetProperty`의 PropertyKey와 저장 필드로 옮기는 규칙은 속성 모델이 정합니다. 예를 들어 위치는 이동 변환이 아니라 inset 또는 흐름으로 저장합니다(속성 모델 4.6절·5장·12장, 계획 IX.11.6, IX.11.9). Frame의 위치 longhand는 IX.11.9가 transform에서 분리하고, Frame이 아닌 node의 위치 longhand는 IX.11.6 item record에 있습니다. Ellipse·Polygon `bounds` 원점과 Transform modifier 이동 성분은 캔버스 이동으로 바꾸지 않습니다. 이 문서는 CSS 속성 이름까지만 정합니다. 영속 record는 longhand만 저장하므로 캔버스 조작은 shorthand(`gap`, `inset`, `padding` 등)를 쓰지 않고 longhand만 바꿉니다. [속성 모델 11장] [ADR-0024 결정 4]

CSS 속성의 PropertyKey·영속 record·migration은 계획 IX.11이 만듭니다. 그 속성을 편집하는 XII.10 행은 해당 IX.11 행을 선행으로 둡니다(24장). [계획 IX.11]

---

## 3. 도구와 수식키

### 3.1 도구

도구 10종과 단일 키는 도구·메뉴 명세 6.2절을 그대로 씁니다. 단일 키는 캔버스가 first responder일 때만 해석합니다. [도구·메뉴 6.2] [가이드 8.2]

| 도구 | 키 | 기본 동작 | 구현 |
|---|---|---|---|
| Move | V | 선택·이동·변형 | XII.10.1, XII.10.3\~XII.10.14 |
| Hand | H | camera 팬 | XII.10.37 |
| Zoom | Z | 클릭 확대, Option-클릭 축소, 영역 드래그 확대 | XII.10.37 |
| Frame, Rectangle, Ellipse, Line, Polygon | F, R, O, L, P | 드래그·클릭 생성. Polygon의 P는 UX 명세 결정 U6(코드 연결 UX.1.3) | XII.10.26 |
| Text | T | Text 생성·편집 | XII.10.33 |
| Image | 메뉴 ⇧⌘K | 도구를 고르면 즉시 파일 선택, 클릭은 원래 크기·드래그는 그 크기로 삽입. 취소하면 이전 도구(UX 명세 결정 U11, UX.3.3) | XII.10.27 |

- 생성 도구로 하나를 만든 뒤에는 Move로 돌아갑니다. [관례]
- 도구 상태는 WindowViewState에 두고 저장·history에 넣지 않습니다. [계획 VII.3.1, XII.10.1]

### 3.2 일시 전환

| 입력 | 동작 | 끝 | 구현 |
|---|---|---|---|
| Space 누름 | 누르는 동안 Hand | Space 놓음 | XII.10.1 |
| 생성·마퀴 드래그 중 Space | 시작점을 함께 옮김(크기 유지) | Space 놓음 | XII.10.26 |
| ⌘ 누름(Move 도구) | 누르는 동안 깊은 선택 hover·클릭 | ⌘ 놓음 | XII.10.4 |
| Escape | 아래 순서에서 처음 해당하는 하나: 진행 중 제스처 취소, Move가 아닌 도구이면 Move로 복귀, 편집 범위가 최상위가 아니면 한 단계 위, 최상위면 선택 해제 | — | XII.10.1, XII.10.3, XII.10.4 |

- 텍스트 편집 모드와 벡터 편집 모드에서는 Escape가 먼저 그 모드를 끝냅니다(12.2절, 14.1절). 위 순서는 모드 밖에서 씁니다.

### 3.3 수식키 의미(제안)

가이드 열린 결정 O10에 대한 이 문서의 제안입니다. 확정은 XII.10.2 ADR이 합니다. 같은 수식키는 조작 종류가 달라도 비슷한 뜻을 갖게 했습니다. [가이드 8.1, 11장 O10] [HIG Keyboard] [관례]

| 수식키 | 클릭 | 드래그로 이동 | 크기 조절·생성 | 회전 | 기타 |
|---|---|---|---|---|---|
| Shift | 선택 토글 추가 | 축 고정(가로·세로·45도) | 비율 고정(비율 고정 속성이 켜져 있으면 일시 해제) | 15도 단위 | 화살표 이동을 큰 단위로 |
| Option(⌥) | 다중 선택에서 이미 선택된 node를 primary로 지정(8장) | 시작할 때 누르면 복제 후 이동 | 중심 기준 | — | 선택 후 hover로 거리 측정, 마퀴는 완전히 포함된 node만 |
| Command(⌘) | 깊은 선택 | 드래그 도중 새로 누르면 스냅 일시 해제 | 같음 | 같음 | pointer down 때 이미 누른 ⌘는 깊은 선택에만 쓰고 스냅 해제로 해석하지 않음 |
| Control(⌃) | 문맥 메뉴(시스템 규칙) | 쓰지 않음 | 쓰지 않음 | 쓰지 않음 | HIG에 따라 단축키에 쓰지 않음 |

- 마우스 Control-클릭은 macOS에서 보조 클릭이므로 수식키 의미를 주지 않습니다. [HIG Keyboard] [AppKit 문서 NSEvent]
- 수식키는 드래그 도중 눌렀다 떼어도 즉시 반영합니다(⌘ 예외는 위 표). [관례]

---

## 4. 선택

### 4.1 클릭 선택

| 입력 | 결과 | Command | 구현 |
|---|---|---|---|
| node 클릭 | 편집 범위의 직계 자식 중 hit node를 포함한 것을 선택 | `SetSelection` | XII.10.3 |
| Shift-클릭 | 선택에 있으면 빼고, 없으면 더함(primary는 마지막으로 더한 것) | `SetSelection` | XII.10.3, VII.3.1 |
| 빈 캔버스·Root Frame 빈 곳 클릭 | 선택 해제 | `SetSelection` | XII.10.3 |
| ⌘A | 편집 범위의 형제 전체 | `SetSelection` | XII.10.3 |
| ⇧⌘A | 선택 해제 | `SetSelection` | 도구·메뉴 6.1 |

- 시작 편집 범위는 현재 View의 Root Frame입니다. 따라서 Root Frame 바로 아래 자식은 바로 클릭해 선택합니다. [아키텍처 §12.2] [관례]
- Fragment는 자체 박스가 없으므로 hit test는 자식에서 일어나지만 선택은 Fragment 단위입니다. 즉 Fragment가 묶음(group) 역할을 합니다. [아키텍처 §12.4]
- ComponentInstance는 한 덩어리로 선택합니다. 안쪽은 15장의 깊은 선택으로만 봅니다. [아키텍처 §15.1]
- 잠금·숨김 node는 캔버스 클릭으로 선택되지 않습니다. [계획 XI.3.3, VII.3.2]

### 4.2 깊은 선택과 편집 범위

| 입력 | 결과 | 구현 |
|---|---|---|
| ⌘-클릭 | 커서 아래 가장 깊은 선택 가능 node를 바로 선택하고 편집 범위를 그 부모로 | XII.10.4 |
| 더블클릭, Return | 선택한 컨테이너 안으로 한 단계 들어가 hit 자식(Return은 첫 자식) 선택 | XII.10.4 |
| Shift-Return | 부모를 선택하고 편집 범위를 한 단계 위로 | XII.10.4 |
| Tab, Shift-Tab | 같은 부모 안 다음·이전 형제(OrderKey 순) | XII.10.4 |
| Shift-더블클릭 | 깊은 node를 선택에 더함 | XII.10.4 |

- 선택이 바뀌어 편집 범위 밖으로 나가면, 범위를 선택들의 가장 가까운 공통 조상으로 맞춥니다. 사이드바에서 깊은 레이어를 고른 경우도 같습니다. [계획 XII.2.5] [사이드바 4.1]
- 더블클릭이 컨테이너가 아닌 node에 일어나면 그 종류의 편집 모드로 들어갑니다(Text는 14장, Path는 12장). Shape는 편집 모드로 바로 들어가지 않습니다(12.2절). [아키텍처 §12.6]

### 4.3 선택 우선 드래그와 아래 레이어 고르기(select-through)

- pointer down이 현재 선택의 bounds 안이면, 그 위에 다른 node가 겹쳐 있어도 드래그는 현재 선택을 옮깁니다. 움직이지 않고 놓으면 그 위치에서 클릭 규칙으로 고른 node를 선택합니다. [관례] [계획 XII.10.5]
- 겹친 node 중 아래 것을 고르려면 문맥 메뉴의 겹친 레이어 고르기 하위 메뉴(Select Overlapping ▸)를 씁니다. 커서 아래 node를 z 역순으로 보여 주고, 잠긴 node도 표시해 고를 수 있게 합니다(고르면 선택만 되고 잠금은 유지). 아래 레이어를 고르는 기능은 관례이고, 명령 이름은 Stellar가 정한 것입니다(1.2 원칙 1). [관례] [계획 XII.10.5, XII.10.39]

### 4.4 영역 선택(마퀴)

| 입력 | 결과 | 구현 |
|---|---|---|
| 빈 캔버스·Root Frame 빈 곳에서 드래그 | 영역에 걸친 편집 범위 node로 선택 교체 | XII.10.6 |
| Option | 영역에 완전히 포함된 node만 | XII.10.6 |
| ⌘ | 편집 범위와 무관하게 가장 깊은 선택 가능 node | XII.10.6 |
| Shift | 시작 전 선택에 더함(`AreaSelectionMode::Add`) | XII.10.6, VII.3.3 |

- 진행 중 결과는 transient preview이고 pointer up에서 `SetSelection` 하나만 만듭니다. [계획 VII.3.3]
- 마퀴 수식키의 의미는 도구마다 다르므로 위 표를 Stellar 규칙으로 고정합니다. [관례]

### 4.5 hover

- hover 윤곽은 지금 클릭하면 선택될 node를 그대로 보여 줍니다(1.2 원칙 5). ⌘를 누르면 깊은 대상으로 바뀝니다. [계획 XII.10.7]
- 사이드바 레이어 행 hover와 캔버스 hover는 같은 hover 상태 하나를 씁니다. 행에 올리면 캔버스에 윤곽이, 캔버스에서 올리면 해당 행 강조가 보입니다. [관례] [사이드바 1장]
- hover는 history와 저장 대상이 아닙니다. 예산은 목표 5ms·상한 8ms입니다. [아키텍처 §6.4, §23.3]

### 4.6 캔버스와 레이어 목록의 차이

레이어 목록은 `NSOutlineView` 규칙(Shift 범위 선택, ⌘ 개별 추가)을 그대로 쓰고, 캔버스는 4.1절 규칙(Shift 토글 추가)을 씁니다. 두 곳의 선택은 같은 WindowViewState 하나입니다. [가이드 6.2] [HIG Lists and tables] [계획 VII.3.1]

---

## 5. 변형 핸들

### 5.1 핸들 구성

| 핸들 | 위치 | 표시 조건 | 구현 |
|---|---|---|---|
| 크기 | 네 모서리, 네 변(변 전체가 hit 영역) | 선택 1개 이상 | XII.10.8 |
| 회전 | 모서리 바로 바깥 영역(보이는 표식 없음, 커서로 알림) | 회전 가능한 node | XII.10.8, XII.10.10 |
| 모서리 반경 | 모서리 안쪽 | Frame·Rectangle·Polygon 단일 선택, 화면 크기가 기준 이상 | XII.10.8, XII.10.11 |
| padding·gap | 흐름 컨테이너 안쪽 가장자리와 자식 사이 | 흐름 컨테이너 선택 후 hover | XII.10.25 |
| 간격·자리 바꾸기 | 같은 간격으로 놓인 선택의 사이와 가운데 | 다중 선택이 등간격 | XII.10.22 |

- hit 우선순위는 반경 → 크기 → padding·gap → 회전 → node 본체입니다. 화면에서 너무 작아진 선택은 크기 변 핸들과 반경 핸들을 숨기고 본체 드래그를 우선합니다. 기준 크기는 XII.10.2가 정합니다. [관례] [계획 XI.3.5]
- 회전된 node의 핸들은 node local 좌표로 계산합니다. [계획 XII.10.8]
- 핸들은 transient overlay이며 project snapshot에 들어가지 않습니다. [계획 XI.3.5, XII.4.4]

### 5.2 크기 조절

| 조작 | 결과 속성(속성 모델) | 구현 |
|---|---|---|
| 모서리·변 드래그 | `width`·`height`와 필요하면 위치 | XII.10.9 |
| Shift | 비율 유지. `aspect-ratio`가 고정이면 반대로 일시 해제 | XII.10.9 |
| Option | 중심 기준(반대편도 같이 움직임) | XII.10.9 |
| 반대편을 넘어감 | 0에서 멈추지 않고 뒤집기로 해석(Text는 0에서 멈춤). 결과는 5.6절과 같은 `scale` 부호 반전이고, 고정된 반대편 변의 world 위치가 남도록 위치를 보정 | XII.10.9 |
| 다중 선택 | 선택 bounds 기준으로 각 node의 위치·크기를 비례 배치 | XII.10.9 |
| 흐름 컨테이너 자식 | 조절한 축을 Fixed preset으로 바꿈(채움·내용 맞춤 해제) | XII.10.9, IX.11.7 |
| 변 핸들 더블클릭 | 그 축을 HugContent preset(`width: fit-content` 등)으로 | XII.10.9, IX.11.7 |

- sizing preset과 longhand의 대응은 속성 모델 4.3절 표이고, 변환은 계획 IX.11.7 함수가 합니다. 크기 longhand는 IX.11.6 영속 record, Shape·Image·Text의 PropertyKey는 IX.11.3이 만듭니다. [속성 모델 4.3, 6장] [계획 IX.11.3, IX.11.6, IX.11.7]
- 크기 조절은 선 두께, 반경, 효과 반경, 글자 크기를 바꾸지 않습니다. 그것까지 비례로 바꾸는 것은 5.5절의 Scale입니다. [관례]
- 부모 크기가 바뀔 때 위치 지정 자식이 반응하는 규칙은 레이아웃 엔진의 anchor 규칙을 따릅니다. 캔버스는 별도 규칙을 두지 않습니다. [계획 X.2.2, X.2.3]

### 5.3 회전

- 회전 영역을 드래그하면 pivot을 기준으로 돌리고, Shift는 15도 단위입니다. pivot은 단일 선택이면 node border box 중심, 다중 선택이면 선택 bounds 중심입니다. [관례] [계획 XII.10.10]
- 결과는 `rotate` 편집입니다. 제스처는 저장된 `transform-origin`을 바꾸지 않습니다. migration된 노드는 `transform-origin: 0 0`을 저장하므로(속성 모델 5장, 계획 IX.11.9) origin을 고쳐 쓰면 선형 `transform` 행렬을 보존한 노드(이후 사용자가 `scale`을 더한 경우 포함)의 외형이 움직이기 때문입니다. 저장된 origin이 pivot과 다르면 pivot의 world 위치가 그대로 남도록 위치를 보정합니다. 위치 지정 자식은 현재 anchor의 inset을, 흐름 자식은 `translate`를 바꿉니다(흐름 자식의 위치는 흐름이 정함). 새 노드의 기본 origin은 `50% 50%`이므로 단일 선택 회전에는 보정이 없습니다. [속성 모델 5장, 12장] [계획 XII.10.10, IX.11.9]
- 회전은 흐름 크기에 영향을 주지 않습니다. 회전한 흐름 자식은 형제 배치를 바꾸지 않습니다. [계획 X.2.4]
- 회전 중심을 캔버스에서 옮기는 핸들은 열린 결정 C3입니다. 핸들을 둔다면 그 핸들만 `transform-origin`을 바꾸며, 같은 보정으로 외형을 유지합니다.

### 5.4 모서리 반경

- Frame·Rectangle에서 반경 핸들 드래그는 네 모서리를 함께, Option은 그 모서리만 바꿉니다. 결과는 `border-*-radius` longhand입니다(속성 모델 5장·6장). [관례] [계획 XII.10.11]
- Polygon은 모든 꼭짓점에 쓰는 반경 하나만 가지므로(속성 모델 6장 `stellar-only`) 핸들은 그 값 하나를 바꾸고 Option 개별 편집이 없습니다. [계획 VI.5.5, XII.10.11]
- Frame·Rectangle에서 bounds보다 큰 반경은 저장값을 유지하고 표시만 평가 규칙으로 줄입니다. 핸들 옆 수치는 저장값을 보여 줍니다. Polygon 표시는 Polygon path 평가를 따릅니다. [계획 VI.5.2, VI.5.6]

### 5.5 비례 확대(Scale)

- Object 메뉴 명령과 인스펙터 비율 입력으로 실행하며, 크기와 함께 선 두께(modifier Stroke 두께와 Frame `border-*-width`)·반경·그림자와 블러 반경·글자 크기(`font-size`)를 같은 비율로 바꿉니다. modifier 값은 `SetModifierProperty`로 함께 바꿔 한 transaction으로 묶습니다. `font-size`는 Typography record(계획 IX.11.11)에, Frame border longhand는 IX.11.10에, Shape·Text 속성의 PropertyKey는 IX.11.3에 생깁니다. Scale은 메뉴 막대 Object 메뉴에도 추가합니다(XII.10.12). [관례] [속성 모델 7장] [계획 XII.10.12, VII.2.6, IX.11.3, IX.11.10, IX.11.11]
- 캔버스 단일 키를 줄지는 XII.10.2가 정합니다(도구·메뉴 6.2절은 Shift+글자를 도구 변형용으로 비워 두었습니다).

### 5.6 뒤집기

Object ▸ Flip Horizontal·Flip Vertical은 선택 bounds 중심 기준 반전 편집입니다. 반전은 `scale` 부호(예: `scale: -1 1`)로 표현합니다. [도구·메뉴 3.2] [속성 모델 5장] [계획 XII.10.44]

- 반전 명령은 회전(5.3절)과 같이 저장된 `transform-origin`을 바꾸지 않습니다. 선택 bounds 중심이 그대로 남도록 위치를 보정합니다. 위치 지정 자식은 현재 anchor의 inset을, 흐름 자식은 `translate`를 바꿉니다. 새 노드(origin `50% 50%`)의 단일 선택 반전에는 보정이 없고, migration 노드(`transform-origin: 0 0`)는 bounds 중심 world 위치가 남도록 보정합니다. [속성 모델 5장] [계획 XII.10.44, IX.11.9]
- 회전된 node를 local 축과 world 축 중 어느 축 기준으로 반전할지, `scale` longhand가 없는 node 종류(속성 모델 12장 Transform 대상 밖)를 어떻게 반전할지는 XII.10.44가 정하고 결정과 근거를 작업 보고서에 기록합니다. 크기 조절의 뒤집기(5.2절)는 핸들이 local 좌표이므로 local 축 기준입니다. [계획 XII.10.8, XII.10.9, XII.10.44]

---

## 6. 이동과 키보드 이동

### 6.1 드래그 이동

| 조작 | 결과 | Command | 구현 |
|---|---|---|---|
| 선택 드래그 | 위치 longhand 편집(위치 지정 자식은 현재 anchor의 inset, Frame은 계획 IX.11.9, 그 밖의 node는 IX.11.6 item record) | `SetProperty`(transaction) | XII.10.13 |
| Shift | 처음 움직인 축으로 고정 | 같음 | XII.10.13 |
| Option으로 시작 | 복제 후 복제본 이동 | `DuplicateSubtree` + `SetProperty` | XII.10.13, VII.2.8 |
| 자유 배치 컨테이너 사이로 끌기 | world 위치를 유지하며 부모 변경 | `MoveNode` + `SetProperty` | XII.10.13, VII.2.3 |
| 흐름 컨테이너 안·밖으로 끌기 | 10.1절 | `MoveNode` | XII.10.24 |
| 인스턴스 안으로 끌기 | 공개 Slot에만 놓임, 그 밖은 거부 | Slot 삽입 | XII.10.34, IX.3.3 |

- 새 부모 후보는 pointer 아래에서 자식을 받을 수 있는 가장 깊은 컨테이너입니다. 재부모화를 막는 수식키는 열린 결정 C4입니다. [관례]

### 6.2 키보드 이동

| 입력 | 결과 | 구현 |
|---|---|---|
| 화살표 | 작은 단위 이동 | XII.10.14 |
| Shift-화살표 | 큰 단위 이동 | XII.10.14 |
| 키보드 크기 조절 키(XII.10.2) | 오른쪽·아래 가장자리 이동으로 크기 조절 | XII.10.14 |
| 흐름 컨테이너 자식에서 주축 화살표 | 흐름 순서 앞뒤로 이동(`MoveNode`) | XII.10.14 |
| 선택 없음에서 화살표 | camera 팬(키보드만으로 팬하는 경로) | XII.10.14 |

- 키 반복은 같은 command signature와 간격 조건으로 한 entry로 합칩니다. [계획 VII.4.4]
- 이동 단위의 값은 XII.10.2가 정하고 앱 설정에서 바꿀 수 있게 합니다. [가이드 11장 O10] [관례]

---

## 7. 스냅·가이드·눈금자·그리드

### 7.1 스냅 엔진

| 후보 | 설명 | 구현 |
|---|---|---|
| 가이드, 레이아웃 그리드 | 사용자가 둔 기준 | XII.10.15, XII.10.19, XII.10.20 |
| 객체 가장자리·중심 | 편집 범위 형제와 부모 | XII.10.15 |
| 같은 간격 | 이미 같은 간격으로 놓인 형제와 같은 간격 | XII.10.15 |
| 같은 크기 | 크기 조절 중 같은 너비·높이를 가진 형제 | XII.10.15 |
| 픽셀 그리드 | 논리 pixel 경계 | XII.10.15, XII.10.20 |

- 우선순위는 위 표 순서입니다. 허용 오차는 화면 pt 기준이라 줌과 무관하게 같은 손맛을 냅니다. 값은 XII.10.2가 정합니다. [관례]
- 후보 집합은 제스처 시작 때 가시 영역 공간 인덱스에서 한 번 만들고 update마다 재사용합니다. 계산은 Hit Test·인터랙션 해석 예산(목표 0.35ms, 상한 0.75ms) 안에 둡니다. [아키텍처 §6.2] [계획 XI.3.1, XI.3.2]
- 레이아웃 결과는 1/64 논리 pixel 단위로 반올림하고 device pixel snapping은 렌더 단계 몫입니다. 픽셀 그리드 스냅은 이것과 별개로 사용자 편집 값을 정수 논리 pixel에 맞추는 기능입니다. [계획 X.1.5]
- 흐름 컨테이너 자식은 위치를 흐름이 정하므로 객체 스냅을 받지 않습니다. 대신 10.1절 삽입 위치 표시가 같은 역할을 합니다.

### 7.2 스마트 가이드와 측정

- 이동·생성·크기 조절 중 정렬선, 가장자리 사이 거리 수치, 같은 간격 표시, 같은 크기 표시를 transient overlay로 그립니다. [관례] [가이드 1.2 원칙 2] [계획 XII.10.16]
- node를 선택한 채 Option을 누르고 다른 node에 hover하면 두 node 사이 거리를 표시합니다. Text는 상자 기준이 기본이고 baseline 기준 측정 전환은 열린 결정 C5입니다. [관례]
- 대비 높이기에서는 선을 더 굵고 진하게, 투명도 줄이기에서는 반투명 바탕을 불투명으로 그립니다. 값은 ShellTheme 이벤트로 받습니다. [가이드 8.4, 11장 O6] [계획 XII.9.3]

### 7.3 스냅 토글

- View 메뉴에 객체 스냅, 픽셀 그리드 스냅, 스마트 가이드 표시 토글을 `ShellCommand`로 둡니다. 값은 앱 설정이며 문서·Git에 넣지 않습니다. [가이드 8.1] [계획 XII.10.17]
- 드래그 도중 ⌘(3.3절)로 스냅을 일시 해제합니다. [관례]

### 7.4 눈금자와 가이드

- ⌘R로 눈금자를 보이고 숨깁니다(도구·메뉴 6.1절의 `view.toggle_rulers`). 눈금 원점은 선택된 최상위 Frame의 왼쪽 위입니다. [도구·메뉴 6.1] [관례] [계획 XII.10.19]
- 눈금자에서 끌어 가이드를 만들고, 눈금자로 다시 끌어 지웁니다. 가이드는 스냅 후보가 됩니다. 가이드 위치 숫자 입력은 `NSPopover`와 `NSTextField`로 합니다. [관례] [가이드 5.1]
- 가이드를 문서 데이터로 저장할지 창별 view 상태로 둘지는 XII.10.18 ADR이 정합니다. 문서 데이터가 되면 가이드 추가·이동·삭제는 일반 Command로 history와 Semantic Diff에 들어갑니다. [아키텍처 §3.3, §3.4]
- 눈금자를 렌더러 transient layer로 그릴지 AppKit 표준 구성요소로 그릴지는 열린 결정 C1입니다. camera와 같은 프레임에 움직여야 하는 점과 가이드 1.2 원칙 3이 함께 걸립니다.

### 7.5 그리드

- 레이아웃 그리드(열·행·폭·gutter)는 Frame에 딸린 설계 기준이며 스냅 후보가 됩니다. 저장 위치는 가이드와 같은 ADR(XII.10.18)이 정합니다. 이 그리드는 CSS Grid 컨테이너(흐름 레이아웃)와 다른 것으로, 자식 배치를 바꾸지 않습니다. [관례] [계획 XII.10.20]
- 픽셀 그리드는 일정 줌 이상에서만 보이고, 스냅은 표시 여부와 무관하게 7.3절 설정을 따릅니다. [관례]

---

## 8. 정렬·분배·간격

| 명령 | 기준 | Command | 구현 |
|---|---|---|---|
| Align 6종(왼쪽·가로 가운데·오른쪽·위·세로 가운데·아래) | 단일 선택은 부모, 다중 선택은 선택 bounds, 기준 객체가 있으면 그 객체 | `SetProperty` transaction | XII.10.21 |
| Distribute 2종 | 가장 바깥 두 객체 고정, 사이를 같은 간격으로 | 같음 | XII.10.21 |
| 행·열 정리(Arrange in Rows and Columns) | 선택을 같은 간격 행·열로 재배치. 흐름 컨테이너를 만들지 않고 각 node의 위치만 바꿈 | 같음 | XII.10.21 |
| 간격 핸들 드래그 | 등간격 선택의 간격을 함께 바꿈 | 같음 | XII.10.22 |
| 자리 바꾸기 핸들 드래그 | 이웃과 위치 교환 | 같음 | XII.10.22 |

- 기준 객체는 primary selection입니다. 다중 선택에서 이미 선택된 node를 Option-클릭하면 primary로 지정합니다. [계획 VII.3.1] [관례]
- 회전한 node는 world bounds로 정렬합니다. [계획 XII.10.21]
- 흐름 컨테이너 자식의 정렬 명령은 컨테이너 방식에 따라 다음 규칙을 씁니다. 분배와 행·열 정리는 흐름 자식에서 모두 비활성화합니다. [속성 모델 4.2, 4.4] [계획 X.3.1, IX.11.4, IX.11.5, IX.11.6]
  - Stack 자식: 주축 정렬은 흐름이 정하므로 비활성화하고, 도움말 태그로 컨테이너의 `justify-content`를 바꾸라고 안내합니다. 교차축 정렬은 그 자식의 `align-self`(`flex-start`·`center`·`flex-end`)를 바꿉니다. `align-self`는 계획 IX.11.4가 레이아웃 엔진에, IX.11.6이 영속 record에 추가합니다.
  - Grid 자식: 가로 정렬은 `justify-self`, 세로 정렬은 `align-self`(`start`·`center`·`end`)를 바꿔 grid 영역 안에서 맞춥니다. 두 속성은 계획 IX.11.5가 엔진에, IX.11.6이 영속 record에 추가합니다.
  - Overlay 자식: 컨테이너 `justify-items`·`align-items`가 정하므로 비활성화하고 도움말 태그로 안내합니다.
- 흐름 밖 node의 정렬·분배·행·열 정리는 위치 longhand(위치 지정 자식은 현재 anchor의 inset)를 바꿉니다. [계획 IX.11.6, IX.11.9]
- 바꾼 값은 인스펙터 Position 섹션에 보입니다(속성 모델 12장).
- 메뉴 위치와 단축키는 도구·메뉴 명세의 Object ▸ Align·Distribute를 씁니다. 인스펙터 위쪽에 같은 명령의 momentary segmented control을 둘지는 인스펙터 명세가 정합니다. [도구·메뉴 3.2, 6.1] [HIG Segmented controls]

---

## 9. 순서와 묶음

| 명령 | 단축키(도구·메뉴 6.1) | Command | 구현 |
|---|---|---|---|
| Bring Forward, Send Backward | ⌘], ⌘[ | `MoveNode`(OrderKey 변경) | XII.10.23 |
| Bring to Front, Send to Back | 없음 | 같음 | XII.10.23 |
| Group | ⌥⌘G | `InsertNode`(Fragment) + `MoveNode`×n | XII.10.23 |
| Frame Selection | 없음 | `InsertNode`(Frame) + `MoveNode`×n | XII.10.23 |
| Ungroup | ⌥⇧⌘G | `MoveNode`×n + `DeleteSubtree`(빈 Fragment) | XII.10.23 |
| Convert to Frame(Fragment 선택) | 없음 | `ConvertFragmentToFrame` | VII.2.7 |

- Group은 좌표계가 없는 Fragment로 만듭니다. 변형·효과가 필요하면 Frame Selection 또는 Convert to Frame을 씁니다. [아키텍처 §12.4]
- Frame을 풀 때는 자식의 world 위치를 보존합니다. Frame의 칠·테두리는 사라지지만 Undo로 돌아오므로 확인 대화상자를 두지 않습니다. [아키텍처 §23] [HIG Alerts]
- 흐름 컨테이너에서는 그리기 순서가 곧 흐름 순서입니다. 따라서 순서 명령이 자식 위치도 바꿉니다. 메뉴 도움말 태그와 접근성 알림으로 이 사실을 알립니다. 흐름 순서와 쌓임 순서는 나누지 않습니다. CSS `order`와 `z-index`를 쓰지 않고 자식 visual order가 곧 paint 순서입니다. [계획 X.5.1] [속성 모델 3장, 14장 E3]

---

## 10. 레이아웃 인지 편집

PO 방향에 따라 레이아웃은 CSS의 Flexbox·Grid·positioning 개념으로 편집합니다. 속성 이름과 기본값은 속성 모델이 정하고, 캔버스는 그 속성을 직접 조작하는 손잡이입니다. [PO 2026-11-14 편집 UX] [속성 모델] [아키텍처 §13]

이 장이 편집하는 속성은 지금 `crates/layout` 엔진 입력에만 있고 문서에 저장되지 않습니다. Frame payload는 layout 정의 참조(`layout_mode: Option<StableId>`, `crates/document/src/frame.rs`)만 갖고, Stack 엔진에는 자식별 `align-self`가 없습니다(`crates/layout/src/stack.rs`는 컨테이너 `cross_alignment`만 가짐). 레이아웃 container·item longhand의 영속 record는 계획 IX.11.6, Flexbox 의미 정렬은 IX.11.4, Grid·positioning 의미 정렬은 IX.11.5, sizing preset 변환은 IX.11.7이 만듭니다. 그래서 이 장의 구현 행(XII.10.21, XII.10.24, XII.10.25)은 IX.11.6을 선행으로 둡니다. [속성 모델 4장] [계획 IX.11.4\~IX.11.7]

### 10.1 흐름 컨테이너 안 드래그

- 자식을 끄는 동안 놓일 자리를 삽입 표시선으로 보여 주고, 놓으면 OrderKey를 바꾸는 `MoveNode` 하나를 만듭니다. 형제는 preview 단계에서 증분 레이아웃으로 비켜 줍니다. [관례] [계획 XII.10.24, X.3.2, X.4.4]
- Grid는 자동 배치 순서에서의 index를 바꿉니다. 명시 배치(행·열 지정) 자식은 셀 이동으로 해석할지 열린 결정 C6입니다.
- 컨테이너 밖으로 끌어내면 대상 부모의 규칙(흐름이면 삽입, 자유 배치면 world 위치 유지)으로 옮깁니다. [계획 XII.10.24]
- ComponentInstance 내부 재정렬은 거부합니다. [계획 IX.2.5]

### 10.2 위치 지정 자식

흐름에서 빠진 자식(CSS `position: absolute`)과 자유 배치 컨테이너의 자식은 inset(`left`·`top`·`right`·`bottom`) 편집으로 움직입니다. 어느 변에 고정되는지(anchor)는 인스펙터에서 바꾸고, 캔버스 드래그는 현재 anchor의 inset 값만 바꿉니다. inset의 기준은 부모 padding box입니다(계획 IX.11.5). [속성 모델 4.5] [계획 X.2.2, IX.11.5, IX.11.6]

### 10.3 padding·gap 핸들

| 조작 | 결과 속성 | 구현 |
|---|---|---|
| 흐름 컨테이너 선택 후 안쪽 가장자리 드래그 | `padding-top`·`padding-right`·`padding-bottom`·`padding-left` | XII.10.25 |
| Option + padding 드래그 | 마주 보는 변을 함께 | XII.10.25 |
| Stack 자식 사이 핸들 드래그 | 주축 gap longhand 하나. `flex-direction`이 `row`·`row-reverse`이면 `column-gap`, `column`·`column-reverse`이면 `row-gap` | XII.10.25 |
| 줄바꿈 Stack(`flex-wrap: wrap`)의 줄 사이 핸들 드래그 | 교차축 gap longhand 하나(주축 핸들이 바꾸지 않는 쪽) | XII.10.25 |
| Grid 열 사이·행 사이 핸들 드래그 | 열 사이는 `column-gap`, 행 사이는 `row-gap` | XII.10.25 |
| Shift + 드래그 | 큰 단위 | XII.10.25 |
| 핸들 클릭 | 그 핸들의 longhand 숫자 입력 popover(`NSPopover` + `NSTextField`) | XII.10.25 |

- gap 핸들은 자기 축의 longhand 하나만 바꾸고 다른 축 gap 값은 그대로 둡니다. 캔버스는 `gap` shorthand를 쓰지 않습니다. `gap`은 인스펙터 입력·복사 표현이며, 입력하면 `row-gap`·`column-gap` 두 longhand로 펼칩니다. Stellar의 main·cross 기준 gap을 축 고정 CSS 이름으로 바꾸는 규칙은 속성 모델 4.2절을 따릅니다. [속성 모델 4.2, 11장] [ADR-0024 결정 4]
- gap은 음수가 되지 않습니다(CSS `row-gap`·`column-gap`과 같음). 겹쳐 놓기는 Overlay 컨테이너나 위치 지정 자식으로 합니다. [속성 모델 4.2]
- 남는 공간 분배(CSS `justify-content`의 `space-between` 등)가 켜진 축의 gap 핸들은 값을 보여 주기만 하고 드래그를 받지 않습니다. [관례]

### 10.4 크기 방식

흐름 자식의 크기 방식(고정, 채움, 내용 맞춤)은 5.2절 규칙으로 바뀝니다. 채움은 CSS의 `flex-grow`·`align-self: stretch` 계열, 내용 맞춤은 `fit-content`로 표현합니다(속성 모델 4.3절 sizing preset, 계획 IX.11.7). 부모가 내용 맞춤이고 자식이 채움인 경우처럼 서로 의존하는 조합은 레이아웃 엔진 진단을 인스펙터와 캔버스 윤곽에 함께 표시합니다. [계획 X.6.5] [속성 모델]

---

## 11. 생성 도구

| 조작 | 결과 | Command | 구현 |
|---|---|---|---|
| 드래그 | 영역 크기로 생성 | `InsertNode` + `SetSelection` | XII.10.26 |
| 클릭 | 기본 크기로 생성 | 같음 | XII.10.26 |
| Shift / Option / Space | 정비례·45도 / 중심 기준 / 시작점 이동 | 같음 | XII.10.26 |
| 흐름 컨테이너 위에서 생성 | pointer 위치의 흐름 순서에 삽입 | 같음 | XII.10.26 |
| root Frame 밖 빈 캔버스에서 생성 드래그 | 만들지 않음. 금지 커서와 도구 막대 위 한 줄 "Frame 안에 그립니다"(2.5초, VoiceOver 알림) | 없음 | UX 명세 5.2(결정 U5·U7), UX.2.8 |
| Insert 메뉴 항목 선택(키보드) | 화면 가운데에 기본 크기로 생성 | 같음 | XII.10.41 |
| Image… / 파일 끌어 놓기 | asset import 후 Image 삽입 | import + `InsertNode` | XII.10.27, XIII.5.2 |

- 생성과 새 선택은 한 entry입니다. Undo 한 번에 생성 전 상태와 선택으로 돌아갑니다. [계획 XII.10.40]
- Frame 도구로 기존 node 위를 그려도 그 node를 자동으로 품지 않습니다. 품기는 Frame Selection으로 합니다. [관례]

현재 macOS 기본 창에서는 자유 배치 Frame 안에서 Frame·Rectangle·Ellipse·Line·Polygon을 드래그해 추가합니다.
화면에서 두 변이 각각 3pt 이상일 때 확정합니다. Line은 두 점 사이의 거리만 3pt 이상이면 수평·수직·역방향 모두 확정합니다.
Ellipse·Polygon은 클릭하면 부모 좌표의 100×100, Line은 오른쪽 100 길이로 생성합니다. Polygon 기본값은 도구 아이콘과 같은 정오각형이며 첫 꼭짓점은 위를 향합니다.
이 세 도구에서 Shift는 정비례 또는 Line의 45도 간격, Option은 중심 기준입니다. 생성 직후 새 node를 선택하고 Move로 돌아갑니다.
Frame·Rectangle의 기본 크기 클릭과 수식키, 드래그 중 Space로 시작점 이동 및 흐름 배치는 후속 범위입니다.

---

## 12. 벡터 편집

### 12.1 Path 모델

Path command는 MoveTo·LineTo·CubicTo·Close만 씁니다. point와 handle은 StableId를 가져 Semantic Diff가 point 단위로 됩니다. [계획 VI.6.1, VI.6.3]

### 12.2 벡터 편집 모드

- Path에서 Return·더블클릭으로 들어가고 Escape로 나옵니다. point·handle 선택, 이동, 추가·삭제, 대칭 모드 전환은 Path point 편집 Command(XII.10.28)로 기록하며 point drag 하나가 한 entry입니다. [관례] [계획 XII.10.28, XII.10.29]
- Shape는 원본 형상을 유지하는 노드이므로 더블클릭으로 Path로 바꾸지 않습니다. 사용자가 Convert to Path를 명시적으로 실행한 뒤에만 벡터 편집으로 들어갑니다. [아키텍처 §12.6] [계획 VI.6.5, VII.2.7]
- 한 point에서 여러 방향으로 갈라지는 vector network는 지원하지 않습니다. Path는 subpath 목록입니다. 꼭짓점별 둥글리기는 Corner modifier로 합니다. [계획 VI.6.2, VI.9.2]

---

## 13. Boolean 연산

- 같은 부모의 Shape·Path를 둘 이상 선택하고 Object ▸ Boolean(Union, Subtract, Intersect, Exclude) 또는 문맥 메뉴로 실행합니다. 업계 공통의 4가지 연산입니다. [관례] [계획 XII.10.31]
- 결과는 비파괴입니다. 맨 뒤 node에 Boolean modifier를 넣고 나머지 node를 operand로 연결합니다(`InsertModifier`). operand node는 문서에 남아 계속 편집할 수 있습니다. [아키텍처 §12.6] [계획 VI.9.4, VII.2.6]
- operand로 쓰인 node를 따로 그릴지와 operand 순서 규칙은 XII.10.30 ADR이 정합니다.
- 연산 종류 바꾸기는 `SetModifierProperty`, operand 분리는 modifier 갱신입니다. 결과를 Path로 굽는 평탄화는 별도 명시 명령으로만 실행합니다. [아키텍처 §12.6] [계획 XII.10.31]

---

## 14. 캔버스 텍스트

### 14.1 만들기와 들어가기·나오기

| 입력 | 결과 | 구현 |
|---|---|---|
| Text 도구 클릭 | 자동 폭 Text(CSS `width: max-content` 계열) 생성 후 편집 모드 | XII.10.33 |
| Text 도구 드래그 | 고정 폭 Text(폭 고정, 높이 자동) 생성 후 편집 모드 | XII.10.33 |
| Text 더블클릭, Return | 편집 모드(더블클릭은 클릭 위치에 caret) | XII.10.33 |
| 편집 중 더블클릭·세 번 클릭 | 단어·문단 선택 | XII.10.33 |
| Escape, 바깥 클릭 | 편집 확정, Text node 선택으로 돌아감 | XII.10.33 |
| 상자 변 드래그 | 자동 폭을 고정 폭으로, 높이 변은 고정 크기로 | XII.10.33 |

크기 방식의 속성 표현은 속성 모델을 따릅니다. 새로 만든 Text를 입력 없이 나오면 생성 자체를 취소해 history를 남기지 않습니다. 이미 있던 Text를 비우면 빈 Text node로 남습니다. [계획 VI.7.4] [관례]

### 14.2 입력과 IME

- 캔버스 Text 편집은 공통 editing session과 `NSTextInputClient` adapter로 marked text, 선택 범위, 후보창 위치를 연결하고, 표시는 공통 텍스트 엔진이 합니다. 네이티브 텍스트 컨트롤을 캔버스 위에 겹쳐 그리지 않습니다. [아키텍처 §11.1] [가이드 5.2] [계획 XII.5.3, XII.5.4, XII.5.6]
- 조합 중 문자열은 transient로 그리고, 확정될 때만 Text 범위 편집 Command(XII.10.32)를 만듭니다. 연속 입력은 VII.4.3 규칙으로 묶습니다. [계획 VII.4.3]
- caret, 선택 사각형, 클릭 위치 → 문자 위치 대응은 텍스트 레이아웃이 만든 값을 씁니다. [계획 X.10.3]
- 편집 중 단일 키 도구 전환은 일어나지 않습니다. 캔버스 뷰가 키를 먼저 `interpretKeyEvents(_:)`로 텍스트 입력에 넘기기 때문입니다. [가이드 8.2]
- 선택 범위의 글자 속성(인스펙터 Character 섹션) 편집도 같은 Command의 범위 style 설정으로 보냅니다. [인스펙터 9장] [계획 XII.10.32]

---

## 15. 컴포넌트 인스턴스 편집

| 조작 | 결과 | Command | 구현 |
|---|---|---|---|
| 인스턴스 클릭 | 인스턴스 전체 선택 | `SetSelection` | XII.10.3 |
| 인스턴스 안 깊은 선택 | 안쪽 node를 읽기 전용 윤곽으로 선택 | `SetSelection` | XII.10.34 |
| 공개 Property에 연결된 값 편집(캔버스·인스펙터) | override 생성·변경 | `PropertyOverrideCommand::Set` | XII.10.34, IX.2.4 |
| 허용되지 않은 편집 시도 | 적용하지 않고 이유와 Detach 경로를 표시 | 없음 | XII.10.34, IX.2.3, IX.4.2 |
| 문맥 메뉴 Reset | 속성 하나 또는 전체 override 삭제 | `PropertyOverrideCommand::Reset` | XII.10.34 |
| 문맥 메뉴 Detach Instance | 가상 subtree를 실제 node로 복제 | detach | XII.10.34, IX.2.6 |
| 공개 Slot에 끌어 놓기 | Slot 콘텐츠 삽입 | Slot 삽입 | XII.10.34, IX.3.3 |
| 문맥 메뉴 Swap | 다른 정의로 교체, 이름·타입이 같은 공개 Property override 보존 | 인스턴스 교체 | XII.10.35 |
| 문맥 메뉴 정의로 이동 | 정의의 Root Frame을 캔버스에 열고 선택 | 없음(view 상태) | XII.10.34 |

- 인스턴스 편집은 공개 Property, 허용된 Override, 공개 Slot으로 제한하고 자유 편집은 명시적 Detach로만 엽니다. 중첩 인스턴스 내부를 바깥에서 직접 고치지 않으며, 안쪽 Property를 바깥에서 다루려면 명시적 Property Drilling을 거칩니다. [아키텍처 §14.3, §15.1] [계획 IX.4.2, IX.4.3]
- override된 값은 인스펙터와 캔버스 윤곽에서 기본값과 구분해 표시합니다. 표시 방식은 인스펙터 명세가 정합니다. [인스펙터 9장]
- Swap 전에 보존하지 못하는 override 목록을 preview로 보여 줍니다. override가 사라지는 일을 조용히 일으키지 않는 것이 목적입니다. [관례] [계획 XII.10.35]
- Context 공급·소비(PO 방향)는 ADR-0025 결정 3과 아키텍처 §14.5가 선언된 인터페이스로 정했습니다. 인스펙터의 Value Binding 선택기, 제공한 조상 node로 이동, requirement·Slot 공개 목록 표시는 로드맵 XXX.7.12가 맡고, 캔버스에는 별도 Context 표시를 두지 않습니다. [ADR-0025 결정 3] [아키텍처 §14.5] [계획 XXX.7.12]

---

## 16. 줌·팬

| 입력 | 결과 | 구현 |
|---|---|---|
| 트랙패드 핀치 | 커서 기준 연속 줌 | XII.10.36, XII.10.37 |
| 트랙패드 두 손가락 더블탭(smart magnify) | 선택 맞춤과 이전 배율 사이 전환 | XII.10.36, XII.10.37 |
| 두 손가락 스크롤, 마우스 휠 | 팬(관성 phase 유지) | XII.10.37 |
| 마우스 Shift + 휠 | 가로 팬 | XII.10.37 |
| ⌘ + 스크롤 | 커서 기준 줌 | XII.10.37 |
| Space 드래그, Hand 도구 | 팬 | XII.10.1, XII.10.37 |
| Zoom 도구 클릭·Option-클릭·영역 드래그 | 확대·축소·영역 맞춤 | XII.10.37 |
| ⌘+ ⌘- ⌘0 ⌘9 ⌘* | 확대·축소·실제 크기·전체 맞춤·선택 맞춤 | 도구·메뉴 6.1, XII.4.3 |
| 선택 없음에서 화살표 | 팬 | XII.10.14 |

- macOS의 Zoom 도구(Z)에서 클릭하면 그 위치를 고정한 채 한 단계 확대하고 Option-클릭하면 한 단계 축소합니다. 사각형을 드래그하면 그 영역이 캔버스 가운데에 맞춰집니다. 드래그 중 Escape로 취소할 수 있습니다.
- 줌·팬은 camera 입력일 뿐 문서 geometry를 바꾸지 않고 history에 넣지 않습니다. [계획 XII.4.3] [아키텍처 §23.3]
- 현재 `InputEvent`에는 확대 제스처 종류가 없으므로 XII.10.36에서 추가합니다. 핀치는 `NSEvent` magnify 이벤트를 직접 받아 연속 move처럼 병합합니다. [계획 XII.3.1, XII.3.4] [AppKit 문서 NSEvent]
- 지금 `StellarMacEvent`에는 확대 값 필드가 없고 셸 변환은 magnify 이벤트를 변환하지 않습니다(`crates/ffi/src/input_macos.rs`). 그래서 XII.10.36은 구조체 필드와 입력 encoding을 함께 바꾸며, ADR-0014 Compatibility에 따라 `STELLAR_MAC_EVENT_VERSION`과 `INPUT_EVENT_SCHEMA_VERSION`을 올리고 이전 version은 거부합니다. `crates/ffi` 변경이므로 같은 diff에 ADR-0014 갱신이 있어야 합니다. [ADR-0014] [계획 I.3.5, XII.10.36]
- 줌 단계와 상하한은 XII.10.2가 정합니다. 캔버스 Pan·Zoom은 장식 애니메이션을 두지 않고 입력에 즉시 반응합니다. 맞춤 명령의 이동 애니메이션은 동작 줄이기가 켜지면 즉시 전환합니다. [가이드 8.5] [아키텍처 §6.4]
- 스크롤 방향은 시스템 설정을 따릅니다. 캔버스에는 끝이 없으므로 탄성 경계를 두지 않습니다. [HIG Mouse and trackpads]

---

## 17. 클립보드

- Copy·Cut은 선택 subtree를 Stellar 전용 pasteboard type으로 쓰고, Text가 있으면 일반 텍스트 표현도 함께 씁니다. 이미지·SVG 표현은 내보내기 로드맵(Phase XX)에 맞춰 나중에 더합니다. [계획 XII.10.38] [아키텍처 §34]
- Paste 위치: 선택한 컨테이너가 자식을 받을 수 있으면 그 안, 아니면 원본 위치가 화면에 보이면 같은 위치, 아니면 화면 가운데입니다. 붙여 넣은 subtree는 새 StableId를 받습니다. [관례] [계획 III.1.5, VII.2.8]
- 다른 프로젝트에서 온 Component·asset 참조의 처리는 XII.10.38에서 진단과 함께 고정합니다.
- 큰 Paste는 최초 시각 반응 10ms 이하, 전체 처리는 청크 분할입니다. [아키텍처 §6.4]
- 텍스트 편집 중 표준 편집 명령은 텍스트 경로가 먼저 받습니다. [도구·메뉴 5.3] [가이드 8.2]

---

## 18. 문맥 메뉴

- 우클릭과 Control-클릭은 hit 결과와 현재 선택으로 만든 `NSMenu`를 엽니다. 항목은 모두 메뉴 막대와 같은 `ShellCommand`이며 새 의미를 만들지 않습니다. 아래 표는 항목의 자리이고, 실제 메뉴에는 구현 행이 연결된 명령만 넣습니다. 메뉴에 있는 항목이 잠깐 쓸 수 없으면 숨기지 않고 비활성화합니다. [가이드 8.1] [HIG The menu bar] [AppKit 문서 NSMenuItemValidation] [계획 XII.10.39] [UX 명세 2장 P1, 5.6]
- 클릭한 node가 선택에 없으면 먼저 그 node를 선택합니다(macOS 목록 관례와 같음). [사이드바 4.2] [HIG Lists and tables]

| 대상 | 항목 묶음(구분선 순서) |
|---|---|
| node | Cut, Copy, Paste, Duplicate, Delete · Select Overlapping ▸(겹친 레이어 고르기, 4.3절) · Bring Forward, Send Backward, Bring to Front, Send to Back · Group, Ungroup, Frame Selection · Boolean ▸ · Flip Horizontal, Flip Vertical · Lock, Hide |
| Shape·Fragment 추가 | Convert to Path(Shape), Convert to Frame(Fragment) |
| ComponentInstance 추가 | Reset Overrides, Swap…, Detach Instance, 정의로 이동 |
| 빈 캔버스 | Paste, Select All, Zoom to Fit, Zoom to Selection, Show Rulers |

- 메뉴 막대(도구·메뉴 3.2절)에 아직 없는 항목은 구현 행이 Object 메뉴에 함께 추가합니다. Scale은 XII.10.12, 행·열 정리는 XII.10.21, Boolean ▸은 XII.10.31, Reset Overrides·Detach Instance·정의로 이동은 XII.10.34, Swap…은 XII.10.35, Convert to Path·Convert to Frame은 XII.10.39입니다. Select Overlapping의 레이어 목록은 hit 결과로 만드는 동적 항목이라 메뉴 막대에 두지 않습니다. [계획 XII.10.39]
- VoiceOver는 AXShowMenu 동작으로 같은 메뉴를 엽니다. [HIG Accessibility] [AppKit 문서 NSAccessibility]

---

## 19. Undo 단위

| 제스처 | history entry | 근거 |
|---|---|---|
| 클릭 선택, 마퀴 | 최종 선택 1건 | VII.3.3, §23.3 |
| 선택하지 않은 node를 눌러 바로 드래그 | 선택 변경과 이동을 합쳐 1건 | XII.10.40 |
| 드래그 이동·크기 조절·회전·반경·padding·gap 핸들 | pointer down\~up 1건 | VII.4.2 |
| Option 복제 이동 | 1건 | VII.1.6 |
| 화살표 반복 | 짧은 연속 입력 병합 1건 | VII.4.4 |
| 인스펙터 스크럽·stepper | 시작\~끝 1건 | VII.4.5 |
| 정렬·분배·행·열 정리·묶음·Boolean | 명령당 1건 | VII.1.6 |
| 생성 도구 | 생성과 새 선택 1건 | XII.10.40 |
| 기존 Path의 point 드래그 | 1건 | XII.10.28 |
| 텍스트 연속 입력 | 같은 위치 연속 입력 1건 | VII.4.3 |
| 새 Text를 입력 없이 끝냄, 취소한 제스처, hover, 줌·팬, 도구 전환, 편집 범위 이동 | 0건 | §23.3 |

- 선택 변경과 문서 편집은 같은 시간순 스택에 있고, Undo 중 복원된 선택은 새 entry를 만들지 않습니다. [아키텍처 §23.2] [계획 VII.4.6, VII.5.5]
- 한 제스처 안에서 선택과 편집이 함께 바뀌면 entry 하나가 둘을 함께 되돌려야 합니다. 문서 항목은 이미 편집 직전·직후 선택(`SelectionRestore { before, after }`)을 담습니다(`crates/history/src/entry.rs`). 그러나 pointer gesture preview 중에는 `record_selection`이 `PreviewActive`로 거부됩니다(`crates/history/src/history.rs`). 그래서 HistoryEntry 타입은 바꾸지 않고, 제스처 안 선택 변경을 별도 항목 없이 그 제스처의 문서 항목에 담는 규칙을 XII.10.40이 구현합니다. [계획 VII.4.2, VII.5.5, XII.10.40]
- Undo 메뉴 제목은 entry 대상 이름을 붙입니다(예: "Undo Move"). [도구·메뉴 5.3]

---

## 20. 키보드 전용·VoiceOver 경로

### 20.1 키보드 전용

| 동작 | 키보드 경로 | 구현 |
|---|---|---|
| 캔버스로 포커스 | Tab·Shift-Tab, Full Keyboard Access | XII.10.41, XII.8.1 |
| 선택 이동 | Tab·Shift-Tab(형제), Return(안으로), Shift-Return(밖으로) | XII.10.4 |
| 전체 선택·해제 | ⌘A, ⇧⌘A, Escape(최상위 편집 범위에서, 3.2절 순서) | XII.10.3 |
| 이동 | 화살표, Shift-화살표 | XII.10.14 |
| 크기 조절 | 키보드 크기 조절 키, 인스펙터 W·H 필드 | XII.10.14, 인스펙터 5장 |
| 회전·반경·padding·gap | 인스펙터 필드 | 인스펙터 5장 |
| 생성 | Insert 메뉴(화면 가운데 기본 크기) | XII.10.41 |
| 정렬·분배·순서·묶음·Boolean | Object 메뉴와 단축키 | XII.10.21, XII.10.23, XII.10.31 |
| 흐름 순서 바꾸기 | 주축 화살표 | XII.10.14 |
| 문맥 메뉴 | VoiceOver AXShowMenu, 메뉴 막대의 같은 명령 | XII.10.39 |
| 텍스트 편집 | Return으로 들어가고 Escape로 나옴 | XII.10.33 |
| 인스턴스 Reset·Detach·Swap | 문맥 메뉴와 인스펙터 Component 섹션 | XII.10.34, XII.10.35 |
| 줌·팬 | ⌘+ ⌘- ⌘0 ⌘9 ⌘*, 선택 없음에서 화살표 | XII.10.14, XII.10.37 |

- 캔버스가 first responder이면 시스템 포커스 표시 규칙을 따릅니다. 캔버스 안 현재 선택은 선택 윤곽으로 보입니다. [가이드 8.2] [HIG Focus and selection]
- 메뉴 막대·툴바·창 순환 같은 시스템 키보드 탐색 단축키를 가로채지 않습니다. [가이드 8.1]

### 20.2 VoiceOver

- 선택된 node를 접근성 요소로 노출합니다. 역할은 노드 종류, 이름은 레이어 이름, 값은 위치와 크기입니다. 디자인 문서의 접근성 의미 트리(§18)와는 다른 대상이며 그것은 인스펙터 preview로만 읽습니다. [가이드 8.3] [아키텍처 §18] [계획 XII.8.1, XII.8.3]
- 이동, 크기 조절, 안으로, 밖으로, 메뉴 열기를 custom action으로 연결해 VoiceOver로도 같은 Command를 만듭니다. [가이드 8.3] [HIG Accessibility]
- 선택 변경과 제스처 결과(예: "Moved to 120, 48")는 접근성 알림으로 짧게 알리고, 연속 조작은 끝날 때 한 번만 알립니다. [HIG Accessibility] [계획 XII.10.42]
- 도구 상태도 접근성으로 노출합니다. [계획 XII.8.1] [도구·메뉴 7장]

---

## 21. 성능 예산

| 동작 | 계약 | 측정 |
|---|---|---|
| hover 윤곽, 클릭 선택, 핸들 표시 | 목표 5ms, 상한 8ms | XII.10.43 |
| 마퀴 갱신 | 목표 5ms, 상한 8ms(선택 표시) | XII.10.6, XII.10.43 |
| 이동(스냅 포함)·크기 조절·회전·padding·gap 핸들 | Present 예약까지 10ms 이하 | XII.10.43 |
| Path point 드래그 | 10ms 이하 | XII.10.43 |
| 텍스트 입력·IME 조합 표시 | 10ms 이하(단일 텍스트 변경) | XII.10.43 |
| Pan·Zoom(핀치 포함) | 10ms 이하 | XII.10.37 |
| 대규모 Paste·Delete | 최초 시각 반응 10ms 이하, 전체는 청크 분할 | XII.10.38 |

- 스냅 계산은 Hit Test·인터랙션 해석 단계(목표 0.35ms, 상한 0.75ms) 안에, overlay 갱신은 Scene Snapshot 갱신 단계(목표 1.00ms, 상한 1.75ms) 안에 둡니다. [아키텍처 §6.2]
- UI 스레드는 한 번에 2ms를 넘게 점유하지 않습니다. 커서 변경과 문맥 메뉴 구성도 이 안에서 끝냅니다. [아키텍처 §6.3] [가이드 9장]
- 판정은 2만 가시 node fixture(XVI.3.2)에서 p50·p95·p99로 하며 평균으로 판정하지 않습니다. [아키텍처 §6.3] [계획 XVI.4.2]

---

## 22. 시각 표현

- 선택 윤곽과 핸들의 색은 시스템 강조 색을 따르고, ShellTheme이 실어 나르는 셸 색(가이드 열린 결정 O9)으로 받습니다. 다른 제품의 고유 색이나 모양을 쓰지 않습니다. [가이드 4.2, 11장 O9] [PO 2026-11-14 편집 UX]
- 측정 수치 라벨은 시스템 글꼴의 의미와 고정폭 숫자를 씁니다. 라벨 glyph를 공통 텍스트 엔진으로 그릴지는 열린 결정 C2입니다. [가이드 4.1, 11장 O7]
- 커서는 시스템 커서(`NSCursor`)가 있는 동작에는 시스템 커서를 쓰고, 회전처럼 시스템에 없는 커서만 Stellar가 정합니다. [가이드 1.2 원칙 2, 3] [AppKit 문서 NSCursor]
- overlay는 렌더러 transient layer로 그리며 셸 뷰로 겹쳐 그리지 않습니다. 동작 줄이기·대비 높이기·투명도 줄이기 대응은 7.2절과 같습니다. [가이드 1.2 원칙 2, 8.4] [계획 XII.4.4]

---

## 23. 열린 결정

| ID | 내용 | 정할 곳 |
|---|---|---|
| C1 | 눈금자를 렌더러 transient layer로 그릴지, AppKit 표준 구성요소로 그릴지 | XII.10.19 (필요하면 ADR) |
| C2 | 측정 라벨 glyph 출처(공통 텍스트 엔진 대 셸) | XII.10.16 |
| C3 | 회전 중심을 캔버스에서 옮기는 핸들 제공 여부 | XII.10.10 |
| C4 | 드래그 중 재부모화를 막는 수식키 | XII.10.2 |
| C5 | Text 측정을 baseline 기준으로 바꾸는 방법 | XII.10.16 |
| C6 | Grid 명시 배치 자식 드래그를 셀 이동으로 볼지 | XII.10.24 |
| C7 | 같은 속성 node 모두 선택, 반전 선택 같은 고급 선택 명령 | 후속 계획 개정 |
| 가이드 O10 | 수식키·이동 단위 | XII.10.2 ADR |

C1\~C6은 "정할 곳" 행에서 정합니다. 그 행은 결정과 근거를 작업 보고서(XII.10.2는 ADR)에 기록하고 고른 규칙의 fixture를 검증 기준에 둡니다. [계획 3. 계획 변경 규칙]

---

## 24. 계획 대응 요약

이 문서의 동작 중 기존 행으로 구현되는 것은 각 표의 "구현" 열에 적었습니다. 기존 행이 없던 동작은 계획 `XII.10 캔버스 편집 UX`의 XII.10.1\~XII.10.44로 추가했습니다.

| 묶음 | 행 |
|---|---|
| 도구·수식키 결정 | XII.10.1, XII.10.2 |
| 선택 | XII.10.3\~XII.10.7 |
| 변형·이동 | XII.10.8\~XII.10.14, XII.10.44 |
| 스냅·가이드·눈금자·그리드 | XII.10.15\~XII.10.20 |
| 정렬·순서·묶음 | XII.10.21\~XII.10.23 |
| 레이아웃 인지 편집 | XII.10.24, XII.10.25 |
| 생성·벡터 편집·Boolean | XII.10.26\~XII.10.31 |
| 텍스트 | XII.10.32, XII.10.33 |
| 인스턴스 | XII.10.34, XII.10.35 |
| 줌·팬·클립보드·문맥 메뉴 | XII.10.36\~XII.10.39 |
| history·접근성·성능 | XII.10.40\~XII.10.43 |

CSS 속성을 편집하는 행은 그 속성의 PropertyKey·영속 record·migration을 만드는 IX.11 행을 선행으로 둡니다. [속성 모델 17장]

| 행 | 편집하는 속성 | IX.11 선행 |
|---|---|---|
| XII.10.9 | `width`·`height`, sizing preset(`fit-content` 등), 뒤집기 `scale` 부호와 위치 보정 | IX.11.3, IX.11.6, IX.11.7, IX.11.9 |
| XII.10.10 | `rotate`, 위치 보정(inset·`translate`) | IX.11.9 |
| XII.10.11 | `border-*-radius`(Frame·Rectangle), Polygon 단일 반경 | IX.11.3, IX.11.22 |
| XII.10.12 | 크기, `border-*-width`, `font-size` 등 비례 값 | IX.11.3, IX.11.10, IX.11.11 |
| XII.10.13, XII.10.14 | 위치 longhand(Frame은 IX.11.9, 그 밖의 node는 IX.11.6 item record) | IX.11.6, IX.11.9 |
| XII.10.21 | `align-self`·`justify-self`, 위치 longhand | IX.11.4, IX.11.5, IX.11.6, IX.11.9 |
| XII.10.22 | 위치 longhand | IX.11.6, IX.11.9 |
| XII.10.24 | inset(흐름 순서는 OrderKey) | IX.11.6 |
| XII.10.25 | `padding-*`, `row-gap`, `column-gap` | IX.11.6 |
| XII.10.44 | 뒤집기 `scale` 부호, 위치 보정(inset·`translate`) | IX.11.6, IX.11.9 |

가이드 10.4절이 "WBS 없음"으로 두었던 항목 중 생성 도구(XII.10.26, XII.10.27, XII.10.33), 가이드·측정 표시(XII.10.16), 눈금자(XII.10.19)가 이 행들로 채워졌고, 가이드와 도구·메뉴 명세의 해당 문장도 이 행 번호로 고쳤습니다. 셸 프레임 명세 6.1절의 같은 문장은 그 문서를 고치는 다음 작업에서 맞춥니다. [가이드 0.2, 10.4]
