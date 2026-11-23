# ADR-0125: scene 문서 변환이 layout 결과 위치·크기로 그린다

## Status

Accepted (2026-11-24).

- ADR-0086 Consequences의 마지막 한계("scene·runtime은 아직 레이아웃 결과로 그리지 않습니다")를 닫습니다(lane 계획의
  SD.2).
- 계획 행 IX.11.9의 "layout 결과 위치와 `translate`·`rotate`·`scale`·`transform`을 CSS 순서로 합성해 scene
  `NodeInput.transform`을 만드는" 연결과, XII.10.24의 증분 layout preview·`position: absolute` 자식 inset 쓰기를 다룹니다.
- ADR-0074, ADR-0076, ADR-0082, ADR-0084, ADR-0086, ADR-0105, ADR-0116 결정 1, ADR-0117 결정 5를 따릅니다.

## Context

- layout 엔진(`stellar-layout`)은 CSS longhand 입력으로 증분 계산(`IncrementalLayout`)을 하지만, 문서 scene 변환
  (`stellar_scene::document`)은 Frame transform·payload 크기를 그대로 그렸습니다. 흐름 컨테이너(`display: flex`·`grid`)
  자식은 layout 결과와 다른 자리에 그려졌습니다.
- crate 그래프는 scene → layout 의존을 허용하지 않았습니다. 그래서 runtime 흐름 drag는 형제 preview를 흐름 순서
  permutation 근사(`reflow_preview`)로 따로 계산했습니다. 같은 배치를 두 곳에서 다르게 셈한 이중 경로입니다.
- IX.11.9 migration step(`frame_position`)은 이동 성분을 `position: absolute; left; top`으로 옮깁니다. scene이 그 값을
  그리지 않아 제품 step 목록에 등록할 수 없었습니다(ADR-0115 결정 5).
- 위치 원본이 아직 저장 transform인 문서가 대부분입니다. 흐름 밖 자식을 CSS `display: block` 흐름대로 쌓으면 기존
  문서의 화면이 모두 바뀝니다.

## Decision

1. **crate 그래프.** `stellar-scene`의 `document` feature가 `stellar-layout`에 의존합니다(`tools/xtask/src/crate_graph.rs`,
   `docs/architecture/crate-dependency-graph.md`). renderer는 이 feature를 켜지 않으므로 renderer 쪽 의존은 그대로입니다.
   runtime은 layout에 직접 의존하지 않고 scene의 `document::layout`(엔진 재수출 `engine` 포함)을 씁니다.
2. **layout 계산은 한 곳.** `stellar_scene::document::layout::DocumentLayout`이 View root subtree의 layout 입력을 문서에서
   만들고 `IncrementalLayout`으로 계산해 결과 상자를 보관합니다. `document_scene_source`·`scene_edits`·runtime 캔버스
   기능은 이 결과만 읽습니다.
   - 값: node마다 `style_defaults::resolve` → `ResolvedLayout::declarations` → `style_from_declarations`(ADR-0086 결정 8).
     record가 없는 node의 style은 (종류, payload 크기, 배치 규칙)으로 한 번만 만듭니다.
   - tree: Fragment 펼치기는 layout crate `view` 규칙 하나입니다. 증분 경로용 `view::flatten_children`을 더했고
     `build_layout_tree`도 같은 펼치기 함수(`flatten_core`)를 씁니다. 원천은 `LayoutSource::fragment`로 box 내용을 만들지
     않고 Fragment 여부만 답할 수 있습니다.
   - 숨긴 node(문서 `visible`과 View Variant 표시 patch)와 Boolean operand는 `display: none`입니다. 숨긴 Fragment 아래
     box도 view 규칙대로 `display: none`입니다. 숨김은 CSS `display: none`처럼 흐름에서 빠집니다.
   - 옛 `layout_mode` Frame과 ComponentInstance는 layout 자식을 넣지 않습니다. 인스턴스 크기는 정의 root Frame 크기
     (intrinsic)입니다. Line·Path는 geometry bounds가 intrinsic 크기입니다.
   - Text: `SceneSourceOptions::text_measure`(`TextLayoutHook`)가 있으면 그 hook이 줄바꿈 측정을 하고, 없으면 scene Text
     hook이 낸 줄바꿈 없는 크기를 고정 intrinsic으로 씁니다. 측정한 Text 입력은 같은 너비로 그릴 때 다시 씁니다.
3. **배치 규칙(`PlacementRule`).**
   - **Layout**: 흐름 컨테이너 자식(`commands::arrange::align::flow_parent`가 Flex·Grid)과 layout record에 `position`을
     명시한 node. local transform = `css_node_transform(layout 결과 border box 원점, origin 0 0, 저장 transform)`이고 크기는
     layout 결과 크기입니다. Frame은 payload 크기 대신 결과 크기, Rectangle·Ellipse·Polygon·Image도 결과 크기로 그립니다
     (Ellipse·Polygon bounds 원점, Line geometry, Path geometry는 그대로). 흐름 자식의 저장 transform 이동 성분은 CSS
     `translate`처럼 layout 위치 뒤에 더해집니다(IX.11.9의 flow 부모 분기와 같은 뜻).
   - **Stored**: 그 밖의 자식(옛 자유 배치). 저장 transform·payload 크기로 이전과 같게 그립니다. layout 입력에서는
     `position: absolute; left: 0; top: 0; right: auto; bottom: auto`로 두어 부모 흐름에 참여하지 않게 합니다. IX.11.9
     migration이 이 node에 `position: absolute; left; top`과 선형 `transform`을 쓰면 Layout 규칙으로 바뀌고
     `translate(left, top) ∘ 선형 부분`이 옛 transform과 같은 화면 위치입니다(fixture로 확인).
   - **Root**: View root는 캔버스 배치(저장 transform)와 payload 크기입니다.
   - `transform-origin` longhand는 아직 저장하지 않아 합성 origin은 언제나 `0 0`입니다.
4. **Text 줄 너비.** Layout 규칙 Text는 Text hook에 줄 너비를 넘깁니다(`TextHook`의 마지막 인자). 풀린 `width`가
   `max-content`면 `None`(줄바꿈 없음), 그 밖에는 layout content box 너비입니다(`<length>`는 그 px, 흐름이 줄이거나 늘린
   값 포함). Stored·Root Text는 이전처럼 `None`입니다.
5. **실패.** layout 입력을 만들지 못하거나 계산이 실패하면(엔진이 받지 않는 조합 등) `DocumentLayout::error`에 이유를
   남기고 Layout 규칙 node만 `SkipReason::LayoutNotEvaluated`로 뺍니다. Stored·Root node는 영향을 받지 않습니다. 숨겨
   상자가 없는 Layout 규칙 node는 저장 값으로 둡니다(보이지 않음).
6. **증분 전파.** `scene_edits`는 같은 change set으로 `DocumentLayout::update`를 먼저 부릅니다.
   - 다시 만들 layout node: 바뀐·삽입된 node, 표시 patch가 바뀐 node. 경로 없는 구조 변경·부모·순서·표시 변경은 box
     부모의 자식 목록을 다시 펼치고, container longhand(`0x0110`–`0x0121`) 변경은 자식 전체를 다시 만듭니다(흐름 여부가
     바뀜). Frame의 옛 `layout_mode` 변경도 같게 다룹니다(자식이 layout 입력에 들어가는지가 바뀜). 활성 기본값 세트 이름이나 측정 hook 유무가 바뀌면 전체를 다시 계산합니다.
   - `IncrementalLayout::apply` 뒤 다시 배치한 node와 그 자식, 다시 만든 node, 표시가 바뀐 subtree의 상자를 비교합니다.
     border box·content box가 바뀐 node는 scene에서 다시 변환합니다(baseline·overflow만 바뀐 상자는 다시 변환하지 않음).
     그래서 흐름 자식 하나의 크기 변경이 change set에 없는 형제의 Upsert로 이어지고, `IncrementalScene::update`는 그
     Upsert를 geometry dirty로 봅니다.
   - 증분 갱신이 실패하면 전체 다시 계산으로 복구합니다. 증분 결과는 처음부터 계산한 결과와 같아야 합니다(상자 비교
     test).
7. **흐름 drag preview(XII.10.24).** `DocumentLayout::reorder_preview`는 컨테이너 자식 순서만 바꾼 입력을 복사한 증분
   layout에 넣고, `DocumentSceneSource::reorder_preview`는 상자가 바뀐 node를 다시 변환해 `ScenePreview`(그 node 대체
   입력)를 만듭니다. runtime `FlowDrag::update`는 이 결과의 world 상자와 `ScenePreview`를 냅니다. permutation 근사
   (`reflow_preview`)는 지웠습니다. 삽입 위치 계산과 컨테이너·형제 상자도 layout 결과 source(`CanvasLayout`)에서 읽습니다.
8. **`position: absolute` 자식 이동(XII.10.24).** `runtime::canvas::flow_inset::InsetDrag`는 Layout 규칙이고 풀린
   `position`이 `absolute`인 node를 inset longhand `SetLayoutProperty`로 옮깁니다(저장 transform은 그대로).
   - 축마다 현재 anchor: 시작만 정해졌으면 시작, 끝만 정해졌으면 끝(반대 방향), 둘 다면 같이 옮겨 크기 유지, 둘 다
     `auto`면 시작 inset에 지금 위치 + 이동량 px.
   - 백분율 inset은 containing block(부모 padding box) 기준 백분율로 둡니다. world 이동량은 부모 world 선형 부분의 역으로
     부모 local로 바꿉니다. 한 gesture가 한 transaction입니다.
9. **runtime source.** `runtime::canvas::scene_layout::CanvasLayout`은 활성 View를 `document_scene_source`로 변환한
   source를 보관하고 world 상자·preview를 냅니다. `CanvasLayout::for_scene`은 게시한 캔버스 행 목록이 이미 layout한
   Text 크기를 Text 입력으로 써서 Text 흐름 자식도 크기와 상자를 갖게 합니다(생성 도구의 흐름 삽입 위치가 씀). 옛 캔버스 행 목록(`CanvasScene`, `canvas.rs`)은 저장 transform으로
   그리는 이전 경로로 남습니다. UI 재설계 때 화면은 이 source를 그립니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| runtime이 layout crate에 직접 의존해 따로 계산 | 채택하지 않음. scene 변환과 runtime이 같은 배치를 두 번 셉니다 |
| 흐름 밖 자식도 CSS block 흐름대로 그림 | 채택하지 않음. 위치 원본이 저장 transform인 기존 문서의 화면이 모두 바뀝니다. IX.11.9 migration이 그 위치를 inset으로 옮긴 뒤에는 같은 결과가 Layout 규칙으로 나옵니다 |
| 모든 box의 크기를 layout 결과로 그림(Stored 포함) | 채택하지 않음. subpixel 반올림과 content-box padding 때문에 record가 없는 기존 node의 크기가 달라집니다 |
| 매 편집마다 layout 입력 전체를 다시 만들어 비교 | 채택하지 않음. node마다 기본값 세트 해석과 선언 읽기를 해 편집 비용이 문서 node 수에 비례합니다 |
| 숨긴 node를 `visibility: hidden`처럼 자리 유지 | 채택하지 않음. layout view는 숨긴 Fragment 아래를 `display: none`으로 둡니다. 같은 규칙을 box node에도 씁니다 |
| permutation 근사 preview 유지 | 채택하지 않음. 크기가 다르거나 분배 정렬이면 놓은 뒤 화면과 다릅니다 |

## Consequences

- 흐름 컨테이너 자식과 `position`을 명시한 node는 Web export와 같은 CSS 배치로 그려집니다. 기존 문서(record 없음)의
  scene 결과는 그대로입니다(기존 scene·runtime·ops test 통과).
- IX.11.9 제품 step 등록(`crates/project` `PRODUCT_STEPS`)의 scene 조건이 충족됩니다. 등록은 project crate 소유
  작업입니다.
- 남은 일:
  - 옛 캔버스 행 목록(`CanvasScene`)과 그것을 읽는 runtime 제스처(이동·크기·회전, 선택 hit)는 저장 transform 기준입니다.
    UI 재설계 때 `CanvasLayout` source로 바꿉니다.
  - xtask replay의 layout 변환(`tools/xtask/src/replay/layout.rs`)은 scene source에서 따로 layout 입력을 만듭니다. 이
    결과(`DocumentSceneSource::layout`)를 쓰도록 바꿔야 합니다(XVI.2.2).
  - `transform-origin` longhand, View·ComponentDefinition root의 캔버스 배치 속성은 IX.11.9 남은 범위입니다.
  - 인스턴스 가상 subtree 안의 흐름 배치는 정의 tree layout(ADR-0061 형식 개정 뒤)에서 엽니다.

## Compatibility

- 저장 형식·command codec·FFI 변경은 없습니다.
- 공개 API: `TextHook`에 줄 너비 인자가 생겼고(`Option<f32>`), `SceneSourceOptions`에 `text_measure`가 생겼습니다.
  `LayoutSource`에 기본 구현이 있는 `fragment`가 생겼습니다. runtime `FlowDrag`의 `begin`·`update`·`commit`은
  `CanvasLayout`을 받고, `FlowUpdate::Inside`에 `scene`(대체 입력)이 생겼습니다. `reflow_preview`는 지웠습니다.

## Performance

- 2만 node 흐름 문서(가로 wrap flex 컨테이너 100개 × 자식 199개, release, DEV-MAC M3 Max, 다른 작업 동시 실행): 처음
  변환 약 0.32 s, 자식 하나 너비 편집의 `scene_edits` 중앙값 약 0.34 ms(최대 1.4 ms), Upsert 15건입니다
  (`flow_document_edit_performance_with_20000_nodes`).
- 편집 비용은 다시 배치한 영역과 비켜난 형제 수에 비례합니다. container longhand 변경은 자식 수에 비례합니다.
- 처음 변환에 layout 입력 구성·전체 계산이 더해집니다. 기준 장비(M1 16GB) 재측정이 필요합니다.

## Security

- 새 입력 경로가 없습니다. layout 값은 기존 record·기본값 세트 해석과 엔진 입력 검증을 그대로 거칩니다.

## Migration

- 데이터 migration은 없습니다. record가 없는 문서의 화면은 같습니다.
- 코드: `TextHook`을 구현한 호출자는 줄 너비 인자를 받아 text layout 최대 너비로 씁니다. runtime 흐름 drag 호출자는
  `CanvasLayout::current(window)`를 만들어 넘깁니다.
