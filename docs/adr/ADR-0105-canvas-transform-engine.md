# ADR-0105: 캔버스 변형·정렬 엔진을 코어 순수 계산과 배치 쓰기 한 곳으로 만든다

## Status

Accepted (2026-11-23). 계획서 XII.10.9~XII.10.16, XII.10.21, XII.10.23, XII.10.44의 코어 부분입니다. 셸 UI와 FFI는
UI 재설계 때 정합니다(ADR-0098). ADR-0071의 캔버스 직접 조작 결정을 이어 받으며 ADR-0071 본문은 고치지 않습니다.

## Context

- 사용자 방향(2026-11-23): 지금은 엔진·기능만 만들고 Rust 테스트·CLI·MCP로 검증합니다. 셸 UI는 지워졌습니다(ADR-0098).
- 캔버스 편집의 앞선 코어(ADR-0071)는 셸 ABI 하나에 묶인 단일 node 크기·반경·이동과 스냅 계산
  (`stellar_runtime::app::snap`)을 가집니다. 뒤집기, 회전, 다중 선택 크기 조절, 비례 확대, 정렬·분배, 순서·묶음,
  재부모화, 스마트 가이드 자료는 없습니다.
- 저장 형식: 위치·transform 분리 migration(IX.11.9)의 step은 아직 제품 step 목록에 없고, scene은 layout record 위치를
  그리지 않습니다(ADR-0086 결정 3). 그래서 지금 화면 위치를 정하는 저장 값은 Frame `transform`(`matrix()`)과 Frame이 아닌
  node의 위치 Transform modifier입니다. `scale`·`rotate`·`translate`·`transform-origin` longhand는 아직 없습니다.
- Frame 테두리는 묶음 값(0x0105)으로만 저장되고 longhand command가 없습니다(IX.11.10 남은 범위). Text 글자 크기는 문서
  Typography resource에 있고 node 하나의 `font-size` command가 없습니다(IX.11.11 남은 범위).
- XII.10.2(단위·수식키 ADR)는 같은 날 다른 작업(ADR-0104)이 정합니다.
- crate 그래프: commands·runtime은 layout crate에 의존할 수 없습니다.

## Decision

1. **배치 쓰기는 한 곳(`stellar_commands::arrange::place`)입니다.** 모든 변형을 "node local → 부모 공간 변환을 부모 공간
   affine `change`만큼 바꾸기"(`apply_affine`, 이동은 `translate`)와 "크기 longhand 바꾸기"(`set_size`) 두 가지로 줄입니다.
   - 지금 저장 위치: Frame `transform` = `change ∘ 지금`, 그 밖은 위치 Transform modifier = `change ∘ 지금 위치 modifier`
     (앞부분 Transform이 여럿이어도 합성 결과가 정확), 없으면 맨 앞에 새 Transform modifier. 배치한 인스턴스는 배치
     Transform 하나의 평행 이동만 받습니다.
   - 기존 캔버스 이동 경로(`canvas_gesture::move_command`)도 이 함수를 부릅니다. 같은 저장 규칙이 두 곳에 있지 않습니다.
   - IX.11.9 step이 등록되면(위치는 layout record `left`·`top` 또는 `translate`, 선형부는 `rotate`·`scale`·`transform`) 이
     module만 바꿉니다. 그 전까지 행 완료 기준의 "item record 위치 longhand만 바뀜", "이동 후 `transform` 불변", "`scale`
     부호만 반전"은 충족할 수 없고 부분 완료로 남깁니다.
   - 회전 행렬은 O13 규칙(직각 배수는 정확한 0·±1, 그 밖은 f64 `sin_cos`의 f32 반올림)을 `place::rotation`에 둡니다.
     `stellar_layout::transform::rotate_matrix`와 같은 규칙이며 crate 그래프 때문에 따로 둡니다.
2. **제스처는 시작 때 한 번 읽고 update는 순수 계산, 확정은 transaction 하나입니다.** `stellar_runtime::canvas::transform`의
   `ResizeGesture`·`RotateGesture`·`MoveDrag`는 게시된 캔버스 scene에서 대상 기하를 시작 때 읽고, update는 문서·history를
   바꾸지 않으며, `commit`이 CommandTransaction 하나를 창 history에 기록합니다. 그래서 drag 500 update는 history 1건,
   확정하지 않은 취소(Escape)는 0건입니다.
   - 크기 조절: 상자 계산은 핸들 미리보기와 같은 `gesture_math::preview`. `aspect-ratio`가 `auto`가 아니면(생략값은 활성 기본값
     세트로 풂, ADR-0082) 비율 유지가 기본이고 Shift가 일시 해제합니다. 반대편(Option이면 중심)을 넘으면 그 축을 뒤집습니다. 단일 선택은 핸들이 local
     좌표라 local 축 기준이고 고정 변의 world 위치가 남습니다. 다중 선택은 선택 경계를 늘리고 node마다 자기 축 방향
     길이 비율로 크기를 바꿉니다(회전 node가 기울지 않음). 흐름 자식은 조절한 축의 `width`·`height`만 씁니다.
   - 회전: pivot은 단일 선택이면 border box 중심, 다중 선택이면 선택 경계 중심. Shift는 단일 선택이면 결과 world 각도,
     다중 선택이면 회전량을 15도 배수로 맞춥니다. **C3: 회전 중심 이동 핸들을 두지 않습니다.** 저장된
     `transform-origin`이 없어 핸들이 바꿀 값이 없고, 중심 고정 회전이 CSS `rotate`(origin 50% 50%)와 같은 결과입니다.
   - 반경: 기존 `CanvasEdit::Radius` 경로를 그대로 쓰고, 핸들 수치는 저장값, 표시는 평가 규칙(VI.5.2)으로 줄인 값입니다.
   - 이동: 같은 부모 안 이동·Option 복제는 기존 `CanvasEdit::Move` 경로 하나입니다. Shift는 더 많이 움직인 축만 남깁니다.
     자유 배치 Frame 위에 놓으면 world 위치를 지킨 채 `MoveNode`로 부모를 바꿉니다(복제와 함께면 복제본을 새 부모에 바로
     만듦). 흐름 컨테이너 안으로 놓기는 XII.10.24 몫이라 거부하고, 인스턴스 투영 행 안은 공개 Slot 편집이 열릴 때까지
     거부합니다.
   - 키보드: 화살표 이동은 기존 부모 공간 이동 경로(0.5초 반복 grouping)이고, 흐름 자식의 주축 화살표는 흐름 순서를 한 칸
     바꾸며, 선택이 없으면 camera를 팬합니다. 크기 조절 키는 오른쪽·아래 가장자리를 옮기고 같은 node·속성 반복을 VII.4.4
     규칙으로 한 항목에 합칩니다. 단위·키 배정은 XII.10.2(ADR-0104) 값이고 호출자가 넘깁니다.
3. **스냅 엔진은 `stellar_runtime::canvas::snap` 하나입니다.** 앞선 `app::snap`을 옮겨 확장하고 `app::snap`은 같은 module을
   다시 내보냅니다.
   - 우선순위: 가이드 → 레이아웃 그리드 → 부모·형제 가장자리·중심(부모 먼저) → 같은 간격 → 같은 크기 → 픽셀 그리드. 같은
     종류면 가까운 쪽, 그다음 작은 좌표입니다. 가이드·그리드 선은 호출자가 부모 local 좌표로 넘깁니다(XII.10.18~XII.10.20이
     저장 위치를 정할 때까지 선 목록만 받음).
   - 흐름 컨테이너 자식에는 객체 스냅을 적용하지 않습니다. 가시 영역 밖 형제는 후보가 아닙니다.
   - 제스처 시작 때 좌표·크기를 축마다 정렬한 표와 형제 공간 인덱스(`GridIndex`)를 만들고 update는 이분 탐색과 띠 질의만
     합니다. 형제 상한은 65,536(넘으면 시작 상자에 가까운 것만)입니다.
   - 허용 오차는 화면 pt를 부모 local 1px의 화면 배율로 나눈 값이라 줌과 무관하게 화면에서 같습니다.
   - 스마트 가이드·측정(`snap::guides`)은 정렬선·가장자리 거리·같은 간격·같은 크기 자료와 Option-hover 측정을 만들고,
     화면 좌표 transient scene layer(선·라벨 배경)로 바꿉니다. 이 layer는 문서·저장 경로 밖입니다.
   - **C2: 측정 라벨 glyph는 셸의 시스템 UI 글꼴입니다.** 코어는 라벨 문자열(CSS 길이 표기)과 자리만 냅니다.
   - **C5: Text 측정은 layout 상자 가장자리 기준입니다.** baseline 사이 거리는 내지 않습니다. 측정값이 그대로 CSS
     `margin`·`gap`·inset이 되어야 하기 때문입니다.
   - 대비 높이기·투명도 줄이기는 셸이 `GuideStyle`로 넘깁니다(ShellTheme XII.9.3).
4. **비례 확대(Scale)**: 고른 node와 자손 전체의 크기·위치, Stroke 두께, StrokeExpand 폭, Corner·Frame·Rectangle·Polygon
   반경, Shadow blur·offset 두 longhand(0x1094·0x1095, 묶음 key 0x1092 아님), Blur 반경을 같은 비율로 바꿉니다. 지금 command로
   쓸 수 없는 값(Frame `border-*-width`, Text `font-size`, 인스턴스 크기, Path point)은 건너뛰고 이유 코드로 돌려줍니다. 묶음
   key 0x0105를 쓰지 않습니다.
5. **뒤집기(Flip)**: 축은 world 축(선택 world 경계 중심을 지나는 세로선·가로선)입니다. 회전한 node도 화면에 보이는 그대로
   거울상이 됩니다(45도 node의 가로 뒤집기 = -45도 회전 + local 좌우 반전). 표현은 위치 변환 선형부의 부호 반전이고
   (`transform: matrix(-1, 0, 0, 1, tx, ty)`), Path·Polygon geometry를 따로 반전하지 않습니다. 선형 변경을 받는 모든 상자
   node(Frame·Shape·Path·Text·Image)가 같은 규칙이고 배치한 인스턴스·Fragment는 비활성입니다. 크기 핸들로 넘어 뒤집는
   경우(결정 2)는 local 축 기준이라 이 명령과 다릅니다.
6. **정렬·분배·행열 정리**: 계획은 `stellar_commands::arrange::{align, plan}` 하나이고 상자 원천만 호출자마다 다릅니다
   (`plan::Geometry`: runtime은 scene, CLI·MCP는 문서 값 `ModelGeometry`). 회전 node는 world 경계 기준입니다. 단일 선택은
   부모, 다중 선택은 선택 경계 또는 primary(기준 객체) 기준이고, 분배는 축 중심 순서의 첫·끝 객체를 고정합니다. 행열
   정리는 읽는 순서(위, 왼쪽)로 `ceil(√n)`열 격자에 열 최대 너비·행 최대 높이와 간격으로 놓으며 흐름 컨테이너를 만들지
   않습니다. 기본 간격 `DEFAULT_GRID_GAP`(10px)은 XII.10.2 값이 다르게 정하면 그 값으로 바꿉니다. 흐름 자식은 flex 주축
   정렬 비활성(`justify-content` 안내), 교차축은 `align-self`(`flex-wrap: wrap-reverse`면 시작·끝이 바뀜), grid는
   `justify-self`·`align-self`이고 흐름 자식의 분배·행열 정리는 비활성입니다. ADR-0074가 Overlay 같은 비CSS 배치를 없앴으므로
   "Overlay 자식" 규칙은 대응할 것이 없고, 옛 `layout_mode` 참조 Frame의 자식은 배치 편집 전부를 비활성으로 둡니다. 배치
   문맥의 `display`·`position`·`flex-*`는 활성 기본값 세트로 풉니다(ADR-0082, 내장 세트가 아니면 `css`).
7. **순서·묶음**: Bring Forward·Send Backward·Bring to Front·Send to Back은 부모마다 고른 node의 상대 순서를 지키며 OrderKey만
   바꾸는 `MoveNode`입니다(아직 옮기지 않은 형제 key와 겹치지 않게 만듦). 흐름 컨테이너 자식이면 흐름 순서도 바뀌었다는
   알림 값(`flow_order_changed`)을 돌려줍니다. Group은 가장 앞 node 자리에 Fragment를 넣고 원래 OrderKey로 옮기며, Frame
   Selection은 배경·테두리 없는 Frame을 넣고 자식 위치를 그 원점만큼 되돌리고(흐름 자식은 거부), Ungroup은 자식을 같은
   자리로 옮기고 빈 묶음을 지웁니다. Frame을 풀면 Frame 변환을 자식 위치에 합쳐 world 위치를 지킵니다.
8. 셸·FFI 연결(ABI 모양, overlay 그리기, 메뉴·`ShortcutPolicy`)은 UI 재설계 때 정합니다. 계획서가 가리키는 지워진 UI
   문서(`docs/design/ui/tools_menus.md` 등) 갱신도 그때 합니다.

## Alternatives

- 행마다 저장 위치를 직접 쓰는 함수: 같은 규칙이 여러 곳에 생기고 IX.11.9 step 등록 때 모두 고쳐야 해서 채택하지
  않았습니다.
- 뒤집기를 local 축 기준으로: 회전한 node의 메뉴 뒤집기가 화면 방향과 달라 보이고, 다중 선택의 공통 축과 섞여 규칙이 둘이
  되어 채택하지 않았습니다.
- 다중 선택 크기 조절을 선택 affine 그대로 적용: 회전 node에 기울기(skew)가 생겨 CSS 값으로 되돌릴 수 없습니다.
- Scale에서 Frame 테두리를 묶음 key 0x0105로 쓰기: 계획서가 금지하고 longhand 저장(IX.11.10)과 어긋납니다.

## Consequences

- 같은 변형을 셸 없이 Rust 호출로 재현하고 test로 고정할 수 있습니다. CLI·MCP는 `arrange::plan`과 `ModelGeometry`로 같은
  계획을 씁니다.
- IX.11.9 step 등록 전까지 위치·회전·뒤집기는 `transform`/위치 Transform modifier 행렬에 기록됩니다. migration step은 그
  행렬을 위치와 선형부로 나눕니다(ADR-0024 결정 8).
- `app::canvas_gesture`의 몇 함수(`apply_edit`, `view_root`, `selection_roots`, `parent_of`)가 crate 안에서 보이게
  되었습니다(동작 변화 없음).
- 앞선 핸들 크기 조절 경로(`app::canvas_gesture`의 `CanvasEdit::Resize`, ADR-0071)는 아직 자기 저장 쓰기(Frame
  `transform`·위치 modifier 보정, Line 끝점 비율)를 가집니다. 이동만 `place`로 옮겼고, 크기 쓰기는 IX.11.9 step 등록 때
  `place::set_size`·`place::apply_affine`으로 합칩니다.

## Compatibility

파일 포맷·command 종류·FFI는 바뀌지 않습니다. 새 command 종류 없이 기존 SetProperty·SetModifierProperty·InsertModifier·
SetLayoutProperty·MoveNode·InsertNode·DuplicateSubtree·DeleteSubtree만 씁니다. `app::snap`의 공개 이름은 그대로이고
`SnapResult`에 `guide_kind`가 더해졌습니다.

## Performance

`canvas/snap-candidates-20k`(형제 2만 개·가이드·그리드) fixture에서 update당 스냅 계산 5,000회: 중앙값 30.75µs, p99
44.5µs, 최대 53.6µs(2026-11-23, 개발 장비 Apple M3 Max, release, 다른 작업 동시 실행 중, REF-MAC 판정 아님). 예산 p99
0.35ms 안입니다.

## Security

새 입력은 좌표·비율·각도이며 비유한 값과 범위 밖 비율(0.001~1000배 밖)을 거부합니다.

## Migration

없습니다. 저장 형식이 바뀌지 않습니다.
