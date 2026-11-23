# 작업 보고서: XII.10.19 눈금자와 가이드

- WBS: XII.10.19
- 작업명: 눈금자와 가이드(만들기·옮기기·지우기, 눈금 원점, 눈금자 표현 C1)
- 상태: 부분 완료 (`[~]`) — 코어 값·계산·제스처 완료, 문서 모델·command·저장 연결과 ⌘R 셸 연결 남음
- 결정: ADR-0117 결정 2·3
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/document/src/guides.rs`(신규: `RulerGuide`·`LayoutGrid`·`NodeGuides`·`GuideEdit`, canonical bytes, 텍스트 원본 표기), `crates/document/src/lib.rs`(mod 등록)
- `crates/runtime/src/canvas/ruler.rs`(신규), `crates/runtime/src/canvas.rs`(mod 등록)

## 구현 요약

- **C1 결정과 근거**: 눈금자는 셸이 화면 고정 띠로 그리고, 눈금 값과 화면 위치는 코어 `ruler_ticks`가 scene camera와 같은 `CanvasViewport`에서 계산합니다. 눈금자는 camera와 함께 움직이지 않는 화면 크롬이라 scene layer에 넣으면 camera 변환을 되돌리는 계산이 한 번 더 필요하고, 셸의 눈금자 hit와 그리기 위치가 갈라집니다. 같은 viewport 값에서 계산하므로 camera와 어긋날 수 없고 fixture가 world→view 변환 결과와 비교합니다.
- 큰 눈금은 1·2·5×10ⁿ px 중 화면 64pt 이상인 가장 작은 간격, 작은 눈금은 5등분(2 계열은 4등분), 라벨은 CSS 수 formatter(`css::value::format_number`)입니다.
- 눈금 원점(`ruler_origin`): primary 선택의 최상위 Frame local 원점(평행 이동만 있을 때), 아니면 View root.
- 가이드 제스처(`GuideDrag`): 눈금자에서 끌어 만들기(캔버스에서 놓으면 `GuideEdit::Add`, 눈금자에서 놓으면 없음), 선 위 3pt 안을 잡아 옮기기(`Move`, 움직이지 않으면 없음), 눈금자로 끌어 지우기(`Remove`). 새 가이드의 소유자는 pointer 아래 최상위 Frame, 없으면 View root. 픽셀 스냅은 호출자가 토글·Control 해제를 반영해 넘깁니다.
- 가이드 값의 편집은 `NodeGuides::apply`가 역 편집을 돌려주어, command로 감싸면 제스처 하나 = history 항목 하나이고 undo가 정확히 되돌립니다(fixture가 역 편집 연쇄로 확인).
- `guide_snap_lines`가 소유자 local 가이드를 끄는 node 부모 local의 스냅 가이드 선으로 바꿉니다(축 정렬 부모만).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib guides` | 통과(5): 편집 역 연쇄, 잘못된 편집 거부와 기록 불변, bytes·텍스트 정규형 왕복, 12열 트랙, 고정 트랙 정렬·정사각 칸 |
| `cargo test -p stellar-runtime --lib canvas::ruler` | 통과(4): 1·2·5 간격, pan·zoom 뒤 눈금이 camera 변환과 같은 화면 위치(C1 camera 동기), 최상위 Frame 원점·소유자, 가이드 생성·이동·삭제·스냅 |

## 차이(작업 지시와 다른 점)

- 가이드 추가·이동·삭제를 "일반 Command로 history에" 넣는 일은 문서 모델 필드·command 종류·저장 chunk가 필요하고 모두 이 작업의 소유 범위(`crates/document/src/guides*`, `crates/runtime/src/canvas/**`) 밖입니다. ADR-0117 결정 2.4에 연결 범위(`SetNodeGuides`, chunk·record kind 예약, 텍스트 원본 블록, Semantic Diff·Merge)를 정했고, 지금은 값과 역 편집까지 구현했습니다.
- ⌘R 표시 토글은 `CanvasViewOptions`(XII.10.17)의 `view.show_rulers`이고 셸 메뉴 연결은 셸 재설계 때 합니다.

## 남은 위험 / 한계

- 문서 연결 전에는 가이드가 저장되지 않습니다. 연결 작업이 `NodeGuides::encode`/`to_text`를 그대로 써야 이중 경로가 생기지 않습니다.
- 회전·확대된 최상위 Frame에는 Frame 기준 눈금·가이드를 두지 않습니다(View root 기준으로 대체).
