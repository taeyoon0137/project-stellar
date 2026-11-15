# QA1 문서 보기 확인 목록

샘플 프로젝트의 문서(페이지·View·레이어)를 보고 살펴보는 동작을 확인합니다. 진행 방식과 피드백 형식은 [README](README.md)를 따릅니다. 샘플 내용은 README 3.1절에 있습니다.

- 실행: `./scripts/macos/qa.sh`(샘플이 바뀌었으면 `--fresh`)
- 기준 명세: [사이드바](../design/ui/sidebar.md), [인스펙터](../design/ui/inspector.md), [셸 프레임](../design/ui/shell_frame.md), [macOS 셸 디자인 가이드](../design/macos-shell-design.md) 6장
- 열린 창은 저장된 활성 View의 Layers를 읽고 지원하는 Free Frame을 캔버스에 표시합니다. 각 항목에서 전체 샘플과 비교할 때는 미지원 노드·modifier, 페이지·View 전환을 따로 확인합니다.

## 1. 지금 확인할 수 있는 것

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-01 | 상단 바 제목을 눌러 Show in Finder(또는 File ▸ Show in Finder) → 샘플 폴더 | `stellar.yaml`, `stellar.lock.yaml`, `design`(`current`, `generations`, `objects`), `assets`(`<prefix>/<hash>.jpg` 하나) |
| QA1-02 | 터미널에서 `git --no-pager -C <샘플 폴더> log --oneline` | 커밋 2개("New Stellar project initialized", "QA 샘플 문서와 에셋 추가"). Git identity가 없었으면 커밋 없음 |
| QA1-03 | 사이드바 필터 필드 보기 | 문서 필터를 연결할 때까지 필터 필드는 보이지 않습니다. **구현 대기**(전용 WBS 없음) |

## 2. 페이지·View 고르기(사이드바 머리 줄)

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-10 | 샘플을 열고 사이드바 머리 줄 보기 | `Home › Desktop ▾`처럼 활성 Page와 View가 팝업 한 줄로 보입니다. 따로 차지하는 Pages 구획은 없습니다. |
| QA1-11 | 팝업에서 Mobile View 고르기 | Mobile이 활성 View가 되고 Layers와 캔버스가 Mobile 내용으로 바뀝니다. |
| QA1-12 | Page·View 이름 바꾸기 | **구현 대기**(UX.12). |

## 3. 레이어 트리(사이드바 Layers)

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-20 | Desktop View의 Layers 구획 보기 | 최상위에 Header, Hero, Cards, Accent (Hidden), Footer (Locked). root Frame(Desktop)은 행으로 나오지 않음 |
| QA1-21 | Cards ▸ Card 1 펼치기 | Thumbnail, Caption. disclosure로 깊이 제한 없이 펼쳐짐. 화살표 키로 이동·펼침 |
| QA1-22 | 종류 아이콘 비교 | Frame, Fragment(Icon Group), 사각형·타원·선·다각형, path(Star), 텍스트, 이미지가 서로 다른 SF Symbol |
| QA1-23 | Accent (Hidden), Footer (Locked) 행 보기 | 숨김 행은 `eye.slash`, 잠금 행은 자물쇠(`lock.fill`) 아이콘으로 구분됨. 색만으로 구분하지 않음 |
| QA1-24 | 필터에 "Card" 입력 | 일치 행(Card 1\~3)과 조상(Cards)만 보이고 모두 펼쳐짐. 조상 행은 보조 색. 없는 이름이면 "No Results"와 Clear Filter. **구현 대기** |
| QA1-25 | Components ▸ Buttons의 레이어 보기 | Button / Primary·Secondary(▸ Label), Icon Group(▸ Dot, Square), Divider, Hexagon, Star |

## 4. 캔버스 렌더

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-30 | 새 프로젝트와 샘플의 캔버스 비교 | 새 프로젝트의 흰 Frame, 지원되는 자유 배치 Frame의 단색·테두리·모서리·자식 클리핑을 확인합니다. 그림자 등 미지원 효과가 있는 레이어와 샘플의 연결되지 않은 이미지는 별도로 확인합니다. |
| QA1-31 | Headline 텍스트 | 샘플에 사용 가능한 프로젝트 글꼴이 지정되어 있는지 확인합니다. 새 프로젝트의 기본 글꼴과 한글 제목 표시는 QA2-60에서 검사합니다. |
| QA1-32 | Hero Image | 샘플의 Hero Image는 JPEG 파일을 참조하지 않으므로 그 이미지가 나타나지 않습니다. QA2-06에서 직접 배치한 PNG는 실제 파일의 픽셀로 표시됩니다. |
| QA1-33 | 창 크기·화면 배율(다른 모니터로 옮기기) 바꾸기 | 흐려지거나 늘어나지 않고 다시 그려짐. **구현 대기**(XII.4.1) |
| QA1-34 | 다크 모드 전환 | 앱 크롬만 바뀌고 문서 색은 그대로. **구현 대기**(XII.9.3) |

## 5. 이동·확대

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-40 | 트랙패드 두 손가락 스크롤, 핀치 | 캔버스가 바로 따라 움직이고 확대·축소됨. 문서는 바뀌지 않음 |
| QA1-41 | Hand(H)로 끌기, Zoom(Z)으로 클릭·Option-클릭 | 문서 위치는 그대로 두고 화면이 이동·확대·축소됩니다. 다른 도구에서 Space를 누르는 동안 끌면 Hand처럼 화면이 이동하고, 떼면 원래 도구로 돌아옵니다 |
| QA1-42 | ⌘+, ⌘-, ⌘0, ⌘9와 툴바 줌 메뉴 | 확대·축소·실제 크기·맞춤, 툴바 배율 제목 갱신. 새 프로젝트는 창에 맞춰 열립니다. 선택 맞춤(Zoom to Selection)은 메뉴에 없음(아직 안 됨) |
| QA1-43 | 이동·확대 뒤 저장 상태 | 문서가 바뀌지 않음(`git status` 변화 없음). **구현 대기**(XII.4.3 완료 기준) |

## 6. 선택과 인스펙터

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-50 | 캔버스에서 Card 1 클릭 | 선택 상자와 크기 핸들이 보이고 사이드바 Card 1 행도 선택됩니다. |
| QA1-51 | 새 프로젝트(제한 모드 그대로)에서 F로 추가한 자식 Frame 하나를 Layers에서 선택 | 캔버스 선택과 속성 탭이 같은 Frame을 가리킵니다. 행 레이블은 CSS 이름(`width`, `height`, `left`, `top`, `background-color`, `border`, `border-radius`, `overflow` 등)이고 값은 CSS 값(`320px`)입니다. |
| QA1-52 | Headline(Text) 선택 | 속성 탭에 Text에 해당하는 섹션(크기·위치, `color` 등)만 보입니다. 글꼴 longhand(Typography 편집)는 **구현 대기**(IX.11.11, UX.8.5). |
| QA1-53 | Hero Image 선택 | `width`·`height`·`object-fit` 행이 보입니다. 누락 에셋 진단 표시는 **구현 대기**. |
| QA1-54 | Icon Group(Fragment) 선택 | Fragment에 해당하는 행만 보이고 빈 섹션은 없습니다. |
| QA1-55 | Divider(Line), Hexagon(Polygon) 선택 | Divider는 `x1`·`y1`·`x2`·`y2`·`stroke`·`stroke-width`, Hexagon은 `width`·`height`·`polygon-sides`·`fill`·`stroke` 등 그 종류의 행이 보입니다. Path(Star)의 `fill`·`stroke` 행은 캔버스가 Path를 그리지 않아 두지 않습니다. |
| QA1-56 | Card 1\~3 다중 선택(Shift-클릭) | 같은 값은 그대로, 다른 값은 "혼합"으로 비어 보입니다. |
| QA1-57 | Footer (Locked) 선택 | 잠긴 레이어의 값 행은 바꿀 수 없고 `locked`(잠금 해제)만 바꿀 수 있습니다(D8). |
| QA1-58 | 새로 연 창에서 아무 레이어도 선택하지 않고 속성 탭 보기 | 현재 `Page · View` 이름과 도구 키 안내만 보이고 편집 필드는 없습니다. |

## 7. Component

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-60 | Components 페이지의 버튼을 Component 정의로 보기, 인스턴스 선택 시 Component · Properties · Slots 섹션 | 샘플의 두 버튼은 일반 Frame입니다. 공유 정의와 인스턴스 표시(제한 모드 포함)는 QA2-78에서 확인합니다. 인스펙터의 Properties·Slots 편집은 **구현 대기**입니다. |

## 8. 여러 창

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA1-70 | 같은 샘플을 새 창에서 열기 | 창마다 다른 active View·선택·viewport. **구현 대기**(XII.2.4, XII.2.5). 지금: 같은 폴더를 다시 열면 기존 창이 앞으로 옴(QA0-09) |
| QA1-71 | `--name other`로 샘플 하나를 더 만들고 두 프로젝트를 함께 열기 | 프로젝트마다 창 하나, 제목은 둘 다 "Stellar QA Sample". Window 메뉴에 두 창. 알려진 한계: 둘째 창부터 창 크기가 저장되지 않고 pane 폭 기록은 서로 덮어씀(창별 저장 이름 규칙은 XII.2.4에서 정함) |
