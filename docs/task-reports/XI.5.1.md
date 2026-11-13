# 작업 보고서: XI.5.1 RendererBackend trait

- WBS: XI.5.1
- 작업명: `RendererBackend` trait에 device init, resize, resource create/update/destroy, encode, submit, present 예약, timestamp query 정의
- 상태: 완료 (`[x]`)
- 선행: XI.4.1 (이 lane에서 `290e98f`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/backend/mod.rs`(신규) — `RendererBackend` trait, `DeviceConfig`,
  `SurfaceConfig`, `SurfaceSize`, `DeviceCapabilities`, `BufferDesc`/`BufferUsage`, `GpuTextureDesc`/
  `TextureUsage`, `TextureRegion`, `BufferHandle`, `TextureHandle`, `FrameId`, `SubmissionId`,
  `ImportBinding`/`ImportTarget`, `FrameEncodeRequest`, `EncodeOutcome`/`SkipReason`, `EncodedFrame`,
  `GpuProgress`, `TimestampQuery`/`GpuTimings`/`PassGpuTime`, `BackendError`/`BackendErrorKind`.
- `crates/renderer-core/src/backend/command.rs`(신규) — `PipelineKey`(`ShaderProgram`, `BlendState`,
  `DepthMode`, `StencilMode`), `DrawCommand`, `TextureSource`, `ScissorRect`, `PassCommands`.
- `crates/renderer-core/src/backend/call_order.rs`(신규) — `CallOrderValidator`, `CallOrderState`,
  `CallOrderError`, `BackendCall`, `CheckedBackend<B>`.
- `crates/renderer-core/src/backend/mock.rs`, `backend/tests.rs`(신규, `#[cfg(test)]`) — 호출 기록·GPU
  진행 제어·오류 주입이 되는 mock backend와 테스트.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.5.1.md`(신규), 계획서(XI.5.1 `[ ]`→`[x]`).

## 구현 요약

- **trait 메서드**: `init`(device·queue·surface, capability 반환), `resize`(0 크기 허용),
  `create_buffer`/`create_texture`, `update_buffer`/`update_texture`, `destroy_buffer`/
  `destroy_texture`, `encode`(graph → command buffer, drawable 획득, 없으면 `Skipped`),
  `schedule_present`, `submit`, `poll`(GPU 완료·present 진행, 비대기), `query_timestamps`(pass별 GPU
  시간, 비대기). 모든 메서드는 `BackendError`를 돌려주고 panic하지 않는 계약입니다. trait은 object
  safe합니다(`Box<dyn RendererBackend>`).
- **native 객체 격리**: trait 인자와 반환값은 descriptor·handle·`CompiledRenderGraph`·
  `DrawCommand` 값뿐입니다. native surface는 backend 생성 시 platform 코드가 넘기므로 trait에 없습니다.
  `EncodedFrame`은 backend 안 command buffer를 가리키는 token이고, `submit`이 값으로 소비해 한 frame을
  두 번 제출할 수 없습니다.
- **present 예약 순서**: present는 `submit` **전**에 열린 frame에 겁니다. Metal(command buffer에
  present 첨부 후 commit)과 D3D12(execute 후 Present) 모두 `submit` 반환 시점에 제출과 present 예약이
  끝나므로 T5 정의(아키텍처 §6.1)와 맞습니다.
- **backend 중립 command**: `PipelineKey`(program·blend·depth·stencil·color format·sample 수)와
  `DrawCommand`(pipeline·scissor·stencil reference·vertex/index/uniform bind·texture bind·draw)를
  두었습니다. texture는 backend handle 또는 같은 frame graph의 version(`TextureSource::Graph`)입니다.
- **호출 순서 검사**: `CallOrderValidator` 상태 기계(Uninitialized → Idle ⇄ FrameOpen, device lost 시
  Lost)가 계약을 검사합니다. `CheckedBackend<B>`는 위반 호출을 내부 backend에 넘기지 않고
  `BackendErrorKind::InvalidUsage` + `CallOrderError`로 거부합니다. 검사 항목: init 전 호출·중복 init·
  frames in flight 1..=3, 열린 frame 중 resize·resource 호출·재 encode 금지, 열린 frame 없는 present·
  submit, frame 불일치, present 중복·offscreen frame present, frame·submission 번호 엄격 증가, 없는
  submission timestamp, encode 요청의 pass(graph에 있는지·중복)와 import binding(모든 imported
  resource에 하나, 종류 일치). device lost 뒤에는 `init`만 허용하고, 새 device는 submission 번호를
  다시 셉니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 52 passed, 0 failed; doctest 1 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| mock backend가 `CheckedBackend`를 통해 init → resource 생성·갱신 → encode → present 예약 → submit → timestamp(Pending) → poll → timestamp(Ready) → destroy 순서로 정확히 호출됨(`full_frame_reaches_backend_in_contract_order`) | 통과 |
| 순서 위반 호출 16종이 `CallOrderError`로 거부되고 mock 호출 기록에 나타나지 않음(`out_of_order_calls_never_reach_the_backend`) | 통과 |
| encode 요청의 미지 pass·중복 pass·import 누락·종류 불일치·중복·비 imported binding 거부, 거부 요청은 frame 번호를 소비하지 않음(`encode_requests_are_validated`) | 통과 |
| 0 크기 surface·drawable 없음은 `Skipped`이고 frame을 열지 않음, skip한 frame 번호는 소비(`skipped_frames_do_not_open_a_frame`) | 통과 |
| device lost 뒤 `init`만 허용, 재 init 후 submission 번호 재시작, 다른 오류는 상태 불변(`device_lost_allows_only_reinit`) | 통과 |
| trait object 생성(`trait_is_object_safe`), handle은 값(`backend_handles_are_plain_values`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XI.4.1은 이 lane에서 먼저 완료했습니다(명시 row-ID, `[x]`). 완화 적용은 없습니다.
- handle은 이 작업에서 backend가 발급하는 opaque 값으로만 두었습니다. generational ID와 stale handle
  검출은 XI.5.2에서 이 타입을 대체합니다.
- 오류 분류의 복구 가능 여부·정책은 XI.5.4 범위라 여기서는 `BackendErrorKind`만 정의했습니다.
- mock backend는 `#[cfg(test)]`입니다. 다른 crate용 test-support feature는 만들지 않았습니다.

## 남은 위험 / 한계

- 실제 Metal 구현(XI.8)이 이 계약을 그대로 만족하는지는 XI.8.x에서 `CheckedBackend`로 감싸 다시
  검증해야 합니다.
- `PipelineKey`의 program·stencil 목록은 XI.4.3(clip)·XI.4.5(batching)·XI.10에서 늘어날 수 있습니다.
