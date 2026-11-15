# Stellar UX 구현 WBS 초안

- 상태: 초안(계획서 등록 전). 행 ID `UX.n.m`은 임시 번호이고, `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`에 등록할 때 정식 ID를 받습니다. 커밋 제목은 정식 ID로 `Task(<WBS-ID>): …` 형식을 씁니다.
- 명세: `stellar-ux-spec.md`(이하 "명세"). 행의 "명세 n절"은 그 문서의 절입니다.
- 착수 전제: 정리 통합(되돌림 결과 `02397601` 위의 수정 묶음)이 main에 들어간 뒤입니다. 선행 칸의 "정리"는 이 전제를 뜻합니다.
- 선행 칸의 `U1`–`U23`은 명세 16장 결정 항목입니다. 2026-11-15 모두 권장안으로 확정했습니다.
- 한 행 = 한 커밋입니다. 삭제 행은 커밋 본문에 삭제 사유를 적습니다.
- `crates/ffi`·`storage`·`core-types`·`schemas`·`config/performance-budgets.yaml`을 바꾸는 행은 같은 diff에 ADR 기록을 넣습니다. 새 FFI는 ADR-0014·ADR-0028 갱신 대상입니다.
- 검증은 테스트 통과만이 아니라 앱에서 그 흐름을 직접 끝까지 해 보는 행동 확인을 포함합니다. 제품 테스트는 로컬 `cargo xtask check-core`로 하고 CI에 넣지 않습니다.

lane(병렬 단위, crate가 겹치지 않게 나눔)

| lane | 범위 |
|---|---|
| D | 문서·계획(`docs/**`) |
| S | macOS 셸(`apps/macos/Stellar/**`) |
| C | 코어 명령·FFI(`crates/commands`, `crates/document`, `crates/ffi`) |
| R | runtime·project(`crates/runtime`, `crates/project`) |
| G | Git(`crates/git-porcelain`, runtime Git 경로) |
| A | MCP·에이전트(`crates/mcp-core`, `tools/stellar-mcp`, `crates/agent-host`) |

권장 착수 순서: UX.0 → UX.1(S)과 UX.2 코어 행(C)을 병렬 → UX.2 셸 행 → UX.3(S)과 UX.4(R)를 병렬 → UX.5(G·S) → UX.6과 UX.7을 병렬 → UX.8 → UX.9 → UX.10 → UX.11 → UX.12. UX.13은 결정 뒤.

---

## UX.0 기준선과 문서

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.0.1 | 정리 통합 결과를 UX 기준선으로 확인하고 결과를 기록합니다. 확인 항목: Pen 삭제, 도구 10종이 모두 반응, 편집 경로 신뢰 게이트 0건, D6 sheet, D7 Settings ▸ 캔버스, 컴포넌트 라이브러리 창·New Component alert·Document Font UI 0건, 인스펙터 편집 FFI 단일화, Git 패널 Stage 경로의 남은 범위. 결과 기록에 후속 작업 하나를 함께 넣습니다: 디자인 문서(`docs/design/`)의 "기준선 `02397601`" 서술만 다시 훑어, 정리 통합으로 사라진 항목은 "기준선" 표기를 지우고 남은 항목은 처리할 UX.n 행을 가리키게 합니다 | `gitSource.trusted`·`trusted_canvas_window`·`case pen`·`selectPenTool` 검색 결과가 편집 경로에서 0건. 새 프로젝트에서 10개 도구를 눌러 각각 생성·이동·확대가 실제로 됨. 미충족 항목은 UX.1.5 또는 해당 행 선행으로 기록. "기준선 `02397601`" 서술 재점검 결과(고친 문서·줄)가 결과 기록에 있음 | 정리 | `apps/macos/Stellar/UI/Tools/CanvasTool.swift`, `apps/macos/Stellar/UI/Shell/`, `crates/runtime/src/app/`, `docs/design/` | D |
| UX.0.2 | 디자인 문서를 명세와 결정에 맞춰 정리합니다. 셸 가이드 6.1(툴바 가운데 도구 → 하단 도구 막대), 2.4·6.4·6.5(하단 패널·Git·에이전트 위치), O4·O8 닫기와 O2 방향 기록(닫기는 UX.3.10), 7장 부제목, "핵심 흐름과 완료 기준" 절 신설. tools_menus의 남은 11개 도구 기록과 Polygon 키(기준선에서 Pen은 이미 빠짐). inspector 2.1 범주 → property_model 12장. sidebar 238행 신뢰 조건 삭제. git_panel 위치·inline 동반 변경·단계별 Stage. agent_panel·shell_frame 하단 패널 삭제. property_model 15장 O9 | 문서 사이 충돌 목록(명세 11.2절)이 0건. 문서 이스케이프 규칙(코드 표시 밖 `\~`·`\<`) 위반 0건. 다른 제품 고유 명칭·문구 0건 | UX.0.1, U1–U6 | `docs/design/macos-shell-design.md`, `docs/design/ui/{tools_menus,inspector,sidebar,git_panel,agent_panel,shell_frame}.md`, `docs/design/property_model.md` | D |
| UX.0.3 | 계획서와 아키텍처 §24.3을 개정합니다. XII.10.1(10종)과 XII.10.29(펜 도구 부분 삭제)가 기준선에 들어 있는지 확인, XIII.3.1을 Synthetic Command(Undo 가능) 반영으로, XIV.13.5에 패널 위치, XVI.11.8에 코드 탭·에이전트 탭 노출 조건, XIV.2.8을 커밋 직전 flush 후 상태 재검증(UX.5.5)으로, ADR-0074(CSS 값 크기·정렬·간격)에 따라 IX.11.7(sizing preset) 삭제·IX.11.17과 XII.10.9의 preset 문구를 CSS 값 입력으로·XII.10.22(간격·자리 바꾸기 핸들)와 XII.10.25(padding·gap 캔버스 핸들) 삭제·XII.10.43의 padding 핸들 scenario 삭제, 아키텍처 §24.3 "처음 여는 프로젝트는 제한 모드"에 Stellar가 §25.1로 만든 폴더의 신뢰 등록 예외(U8, ADR-0073 결정 10. Clone·열기는 그대로), 이 문서의 행 등록 | `cargo xtask sync-plan`·`validate-task-status` 통과. 삭제 행을 참조하는 행 0건. 개정 기록에 사유. §24.3 문구가 ADR-0073 결정 10과 일치 | UX.0.2, U8, U12, U13 | `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`, `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`, `planning/task-status.yaml`(생성) | D |
| UX.0.4 | 셸 배치·상태 표시 결정과 새 프로젝트 신뢰 등록을 ADR로 기록합니다(U1·U4·U5·U8·U15·U16·U17). 하단 보조 패널 삭제, 탭 구성, 부제목 문법, 한 줄 안내, 공용 상태 사전, Git 메뉴, §24.3 예외인 새 프로젝트 신뢰 등록 | ADR이 명세 3장과 일치. ADR 본문에 다른 제품 이름·문구 없음 | UX.0.2 | `docs/adr/ADR-0073-shell-layout-and-status-display.md` | D |

## UX.1 걷어 내기(흐름 1)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.1.1 | 캔버스 아래 보조 패널과 그 토글을 삭제합니다. Agent 탭과 `DisconnectedAgentPanelSource` 연결을 지웁니다. Git 패널 뷰 컨트롤러는 지우지 않고 UX.5.2에서 사이드바에 다시 붙입니다 | 창에 보조 패널 영역 0, View 메뉴에 패널 토글 없음. `grep ShellPanelTabsSlot toggleAuxiliaryPanel` 0건. 창 크기 복원이 깨지지 않음 | UX.0.1, U1 | `UI/Shell/ShellAreaSlots.swift`, `UI/Shell/ShellFrameMenuItems.swift`, `UI/Shell/ShellWindowController.swift`, `UI/Shell/ShellSplitViewController.swift`, `UI/Shell/ShellAppCoordinator.swift` | S |
| UX.1.2 | 영구 비활성 UI를 삭제합니다. 툴바 뒤로·앞으로·프로젝트 검색, 메뉴의 영구 비활성 항목(Deselect All, Find 계열, Zoom to Selection, Rulers, Align·Distribute·Flip, Group, Lock, Hide, Stellar Help 등). 구현된 명령만 남깁니다. Cut·Select All은 텍스트 편집 중 field editor가 처리하므로 남깁니다 | 메뉴 막대와 툴바를 모두 열어 비활성 항목이 "선택 없음" 같은 일시 조건뿐임을 확인. 단축키 감사(`ShortcutPolicy.swift`)가 지운 항목을 가리키지 않음 | UX.0.1 | `UI/Shell/ShellToolbarController.swift`, `UI/Tools/MainMenuBuilder.swift`, `UI/Tools/ShortcutPolicy.swift`, `UI/Shell/ShellProjectWindow.swift`(활성 명령 집합) | S |
| UX.1.3 | Polygon에 단일 키 P를 주고, 도구 막대를 그룹 간격(선택·컨테이너·도형·내용·시점)으로 나눕니다. 접근성 도움말의 키 안내를 실제 배정과 맞춥니다 | 캔버스 포커스에서 P가 Polygon을 고름. 텍스트 입력 중 P는 글자. 도움말·툴팁·메뉴의 키 표시가 `CanvasTool.singleKey`와 같음 | UX.0.1, U6 | `UI/Tools/CanvasTool.swift`, `UI/Shell/ShellToolBarView.swift`, `UI/Shell/ShellProjectWindow.swift`(접근성 도움말) | S |
| UX.1.4 | 툴바 "…" 메뉴 항목을 제목 문서 메뉴로 옮기고 "프로젝트 신뢰…"를 늘 둡니다. 신뢰 alert 문구를 "Git·hook·터미널·에이전트 실행만 막는다"로 바꿉니다 | 제한 모드 창에서 제목을 눌러 신뢰를 부여할 수 있음. 신뢰 문구에 "edit" 의미 없음. 신뢰 전에도 편집·저장·이미지 표시가 됨 | UX.0.1 | `UI/Shell/ShellProjectWindow.swift`, `UI/Shell/ShellToolbarController.swift` | S |
| UX.1.5 | (조건부) 정리 통합 뒤에도 반응하지 않는 도구가 있으면 그 생성 경로를 연결합니다. 작성 기준 main의 runtime `CanvasItem`은 Frame·Rectangle·Text뿐이었고, 기준선 `02397601`에는 Shape(Rectangle·Ellipse·Line·Polygon) 생성 경로가 있습니다. 반응하지 않는 도구와 빠진 D7 스타일 입력만 연결합니다. 통합 검토(2026-11-15) 기준 남은 것: Text 드래그로 고정 폭 Text 만들기(지금은 클릭 생성만), Image 드래그로 크기 정하기와 도구를 고르는 즉시 파일 선택(지금은 도구를 고른 뒤 Frame을 클릭해야 선택이 열림, U11) | 10종 도구 각각 클릭·드래그 생성이 되고 Undo 한 번. 생성 스타일이 Settings 값과 같음(코어 하드코딩 없음) | UX.0.1, XII.10.26, XII.10.33(Text 드래그), UX.3.3(Image 드래그·파일 선택) | `crates/runtime/src/app/frame_insert.rs`, `crates/ffi/src/window_session.rs`, `UI/Shell/ShellCanvasFrameInsert.swift` | C·S |
| UX.1.6 | 레이어 빈 상태를 키 안내 한 줄로 바꿉니다("F로 Frame, R로 사각형, T로 텍스트를 그립니다") | 새 프로젝트의 레이어 탭에 안내가 보이고 버튼이 없음. VoiceOver가 안내를 읽음 | UX.1.3 | `UI/Sidebar/SidebarSymbols.swift`, `UI/Sidebar/SidebarLayersController.swift` | S |

완료 기준(흐름 1): 새 프로젝트를 만든 뒤 첫 드래그까지 대화상자 0개. 눌러서 반응 없는 UI 0개.

## UX.2 만들고 다듬기(흐름 2)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.2.1 | 코어 노드 속성 정보를 Shape(size, corner radius, Polygon 변 수, Line 끝점), Fill·Stroke modifier, Text, Image로 넓힙니다. 각 longhand에 PropertyKey를 할당합니다 | `node_property_info`가 7종 노드의 키를 반환하는 단위 테스트. 키 상위 byte 규칙 준수 | 정리, IX.11.3 | `crates/commands/src/property.rs`, `crates/document/src/{frame_metadata,modifier_metadata}.rs`와 새 metadata 파일 | C |
| UX.2.2 | 범용 속성 편집 경로 하나(노드, 키, CSS 값)와 노드별 속성 snapshot 읽기를 FFI로 노출합니다. 정리 통합의 단일 인스펙터 편집 FFI가 있으면 그것을 넓히고 새로 만들지 않습니다 | 모든 경로가 history_sequence를 검사. 속성별 개별 FFI(`set_frame_background` 등)를 새로 추가하지 않음. ADR 기록 | UX.2.1, IX.11.13, IX.11.14 | `crates/ffi/src/window_session.rs`, `crates/runtime/src/app/` | C |
| UX.2.3 | 셸 인스펙터 데이터 소스를 모든 노드 종류와 다중 선택에 연결합니다. Frame 하나일 때만 값을 만들던 분기를 없앱니다 | Rectangle·Ellipse·Line·Polygon·Text·Image·다중 선택을 고르면 속성이 나옴. 선택이 있는데 "No Selection" 0건 | UX.2.2 | `UI/Shell/ShellFrameInspectorSource.swift`, `CanvasFrameScene.swift` | S |
| UX.2.4 | 섹션을 property_model 12장 CSS 범주로 바꾸고 코어가 받는 키의 행만 보입니다. 섹션 접힘을 노드 종류별로 기억합니다 | 노드 종류마다 명세 6.2절 표의 섹션만 보임. 키 없는 속성 행 0건 | UX.2.3, IX.11.17, U3 | `UI/Inspector/InspectorPropertyCatalog.swift`, `UI/Inspector/InspectorSectionView.swift` | S |
| UX.2.5 | 행 레이블을 U3 결정대로 바꿉니다(권장: CSS 이름, 시스템 글꼴, shorthand 묶음 행과 펼침). 툴팁에 쉬운 설명·longhand·진단 | 레이블 글꼴이 시스템 글꼴. 묶음 행을 펼치면 longhand 행. 320pt 폭에서 레이블 잘림 0건 | UX.2.4, U3 | `UI/Inspector/InspectorPropertyCatalog.swift`, `UI/Inspector/InspectorSectionView.swift` | S |
| UX.2.6 | 값 입력을 CSS 값 문법으로 받습니다. 레이블 scrub, 화살표 ±1·Shift ±10, Return·Escape·Tab, 잘못된 값은 행 아래 진단, 다중 선택 "혼합". 상시 `NSStepper`를 지웁니다 | `320`, `50%`, `1.5turn`, `fit-content`, `var(--x)` 입력이 정규화되어 저장됨. 잘못된 값이 적용되지 않음. scrub 한 번이 Undo 한 번. Escape 뒤 포커스가 캔버스 | UX.2.4, U10 | `UI/Inspector/`, `apps/macos/Stellar/NativeFieldsPanel.swift` | S |
| UX.2.7 | `fill`·`stroke` 행을 늘 있는 행으로 둡니다. 색 견본 → popover → 시스템 색 패널, hex·`rgb()`·`color(display-p3 …)` 텍스트 입력 | 새 사각형 선택 즉시 `fill` 행이 D7 값으로 보임. "+" 메뉴 없이 색 변경. Undo 한 번 | UX.2.4 | `UI/Inspector/` | S |
| UX.2.8 | 도구 막대 위 한 줄 안내 자리를 만들고, root Frame 밖 빈 캔버스에서 생성 도구를 드래그하면 금지 커서와 "Frame 안에 그립니다"를 보입니다 | 빈 캔버스 드래그에 반응이 있음(커서·안내). 2.5초 뒤 사라짐. VoiceOver 알림. 캔버스 렌더 결과에 안내가 섞이지 않음 | UX.1.3, U5 | `UI/Shell/ShellToolBarView.swift`, `UI/Shell/ShellCanvasFrameInsert.swift` | S |
| UX.2.9 | 크기 조절 핸들 8개와 크기 라벨(`320 × 200`, 고정이 아니면 CSS 값)을 연결합니다 | 7종 노드 크기 조절, Shift 비율 유지, Option 중심 기준. 드래그 한 번이 Undo 한 번 | UX.2.2, XII.10.8, XII.10.9 | `crates/runtime/src/app/`, `UI/Shell/ShellCanvasFrameMove.swift` | R·S |
| UX.2.10 | 모서리 반경 핸들(Frame·Rectangle·Polygon, Option은 한 모서리)을 연결합니다 | 핸들 드래그가 `border-*-radius`를 바꾸고 속성 탭 값과 같음 | UX.2.9, XII.10.11 | `crates/runtime/src/app/`, `UI/Shell/` | R·S |
| UX.2.11 | 자동 저장 트리거를 runtime에 연결합니다(debounce, 최대 지연, 비활성화·닫기 flush). Save Now는 대기 저장을 합칩니다 | 편집 뒤 정해진 지연 안에 저장되고 연속 드래그 중 쓰기 반복 0. Save Now 이중 쓰기 0. 대형 문서에서 document thread 점유를 Release로 측정해 기록 | 정리, XIII.1.3, XIII.1.4, XIII.1.5 | `crates/runtime/src/`, `crates/project/src/autosave.rs` | R |
| UX.2.13 | `border`를 변별 longhand로 나눕니다. 저장 key 0x0105를 `border-{top,right,bottom,left}-{width,style,color}` 12개 key로 나누고(속성 모델 5장 `border` 행, IX.11.22), 속성 탭 `border` 묶음 행을 펼치면 변별 longhand 행이 되게 합니다. 그 전까지 `border` 행은 펼쳐지지 않는 한 행입니다 | 변 하나의 두께·색을 바꾸면 그 longhand만 저장되고 Undo 한 번. 묶음 행 도움말이 코어가 넘긴 longhand 이름 목록과 같음 | UX.2.5, IX.11.22, IX.11.10 | `crates/commands/src/inspector.rs`, `crates/document/src/frame_metadata.rs`, `UI/Inspector/` | C·S |
| UX.2.12 | 드래그·크기 조절·scrub 중 속성 탭 섹션 구성을 고정하고 값만 갱신합니다. 텍스트 편집에 들어가면 Typography 섹션을 펼칩니다 | 제스처 중 섹션 추가·삭제·스크롤 0. 입력→Present 10ms 계약 측정 유지 | UX.2.4 | `UI/Inspector/`, `UI/Shell/ShellFrameInspectorSource.swift` | S |

완료 기준(흐름 2): 새 프로젝트 → 사각형 → 크기·색·반경 변경이 대화상자 없이 되고 모두 Undo가 된다. 7종 노드 모두 속성 탭에 나온다.

## UX.3 손에 익는 캔버스와 구조

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.3.1 | 핀치 줌, ⌘ 스크롤 줌, 두 손가락 팬, Space를 누르는 동안 Hand를 연결합니다 | 도구를 바꾸지 않고 팬·줌. 카메라 조작이 history에 없음 | 정리, XII.10.36, XII.10.37 | `CanvasView.swift`, `crates/runtime/src/canvas.rs` | S·R |
| UX.3.2 | 캔버스 문맥 메뉴를 구현된 `ShellCommand`만으로 만듭니다 | 우클릭 항목이 모두 동작. 메뉴 막대와 같은 command ID | UX.1.2, XII.10.39 | `CanvasView.swift`, `UI/Tools/` | S |
| UX.3.3 | Image 도구를 고르면 즉시 파일 선택을 열고, 클릭은 원래 크기·드래그는 그 크기로 배치합니다. Finder 끌어 놓기·붙여넣기도 같은 경로 | 취소하면 이전 도구로 돌아감. 끌어 놓은 파일이 같은 import 경로로 들어감 | UX.2.8, U11, XII.10.27 | `UI/Shell/ShellImagePlacement.swift`, `CanvasView.swift` | S |
| UX.3.4 | 레이어 표시·잠금 토글을 코어 `SetProperty::visibility`·`locked`에 연결합니다. 켜져 있으면 hover로만 보이고 숨김·잠금 행은 늘 보입니다 | 토글이 캔버스에 즉시 반영되고 Undo 한 번. 잠긴 노드는 캔버스에서 선택되지 않음. CLI `stellar apply`도 같은 잠금을 거부(D8) | UX.2.2 | `UI/Sidebar/SidebarLayersController.swift`, `UI/Shell/ShellSidebarSource.swift`, `crates/ffi/src/sidebar.rs` | S·C |
| UX.3.5 | 사이드바 Pages 구획을 머리 줄 `Page › View ▾` 팝업 한 줄로 바꿉니다. 지금은 전환·이름 변경만 두고 새 Page·View는 UX.12에서 | 사이드바 높이에서 Pages 구획 0. 팝업 항목이 모두 동작 | UX.1.6 | `UI/Shell/ShellSidebarSource.swift`, `UI/Sidebar/` | S |
| UX.3.6 | Shift-클릭 추가·제거, ⌘-클릭 깊은 선택, 영역 선택(Option은 완전 포함)을 연결합니다 | 다중 선택 뒤 속성 탭 "혼합" 표시. 레이어와 캔버스 선택이 같음 | UX.2.3, XII.10.3, XII.10.4, XII.10.6 | `crates/runtime/src/app/frame_move.rs`, `UI/Shell/ShellCanvasFrameMove.swift` | R·S |
| UX.3.7 | Text·Image를 포함한 모든 노드를 드래그로 이동하고, Option 드래그 복제, 화살표 이동을 연결합니다 | 7종 노드 이동, 복제본이 원본 바로 위, 화살표 단위가 XII.10.2 ADR과 같음 | UX.3.6, XII.10.2, XII.10.13, XII.10.14 | `crates/runtime/src/app/frame_move.rs`, `UI/Shell/ShellCanvasFrameMove.swift` | R·S |
| UX.3.8 | 스냅 엔진과 스마트 가이드·측정 overlay, View 메뉴 스냅 토글을 연결합니다. 가이드 거리는 CSS 단위로. 스냅이 같은 입력을 보므로 이 행에서 셸이 해석하는 편집 의미를 코어로 옮깁니다(핸들·생성 해석 코어 이전): 크기 핸들 Shift 비율 유지·Option 중심 기준·최소 1px, Line 끝점 Shift 45° 단위, 반경 핸들 포인터→값과 반올림, 화살표 이동 단위(1·10px), 클릭 생성 기본 크기(100)와 생성 Shift 정비례·Option 중심, root Frame을 끌 수 있는지. 셸은 핸들 index, local 포인터, 수식키 flag(생성은 click·drag 구분)만 보내고, 현재 도구도 코어 창 view state에 둡니다(XII.10.1) | 부모·형제 가장자리·중심 스냅, ⌘로 일시 해제. 가이드가 렌더 결과·내보내기에 없음. 위 해석마다 코어 단위 테스트가 같은 입력으로 같은 상자·반경·끝점을 재현하고, 미리보기와 확정 값이 같은 코어 계산. Swift에 비율·각도·반올림·기본 크기 계산 0건 | UX.3.7, XII.10.1, XII.10.15, XII.10.16, XII.10.17 | `crates/runtime/src/`, `UI/Shell/` | R·S |
| UX.3.9 | 레이어 드래그로 순서·부모 바꾸기를 기존 형제 순서 편집 Command에 연결합니다. 묶기·풀기는 Command가 연결된 뒤 | 드래그 결과가 메뉴 순서 편집과 같고 Undo 한 번. 사이드바 validate 기본 거부가 연결된 동작에서는 사라짐 | UX.3.5, XII.10.23 | `UI/Shell/ShellSidebarSource.swift`, `UI/Sidebar/SidebarLayersController.swift` | S |
| UX.3.11 | 편집 가능 판정을 코어 하나로 모읍니다((model, view, node, key) → 편집 가능 여부). 속성 탭, 캔버스 선택 항목 flag, 선택 능력 mask, Layers 표시·잠금·이름, CLI `stellar apply`가 모두 이 판정을 씁니다. root Frame과 흐름 부모 규칙을 속성 탭에도 적용합니다 | 같은 node·key가 캔버스·속성 탭·Layers·CLI에서 같은 결과(root의 `left`·`top`·`visible`·`locked`, flex 부모 아래 자식의 `left`·`top`, 숨긴 조상 아래 node). `commands::lock` 주석과 ADR-0070 결정 5가 실제와 같음 | UX.3.4, UX.3.7 | `crates/commands/src/lock.rs`, `crates/commands/src/inspector.rs`, `crates/runtime/src/app/{frame_move,sidebar}.rs`, `crates/ops/src/apply.rs` | C·R |
| UX.3.12 | 레이어 표시·잠금 토글 ABI를 "ids와 flag, value 없음" 하나로 합쳐 방향을 늘 코어가 정하게 합니다. 순서 bit(`ORDER_*`)는 선택 능력 mask에 합칩니다 | Swift에 토글 방향 계산 0건. 선택과 같은 행·다른 행 토글 결과가 같은 규칙. ADR-0028 갱신 | UX.3.4 | `crates/ffi/src/sidebar.rs`, `crates/runtime/src/app/sidebar.rs`, `UI/Sidebar/SidebarLayersController.swift`, `UI/Shell/ShellSidebarSource.swift` | C·S |
| UX.3.13 | Select All을 코어 요청으로 옮깁니다(영역 선택과 같은 filter: root 직계 자식 중 보이고 잠기지 않은 노드). Swift의 레이어 페이지 순회를 지웁니다. 1만 개 선택의 속성 행 snapshot과 편집 비용 상한을 ADR-0070에 기록합니다 | 셸이 레이어 페이지를 읽지 않고 ⌘A 한 요청. 영역 선택과 같은 노드 집합. 1만 개 선택 측정 기록 | UX.3.6 | `crates/runtime/src/app/canvas_gesture.rs`, `crates/ffi/src/`, `UI/Shell/ShellCanvasSelectAll.swift` | R·S |
| UX.3.14 | 캔버스 크기·반경·끝점 ABI가 누른 시점의 revision과 history 위치를 받고, 다르면 stale로 거부합니다(이동·속성 탭 ABI와 같은 규칙) | 누른 뒤 다른 창·CLI가 바꾸면 놓아도 덮어쓰지 않음. ADR-0071 C ABI 표 갱신 | UX.2.9 | `crates/ffi/src/canvas_gesture.rs`, `crates/runtime/src/app/canvas_gesture.rs`, `UI/Shell/ShellCanvasFrameMove.swift` | C·S |
| UX.3.10 | 캔버스를 사이드바·오른쪽 영역 아래로 확장하는 비용을 측정하고, 계약 안이면 켭니다(셸 가이드 O2). 가운데 항목 `automaticallyAdjustsSafeAreaInsets`와 `NSBackgroundExtensionView`로 확장하고, 패널 아래 영역의 drawable 크기·hit test·렌더 비용을 Release로 잽니다 | 측정 기록(drawable 크기, 프레임 시간, 입력→Present 예약 10ms, UI 스레드 단일 점유 2ms, 아키텍처 §6). 계약 밖이면 켜지 않고 결과만 기록하며 O2는 열린 채 둠. 켜면 패널을 여닫아도 캔버스 내용이 밀리지 않고, 패널 위 클릭이 캔버스 입력으로 가지 않으며, O2를 닫음 | UX.1.1, U2 | `UI/Shell/ShellSplitViewController.swift`, `UI/Shell/ShellCanvasHostViewController.swift`, `CanvasView.swift` | S |

## UX.4 앱 밖 변경 반영(흐름 3)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.4.1 | 미저장 편집이 없는 창에서 외부의 새 원본(View·컴포넌트)을 다시 읽어 Synthetic Command 하나로 적용합니다. 자기 저장이 만든 event는 무시합니다. 진행 중인 제스처가 끝난 뒤 반영하고 선택은 StableId로 보존합니다 | 앱을 연 채 `stellar apply`로 값을 바꾸면 캔버스와 속성 탭이 갱신되고 ⌘Z로 되돌아감. 자기 autosave 뒤 reload loop 0 | UX.2.11, UX.0.3, XIII.2.4, V.10.6 | `crates/project/src/external_change.rs`, `crates/runtime/src/`, `crates/ffi/` | R |
| UX.4.2 | 반영 표시: 바뀐 노드 0.6초 윤곽 강조(동작 줄이기면 생략), Undo 메뉴 이름 "외부 변경 반영 실행 취소", 변경 탭 머리 줄(탭이 없으면 도구 막대 위 한 줄), VoiceOver 알림, 세션 동안 '외부' 출처 표시 | 강조가 렌더 결과에 없음. 알림 문구가 VoiceOver로 읽힘 | UX.4.1, UX.2.8 | `UI/Shell/`, `UI/Sidebar/` | S |
| UX.4.3 | 같은 프로젝트를 연 다른 Stellar 창이 먼저 저장한 경우 두 선택지 sheet를 띄우고, 미저장 편집 + 외부 writer 변경도 reconciliation 전까지 같은 sheet로 처리합니다. 확인 버튼 하나뿐인 Save Now 실패 alert를 없앱니다. 지금 runtime에는 창 문서를 디스크 generation으로 다시 불러오는 경로와 새 base 위에 이 창 버전을 쓰는 경로가 없어, stale이면 자동 저장이 멈추고 부제목 경고(ADR-0072 7절)만 보입니다. 두 경로를 먼저 runtime에 둡니다 | 두 창 동시 편집에서 두 번째 저장이 sheet를 띄우고 선택별 결과가 XII.7.5·7.6과 같음. 자동 저장 중 stale면 sheet 한 번만 | UX.4.1, U20, XII.7.1, XII.7.2, XII.7.3, XII.7.7 | `UI/Shell/ShellProjectWindow.swift`, `crates/runtime/src/` | S·R |
| UX.4.4 | 잘못된 외부 파일은 반영하지 않고 파일·줄·원인과 [편집기에서 열기]를 보입니다 | 깨진 YAML을 써도 캔버스가 그대로이고 경로·줄이 보임 | UX.4.1 | `crates/project/src/document_io/`, `UI/Shell/` | R·S |
| UX.4.5 | 세 snapshot reconciliation을 연결해 겹치지 않는 외부 변경은 자동 반영하고, 겹치면 흐름 9 충돌로 보냅니다. UX.4.3의 임시 sheet를 외부 writer 경우에서 거둡니다 | non-overlap fixture 자동 반영, overlap은 충돌. 앱 창 writer는 여전히 두 선택지 | UX.4.3, XIII.3.4, XXIV.4.6 | `crates/project/src/external_change.rs`, `crates/runtime/src/` | R |

완료 기준(흐름 3): 앱을 열어 둔 채 CLI가 같은 파일을 고쳐도 앱 편집을 잃지 않고 반영되며 Undo가 된다.

## UX.5 커밋 고리(흐름 4, 원본 단위)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.5.1 | Git 상태 고정값(`head: .unknown`, `remotes: []`, `identityConfigured: false`)을 실제 HEAD·upstream·remote·identity로 바꿉니다. Git은 worker에서만 실행합니다 | identity가 설정된 저장소에서 "identity 없음"이 나오지 않음. 메인 스레드 Git 실행 0 | UX.4.1, XIV.13.1 | `UI/Shell/ShellGitDataSource.swift`, `crates/runtime/src/`, `crates/git-porcelain/` | G·S |
| UX.5.2 | 기존 Git 패널을 왼쪽 사이드바 "변경" 탭으로 옮깁니다. 핵심 동작(메시지·커밋·Push)을 탭 위쪽에 둡니다. 탭 배지(바뀐 객체 수, 충돌 글리프) | 320pt 폭에서 잘림 0. 변경 탭을 열면 캔버스 변경 윤곽 자리(UX.6.3 전에는 없음)를 건드리지 않음 | UX.1.1, UX.5.1, U1 | `UI/Git/GitPanelViewController.swift`, `UI/Git/`, `UI/Sidebar/` | S |
| UX.5.3 | 부제목을 Git 상태 문법으로 바꿉니다(명세 3.3절). "Restricted Mode" 상시 표시를 없앱니다 | 깨끗함·변경·보낼 커밋·제한 모드·저장 실패 상태별 문구가 표와 같음. 부제목이 눌리지 않음 | UX.5.1, U4 | `UI/Shell/ShellProjectWindow.swift` | S |
| UX.5.4 | 원본 단위 "커밋에 포함"을 Git index와 연결합니다. View가 generation이면 View 묶음 하나로만 포함하고 그 사실을 보조 문구에 적습니다. 필수 동반 변경은 inline 확인(자동 포함 없음) | 체크 상태가 `git status`와 같고 앱을 다시 열어도 유지. 터미널 `git add` 결과가 앱에 반영 | UX.5.2, XIV.13.2 | `UI/Git/`, `crates/runtime/src/staging.rs`, `crates/ffi/` | G·S |
| UX.5.5 | 커밋: 메시지 칸, ⌘↩, 커밋 직전 flush, identity 없으면 같은 자리 inline 입력(U9 범위), 성공 머리 줄 "커밋함 `a1b2c3d`" | 흐름 2의 편집을 앱에서 커밋하고 `git log`에 보임. "Save Now / Use Saved Version" 배너 0 | UX.5.4, UX.0.3(XIV.2.8 개정), U9, XIV.13.3 | `UI/Git/`, `crates/runtime/src/`, `crates/git-porcelain/` | G·S |
| UX.5.6 | Push: 보낼 커밋이 있을 때만 [Push], 등록된 remote만, 강제 Push 없음, 실패 이유를 머리 줄에 | 테스트 remote로 Push 성공. 거부 시 Git 메시지와 다음 행동 안내 | UX.5.5, XIV.13.3 | `UI/Git/`, `crates/git-porcelain/` | G·S |
| UX.5.7 | 제한 모드 변경 탭: 설명 한 줄과 [프로젝트 신뢰…] | 제한 모드에서 변경 탭으로 신뢰 부여 후 Git 상태가 채워짐. 편집은 신뢰 전에도 됨 | UX.5.2, UX.1.4 | `UI/Git/` | S |
| UX.5.8 | 메뉴 막대 Git 메뉴(변경 보기, 커밋, Push, 다음 충돌로 이동, 상태 새로 고침, 프로젝트 신뢰…) | 모든 항목이 동작하거나 일시 조건 하나로만 비활성 | UX.5.6, U16, XII.6.5 | `UI/Tools/MainMenuBuilder.swift` | S |
| UX.5.9 | Stellar가 새로 만든 프로젝트를 만들 때 신뢰 목록에 등록하고(ADR-0073 결정 10), 새 프로젝트 sheet 안에 identity 필드를 둡니다 | 새 프로젝트 첫 화면 부제목이 `main · 변경 없음`. Clone·열기는 제한 모드. 생성이 실패·취소되면 등록 없음. 세 경우를 코어 단위 테스트로 고정 | UX.5.3, UX.0.3(§24.3 개정), U8, U9 | `crates/project/src/create.rs`, `crates/runtime/src/app/create.rs`, `UI/Welcome/` | R·S |

완료 기준(흐름 4, 원본 단위): 흐름 2의 편집을 터미널 없이 커밋하고 Push한다.

## UX.6 캔버스 위 diff와 의미 단위 Stage(흐름 4 완성)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.6.1 | 변경 목록을 원본 → 객체 → 속성 3단으로 보이고, 객체 행 선택이 캔버스·레이어 선택과 화면 이동을 하게 합니다(탭은 그대로) | 다른 View의 객체 행을 누르면 그 View로 전환. 100만 change synthetic pagination에서 스크롤 끊김 없음 | UX.5.4, XIV.13.1 | `UI/Git/GitChangeList.swift`, `UI/Git/` | S |
| UX.6.2 | 레이어 행 상태 배지(추가·수정·충돌)를 공용 상태 사전으로 붙입니다 | 배지가 색 외 모양으로 구분되고 접근성 레이블이 있음 | UX.6.1, UX.0.4 | `UI/Sidebar/` | S |
| UX.6.3 | 속성 행 변경 표시와 hover popover("HEAD: 240px · 커밋에 포함: 아니요") | 표시가 semantic status와 같고 flush 뒤 worker 계산 결과로만 갱신 | UX.6.1, UX.2.4 | `UI/Inspector/` | S |
| UX.6.4 | 캔버스 변경 윤곽 overlay(변경 탭 열림 시 자동, View 메뉴 "변경 윤곽 늘 보기") | 추가는 실선, 수정은 점선. 렌더 결과·내보내기에 없음. 대형 문서 갱신 시간을 측정해 기록 | UX.6.1, U15, XII.4.1 | `crates/runtime/src/canvas.rs`, `UI/Shell/` | R·S |
| UX.6.5 | "HEAD 값으로 되돌리기"를 속성 행·캔버스 문맥 메뉴에 둡니다. 일반 편집 Command입니다 | 되돌린 뒤 ⌘Z로 다시 돌아옴. Git 명령을 실행하지 않음 | UX.6.3, U14 | `crates/commands/`, `UI/Inspector/`, `CanvasView.swift` | C·S |
| UX.6.6 | 객체·속성 단위 "커밋에 포함"을 엽니다. 일부 포함 배지 추가 | 속성 하나만 Stage한 결과 blob이 결정론적이고 `git diff --cached`에 그 줄만 있음 | UX.6.1, V.10.7, XIV.13.2 | `crates/runtime/src/staging.rs`, `UI/Git/` | G·S |

## UX.7 코드 탭(흐름 5)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.7.1 | 오른쪽 영역에 코드 탭 자리를 만듭니다. 탭이 하나뿐이면 탭 줄을 그리지 않는 규칙을 함께 넣습니다 | 기능이 연결되기 전에는 탭이 없음. 탭 전환이 선택을 바꾸지 않음 | UX.0.3, U12 | `UI/Shell/ShellAreaSlots.swift`, `UI/Shell/ShellSplitViewController.swift` | S |
| UX.7.2 | 선택 노드의 CSS longhand 선언을 섹션 순서로 보이고 줄마다 진단 글리프를 붙입니다. [CSS로 복사] | 속성 탭 값과 줄 값이 같음. 복사 결과가 IX.11.15 출력과 같음 | UX.7.1, IX.11.13, IX.11.15, IX.11.19 | `crates/ffi/`, `UI/` 새 코드 탭 파일 | C·S |
| UX.7.3 | Edit ▸ CSS로 복사와 캔버스 문맥 메뉴 항목 | 메뉴·문맥 메뉴·탭 버튼이 같은 command ID | UX.7.2, UX.3.2 | `UI/Tools/MainMenuBuilder.swift` | S |
| UX.7.4 | View·Frame 선택 시 HTML 구획: flush 뒤 worker에서 `crates/ops/src/web_frame.rs`로 생성, 진단 목록(누르면 노드로 이동), [HTML 복사]·[저장…] | 저장된 View의 고정 Frame·Rectangle HTML이 CLI `export-web` 결과와 같음. 미지원 노드는 이유가 보이고 빈 결과처럼 보이지 않음 | UX.7.1, UX.2.11 | `crates/ops/src/web_frame.rs`, `crates/ffi/`, `UI/` | C·S |
| UX.7.5 | 원본 구획: 노드의 저장 파일 경로와 줄, [Finder에서 보기], [편집기에서 열기] | 경로·줄이 실제 파일과 같음. 외부에서 고치면 흐름 3으로 반영 | UX.7.2, UX.4.1, 노드→줄 범위 코어 API(신규) | `crates/project/src/document_io/`, `UI/` | R·S |
| UX.7.6 | HEAD 대비 줄 추가·삭제 표시 | `git diff`의 해당 노드 줄과 같음 | UX.7.5, V.10.7 | `UI/`, `crates/git-porcelain/` | G·S |

## UX.8 CSS 레이아웃(흐름 6)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.8.1 | Layout 섹션(`display`, `flex-direction`, `justify-content`, `align-items`, `flex-wrap` 각 한 행, CSS 키워드 선택·값 입력)과 Spacing 섹션(`padding-*`, `row-gap`, `column-gap`) | flex container 값이 저장·재열기 뒤 같음 | UX.2.4, IX.11.6 | `UI/Inspector/` | S |
| UX.8.2 | 자식 크기를 CSS 값(`width`·`height`·`flex`)으로 직접 입력·선택하고 계산값을 표시. 고정·채움·내용 맞춤 같은 크기 방식 개념·캔버스 간격 핸들은 두지 않음 | 입력한 CSS 값이 저장·재열기 뒤 같고 렌더 결과가 CSS 계산과 같음 | UX.8.1, IX.11.6 | `UI/Inspector/` | S |
| UX.8.4 | flow container 안 드래그 삽입선과 형제 비키기 | 놓으면 MoveNode 하나, Undo 한 번 | UX.8.1, XII.10.24 | `crates/runtime/src/`, `UI/Shell/` | R·S |
| UX.8.5 | Typography 섹션(`font-family` 등). Document Font 대체 | Text 선택 시 Typography가 보이고 글꼴 변경이 Undo 한 번 | UX.2.4, IX.11.11 | `UI/Inspector/` | S |

완료 기준(흐름 6): flex 카드 하나를 캔버스와 속성 탭만으로 완성한다.

## UX.9 컴포넌트(흐름 7)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.9.1 | `docs/design/ui/components.md` 명세를 씁니다: 에셋 탭, 새 컴포넌트, 끌어 배치, 정의 편집 문맥, 인스턴스 Property·override·Detach, Git 표시 | 명세 7장과 일치. 다른 제품 고유 명칭 없음 | UX.0.2, U21, U22 | `docs/design/ui/components.md` | D |
| UX.9.2 | 사이드바 에셋 탭: Components 구획, 썸네일(비동기·캐시), 이름, 경로 보조 문구, 파일 Git 배지 | 컴포넌트 파일을 CLI로 추가하면 목록에 나타남. 썸네일 렌더가 입력을 막지 않음 | UX.9.1, UX.5.4 | `UI/Sidebar/`, `crates/runtime/src/app/components.rs` | S·R |
| UX.9.3 | [+] 새 컴포넌트: 목록 안 이름 입력, 파일 하나 생성, 바로 정의 편집 | alert 0. 파일이 `design/components/`에 생기고 Undo 가능 범위가 명세와 같음 | UX.9.2 | `UI/Sidebar/`, `crates/runtime/src/app/components.rs` | S·R |
| UX.9.4 | 에셋 행을 캔버스로 끌어 인스턴스 배치(부모 강조, flow 위면 삽입선) | 놓은 자리에 인스턴스, Undo 한 번. 숫자 입력 배치 0 | UX.9.2, UX.8.4 | `CanvasView.swift`, `crates/runtime/src/app/instance_place.rs` | S·R |
| UX.9.5 | 정의 편집 문맥: 캔버스에 정의만, 사이드바 머리 줄 경로와 [완료], 속성 탭 "편집 대상", Escape로 나오기 | 나온 뒤 원래 View·카메라. 인스턴스 갱신. 정의 편집 변경도 같은 Undo 흐름 | UX.9.3 | `UI/Shell/`, `crates/runtime/src/` | S·R |
| UX.9.6 | 인스턴스 속성: 공개 Property·Slot 맨 위, override 표시와 행별 되돌리기, 비공개 필드 이유와 "정의 편집" 링크, 문맥 메뉴 Detach | 비공개 필드 편집 0. Detach 뒤 자유 편집 | UX.9.5, XII.10.34 | `UI/Inspector/` | S |
| UX.9.7 | 정의 편집 중 행 문맥 메뉴 "인스턴스에서 바꿀 수 있게 공개" | 공개 뒤 인스턴스 속성 탭에 그 Property가 나타남 | UX.9.6, U21 | `UI/Inspector/`, `crates/commands/` | S·C |
| UX.9.8 | 변경 탭에서 컴포넌트를 별도 원본 묶음으로 보이고, 컴포넌트만 커밋할 수 있게 합니다 | 컴포넌트 파일만 포함해 커밋한 결과가 `git show --stat`에 그 파일만 | UX.9.2, UX.5.4 | `UI/Git/` | S |

## UX.10 에이전트(흐름 8)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.10.1 | runtime에 attached MCP local IPC listener를 연결합니다(연결 상대 인증 포함) | 인증 실패 시 데이터 교환 0. 앱 없이 headless는 기존대로 | UX.0.3, XIX.1.7, XIX.9.4 | `crates/runtime/src/`, `crates/mcp-core/src/attach.rs` | A·R |
| UX.10.2 | 첫 연결 승인 sheet(클라이언트, 대상 창, 권한) | 승인 전 Read도 거부(XIX.3.3). 거부하면 연결 종료 | UX.10.1, XIX.3.3 | `UI/Shell/` | S |
| UX.10.3 | 오른쪽 에이전트 탭(연결 목록·끊기)과 상단 바 에이전트 상태 항목(조건부) | 연결이 없으면 탭·항목 모두 없음. 끊기가 lease를 풂 | UX.10.2, U4, U13 | `UI/Agent/`, `UI/Shell/ShellToolbarController.swift` | S |
| UX.10.4 | 에이전트 잠금 윤곽 overlay와 잠긴 노드 편집 시도 시 도구 막대 위 한 줄 안내, 속성 탭 잠금 이유 | 잠긴 범위는 선택·읽기만. 윤곽이 렌더 결과에 없음 | UX.10.3, UX.2.8, XIX.4 | `crates/runtime/src/canvas.rs`, `UI/Shell/`, `UI/Inspector/` | R·S |
| UX.10.5 | 범위 승인 카드와 batch 카드. batch 카드는 변경 탭 diff 행 컴포넌트를 재사용하고 전후 렌더 프리뷰를 보입니다 | 승인 근거가 코어 semantic diff. 첫 응답 하나만 유효 | UX.10.4, UX.6.1, XXIX.4.5, XXIX.4.6 | `UI/Agent/`, `UI/Git/` | S |
| UX.10.6 | 출처 표시 네 곳(레이어 행, 속성 행, 변경 행, 켜고 끄는 캔버스 윤곽) | 네 곳 표시가 같은 출처 데이터에서 나옴 | UX.10.5, UX.6.2, UX.6.3, XXIX.4.10 | `UI/Sidebar/`, `UI/Inspector/`, `UI/Git/` | S |
| UX.10.7 | "이 세션의 변경": batch·세션 단위 [유지]·[되돌리기], 되돌리기 미리보기 sheet(충돌 항목 기본 "현재 유지"), 완료·중단 Undo 항목 하나 | 중단 시 자동 롤백 0. 되돌리기 뒤 ⌘Z 가능 | UX.10.6, XIX.6 | `UI/Agent/`, `crates/runtime/src/` | S·R |
| UX.10.8 | 내장 세션: CLI 선택, 약관 고지, 모드, 채팅·원문 보기, 선택 문맥 표시 | 설치·로그인된 공식 CLI만 목록에 나옴. 출력이 입력을 막지 않음 | UX.10.7, XXIX.2.3, XXIX.4.2, XXIX.4.3, XXIX.4.4 | `UI/Agent/`, `crates/agent-host/`(XXIX.2.1 신설) | A·S |
| UX.10.9 | 에이전트의 commit·push 개별 확인 sheet와 창 닫기·종료 확인 | 확인 없이 Git 실행 0. 닫기 sheet에 "표시는 사라지고 변경은 남습니다" | UX.10.8, UX.5.6, XXIX.4.9 | `UI/Agent/`, `UI/Shell/` | S |

완료 기준(흐름 8): 요청, 승인, 적용, 되돌리기가 탭과 캔버스만으로 끝나고, 무엇이 언제 바뀌었는지 네 곳에서 같게 보인다.

## UX.11 충돌(흐름 9)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.11.1 | 변경 탭 맨 위 충돌 구획, 부제목 `병합 중 · 충돌 N`, 캔버스·레이어 충돌 배지 | 터미널 merge 충돌 뒤 앱이 충돌 수를 정확히 표시 | UX.6.2, XIV.13.4 | `UI/Git/GitConflictList.swift`, `UI/Shell/` | S |
| UX.11.2 | 속성 탭 충돌 행(Base·내 쪽·상대 쪽, B·O·T 키)과 candidate snapshot 캔버스 미리보기 | 고르는 즉시 캔버스가 바뀌고 파일은 쓰지 않음. 고를 수 없는 선택지는 이유와 함께 비활성 | UX.11.1, XIV.12.1 | `UI/Inspector/`, `crates/runtime/src/` | S·R |
| UX.11.3 | 충돌 객체의 3면 렌더 썸네일(비동기) | 썸네일 렌더가 입력을 막지 않음 | UX.11.2 | `crates/runtime/src/canvas.rs`, `UI/Inspector/` | R·S |
| UX.11.4 | [해결 적용]: 검증 → 파일 쓰기 → `git add`, `git merge --continue` 복사 안내, 비 Stellar 파일·conflict marker 안내 | 해결 뒤 `git status`가 resolved. 검증 실패 시 파일 쓰기 0 | UX.11.2, XIV.10.8 | `UI/Git/`, `crates/git-porcelain/` | G·S |

## UX.12 Page·View 만들기

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.12.1 | 코어에 View·Page 추가·이름 변경 Command를 추가하고 FFI로 노출합니다(ADR 기록) | 추가가 Undo 한 번. View 원본 파일 단위(ADR-0059)와 맞음 | UX.3.5 | `crates/commands/`, `crates/document/`, `crates/ffi/` | C |
| UX.12.2 | 사이드바 머리 줄 팝업에 "새 View"·"새 Page"를 둡니다 | 새 View가 active View가 되고 레이어·캔버스가 바뀜 | UX.12.1 | `UI/Shell/ShellSidebarSource.swift` | S |
| UX.12.3 | (결정 뒤) 한 Page에 여러 View를 나란히 렌더하고 빈 캔버스 Frame = 새 View로 바꿉니다 | ADR 결정과 일치. hit test·카메라가 여러 root를 다룸 | UX.12.2, U7(ADR) | `crates/runtime/src/canvas.rs`, `crates/commands/src/view_tabs.rs` | R·C |

## UX.13 브랜치(사용자 결정 뒤)

| ID | 작업 | 검증 기준 | 선행 | 관련 파일 | lane |
|---|---|---|---|---|---|
| UX.13.1 | 계획 개정: §24.4 범위와 XIV.13.7 노출 금지 검사를 결정대로 바꿉니다 | 개정 기록과 ADR | U18 | `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`, `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` | D |
| UX.13.2 | 변경 탭 머리 브랜치 popover: 목록, 전환(flush 뒤 checkout, Snapshot 전환), 새 브랜치 | 전환 뒤 캔버스가 한 번에 바뀌고 부제목 갱신. 미저장 편집 손실 0 | UX.13.1, UX.4.1 | `UI/Git/`, `crates/git-porcelain/src/advanced/` | G·S |
| UX.13.3 | [받기] 버튼(Fetch·Pull)과 받을 커밋 수, 결과를 흐름 9로 | 충돌 없는 pull이 Synthetic Command로 반영 | UX.13.2, UX.4.5 | `UI/Git/`, `crates/git-porcelain/src/advanced/` | G·S |
| UX.13.4 | 충돌 해결 뒤 앱 안 커밋으로 병합 마무리 | 터미널 없이 merge 완료 | UX.13.3, UX.11.4 | `UI/Git/` | G·S |

---

## 사용 메모

- `UI/`는 `apps/macos/Stellar/UI/`입니다.
- 정식 등록 때 기존 행과 내용이 겹치는 UX 행(예: UX.3.1과 XII.10.37)은 새 행을 만들지 않고 기존 행에 셸 연결을 맡기거나, 기존 행을 선행으로 두고 UX 행을 "셸 노출" 행으로만 남깁니다.
- 16장 결정이 모두 확정되어 결정 대기로 막힌 행은 없습니다. U7(여러 View 렌더)과 U18(브랜치)은 확정안이 "별도 ADR·재검토"라서 UX.12.3과 UX.13은 그 결정 뒤에 시작합니다.

## UX.0.1 기준선 기록(2026-11-15 통합 검토)

UX.0.1을 시작하기 전에 통합 검토가 확인한 미충족 항목입니다. UX.0.1 결과 기록은 이 목록에서 시작합니다.

- 도구: Text 드래그 고정 폭 생성과 Image 드래그 크기·즉시 파일 선택이 없습니다(UX.1.5, XII.10.33, UX.3.3).
- 셸 해석: 도구 상태와 핸들·생성 해석이 셸에 있습니다(UX.3.8). 반경 미리보기는 확정과 같은 반올림 값을 씁니다.
- 자동 저장: 실패·stale을 부제목으로만 알립니다. stale 두 선택지 sheet는 runtime 다시 불러오기 경로가 없어 UX.4.3입니다.
- 속성 탭: `border`는 펼쳐지지 않는 한 행입니다(UX.2.13).
- 판정·ABI: 편집 가능 판정 네 곳(UX.3.11), 레이어 토글 ABI 둘(UX.3.12), Swift의 Select All(UX.3.13), 캔버스 편집 ABI의 revision 검사(UX.3.14).
- 제한 모드: 편집·저장·컴포넌트 인스턴스 표시는 신뢰와 무관하게 됩니다. design/components 안의 파일 하나라도 프로젝트 밖을 가리키면 제한 모드에서는 원본 읽기 전체가 실패해 인스턴스가 모두 그려지지 않습니다(외부 target 내용을 읽지 않는 정책의 현재 동작).
