# 작업 보고서: XI.8.5 RenderGraph pass encoding·submit·present 예약·CPU timestamp

- WBS: XI.8.5
- 작업명: RenderGraph pass encoding, submit, `present` 예약, CPU timestamp 구현
- 상태: 완료 (`[x]`)
- 선행: XI.4.6 (`[x]`), XI.8.4 (이 lane에서 `ac3db02`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max, 실제 GPU offscreen render-to-texture·창 밖 CAMetalLayer readback).

## 변경 파일 목록

- `crates/renderer-metal/src/transient.rs`(신규) — graph transient alias slot별 texture pool(재사용,
  descriptor 변경 시 재할당, purge).
- `crates/renderer-metal/src/trace.rs`(신규) — 공개 `MetalFrameTrace`(encode 시작·끝, present 예약,
  submit = T5, pass·draw 수, 고정 형식 line), `MAX_FRAME_TRACES`.
- `crates/renderer-metal/src/encode.rs` — transient 해석, depth·stencil attachment(depth·stencil 양쪽에
  같은 texture, clear depth·stencil), pass가 읽는 graph version의 sampling texture 해석(같은 pass
  attachment는 feedback loop라 불가), attachment sample 수 일치 검사.
- `crates/renderer-metal/src/commands.rs` — `TextureSource::Graph` sampling(pass `reads`에 선언된
  version만), draw 수 집계.
- `crates/renderer-metal/src/texture.rs` — transient용 `render_target_desc`.
- `crates/renderer-metal/src/backend.rs` — encode 전 transient 준비, frame CPU timestamp 기록
  (`set_trace_clock`, `drain_frame_traces`), `transient_texture_count`, 재 init 시 pool purge.
- `crates/renderer-metal/src/lib.rs` — 모듈 등록, `MetalFrameTrace`·`MAX_FRAME_TRACES` 공개.
- `crates/renderer-metal/src/tests/graph.rs`(신규), `tests/geometry.rs`(다중 pass 도우미),
  `tests/shaders.rs`(도우미 공유, graph source 기대값 갱신), `tests/mod.rs`.
- `docs/task-reports/XI.8.5.md`(신규), 계획서(XI.8.5 `[ ]`→`[x]`).

## 구현 요약

- **pass encoding**: compile된 실행 순서대로 raster pass마다 render command encoder 하나를 만듭니다.
  같은 command buffer 안 encoder 순서와 Metal hazard tracking이 graph의 RAW·WAW·WAR 순서를 지킵니다.
  resource는 surface(drawable), external(import binding texture), transient(alias slot pool)로
  해석합니다. compute·transfer pass는 대응 command가 없어 `Unsupported`입니다.
- **transient pool**: 수명이 겹치지 않고 descriptor가 같은 transient는 compile이 준 alias slot 하나의
  texture를 공유합니다. slot descriptor(format, Surface 크기 해석, sample 수)가 바뀌면(resize) 다시
  만들고, `Unused` transient는 할당하지 않습니다.
- **depth·stencil**: `Depth32Float_Stencil8` texture를 depth·stencil attachment 양쪽에 붙이고, 그 pass의
  pipeline은 depth format을 선언하며 depth-stencil state를 함께 bind합니다(XI.8.4 cache).
- **graph sampling**: `TextureSource::Graph(version)`은 그 pass가 `reads`로 선언한 version만 허용하고,
  sampling 가능한(single sample color, sampled 용도) texture여야 합니다. drawable은 framebuffer only라
  sampling할 수 없습니다.
- **submit·present 예약·CPU timestamp**: encode 시작·끝, `schedule_present`(`presentDrawable:`), `submit`
  (`commit` 반환) 시각을 `TraceClock`으로 기록합니다. present를 commit 전에 command buffer에 붙이므로
  commit 반환 시각이 아키텍처 §6.1의 T5입니다(`t5_nanos`). 시계는 공통 계층 T0~T4와 같은 기준점으로
  바꿀 수 있고, trace는 최대 64개 보관합니다. skip한 frame은 trace가 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal` | 37 passed, 0 failed |
| `cargo clippy -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-metal -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| T5 trace: 10 ns 단위 시계로 surface frame encode 10·20, present 30, submit 40, golden line 일치, skip frame은 trace 없음(`frame_trace_has_submit_and_present_timestamps`) | 통과 |
| 실제 시계: 공통 계층 T4 ≤ encode 시작 ≤ encode 끝 ≤ T5, offscreen은 present 없음, pass 1·draw 1, 70 frame 뒤 최근 64개만 보관(`real_clock_orders_t4_encode_present_and_t5`) | 통과 |
| 선언 순서와 반대인 2-pass graph: transient layer(red + green quad)를 다음 pass가 `LayerComposite`로 sampling → 열별 blue·green·red readback 일치, transient 1개(`transient_layer_pass_feeds_composite_pass`) | 통과 |
| depth attachment: 0.2 red와 0.8 green을 두 순서로 그려도 모두 red(`depth_attachment_keeps_the_nearest_draw`) | 통과 |
| stencil clip: ClipStencil Increment → TestEqual green → Decrement(reference + 1 보정) → TestEqual red 결과 clip 안쪽만 green, reference 255 감소는 `InvalidUsage`(`stencil_clip_restricts_drawing_to_the_clip_shape`) | 통과 |
| 수명이 겹치지 않는 transient 2개가 alias slot 1개 공유, 8→8→16→4 resize마다 Surface 크기로 재할당되고 readback 정상(`transient_pool_aliases_slots_and_follows_surface_resize`) | 통과 |
| transfer pass `Unsupported`(`compute_and_transfer_passes_are_unsupported`), trace line 형식(`trace::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- T0~T3는 입력·document 계층 측정이라 이 crate에서 만들 수 없습니다. backend는 T5(와 encode·present
  시각)를 기록하고, T4는 같은 시계를 공유해 비교할 수 있게 했습니다. 전체 T0~T5 trace 조립은
  runtime(XII) 범위입니다.
- compute·transfer pass(blur·copy)는 공통 `DrawCommand`에 대응 command가 없어 `Unsupported`로 둡니다
  (XI.10.4에서 추가).
- `RenderGraphCompileTrace`(XI.4.6)와 실제 연결은 scene snapshot이 필요해 crate graph상 이 crate
  테스트에서 만들 수 없습니다. 같은 `TraceClock`을 쓰는 순서 검증으로 대신했습니다.

## 남은 위험 / 한계

- transient pool은 slot texture를 frame 사이에 유지합니다. 큰 offscreen layer가 많은 scene의 메모리
  상한·축소 정책은 XI.8.7(memory pressure)과 XI.11에서 다룹니다.
- multisample attachment의 resolve는 graph 모델에 없어 지원하지 않습니다(4x MSAA texture 생성만 가능).
- color attachment 없는 depth 전용 pass의 draw는 `Unsupported`입니다.
