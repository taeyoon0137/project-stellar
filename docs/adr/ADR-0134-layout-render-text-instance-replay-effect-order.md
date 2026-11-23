# ADR-0134: layout 결과 그리기 후속 — Text·인스턴스 상자, 재생 layout, 효과 순서, Frame 위치 step 등록 조건

## Status

Accepted (2026-11-24).

- ADR-0125(scene 문서 변환이 layout 결과 위치·크기로 그린다)의 Consequences "남은 일"을 이어 받습니다.
- 계획 행 IX.11.9의 남은 범위, XVI.2.2의 재생 layout 이중 경로, ADR-0130 Consequences의 효과 순서 차이를 다룹니다.
- ADR-0024 결정 8·14, ADR-0074, ADR-0077 결정 4, ADR-0084 결정 9, ADR-0086, ADR-0115 결정 5, ADR-0116 결정 2,
  ADR-0125 결정 1·2·3·9를 따릅니다.

## Context

- runtime의 `CanvasLayout::current`는 컴포넌트 정의 집합과 Text hook 없이 활성 View를 변환했습니다. 그래서 Text 흐름 자식은
  scene에서 빠지고(`text-layout-unavailable`) layout 입력에서 크기가 0이었습니다. ComponentInstance는
  `instance`로 빠져 상자가 없었습니다. 형제 위치와 흐름 drag 삽입 위치가 Text·인스턴스를 세지 못했습니다.
- 흐름 자식 ComponentInstance의 scene key는 Fragment라서 `DocumentSceneSource::world_bounds`가 `None`이었습니다.
- xtask replay의 layout 단계(`tools/xtask/src/replay/layout.rs`)는 scene source 위치를 absolute box로 다시 넣어 별도
  `IncrementalLayout`을 돌렸습니다. 같은 배치를 scene 변환과 재생이 따로 계산한 이중 경로였고, 흐름 배치를 보지
  못했습니다.
- Web 코드 생성(`modifier_plan`)은 Frame에서 Blur 뒤 Shadow를 `filter: blur() drop-shadow()`로 내지만, scene 효과 평가는
  순서를 버렸습니다(ADR-0116 결정 2). 같은 stack을 두 경로가 다르게 읽을 수 있었습니다.
- IX.11.9 step(`frame_position`)은 모든 부모 있는 Frame을 `position: absolute`로 옮겼습니다. 지금 저장 형식에는 layout
  record가 있어(IX.11.6) 흐름 부모의 자식도 있습니다. 이 자식을 absolute로 옮기면 흐름에서 빠져 형제 배치가 바뀝니다.
  `PRODUCT_STEPS` 등록 조건(ADR-0115 결정 5)은 scene 조건만 ADR-0125로 충족됐습니다.

## Decision

1. **캔버스 layout source의 창 자료(`runtime::canvas::layout_text`).**
   - `CanvasLayoutInputs { library, text }`를 `CanvasLayout::with_inputs`·`apply_with`가 받습니다.
     `CanvasLayout::current(window, cache)`·`apply(window, changes, cache)`는 창 cache(`CanvasCache`)의 컴포넌트 정의 집합과
     글꼴로 이 값을 채웁니다(`CanvasCache::with_layout_inputs`).
   - `CanvasText`는 창 글꼴 cache(`FontCache`)로 Text를 배치하는 hook입니다. 측정 hook(`TextLayoutHook`, layout 엔진의
     min-content·줄 배치)과 scene Text hook(줄 너비에서의 Text 입력)이 같은 함수
     `FontCache::layout_input_width`로 같은 문서 Text를 배치하고, 결과를 (node, 줄 너비)로 공유합니다. 그래서 layout이
     잰 크기와 scene이 그리는 크기가 같습니다. 줄 너비는 1/64 px 단위 그대로입니다.
   - min-content 너비는 줄 너비 0으로 배치한 가장 넓은 줄입니다.
   - 측정 hook의 Text stack은 scene Text hook이 받는 stack과 같은 `stellar_scene::document::content_stack`입니다.
   - 캔버스 행 목록(`CanvasScene`)의 Text 배치는 `layout_input`(줄 너비 없음)이고 같은 함수의 줄 너비 `None` 경우입니다.
   - 시험과 다른 호출자는 `CanvasTextHook` trait으로 글꼴 없는 hook을 넘길 수 있습니다.
   - 흐름 drag preview(`reorder_preview_with`)는 변환 때와 같은 창 자료로 다시 배치하고 그립니다. 게시 행 목록 Text
     크기로 만든 source(`for_scene`)의 `reorder_preview`는 같은 Text 크기로 Text를 다시 그립니다. 그렇지 않으면 preview에서
     Text가 크기 0으로 재어지거나 옛 자리에 남습니다.
2. **흐름 자식 ComponentInstance 상자(scene 문서 변환).**
   - Layout 규칙 인스턴스는 정의 root를 layout 결과 위치(저장 배치는 뒤 `translate`)와 **layout 결과 크기**로 그립니다.
     Frame과 같은 규칙입니다(ADR-0125 결정 3). 교차축 `stretch`가 인스턴스 상자를 늘리면 정의 root도 그 크기입니다.
   - Fragment key(문서 Fragment와 인스턴스)의 `world_bounds`는 자식 경계의 합입니다. 인스턴스는 정의 root 하나가 자식이라
     그 상자가 인스턴스 상자입니다. 자식이 없으면 `None`입니다.
   - 인스턴스 가상 subtree 안의 흐름 배치는 여전히 열지 않습니다(ADR-0125 Consequences).
3. **효과 순서 규칙 하나(`stellar_layout::effect_order`).**
   - `effect_slots(효과 목록, boxed)`가 활성 Shadow·Blur의 CSS 자리를 정합니다. 상자 요소(Frame)의 Shadow는 앞에 filter
     효과(Blur·drop-shadow)가 없을 때만 `box-shadow`, 그 밖은 `filter: drop-shadow()`입니다. 상자가 아닌 node의 Shadow는
     언제나 `drop-shadow()`, Blur는 언제나 `filter: blur()`입니다. filter 목록은 stack 순서입니다.
   - Web 코드 생성(`stellar_codegen::modifier_plan`)과 scene 효과 평가(`stellar_scene::document::effects::evaluate_effects`)가
     이 함수를 같이 씁니다. scene 평가 결과 `NodeEffects::order`는 (효과, 자리)를 stack 순서로 싣습니다.
   - 둘이 함께 의존하는 CSS 의미 crate가 layout crate라서 여기에 둡니다(`transform::css_node_transform`과 같은 이유).
4. **IX.11.9 step과 등록 조건.**
   - 흐름 부모(활성 세트로 푼 부모 `display`가 `flex`·`grid`, `commands::arrange::align::flow_parent`)의 자식은
     absolute로 옮기지 않습니다. 저장 transform 이동 성분은 scene 합성에서 이미 CSS `translate`처럼 layout 위치 뒤에
     더해지므로 transform에 그대로 두고 `flow_children`으로 보고합니다. `translate` longhand key가 생기면 그 값으로 옮깁니다.
   - 0x0102를 가리키는 저장 참조는 값 변환 없이 보존하고 `transform_bundle_references`로 나열합니다. View Variant property
     patch 경로는 `stored_reference_paths`가 모읍니다.
   - View root와 ComponentDefinition root의 이동 성분은 캔버스 배치로 보고합니다(`canvas_roots`, `component_root_placement`).
   - **`transform-origin`·캔버스 배치(x·y) longhand는 이 결정에서 key를 할당하지 않습니다.** key 할당(ADR-0024 결정 14)에는
     문서 registry·레코드 codec·텍스트 원본 v4 줄 형식·semantic diff 경로가 함께 바뀌어야 합니다. 텍스트 원본 v4는 다른
     작업이 동시에 바꾸고 있어 이 작업이 줄 형식을 정하지 않습니다. 그 전까지 Frame transform origin은 모델에 고정된
     `0 0`이고(`frame_metadata` 매핑 note) scene 합성 origin도 `0 0`입니다. migration이 `transform-origin: 0 0`을
     명시 저장하는 일은 key가 생긴 뒤입니다.
   - **`PRODUCT_STEPS` 등록은 하지 않습니다.** ADR-0115 결정 5의 두 번째 조건(Frame payload 4변 테두리 타입, layout 입력의
     테두리 두께, IX.11.10)이 남았습니다. 테두리 step은 테두리 있는 부모의 Free 자식 inset에서 두께를 빼는데, scene·layout이
     아직 테두리로 padding box를 줄이지 않아 켜면 그 자식이 두께만큼 옮겨져 그려집니다. 테스트가 통과해도 제품 결과가
     틀리므로 켜지 않습니다. 조건이 채워지면 세 step(위치·테두리·Text style)을 한 1→2 step으로 묶어 등록하고
     `PRODUCT_SCHEMA_VERSION`을 2로 올립니다(ADR-0077 결정 4).
5. **재생 layout은 문서 layout 결과 하나(XVI.2.2).**
   - `DocumentLayout`은 마지막 갱신(`update`·`rebuild`)에서 위치·크기가 바뀐 상자를 `rect_changes`로 남기고,
     `rect_invalidations`가 그것을 `SceneInvalidation::LayoutRect`로 냅니다.
   - 재생은 `scene_edits`가 같은 change set으로 갱신한 layout의 이 무효화를 씁니다. cold 측정은
     `DocumentSceneSource::rebuild_layout`으로 처음부터 다시 배치합니다. 재생 전용 layout 입력 변환은 지웠습니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| runtime에 Text 측정 전용 함수를 따로 둠(줄바꿈만 계산) | 채택하지 않음. 그리기와 측정이 다른 함수가 되어 크기가 어긋날 수 있습니다. 같은 `layout_input_width`를 씁니다 |
| min-content를 단어 분리로 따로 셈 | 채택하지 않음. 줄바꿈 기회 규칙(UAX #14)을 text 엔진과 따로 구현하게 됩니다 |
| 인스턴스는 늘 정의 root 크기로 그림(stretch 무시) | 채택하지 않음. Frame과 다른 규칙이 되고 layout 상자와 그린 상자가 달라집니다 |
| 효과 자리 규칙을 scene과 codegen에 각각 둠 | 채택하지 않음. 이중 경로입니다 |
| 효과 순서를 scene `PaintStack`·snapshot codec에 바로 실음 | 이번에는 채택하지 않음. scene canonical codec·semantic hash golden과 renderer pass compiler가 함께 바뀌는데, 그 renderer 경로는 다른 작업이 동시에 바꾸고 있습니다. 규칙과 평가 결과(`NodeEffects::order`)를 먼저 하나로 두고 renderer 연결은 후속입니다 |
| 흐름 부모 자식도 absolute로 옮김 | 채택하지 않음. 흐름에서 빠져 형제 배치와 부모 내용 크기가 바뀝니다 |
| `transform-origin`·캔버스 배치 key를 지금 할당 | 채택하지 않음. 텍스트 원본 줄 형식 없이 key만 두면 저장되지 않는 속성이 됩니다 |
| 테두리 조건 전에 1→2 step 등록 | 채택하지 않음(결정 4) |
| 재생이 layout 전체를 매 event 복제해 비교 | 채택하지 않음. 문서 크기에 비례하는 비용입니다. 갱신이 바뀐 상자를 직접 남깁니다 |

## Consequences

- runtime 캔버스 기능(흐름 drag 삽입 위치·preview, inset 이동)이 `CanvasLayout::current(window, cache)`로 Text와
  인스턴스를 세는 상자를 읽습니다. 제품 화면 경로는 여전히 옛 캔버스 행 목록이며 UI 재설계 때 이 source를 씁니다
  (ADR-0125 결정 9). 생성 도구의 흐름 삽입(`flow_insert_key`)은 계속 게시한 행 목록 Text 크기(`for_scene`)를 씁니다.
- 흐름 자식 인스턴스가 world 상자를 가집니다. 흐름 drag·삽입이 인스턴스를 형제로 셉니다.
- 재생 layout 단계가 scene과 같은 흐름 배치를 씁니다. T3 측정 구간의 layout 시간은 `scene_edits` 안에 포함됩니다.
- 남은 일:
  - renderer가 `NodeEffects::order`를 받아 Frame의 Blur 뒤 Shadow를 내용 alpha `drop-shadow()`로 그리기(scene
    `PaintStack`·codec과 renderer pass compiler `ShadowShape`). ADR-0130 Consequences의 차이는 그때 닫힙니다.
  - IX.11.9: `transform-origin`·캔버스 배치 longhand key와 텍스트 원본 줄 형식, `translate` longhand, 제품 step 등록(결정 4
    조건), 인스펙터 위치 경로.
  - 글꼴을 읽는 중에 만든 `CanvasLayout`은 기본 chain으로 잰 결과를 증분 layout cache에 둡니다. 글꼴을 다 읽으면 호출자가
    `CanvasLayout`을 다시 만듭니다(창 cache가 게시 결과를 버리는 시점, `CanvasCache::fonts_loaded`).

## Compatibility

- 저장 형식·command codec·FFI 변경은 없습니다.
- 공개 API:
  - runtime: `CanvasLayout::current`·`apply`가 창 cache를 받습니다. `with_inputs`·`apply_with`, `layout_text` 모듈
    (`CanvasText`, `CanvasTextHook`, `CanvasLayoutInputs`)이 생겼습니다. `CanvasCache`가 `pub`입니다.
  - scene: `content_stack`이 공개됐습니다. `evaluate_effects`가 `boxed` 인자를 받고 `NodeEffects::order`·
    `has_drop_shadow`가 생겼습니다. `DocumentLayout::rect_changes`·`rect_invalidations`·`rebuild`,
    `LayoutRectChange`, `DocumentSceneSource::rebuild_layout`이 생겼습니다. Fragment의 `world_bounds`가 `None` 대신 자식
    경계의 합입니다.
  - layout: `effect_order` 모듈.
  - project: `FramePositionReport::flow_children`, `transform_bundle_references`, `stored_reference_paths`,
    `component_root_placement`.
- 흐름 부모 자식이 있는 문서의 IX.11.9 step 결과가 바뀝니다(이전에는 absolute로 옮김). step은 아직 등록하지 않아 제품
  문서에 영향이 없습니다.

## Performance

- Text 측정은 (node, 줄 너비)마다 한 번 배치합니다. flex 축소처럼 같은 Text를 여러 너비로 재면 그 수만큼 배치합니다.
  `CanvasText`는 layout 계산 한 번 동안만 결과를 공유하고, 증분 layout의 측정 cache가 계산 사이의 재사용을 맡습니다.
- 재생: 재생 전용 layout 입력 구성과 두 번째 증분 layout을 지워 T3 비용이 줄어듭니다. 다시 재지 않았습니다.

## Security

- 새 입력 경로가 없습니다. Text 배치는 기존 창 글꼴 cache와 문서 style 표 규칙을 그대로 씁니다.

## Migration

- 데이터 migration은 없습니다.
- 코드: `CanvasLayout::current(window)` 호출자는 창 cache(`&mut CanvasCache`)를 넘깁니다. `evaluate_effects(stack)` 호출자는
  상자 요소 여부를 넘깁니다.
