# 작업 보고서: XI.8.3 vertex/index/uniform buffer의 Metal resource 대응

- WBS: XI.8.3
- 작업명: 공통 vertex/index/uniform buffer abstraction을 Metal resource로 매핑
- 상태: 완료 (`[x]`)
- 선행: XI.5.3 (`[x]`), XI.8.1 (이 lane에서 `df9a2d2`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max, 실제 GPU offscreen render-to-texture pixel readback).

## 변경 파일 목록

- `crates/renderer-metal/src/buffer.rs`(신규) — `MetalBuffer`(shared `MTLBuffer`, 범위 검사 쓰기, index
  범위 조회), texture upload staging(`stage_texture_upload`)과 blit 제출(`submit_uploads`).
- `crates/renderer-metal/src/shaders.rs`(신규) — shader ABI(buffer slot, `DrawUniforms` 32 byte,
  `SolidFill` vertex 24 byte, offset 정렬)와 내장 MSL 소스(`SolidFill`).
- `crates/renderer-metal/src/pipeline.rs`(신규) — 내장 library 지연 compile, `PipelineKey`별 render
  pipeline state cache(Replace·PremultipliedOver blend).
- `crates/renderer-metal/src/commands.rs`(신규) — `DrawCommand` 상태 기계 검사(`plan_commands`)와
  encoder 기록(`record`).
- `crates/renderer-metal/src/encode.rs` — pass 계획에 draw command 계획 포함, encoder에 기록.
- `crates/renderer-metal/src/backend.rs` — `create_buffer`·`update_buffer`·`destroy_buffer`·
  `update_texture` 구현, upload command buffer flush(encode·readback·`wait_idle` 전), submission 번호
  없는 upload command buffer 추적, 재 init 시 buffer table·pipeline cache·대기 upload 초기화.
- `crates/renderer-metal/src/sys.rs` — `MTLScissorRect`, primitive·index type 상수.
- `crates/renderer-metal/src/objc.rs` — `ns_string`을 제품 코드에서 사용(shader 소스).
- `crates/renderer-metal/src/lib.rs` — 모듈 등록.
- `crates/renderer-metal/src/tests/geometry.rs`(신규), `tests/mod.rs` — XI.8.3 테스트.
- `docs/task-reports/XI.8.3.md`(신규), 계획서(XI.8.3 `[ ]`→`[x]`).

## 구현 요약

- **buffer 대응**: `BufferUsage`(Vertex·Index·Uniform·Staging) 모두 `MTLStorageModeShared` buffer 하나로
  대응합니다(Apple Silicon 통합 메모리). `update_buffer`는 범위 검사 후 CPU 즉시 쓰기이고, GPU가 아직
  읽는 범위를 덮지 않는 것은 공통 계층 frame-local ring(XI.5.3)의 책임입니다. 0 크기는 `InvalidUsage`,
  `maxBufferLength` 초과·할당 실패는 `OutOfDeviceMemory`입니다.
- **texture upload**: texture는 private storage이므로 `update_texture`는 영역(mip·범위·data 길이)을 검사하고
  data를 staging buffer로 복사해 쌓습니다. 다음 `encode`(skip 여부와 무관)·`read_texture`·`wait_idle`
  시작 때 blit command buffer 하나로 commit하므로 queue 순서상 다음 frame보다 먼저 실행됩니다. upload
  command buffer도 `poll`에서 상태를 확인합니다(submission 번호는 없음).
- **shader ABI**: buffer 0 = vertex 배열, buffer 1 = `DrawUniforms { float4 tint; float depth; pad[3] }`
  (vertex·fragment), buffer 2 = backend가 pass마다 넣는 target pixel 크기. 좌표는 attachment pixel
  (좌상단 원점), 색은 premultiplied 선형 값입니다. 이 작업은 `SolidFill`만 구현합니다.
- **draw 검사**(encoder 열기 전): pipeline format·sample 수가 pass와 같아야 하고, draw 전에
  pipeline·vertex·uniform(indexed는 index buffer)이 있어야 합니다. vertex 범위를 stride 기준 buffer
  용량과 비교하고, indexed draw는 index 범위를 index buffer와 비교한 뒤 그 범위 index를 CPU에서 읽어
  `base_vertex + index`가 vertex 용량 안인지 확인합니다. buffer 용도·offset 정렬(vertex·index 4,
  uniform 16 byte)·uniform 크기(32 byte 이상)도 검사합니다. scissor는 attachment로 자르고 빈
  scissor면 draw를 건너뜁니다. 거부한 frame은 command buffer를 만들지 않습니다.
- **pipeline**: 내장 MSL library는 처음 draw가 있을 때 한 번 compile하고, pipeline state는
  `PipelineKey`마다 한 번 만들어 보관합니다(depth·stencil key와 다른 program은 XI.8.4).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal` | 23 passed, 0 failed |
| `cargo clippy -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-metal -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| vertex·uniform buffer로 그린 삼각형 (8,1)·(15,15)·(1,15)를 16×16 RGBA8에 readback: edge에서 0.75 px 이상 안쪽 pixel은 모두 green, 0.75 px 이상 바깥은 모두 black(`mock_geometry_triangle_renders_inside_pixels_only`) | 통과 |
| index buffer + `base_vertex = 2` quad가 [4,12)² 안만 정확히 red, uniform offset 32의 tint 0.5 결과 (127\~128, 0, 0, 127\~128), 같은 key pipeline 1개만 생성(`indexed_quad_with_base_vertex_and_uniform_tint`) | 통과 |
| scissor 사각형 안만 그림, attachment 밖으로 넘친 scissor는 잘림, 빈 scissor는 draw 생략, `None`은 전체(`scissor_clamps_to_attachment_and_empty_scissor_skips_draws`) | 통과 |
| PremultipliedOver: blue 위 (0.5,0,0,0.5) → (127\~128, 0, 127\~128, 255)(`premultiplied_over_blends_with_destination`) | 통과 |
| texture 전체·부분 upload 후 readback byte 일치, upload 뒤 `LoadOp::Load` frame이 upload 내용 위에 그림(`texture_upload_is_queue_ordered_before_the_next_frame`) | 통과 |
| 없는 mip·범위 밖·0 크기·overflow·길이 불일치 upload 거부(`texture_upload_regions_are_validated`) | 통과 |
| buffer 범위 쓰기·범위 밖 거부, 0 크기 거부, `u64::MAX` 크기 `OutOfDeviceMemory`, 파괴 후 stale(`buffers_map_usage_bounds_and_lifetime`) | 통과 |
| pipeline·uniform 누락, vertex·index 범위 밖, 음수 base vertex, 잘못된 용도·정렬·크기, format 불일치 12종이 모두 오류이고 frame 미개방·GPU 미제출, 이후 정상 frame 렌더(`invalid_draw_commands_never_reach_the_gpu`) | 통과 |

## 차이(작업 지시와 다른 점)

- "mock geometry"는 공통 계층 tessellation 결과 대신 테스트가 직접 만든 vertex·index byte입니다.
  crate graph상 scene 타입(`Vec2`)을 쓸 수 없어 `FillMesh`를 테스트에서 만들 수 없기 때문입니다.
  공통 계층 mesh → vertex byte 변환은 이 작업 범위 밖입니다.
- triangle 판정은 edge에서 0.75 px 이상 떨어진 pixel만 엄격 비교합니다. 경계 pixel의 rasterization
  규칙(top-left rule) 비교는 XI.11.2 허용 오차 정책 범위입니다.

## 남은 위험 / 한계

- indexed draw의 index 검사는 index 수에 비례하는 CPU 비용입니다. 2만 node benchmark(XI.11.4)에서 encode
  예산(§6.2 0.75 ms)에 영향이 있으면 공통 계층이 index 상한을 함께 넘기는 방식으로 바꿔야 합니다.
- uniform offset 정렬 16 byte는 Apple GPU 요구(4 byte)보다 보수적인 이 backend의 계약입니다. 공통
  계층 frame-local allocator(XI.5.3)는 이 정렬로 할당해야 합니다.
- `update_buffer`는 CPU 즉시 쓰기라 GPU가 읽는 중인 범위를 덮으면 결과가 섞입니다(호출 시점 계약은
  공통 계층 책임).
