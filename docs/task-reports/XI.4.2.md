# 작업 보고서: XI.4.2 scene primitive opaque/transparent/effect pass compiler

- WBS: XI.4.2
- 작업명: scene primitive를 opaque/transparent/effect pass로 분류하는 compiler
- 상태: 완료 (`[x]`)
- 선행: XI.2.2 (`[x]`), XI.4.1 (이 lane에서 `290e98f`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/pass_compiler.rs`(신규) — `compile_frame`, `CompileInput`, `FramePlan`,
  `PassPlan`, `PlannedDraw`, `DrawSource`, `EffectLayer`, `LayerId`, `PassClass`, `PrimitiveClass`,
  `EffectBoundaries`, `FrameCompileError`, `classify_content`, `has_compositing_effect`,
  `draws_content`, `clips_are_pixel_aligned_rects`, 상수(`LAYER_FORMAT`, `DEPTH_FORMAT`, `PIXEL_EPSILON`).
- `crates/renderer-core/src/pass_compiler/tests.rs`(신규) — ordering fixture, painter 의미 coarse raster
  모사(`sim`), 무작위 scene 생성기.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.4.2.md`(신규), 계획서(XI.4.2 `[ ]`→`[x]`).

## 구현 요약

- **분류**:
  - effect: 합성 불투명도 < 1, Normal이 아닌 blend, layer blur > 0인 node는 offscreen layer 경계입니다.
    node 자기 내용과 자손을 layer texture(`Rgba16Float`)에 painter 순서로 그리는 effect pass를 만들고,
    부모 target에는 node z 위치의 layer 합성 draw 하나만 둡니다. 이 작업은 효과가 있는 모든 node를
    경계로 두는 `EffectBoundaries::Conservative`만 제공하며, 최소 경계는 XI.4.4에서 추가합니다.
  - opaque: 모서리 없는 사각형, 최상위 paint layer가 alpha 1 fill, stroke·shadow·합성 효과 없음,
    world→device 변환이 축 정렬이고 네 변이 device pixel 경계(허용 오차 1/256 px), clip은 같은 조건의
    사각형뿐인 경우입니다. 가장자리 AA가 없어 덮어쓰기와 depth write가 정확합니다.
  - transparent: 나머지(반투명·모서리·회전·소수 pixel·stroke·shadow·text·image·layer 합성).
- **순서(painter 의미 보존)**: main target draw마다 paint 순위 `depth`(1부터)를 줍니다. pass 실행 순서는
  effect pass(안쪽 layer 먼저) → opaque pass(surface·depth clear, 앞→뒤, depth test `>` + write) →
  transparent pass(surface Load, 뒤→앞, depth test만)입니다. 각 pixel의 가장 위 opaque보다 아래 draw는
  test에서 빠지고 위 draw는 순서대로 쌓이므로 전체 결과는 painter 순서와 같습니다. 빈 transparent
  pass는 만들지 않고, 그릴 것이 없는 layer도 만들지 않습니다.
- **render graph**: XI.4.1 builder로 surface import, main depth-stencil transient, layer transient를
  선언하고 layer 합성 draw가 있는 pass가 그 layer version을 읽게 해 실행 순서를 graph 의존으로
  보장합니다. 결과는 검증된 `CompiledRenderGraph`입니다.
- **가시성**: `DrawList`(XI.3.2)를 주면 그 안 node만 그리되, 화면 밖 effect node라도 화면 안 자손이
  있으면 layer 경계는 유지합니다.
- **안전성**: snapshot 순회와 layer scope 관리는 명시적 stack이라 깊은 tree에서도 재귀하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 78 passed, 0 failed; doctest 3 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| ordering fixture 12 node(불투명·반투명·effect group·모서리·text·소수 위치·숨김·stroke·shadow): pass 순서 effect → opaque → transparent, 분류와 depth 순위, opaque 앞→뒤·transparent 뒤→앞, layer 합성 위치, draw 수, graph output이 기대값과 같고 painter 모사와 일치(`ordering_fixture_classifies_and_orders_passes`) | 통과 |
| opaque 판정: 정렬 사각형·정렬 clip은 opaque, 소수 위치·회전·소수 clip·alpha 0.99는 transparent, zoom 2배·1.5배·소수 pan에 따른 device 정렬 판정(`opaque_requires_pixel_aligned_device_rect`) | 통과 |
| 무작위 scene 400개(중첩·opacity·blend·blur·모서리·stroke·shadow·text·소수 위치·숨김): plan을 pass 순서·depth 규칙대로 실행한 결과가 tree painter 기준과 모든 cell에서 같음, layer 있는 scene > 100, opaque draw 있는 scene > 100(`compiled_plans_preserve_painter_order_on_random_scenes`) | 통과 |
| 위 비교의 검출력: layer scope를 하나 일찍 닫는 compiler 변경이 이 테스트에서 실패함을 임시 변경으로 확인(변경은 되돌림) | 확인 |
| 가시 목록: 화면 밖 draw 제외, 화면 밖 effect node의 화면 안 자손은 layer로 합성, 그릴 것 없는 layer 미생성, 빈 frame은 clear pass 하나(`visibility_list_filters_draws_but_keeps_layer_boundaries`) | 통과 |
| 600단 중첩(300 effect) 재귀 없이 compile, 안쪽 layer pass가 바깥보다 먼저(`deep_nested_effects_compile_without_recursion`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XI.2.2는 `[x]`, XI.4.1은 이 lane에서 완료했습니다. 완화 적용은 없습니다.
- opaque 판정에 device pixel 정렬을 넣었습니다. 가장자리 AA가 있는 사각형을 opaque pass(덮어쓰기 +
  depth write)에 두면 뒤 내용이 AA 가장자리로 비치지 않아 결과가 painter와 달라지기 때문입니다.
- shadow는 node 내용의 일부(transparent)로 두었습니다. blur shadow의 offscreen 처리는 XI.10.4 범위입니다.
- renderer-core 테스트는 core-types 타입을 이름으로 만들 수 없어 viewport 변환을 fixture node의 world
  transform에서 얻습니다(`viewport_with`).

## 남은 위험 / 한계

- 모사는 coverage를 content 사각형 cell로 근사하고 clip·blur 번짐은 모델링하지 않습니다. 실제 pixel
  결과는 렌더 적합성 테스트(XI.11)가 검증합니다.
- 한 node의 fill·stroke·shadow는 draw 하나로 묶었습니다. layer별 draw 분할·batching은 XI.4.5에서 다룹니다.
- clip 적용 방식(scissor/stencil/mask)은 XI.4.3이 정하며, 이 compiler의 opaque 판정은 그 표의 scissor
  조건과 같은 기준을 씁니다.
