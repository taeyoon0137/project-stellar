# 속성 패널(인스펙터) UI 명세

- 상태: 현행(2026-11-15). 코어 행 snapshot(ADR-0070) 기준 구현을 설명합니다.
- 범위: macOS 셸 오른쪽 속성 탭의 내용(머리, CSS 범주 섹션, 속성 행, 값 입력)
- 코드: `apps/macos/Stellar/UI/Inspector/`, `apps/macos/Stellar/UI/Shell/ShellInspectorSource.swift`,
  `apps/macos/Stellar/CoreRuntimeInspector.swift`
- 상위 기준: UX 명세 6장(속성 패널), `docs/adr/ADR-0070-inspector-css-property-surface.md`(코어 표면과 C ABI),
  `docs/design/property_model.md` 12장(CSS 범주), ADR-0024(CSS longhand 하나 = 속성 하나)

이 문서는 위 기준을 바꾸지 않습니다. 어긋나면 그 문서들이 우선합니다. 이전 초안의 Stub 데이터 소스, 예열·나눠
만들기 페이지, modifier stack 목록, Frame / Appearance / Modifiers 구성은 UX 명세 6.2절에 따라 지웠습니다.

---

## 1. 역할과 경계

- 어떤 행이 있는지, 값 텍스트, 잠금·혼합 여부, 값 해석·검증·저장 위치·Undo 단위는 코어가 정합니다
  (`crates/commands/src/inspector.rs`, `crates/runtime/src/app/inspector.rs`). 셸은 코어가 만든 행을 표시하고
  입력(CSS 값 텍스트, 수치 delta, 편집 단계)만 보냅니다.
- 진입점은 `InspectorViewController`이고, 창의 오른쪽 인스펙터 자리에 들어갑니다. pane 재질은 시스템이 정하므로
  배경을 그리지 않습니다.
- 데이터 소스는 `InspectorDataSource` 프로토콜이며 창에서는 `ShellInspectorSource`가 구현합니다. 모든 호출과
  콜백은 메인 스레드입니다.
- 편집은 프로젝트 신뢰와 무관합니다(D1).

## 2. 화면 구성

| 영역 | 내용 | 구현 |
|---|---|---|
| 머리 | 단일 선택: 종류 글리프, 레이어 이름(없으면 종류 이름), 종류 이름. 다중 선택: "N Layers". 선택 없음: 현재 Page 요약(`Page · View`)과 도구 단축키 안내 | `InspectorHeaderView` |
| 섹션 | 코어 행의 섹션 순서(Layer, Position, Size, Overflow, Typography, Background, Fill & Stroke, Border, 종류별). 행이 있는 섹션만 보입니다 | `InspectorSectionView` |
| 행 | CSS 이름 레이블(시스템 글꼴), 값 컨트롤, 행 아래 진단 한 줄 | `InspectorRowView` |

- 종류별 섹션 제목은 선택한 노드 종류 이름(Line, Polygon, Image)입니다. 종류별 행은 고른 노드가 모두 같은 종류일
  때만 나옵니다.
- 섹션 접힘은 노드 종류 조합별로 앱 설정(`UserDefaults`)에 기억합니다. 문서에는 쓰지 않습니다.
- 도구 단축키 안내는 `CanvasTool.allCases`의 `singleKey`에서 만듭니다(단축키가 없는 도구는 빠짐).
- 텍스트 편집을 시작하면 Typography 섹션을 펼칩니다.

## 3. 행

- 레이블은 CSS longhand 이름(`width`, `background-color`)이고 Stellar 전용 속성은 Stellar 이름(`polygon-sides`)입니다.
  툴팁은 쉬운 설명, longhand 이름, 변환 진단(Stellar 전용이면 "Stellar only")입니다. 진단은 코어 행 flag에서 옵니다.
- `border`는 묶음 행이며 펼치면 현재 네 변 공통 모델 값인 `border-width`와 `border-color`가 보입니다. 두 행은 기존 Border 저장 key 하나에 매핑되고, 변별 12개 속성은 아직 없습니다.
- `border-radius`는 묶음 행입니다. 펼침 단추를 누르면 4모서리 longhand 행이 들여 써서 보입니다. 펼침 상태는 창
  안에서만 기억합니다.
- 컨트롤: 수치·문자 값은 텍스트 필드, keyword는 popup, true·false는 체크박스, 색은 견본(`NSColorWell` minimal)과
  텍스트 필드입니다.
- 레이블 열 폭은 가장 긴 CSS 이름이 잘리지 않는 폭이며 패널 안쪽 폭의 60%를 넘지 않습니다. 패널 폭이 바뀌면
  `viewDidLayout`에서 다시 맞춥니다. 320pt 패널에서 `border-bottom-right-radius`(longhand 들여쓰기 포함)가 잘리지
  않는 폭입니다.
- 잠긴 노드(자신 또는 조상)의 행은 읽기 전용입니다. `locked` 행은 조상이 잠겼을 때만 읽기 전용입니다(D8).

## 4. 값 입력과 Undo 단위

| 입력 | 동작 | Undo |
|---|---|---|
| 텍스트 + Return(또는 포커스 이동) | CSS 값 텍스트를 코어가 해석·정규화해 적용합니다. 잘못된 값은 적용하지 않고 행 아래 진단(`unsupported-unit: …`) | 한 번 |
| Escape | 입력 중인 텍스트를 버리고 snapshot 값으로 돌린 뒤 캔버스로 포커스 | 없음 |
| ↑↓(Shift ×10) | 누를 때마다 바로 적용(반복 입력). 다중 선택은 각 노드 값에 더함 | 같은 속성 0.5초 안 반복은 한 번 |
| 레이블 좌우 끌기(scrub, Shift ×10) | 끄는 동안 preview, 놓으면 확정. 시작·끝은 한 번씩 알림 | 끌기 한 번 |
| 색 견본 → popover → 시스템 색 패널 | 바꿀 때마다 바로 적용(반복 입력). 텍스트로 hex·`rgb()`·`color(display-p3 …)` | 같은 속성 0.5초 안 연속 변경은 한 번 |
| popup·체크박스 | 고르면 적용 | 한 번 |

- 다중 선택에서 값이 다르면 필드는 "Mixed"입니다. 입력하면 모두에 한 번에 적용합니다.
- 화살표와 색은 코어 preview를 열지 않으므로, 누르는 사이 ⌘Z·캔버스 선택·다른 편집이 막히지 않습니다.
- scrub의 끝·취소는 큐에서 기다리지 않고 바로 보냅니다. 선택이 사라졌으면 끝 대신 취소를 보내고, 코어는 취소를
  표시 상태와 무관하게 받아 preview를 닫습니다(ADR-0070 결정 4).
- 상시 `NSStepper`는 두지 않습니다. 화살표 키 경로가 대신합니다.

## 5. 제스처 중 구성 고정

- 캔버스 제스처(`CanvasView.frameGestureActive`: 이동·생성 드래그, Hand, Zoom 드래그)나 패널 scrub 중에는 섹션·행
  구성을 바꾸지 않고 같은 행의 값만 갱신합니다. 미룬 구성은 제스처가 끝나면 반영합니다.
- 패널 scrub은 행 집합으로 셉니다. 행마다 시작·끝을 한 번씩 알리므로 중복 시작이 쌓이지 않습니다. 행이 창에서
  빠지면 진행 중인 scrub을 취소합니다.
- 캔버스에 새 제스처 상태(크기 조절 핸들, 영역 선택, 모서리 반경 핸들)가 생기면 `frameGestureActive`에 더합니다.

## 6. 데이터 경계

- 읽기: `CoreRuntime.inspector` → `stellar_window_inspector`. 게시된 창 상태의 행 snapshot을 복사하므로 UI
  thread에서 문서 질의가 없습니다.
- 편집: `ShellInspectorSource.edit` → 창 → `stellar_window_edit_property`. 요청은 표시한 snapshot의 View·revision·
  history 위치를 함께 보내고, 코어가 오래된 요청을 거부합니다. 같은 속성의 scrub 중간 값과 반복 입력은 보내기
  전에 합칩니다.
- 선택이 없어지면 보내지 않은 편집은 버리고 이유("The selection changed.")를 행 아래에 보입니다.

## 7. 접근성·외형

- 값 컨트롤은 속성 이름을 접근성 레이블로, 툴팁 문구를 도움말로 씁니다. 색 견본은 `<이름> color`입니다.
- 진단 문구는 VoiceOver 알림으로도 읽힙니다.
- 색은 시스템 의미 색만 씁니다(`secondaryLabelColor`, `systemRed` 진단). 글꼴은 컨트롤 크기 small의 시스템
  글꼴이고, 값 필드는 고정폭 숫자입니다.

## 8. 미완료

- `50%`·`fit-content`·`var(--x)`는 문법은 알아보되 저장하지 않고 진단합니다. 크기 방식(IX.11.6)과 토큰 scope가
  생긴 뒤 저장합니다(UX.2.6 미완료 기준).
- Text style longhand(Typography의 `font-*` 등)는 IX.11.11 뒤에 행이 생깁니다. 지금 Text는 `color`만 있습니다.
- Path는 앱 캔버스가 아직 그리지 않아 Layer 행만 보입니다.
- 입력→Present 10ms 계약 측정(UX.2.12)은 하지 않았습니다. Release 입력 trace로 따로 합니다.
- 행 끝 표시(Git 변경, 충돌, 토큰 연결, UX 명세 6.5)와 행 문맥 메뉴는 해당 WBS 행에서 붙입니다.

## 9. 행동 확인 절차

화면 밖 앱 내장 검사는 두지 않습니다. 아래는 앱을 직접 실행해 확인하는 절차이며, 결과는 해당 WBS 행의 완료
근거로 기록합니다.

1. 새 프로젝트를 열고 F로 Frame, R·O·L·Polygon·T·Image로 각 노드를 만든 뒤 하나씩 고릅니다. 7종 모두 속성 행이
   나오고 선택이 있는데 "No Selection"이 나오지 않는지 봅니다.
2. Rectangle과 Ellipse를 함께 고릅니다. 머리가 "2 Layers"이고 크기가 다르면 `width`가 "Mixed"인지 봅니다. `320`을
   입력하면 둘 다 320px이 되고 ⌘Z 한 번에 둘 다 돌아가는지 봅니다.
3. `width`에 `abc`, `10em`을 입력합니다. 값이 바뀌지 않고 행 아래 진단이 보이는지 봅니다.
4. `width` 레이블을 좌우로 끕니다. 놓은 뒤 ⌘Z 한 번이 끌기 전체를 되돌리는지, 끈 뒤 다른 노드를 골랐을 때 섹션이
   그 노드에 맞게 바뀌는지 봅니다.
5. `width` 필드에서 ↑를 몇 번 누른 뒤 바로 ⌘Z와 캔버스 클릭을 합니다. 둘 다 거부되지 않는지, ⌘Z 한 번이 연속
   ↑를 되돌리는지 봅니다.
6. 값을 입력하다 Escape를 누릅니다. 값이 원래대로이고 포커스가 캔버스로 가는지(V·R 같은 단일 키가 바로 먹는지)
   봅니다.
7. `fill` 견본을 눌러 popover → Show Colors… → 색 패널에서 색을 끕니다. 캔버스 색이 따라 바뀌고 ⌘Z 한 번에
   돌아가는지 봅니다. `fill` 필드에 `#3366FF`를 붙여 넣어도 적용되는지 봅니다.
8. 인스펙터 폭을 320pt로 줄이고 `border-radius`를 펼칩니다. `border-bottom-right-radius` 레이블이 잘리지 않는지
   봅니다.
9. Frame을 끌어 옮기는 동안 속성 섹션이 다시 만들어지지 않는지(깜빡임·스크롤 이동 없음) 봅니다.
10. 레이어를 잠근 뒤 값 행이 읽기 전용이고 `locked`만 바꿀 수 있는지 봅니다.

이전 앱 내장 검사(`--frame-insert-check`)가 덮던 Inspector 밖 동작도 같은 방식으로 확인합니다: F·R·V 생성과 이동,
Undo·Redo, 저장 뒤 다시 열기, Layers 이름 편집, 형제 65개에서 다음 페이지, 두 창의 선택·편집 격리.

## 10. SF Symbols

| 용도 | 이름 |
|---|---|
| 선택 없음·다중 | `cursorarrow.rays`, `square.stack.3d.up` |
| node 종류 | `rectangle.dashed`(Frame), `rectangle`, `circle`, `line.diagonal`, `pentagon`, `textformat`(Text), `photo`(Image), `square.on.square.dashed`(그 밖) |

이미지 asset은 쓰지 않습니다.
