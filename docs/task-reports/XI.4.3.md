# 작업 보고서: XI.4.3 clip stack scissor/stencil/offscreen mask decision table

- WBS: XI.4.3
- 작업명: clip stack을 scissor, stencil, offscreen mask 중 하나로 계획하는 backend-neutral decision table
- 상태: 완료 (`[x]`)
- 선행: XI.4.1 (이 lane에서 `290e98f`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/clip_plan.rs`(신규) — `ClipPlan`, `ClipCapabilities`, `ClipAnalysis`,
  `ComplexClip`, `ComplexReason`, `DeviceRect`, `analyze_clip`, `decide_clip`, `plan_clip`,
  `axis_aligned_rect`, `surface_extent`, `PIXEL_EPSILON`.
- `crates/renderer-core/src/pass_compiler.rs` — `CompileInput::clip_caps`, `PlannedDraw::clip`, draw마다
  clip plan 계산·`Culled` draw 제거, layer 안 draw는 경계 안쪽 clip만, layer 합성 draw는 바깥 clip,
  opaque 판정을 clip 분석(`is_scissor_exact`)과 축 보존 사각형 판정으로 교체(XI.4.2의
  `clips_are_pixel_aligned_rects`·`PIXEL_EPSILON` 대체). `classify_content`에 clip 시작 depth 인자 추가.
- `crates/renderer-core/src/pass_compiler/tests.rs` — 새 입력 필드, 90° 회전·임의 회전·둥근 clip 판정.
- `crates/renderer-core/src/test_scene.rs` — path clip fixture(`path_clip`).
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.4.3.md`(신규), 계획서(XI.4.3 `[ ]`→`[x]`).

## 구현 요약

- **분석(capability 무관)**: clip마다 world→device 변환을 봅니다. 모서리 없는 사각형이 축 보존 변환
  (이동·축척·반전·90° 회전)으로 놓이고 네 변이 device pixel 경계(1/256 px 허용)에 있으면 scissor
  가능, 아니면 복합 clip(`RoundedCorners`/`NotAxisAligned`/`FractionalEdges`/`Path`)입니다. 모든 clip의
  device AABB 교집합이 bounds입니다.
- **decision table**:

  | 조건 | 결과 |
  |---|---|
  | 고려할 clip 없음 | `None` |
  | bounds 교집합이 비었거나 surface 밖 | `Culled` |
  | 복합 clip 없음 | `Scissor`(교집합 그대로, 정확) |
  | 복합 clip 있음, 4x MSAA, stencil bit > 0, 복합 수 ≤ 2^bit − 1 | `Stencil { scissor, clips, reference }` |
  | 그 밖 | `Mask { scissor, clips }`(R8 coverage mask) |

  stencil은 sample 단위 coverage라 MSAA가 있어야 AA가 생깁니다. 단순 축 정렬 사각형 clip은 capability와
  무관하게 항상 scissor입니다. scissor는 bounds를 바깥쪽으로 정수화하고 surface로 자릅니다.
- **compiler 연동**: draw마다 plan을 붙이고 `Culled` draw는 만들지 않습니다. offscreen layer 안 draw는
  경계 node 안쪽 clip만(`from_depth` = 경계 clip depth), 바깥 clip은 layer 합성 draw가 담습니다(blur 등
  효과 뒤에 바깥 clip 적용). 합성이 culled인 layer는 만들지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 85 passed, 0 failed; doctest 3 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| 단순 사각형 clip은 MSAA 유무와 무관하게 scissor, 중첩 사각형 교집합, zoom·pan·90° 회전 scissor, clip 없음 `None`(`simple_rect_clip_selects_scissor`) | 통과 |
| 교집합이 빈 clip·surface 밖 clip `Culled`, surface 경계로 자름(`empty_or_offscreen_clips_cull`) | 통과 |
| 둥근·소수 가장자리·임의 회전·path clip: 이유 분류, MSAA면 stencil(reference 1), 아니면 같은 scissor의 mask(`complex_clips_use_stencil_with_msaa_and_mask_otherwise`) | 통과 |
| 정렬 사각형 + 둥근 clip 결합 bounds, 소수 bound 바깥 정수화, stencil 깊이 부족·stencil 없음이면 mask(`scissor_and_complex_clips_combine`) | 통과 |
| `from_depth`로 바깥 clip 건너뜀(`from_depth_skips_outer_clips`), device capability 반영(`capabilities_follow_device`) | 통과 |
| compile된 draw의 clip: scissor, 빈 clip draw 제거, layer 안 자식은 안쪽 clip만, layer 합성은 바깥 둥근 clip stencil, culled 합성 layer 미생성(`compiled_draws_carry_clip_plans`) | 통과 |
| XI.4.2 painter 모사 400 scene 회귀(`compiled_plans_preserve_painter_order_on_random_scenes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XI.4.1은 이 lane에서 완료했습니다(명시 row-ID). 완화 적용은 없습니다.
- XI.4.2 compiler에 clip plan을 연결하면서 opaque 판정 helper를 clip 분석으로 대체했습니다. 그 결과 90°
  회전한 정렬 사각형도 opaque가 됩니다(축 보존·pixel 정렬이면 AA 가장자리가 없음).
- 모서리 둥근 사각형의 analytic(SDF) clip은 decision table 세 가지 방식 밖이라 넣지 않았습니다. 성능상
  필요하면 XI.10.4에서 별도 방식으로 추가할 수 있습니다.

## 남은 위험 / 한계

- stencil·mask에 clip 도형을 실제로 그리는 command 생성(stencil 증가·감소 draw, mask pass)은 backend
  encode(XI.8.5)와 XI.10.4에서 구현합니다. 이 작업은 결정과 그 입력(scissor·clip index·reference)까지입니다.
- path clip bounds는 control point 껍질(보수적)이라 scissor가 실제 clip보다 넓을 수 있습니다(정확성 문제는
  아니고 효율 문제).
