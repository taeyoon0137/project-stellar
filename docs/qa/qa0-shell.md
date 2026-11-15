# QA0 셸 체감 확인 목록

앱 셸(창, 메뉴, 툴바, 패널)을 실제 창에서 확인합니다. 진행 방식과 피드백 형식은 [README](README.md)를 따릅니다.

- 실행: `./scripts/macos/qa.sh`(샘플 프로젝트 창), `./scripts/macos/qa.sh --no-build --welcome`(시작 창)
- 기준 명세: [셸 프레임](../design/ui/shell_frame.md), [도구·메뉴](../design/ui/tools_menus.md), [시작 창](../design/ui/welcome.md), [macOS 셸 디자인 가이드](../design/macos-shell-design.md)
- 열린 프로젝트 창에서 Layers, 선택한 Free Frame의 크기, 저장된 노드 이름의 Git 변경을 확인할 수 있습니다. 편집 절차는 [QA2](qa2-edit.md)에 있습니다.

## 1. 시작 창

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA0-01 | 프로젝트 창 없이 앱 실행(`--welcome`) | 시작 창 하나. 왼쪽에 앱 아이콘, "Stellar", 버전, New Project…·Open Project…·Clone Project… 버튼. 오른쪽에 "No Recent Projects"와 안내문. 두 열 사이 시스템 구분선. 창 제목 글자는 보이지 않음 |
| QA0-02 | 시작 창 가장자리를 끌어 크기 바꾸기 | 크기가 바뀌지 않음 |
| QA0-03 | Window 메뉴 열기 | 창 목록에 "Welcome" |
| QA0-04 | New Project…(⌘N) → 상위 폴더와 새 이름 선택 → Create | 새 Git 저장소와 최초 문서를 만들고 프로젝트 창을 엽니다. 창을 닫고 같은 폴더를 열면 같은 문서가 나타납니다. Git 신원을 설정한 환경에서는 최초 커밋이 있고, 신원이 없으면 설정 방법을 안내합니다. |
| QA0-05 | Clone Project… → Repository URL에 `https://example.com/team/design-system.git` 입력 | Clone sheet(Repository URL, Name, Location). Name이 `design-system`으로 자동 채워짐. URL을 비우면 Clone 버튼 꺼짐. Escape로 닫힘 |
| QA0-06 | Clone sheet에서 Clone | "The Operation Couldn’t Be Completed" 안내(설명 "Cloning projects isn’t available in this build yet…"). 실제 Clone은 **구현 대기**(XII.6.3) |
| QA0-07 | 최근 프로젝트 목록 | **구현 대기**(코어 최근 목록 경로, 전용 WBS 없음). 지금: 프로젝트를 열어도 항상 "No Recent Projects" |

## 2. Open Project

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA0-08 | 시작 창 Open Project…(⌘O) → 샘플 폴더(`qa.sh` 출력의 "샘플 프로젝트:" 경로) 선택 | 폴더만 고를 수 있는 패널. 고르면 잠깐 열기 진행 표시 뒤 프로젝트 창이 열리고 시작 창은 닫힘 |
| QA0-09 | 프로젝트 창이 열린 상태에서 File ▸ Open Project…(⌘O)로 같은 샘플 폴더 선택 | 새 창이 생기지 않고 기존 프로젝트 창이 앞으로 옴 |
| QA0-10 | Open Project로 Git 저장소가 아닌 일반 폴더 선택(예: 빈 새 폴더) | "This Folder Isn’t a Git Repository" 안내와 해결 방법. 프로젝트 창은 생기지 않음. 안내에는 전체 경로 대신 폴더 이름만 |
| QA0-11 | Open Project로 `stellar.yaml`이 없는 Git 저장소 선택(예: 이 저장소 루트) | "This Folder Isn’t a Stellar Project" 안내. 저장소를 변환하지 않고 창도 생기지 않음 |
| QA0-12 | 시작 창에서 연 실패와 메뉴에서 연 실패 비교 | 시작 창에서 열면 시작 창 sheet로, 프로젝트 창이 있을 때 메뉴로 열면 그 창 sheet(또는 alert)로 안내 |

## 3. 창 구성

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA0-13 | 샘플 프로젝트 창 보기 | 제목 "Stellar QA Sample", 부제목 "Restricted Mode". 왼쪽 사이드바, 가운데 캔버스, 오른쪽 인스펙터 3열 |
| QA0-14 | 사이드바 보기 | Pages에는 활성 페이지와 View가, Layers에는 해당 View의 자식이 보입니다. 새 프로젝트는 자식 목록이 비어 있습니다. |
| QA0-15 | 인스펙터 보기 | "No Selection"과 선택 방법 안내 |
| QA0-16 | 새 프로젝트의 캔버스 보기 | 800 × 600의 흰 Frame이 사이드바·인스펙터 사이에 보입니다. |
| QA0-17 | 사이드바 토글: 툴바 왼쪽 버튼, View ▸ Hide Sidebar, ⌃⌘S를 차례로 | 세 경로 모두 같은 사이드바를 접고 폄. 숨기면 캔버스가 그만큼 넓어짐. 메뉴 제목이 Show/Hide로 바뀜 |
| QA0-18 | 인스펙터 토글: 툴바 오른쪽 버튼, View ▸ Hide Inspector, ⌥⌘I | QA0-17과 같은 동작(오른쪽) |
| QA0-19 | View ▸ Show Panels | 캔버스 아래 보조 패널(Git·Agent 탭)이 펼쳐짐. 처음 연 창에서는 접혀 있음. Git 탭은 제한 모드 안내 |
| QA0-20 | 프로젝트 창 닫기(⌘W 또는 File ▸ Close Project ⇧⌘W) | 프로젝트 창이 닫히고 시작 창이 나타남(앱은 종료되지 않음) |
| QA0-21 | 시작 창만 남은 상태에서 시작 창 닫기 | 앱 종료 |

## 4. 하단 도구 막대

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA0-22 | 캔버스 아래쪽 보기 | 캔버스 하단 가운데에 떠 있는 Liquid Glass 캡슐 하나. 도구 10개(Move, Frame, Rectangle, Ellipse, Line, Polygon, Text, Image, Hand, Zoom) 순서, 칸 사이 구분선 없음, Move 뒤에 둥근 회색 선택 표시. 캔버스 아래 가장자리와 간격이 있음 |
| QA0-23 | 사이드바·인스펙터를 각각·함께 접고 펴기, 창 폭 바꾸기 | 막대가 언제나 캔버스 영역의 가운데에 있음 |
| QA0-24 | 막대의 도구 클릭 | 선택 표시가 그 도구로 옮겨 감. View(Move·Hand·Zoom Tool)·Insert 메뉴의 체크 표시도 같은 도구 |
| QA0-25 | 캔버스를 한 번 클릭한 뒤 V, F, R, O, L, T, H, Z 키 | 각각 Move, Frame, Rectangle, Ellipse, Line, Text, Hand, Zoom 선택. Polygon·Image는 단일 키 없음(Image는 Insert ▸ Image ⇧⌘K) |
| QA0-26 | 한글 입력 소스(두벌식)로 바꾸고 캔버스에서 ㄱ(R 자리) 키 | Rectangle 선택(물리 키 위치 기준) |
| QA0-27 | 캔버스에서 ⌘R, Shift-R | 도구가 바뀌지 않음 |
| QA0-28 | 사이드바 필터 필드에 글자 입력 중 R | 필드에 글자가 들어가고 도구는 바뀌지 않음 |
| QA0-29 | 도구 위에 마우스를 올려 기다리기 | 도움말 태그(예: "Rectangle (R)") |
| QA0-30 | 신뢰한 프로젝트에서 F로 흰 Frame 안을 드래그 | 자식 Frame이 생기고 Layers 행이 선택됩니다. |

## 5. 상단 바(툴바)

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA0-31 | 상단 바와 캔버스·사이드바·인스펙터 경계 보기 | 상단 바 아래 구분선이 없음. 콘텐츠 영역 상단 바 뒤는 블러 배경 |
| QA0-32 | 툴바 항목 순서 보기(창 폭 1440pt 정도) | 사이드바 토글 \| 뒤로·앞으로(비활성), 제목·부제목 … 현재 배율을 표시하는 줌 메뉴, 검색(비활성), 더 보기(…) \| 인스펙터 토글. 캔버스 도구는 상단에 없음 |
| QA0-33 | 더 보기(…) ▸ Show in Finder | Finder에서 샘플 폴더가 열림(`stellar.yaml`, `design`, `assets`가 보임) |
| QA0-34 | 창 폭을 점점 줄이기 | 들어가지 않는 항목이 overflow(») 메뉴로 넘어감. 항목이 겹치거나 잘리지 않음 |
| QA0-35 | View ▸ Customize Toolbar…에서 항목 하나를 빼고 Done → 앱 종료 후 다시 실행 | 바꾼 구성이 유지됨. 사이드바 토글은 옮기거나 뺄 수 없음 |
| QA0-36 | 문서가 열린 창에서 줌 메뉴, 뒤로·앞으로, 검색 확인 | 확대·축소 후 배율이 바뀝니다. 뒤로·앞으로와 프로젝트 검색은 **구현 대기**입니다. |

## 6. 메뉴·단축키

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA0-37 | 메뉴 막대 보기 | 순서: 앱 메뉴, File, Edit, View, Insert, Object, Window, Help. 앱 메뉴 이름은 "Stellar" |
| QA0-38 | 편집 전후 메뉴 상태 확인 | New Project는 ⌘N으로 열립니다. 문서가 열리면 Save Now를 쓸 수 있고 Frame 생성 후 Undo, Undo 후 Redo가 활성화됩니다. |
| QA0-39 | Settings…(⌘,) | 설정 창이 열림. 지금은 "Stellar Settings" 글자만 있는 placeholder(설정 창 전용 WBS 없음) |
| QA0-40 | File ▸ Show in Finder | QA0-33과 같음 |
| QA0-41 | ⌃⌘F, ⌘M, ⌘H, ⌥⌘T | 전체 화면 전환, 최소화, 앱 가리기, 툴바 보이기·숨기기 |
| QA0-42 | 앱을 쓰는 중 ⌘Space, ⌘Tab, ⇧⌘5 | 시스템 단축키가 그대로 동작(앱이 가로채지 않음) |
| QA0-43 | 사이드바 필터 필드에 입력 후 ⌘Z, ⇧⌘Z, ⌘A·⌘C·⌘V | 필드 입력이 되돌려지고 다시 적용됨(Edit 메뉴 제목 "Undo Typing"). 선택·복사·붙여넣기도 필드에 적용 |
| QA0-44 | Help ▸ Stellar Help | 시스템 기본 동작(도움말 없음 안내). Help Book은 전용 WBS 없음 |
| QA0-45 | Undo·Redo·Save Now를 활성 창 세션에 연결 | **구현 대기**(XII.6.4). 메뉴 단축키의 코어 command ID 공유는 XII.6.5 |

## 7. 다크 모드

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA0-46 | 앱 실행 중 시스템 설정 ▸ 화면 모드에서 라이트 ↔ 다크 | 시작 창·프로젝트 창·메뉴·툴바·도구 막대·사이드바·인스펙터·sheet가 바로 따라 바뀜. 바뀌지 않고 남는 색(하드코딩 색)이나 읽기 어려운 글자가 없음 |
| QA0-47 | 강조 색 바꾸기(시스템 설정 ▸ 화면 모드 ▸ 강조 색) | 포커스 링·선택 강조가 고른 색을 따름. 앱 고유 강조 색 없음 |
| QA0-48 | 다크 모드에서 도구 막대 보기 | 캡슐과 선택 표시·아이콘이 배경과 구분됨 |
| QA0-49 | 캔버스 문서 색과 앱 외형 분리 | **구현 대기**(XII.9.3, 렌더러 연결 XII.4.1). 새 프로젝트의 흰 Frame 색을 앱 외형 전환 전후로 비교 |

## 8. 창 크기와 복원

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA0-50 | 창을 최소 크기까지 줄이기 | 약 640 × 400pt에서 더 줄지 않음. 좁아지면 사이드바·인스펙터가 시스템 규칙으로 접힘. 레이아웃이 겹치거나 깨지지 않음 |
| QA0-51 | 창 크기 바꾸기 | 캔버스만 늘고 줄어듦. 사이드바·인스펙터 폭은 그대로 |
| QA0-52 | 사이드바·인스펙터 분할선 끌기 | 사이드바는 약 200\~360pt, 인스펙터는 약 240\~400pt 안에서만 바뀜 |
| QA0-53 | 창 크기·위치, pane 폭, 인스펙터 접힘을 바꾸고 ⌘Q → `./scripts/macos/qa.sh --no-build` | 같은 크기·위치·폭·접힘으로 다시 열림. 프로젝트 창도 복원됨(시스템 설정 "앱 종료 시 윈도우 닫기"가 꺼져 있을 때) |
| QA0-54 | 샘플 폴더 이름을 바꾼 뒤 앱 실행(`--welcome`) | 복원된 창에 실패 상태와 복구 동작(Locate Folder…, Try Again, Close Window). Locate Folder…로 바뀐 폴더를 고르면 같은 창에서 열림 |
| QA0-55 | 전체 화면(⌃⌘F) 들어가고 나오기 | 시스템 전체 화면 전환. 툴바와 도구 막대가 제자리 |

## 9. 접근성 기본

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA0-56 | VoiceOver(⌘F5)로 프로젝트 창 훑기 | 툴바 항목, 도구 막대의 각 도구 이름과 선택 상태, "Sidebar"·"Inspector" 영역, 캔버스 "Canvas", 상태 화면 제목을 읽음. 같은 이름을 두 번 읽지 않음 |
| QA0-57 | VoiceOver로 시작 창 훑기 | 버튼 제목, "Recent Projects" 목록, 상태 문구를 읽음. 장식 아이콘은 읽지 않음 |
| QA0-58 | Tab·Shift-Tab(시스템 설정 ▸ 키보드 ▸ 키보드 탐색 켬) | 사이드바 → 캔버스 → 인스펙터 순서로 이동. 사용자가 누르지 않았는데 포커스가 옮겨 가지 않음 |
| QA0-59 | 손쉬운 사용 ▸ 동작 ▸ 동작 줄이기 켜고 사이드바·인스펙터 토글 | 애니메이션 없이 바로 접히고 펴짐 |
| QA0-60 | 손쉬운 사용 ▸ 디스플레이 ▸ 대비 높이기, 투명도 줄이기 (라이트·다크 각각, 둘 다 켠 경우 포함) | 시스템 컨트롤과 Liquid Glass 표면이 설정을 따라 바뀌고, 모든 글자와 아이콘이 읽힘 |
| QA0-61 | 캔버스 선택·도구 상태를 VoiceOver로 듣기 | **구현 대기**(XII.8.1). 앱 셸 적합성 보고서는 XII.9.4 |
