# QA1 문서 보기 확인 목록

샘플 프로젝트의 문서(페이지·View·레이어)를 보고 살펴보는 동작을 확인합니다. 진행 방식과 피드백 형식은 [README](README.md)를 따릅니다. 샘플 내용은 README 3.1절에 있습니다.

- 실행: `./scripts/macos/qa.sh`(샘플이 바뀌었으면 `--fresh`)
- 기준 명세: [사이드바](../design/ui/sidebar.md), [인스펙터](../design/ui/inspector.md), [셸 프레임](../design/ui/shell_frame.md), [macOS 셸 디자인 가이드](../design/macos-shell-design.md) 6장
- 열린 창은 저장된 활성 View의 Layers를 읽고 지원하는 Free Frame을 캔버스에 표시합니다. 각 항목에서 전체 샘플과 비교할 때는 미지원 노드·modifier, 페이지·View 전환을 따로 확인합니다.

## 1. 지금 확인할 수 있는 것

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-01 | 더 보기(…) ▸ Show in Finder → 샘플 폴더 | `stellar.yaml`, `stellar.lock.yaml`, `design`(`current`, `generations`, `objects`), `assets`(`<prefix>/<hash>.jpg` 하나) |
| QA1-02 | 터미널에서 `git -C <샘플 폴더> log --oneline` | 커밋 2개("New Stellar project initialized", "QA 샘플 문서와 에셋 추가"). Git identity가 없었으면 커밋 없음 |
| QA1-03 | 사이드바 필터 필드 보기 | 실제 문서 필터는 **구현 대기**이며 입력이 비활성입니다. |

## 2. 페이지·View 목록(사이드바 Pages)

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-10 | 샘플을 열고 Pages 구획 보기 | 현재 활성 페이지와 View 이름이 표시됩니다. 전체 페이지·View 목록 탐색은 **구현 대기**입니다. |
| QA1-11 | Mobile View 클릭 | Mobile이 active View로 선택되고 Layers 구획과 캔버스가 Mobile 내용으로 바뀜. **구현 대기** |
| QA1-12 | View 행에서 Return 또는 더블클릭 | 인라인 이름 변경 시작(편집 확인은 QA2). **구현 대기** |

## 3. 레이어 트리(사이드바 Layers)

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-20 | Desktop View의 Layers 구획 보기 | 최상위에 Header, Hero, Cards, Accent (Hidden), Footer (Locked). root Frame(Desktop)은 행으로 나오지 않음. **구현 대기** |
| QA1-21 | Cards ▸ Card 1 펼치기 | Thumbnail, Caption. disclosure로 깊이 제한 없이 펼쳐짐. 화살표 키로 이동·펼침. **구현 대기** |
| QA1-22 | 종류 아이콘 비교 | Frame, Fragment(Icon Group), 사각형·타원·선·다각형, path(Star), 텍스트, 이미지가 서로 다른 SF Symbol. **구현 대기** |
| QA1-23 | Accent (Hidden), Footer (Locked) 행 보기 | 숨김 행은 흐린 글자와 `eye.slash` 아이콘, 잠금 행은 `lock` 아이콘이 항상 보임. 색만으로 구분하지 않음. **구현 대기** |
| QA1-24 | 필터에 "Card" 입력 | 일치 행(Card 1\~3)과 조상(Cards)만 보이고 모두 펼쳐짐. 조상 행은 보조 색. 없는 이름이면 "No Results"와 Clear Filter. **구현 대기** |
| QA1-25 | Components ▸ Buttons의 레이어 보기 | Button / Primary·Secondary(▸ Label), Icon Group(▸ Dot, Square), Divider, Hexagon, Star. **구현 대기** |

## 4. 캔버스 렌더

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-30 | 새 프로젝트와 샘플의 캔버스 비교 | 새 프로젝트의 흰 Frame, 지원되는 자유 배치 Frame의 단색·테두리·모서리·자식 클리핑을 확인합니다. 그림자 등 미지원 효과가 있는 레이어와 샘플의 연결되지 않은 이미지는 별도로 확인합니다. |
| QA1-31 | Headline 텍스트 | 샘플에 사용 가능한 프로젝트 글꼴이 지정되어 있는지 확인합니다. 새 프로젝트의 한글 글꼴과 실제 제목 표시는 QA2-60에서 검사합니다. |
| QA1-32 | Hero Image | 샘플의 Hero Image는 JPEG 파일을 참조하지 않으므로 그 이미지가 나타나지 않습니다. QA2-03에서 직접 배치한 PNG는 실제 파일의 픽셀로 표시됩니다. |
| QA1-33 | 창 크기·화면 배율(다른 모니터로 옮기기) 바꾸기 | 흐려지거나 늘어나지 않고 다시 그려짐. **구현 대기**(XII.4.1) |
| QA1-34 | 다크 모드 전환 | 앱 크롬만 바뀌고 문서 색은 그대로. **구현 대기**(XII.9.3) |

## 5. 이동·확대

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-40 | 트랙패드 두 손가락 스크롤, 핀치 | 캔버스가 바로 따라 움직이고 확대·축소됨. **구현 대기**(viewport camera XII.4.3, 도구 상태 기계 XII.3.6) |
| QA1-41 | Hand(H)를 선택해 캔버스 드래그 | 문서의 위치를 바꾸지 않고 화면을 이동합니다. Zoom 도구 클릭 및 Space 일시 전환은 별도 확인 대상입니다. |
| QA1-42 | 열린 창에서 확대·축소·실제 크기 메뉴 사용 | 캔버스와 배율 표시가 함께 바뀝니다. 최초 새 프로젝트는 창에 맞춰 열립니다. |
| QA1-43 | 이동·확대 뒤 저장 상태 | 문서가 바뀌지 않음(`git status` 변화 없음). **구현 대기**(XII.4.3 완료 기준) |

## 6. 선택과 인스펙터

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-50 | 캔버스에서 Card 1 클릭 | 선택 표시와 핸들, 사이드바 Card 1 행 선택. **구현 대기**(XII.3.6, 선택 overlay XII.4.4, 창별 선택 XII.2.5) |
| QA1-51 | 신뢰한 새 프로젝트에서 F로 추가한 자식 Frame 하나를 Layers에서 선택 | 캔버스 선택과 Inspector의 W/H가 같은 Frame을 가리킵니다. 전체 Appearance·Modifiers 편집은 **구현 대기**입니다. |
| QA1-52 | Headline(Text) 선택 | Layer · Text · Character · Paragraph · Modifiers. **구현 대기** |
| QA1-53 | Hero Image 선택 | Layer · Image(fit Cover) · Modifiers. 누락 에셋이면 진단 아이콘. **구현 대기** |
| QA1-54 | Icon Group(Fragment) 선택 | Layer · Fragment 안내와 Convert to Frame. Modifier 섹션 없음. **구현 대기** |
| QA1-55 | Divider, Hexagon, Star 선택 | 도형 섹션 제목이 Line·Polygon, path는 Path 섹션. Modifiers에 Divider는 Transform·Stroke(두께 1), Hexagon은 Transform·Fill·Stroke(두께 2). **구현 대기** |
| QA1-56 | Card 1\~3 다중 선택(⌘-클릭) | 같은 값은 그대로, 다른 값은 혼합 표시. **구현 대기** |
| QA1-57 | Footer (Locked) 선택 | 잠금 안내와 Unlock 버튼, 속성 필드는 비활성(이름·표시·잠금만 편집 가능). **구현 대기** |
| QA1-58 | 새로 연 창에서 아무 레이어도 선택하지 않고 Inspector 보기 | No Selection을 표시합니다. 단일 Free Frame을 선택하면 W/H가 나타납니다. |

## 7. Component

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-60 | Components 페이지의 버튼을 Component 정의로 보기, 인스턴스 선택 시 Component · Properties · Slots 섹션 | 샘플의 두 버튼은 일반 Frame입니다. 공유 정의와 인스턴스 표시를 보려면 QA2-78~80을 사용합니다. 인스펙터의 Properties·Slots 편집은 **구현 대기**입니다. |

## 8. 여러 창

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-70 | 같은 샘플을 새 창에서 열기 | 창마다 다른 active View·선택·viewport. **구현 대기**(XII.2.4, XII.2.5). 지금: 같은 폴더를 다시 열면 기존 창이 앞으로 옴(QA0-09) |
| QA1-71 | `--name other`로 샘플 하나를 더 만들고 두 프로젝트를 함께 열기 | 프로젝트마다 창 하나, 제목은 둘 다 "Stellar QA Sample". Window 메뉴에 두 창. 알려진 한계: 둘째 창부터 창 크기가 저장되지 않고 pane 폭 기록은 서로 덮어씀(창별 저장 이름 규칙은 XII.2.4에서 정함) |
