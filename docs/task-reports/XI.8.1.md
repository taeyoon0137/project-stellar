# 작업 보고서: XI.8.1 Metal device·queue·CAMetalLayer 초기화

- WBS: XI.8.1
- 작업명: `renderer-metal`에서 MTLDevice, command queue, CAMetalLayer 연결과 pixel format 초기화
- 상태: 완료 (`[x]`)
- 선행: XI.5.1 (`[x]`), II.5.4 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max, macOS 27.2, 실제 GPU에서 offscreen render-to-texture와 창 밖 CAMetalLayer
  drawable pixel readback).

## 변경 파일 목록

- `crates/renderer-metal/src/objc.rs`(신규) — 최소 Objective-C runtime FFI: typed `objc_msgSend`
  wrapper(`send`/`MsgArgs`), 정적 cache `sel!`/`class!`, retain·release 소유 `Id`, `autoreleasepool`,
  `NSString` 변환(테스트용).
- `crates/renderer-metal/src/sys.rs`(신규) — Metal·QuartzCore·CoreGraphics C 선언, SDK enum 상수,
  `CGSize`·`MTLClearColor`·`MTLOrigin`·`MTLSize`.
- `crates/renderer-metal/src/format.rs`(신규) — `TextureFormat` → `MTLPixelFormat`, pixel byte 수,
  CAMetalLayer 허용 format.
- `crates/renderer-metal/src/device.rs`(신규) — `MetalDevice`(system default device + command queue),
  capability 조회.
- `crates/renderer-metal/src/surface.rs`(신규) — `LayerOptions`, `LayerSurface`(CAMetalLayer 검사·
  retain, device·pixel format·colorspace·drawable 크기 설정, drawable 획득).
- `crates/renderer-metal/src/texture.rs`(신규) — texture descriptor 검증과 private texture 생성.
- `crates/renderer-metal/src/encode.rs`(신규) — graph attachment 해석(surface drawable·external
  texture)과 clear/load/store render pass encoding(계획 → 기록 2단계).
- `crates/renderer-metal/src/backend.rs`(신규) — `MetalBackend`(`headless`, `with_layer`,
  `with_detached_layer`, `wait_idle`, `read_texture`)와 `RendererBackend` 구현.
- `crates/renderer-metal/src/tests/mod.rs`, `tests/smoke.rs`(신규, `#[cfg(test)]`) — GPU 테스트 도우미와
  smoke 테스트.
- `crates/renderer-metal/src/lib.rs` — 모듈 등록, `MetalBackend`·`LayerOptions` 공개.
- `docs/task-reports/XI.8.1.md`(신규), 계획서(XI.8.1 `[ ]`→`[x]`).

## 구현 요약

- **binding 방식**: 외부 crate를 추가하지 않고 `libobjc`·Metal·QuartzCore·CoreGraphics를 직접
  선언했습니다(`project` crate의 CoreText·FSEvents adapter와 같은 방식). 따라서
  `config/dependencies.yaml`·ADR 변경은 없습니다. arm64 전용(ADR-0006)이라 `objc_msgSend_stret`이
  필요 없습니다. 모든 Objective-C 호출은 crate 내부 typed wrapper에서만 하고 `SAFETY` 주석에 selector
  시그니처를 적었습니다.
- **device·queue**: `MTLCreateSystemDefaultDevice`(없으면 `Unsupported`)와 `newCommandQueue`. capability는
  Metal kind, 최대 texture 16384, 4x MSAA(`supportsTextureSampleCount:`), stencil 8 bit,
  `display_p3_surface`는 layer가 있을 때만 참, timestamp는 XI.8.6 전까지 거짓.
- **CAMetalLayer**: `with_layer`는 `isKindOfClass:`로 CAMetalLayer인지 확인하고 retain합니다. `init`에서
  명시적 `CATransaction`(animation 끔) 안에 device, pixel format, colorspace(색공간 tag 0x01 sRGB /
  0x02 Display P3, RGBA16Float는 extended linear 변형), `framebufferOnly`, `displaySyncEnabled`,
  `maximumDrawableCount`(2..=3 검사), `allowsNextDrawableTimeout = YES`, drawable 크기를 설정합니다.
  CAMetalLayer가 받지 않는 format(RGBA8 등)은 Objective-C 예외 대신 `Unsupported`로 거부합니다.
- **frame**: encode는 attachment를 모두 해석·검사한 뒤에야 command buffer와 encoder를 만듭니다
  (encoder를 연 뒤 실패 경로 없음). surface frame은 `nextDrawable`로 drawable을 얻고 `schedule_present`가
  `presentDrawable:`을, `submit`이 `commit`을 합니다. `discard`는 commit하지 않고 놓습니다. `poll`은
  command buffer `status`로 비대기 조회하며, present 확정은 present를 붙인 command buffer의 GPU
  완료입니다(창 밖 layer는 `presentedTime`이 0으로 남는 것을 probe로 확인).
- **readback**: `read_texture`는 blit으로 shared buffer에 복사하고 기다린 뒤 byte를 돌려줍니다
  (적합성 harness·테스트용, blocking).
- **명시적 미지원(이후 행)**: buffer·texture upload(XI.8.3), draw command(XI.8.3·XI.8.4), transient·
  depth-stencil attachment와 비 raster pass(XI.8.5), timestamp(XI.8.6)는 `Unsupported`로 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal` | 10 passed, 0 failed |
| `cargo clippy -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-metal -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| headless external RGBA8 8×4 texture를 red로 clear → `CheckedBackend` 경유 encode·submit → readback 모든 pixel `[255,0,0,255]`, completed = submission 1, presented 없음(`offscreen_smoke_frame_clears_external_texture`) | 통과 |
| 창 밖 CAMetalLayer(BGRA8, 16×8): pixelFormat 80·device·drawableSize 설정 확인, drawable을 green으로 clear → present 예약 → submit → drawable blit readback 모든 pixel BGRA `[0,255,0,255]`, completed·presented = submission(`layer_smoke_frame_clears_drawable_and_presents`) | 통과 |
| RGBA8 layer format `Unsupported`, depth surface·16385 크기·drawable 4개 `InvalidUsage`, CAMetalLayer 아닌 객체 거부, 같은 backend로 RGBA16Float 재 init 성공(`layer_rejects_unsupported_formats_and_foreign_objects`) | 통과 |
| init 전 호출, headless의 surface graph, external texture 크기·format 불일치, 0 크기 descriptor, 파괴한 handle이 모두 구조화 오류(`misuse_is_reported_as_structured_errors`) | 통과 |
| 재 init 뒤 이전 handle 무효, submission 번호 1부터 재시작(`reinit_invalidates_previous_device_handles`) | 통과 |
| capability(`headless_init_reports_metal_capabilities`), descriptor 규칙(`descriptor_rules`), format 대응(`formats_map_to_distinct_sdk_values`), runtime FFI retain 균형·NSString(`objc::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- 외부 binding crate(objc2-metal 계열)는 승인 목록에 없어 쓰지 않고, 작업 지시가 허용한 최소
  hand-written Objective-C runtime FFI로 구현했습니다. 따라서 ADR-0018과 의존성 목록은 바꾸지
  않았습니다.
- crate graph 정책상 이 crate는 `stellar-renderer-core`만 의존하므로 scene 타입(`ColorSpace`,
  `FiniteF32`)을 이름으로 쓰지 못합니다. backend는 `ColorSpace::tag()`(저장 포맷 계약 tag)와
  `FiniteF32::get()` method로만 값을 읽고, 테스트는 renderer-core 공개 값(placeholder의 sRGB, graph
  builder의 depth clear 1.0)에서 값을 얻습니다.
- 창 밖 CAMetalLayer는 drawable 수 제한·`presentedTime`이 실제 창과 다르게 동작합니다(probe: 8개 연속
  획득, `presentedTime` 0 유지). present 확정을 GPU 완료 기준으로 둔 이유입니다.

## 남은 위험 / 한계

- Metal device가 없는 환경(가상화 macOS runner 등)에서는 GPU 테스트가 메시지를 출력하고 검증 없이
  끝납니다. CI-MAC runner의 GPU 유무는 이 작업에서 확인하지 못했습니다.
- 실제 창(CanvasHost) layer 연결과 display scale 변경은 XII.4.1에서 검증합니다.
- Objective-C 예외는 Rust로 unwind할 수 없으므로 Metal에 넘기기 전 검사로 막습니다. 검사 밖 API 오용은
  여전히 process abort 위험이 있습니다.
