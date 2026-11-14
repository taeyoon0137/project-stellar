# 작업 보고서: XI.8.7 device error·memory pressure·drawable timeout 처리

- WBS: XI.8.7
- 작업명: device error·memory pressure·drawable timeout 처리 테스트 추가
- 상태: 완료 (`[x]`)
- 선행: XI.5.4 (`[x]`), XI.8.5 (이 lane에서 `0c22214`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max, 실제 GPU·device 재생성·할당 경로).

## 변경 파일 목록

- `crates/renderer-metal/src/events.rs`(신규) — 공개 `MetalEvent`(device error·memory pressure·drawable
  timeout·pipeline failure, 고정 형식 line, XI.5.4 안정 코드), `classify_command_buffer_error`
  (`MTLCommandBufferError` → `BackendErrorKind`), `MAX_EVENTS`, `MemoryBudget`.
- `crates/renderer-metal/src/backend.rs` — event queue(`drain_events`), command buffer 실패 처리(NSError
  code 분류, event, device lost 표시 후 init 전까지 `DeviceLost`), device 없음 event, GPU 메모리 예산
  (`set_memory_budget`, `memory_usage`, `purge_transients`, 할당 전 예산 검사·transient 회수),
  drawable timeout event(대기 시간), pipeline failure event, 테스트 주입(`injected_gpu_errors`,
  `fail_device_creation`, `#[cfg(test)]`).
- `crates/renderer-metal/src/transient.rs` — 예산 안에서만 transient 할당(`PoolError`), purge가 놓은 byte
  반환.
- `crates/renderer-metal/src/texture.rs` — descriptor 분리, `texture_footprint`
  (`heapTextureSizeAndAlignWithDescriptor:`), `allocated_size`, `MetalTexture.bytes`.
- `crates/renderer-metal/src/buffer.rs` — `MetalBuffer.bytes`.
- `crates/renderer-metal/src/device.rs` — `recommended_working_set`.
- `crates/renderer-metal/src/lib.rs` — `MetalEvent`·`MemoryUsage`·`classify_command_buffer_error`·
  `MAX_EVENTS` 공개.
- `crates/renderer-metal/src/tests/errors.rs`(신규), `tests/mod.rs`.
- `docs/task-reports/XI.8.7.md`(신규), 계획서(XI.8.7 `[ ]`→`[x]`).

## 구현 요약

- **원칙**: 어떤 GPU 오류도 panic·abort로 가지 않고 `BackendError`(XI.5.4 분류·복구 정책)와
  `MetalEvent`(정적 이름 + 정수, 최대 256개)를 함께 남깁니다.
- **device error**: command buffer `status == Error`면 `-error.code`를 분류합니다(Timeout·NotPermitted →
  `Timeout`, OutOfMemory → `OutOfDeviceMemory`, DeviceRemoved → `DeviceLost`, AccessRevoked →
  `Unsupported`, PageFault·InvalidResource·Memoryless·StackOverflow → `InvalidUsage`, 그 밖 →
  `Internal`). `DeviceLost`면 backend가 lost 상태가 되어 `init` 전까지 모든 호출이 `DeviceLost`입니다.
  device를 찾지 못한 init도 `DeviceError` event입니다.
- **memory pressure**: backend가 만든 buffer·texture·transient의 `allocatedSize` 합을 예산(기본
  `recommendedMaxWorkingSetSize`, 변경 가능·재 init에도 유지)과 비교합니다. 넘는 요청은 Metal 할당 전에
  다시 만들 수 있는 transient pool을 비워 보고, 그래도 넘으면 `MemoryPressure` event +
  `OutOfDeviceMemory`(복구: cache purge 후 재시도)입니다. Metal 할당 거부·device buffer 상한 초과도 같은
  event입니다. transient가 예산을 넘으면 frame encode가 실패하고 frame은 열리지 않습니다.
- **drawable timeout**: nil drawable은 `Skipped(NoDrawable)`(XI.8.2)과 함께 `DrawableTimeout { frame,
  waited_nanos }` event를 냅니다(크기 재동기화 skip은 event 아님).
- **pipeline failure**: encode가 `PipelineCreation`으로 실패하면 `PipelineFailure { stage }` event를
  냅니다(log는 `last_shader_error`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal` (3회 반복) | 매번 49 passed, 0 failed |
| `cargo clippy -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-metal -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |
| `cargo run -q -p xtask -- validate-dependencies` | exit 0 |

| 검사 | 결과 |
|---|---|
| `MTLCommandBufferError` 0~12·미지 코드의 분류·복구 정책 matrix(`command_buffer_error_matrix`), event line 안정 코드(`event_lines_use_stable_codes`) | 통과 |
| GPU에서 끝난 submission을 DeviceRemoved(11)로 보면 `poll`이 `DeviceLost`, event `DeviceError{DeviceLost, submission, 11}`, tracker `RecreateDevice`, validator `Lost`, raw backend도 `DeviceLost` 거부 → 재 init 뒤 submission 1부터 정상 렌더·readback, 이전 handle 무효(`device_removed_reports_event_and_recovers_by_reinit`) | 통과 |
| Timeout(2)·OutOfMemory(8)·PageFault(3)가 각각 RetryNextFrame·PurgeCachesAndRetry·SkipFrame, event 1개씩, device 계속 사용 가능(`transient_gpu_errors_keep_the_device_usable`) | 통과 |
| 2 MiB 예산에서 4 MiB texture·3 MiB buffer 거부 + `MemoryPressure`(요청·0·2 MiB), 작은 texture 성공·사용량 반영·파괴 시 0, 예산 무한이어도 device buffer 상한 초과는 event와 함께 거부(`memory_pressure_rejects_allocations_with_events`) | 통과 |
| transient가 예산을 넘으면 frame 실패(`OutOfDeviceMemory`, frame 미개방, event), 예산을 늘리면 렌더, transient 공간이 있어야 들어가는 할당은 pool 회수 후 성공(event 없음), 다음 frame에서 transient 재생성(`transient_pressure_fails_the_frame_and_purge_reclaims_memory`) | 통과 |
| nil drawable 2회 → `Skipped(NoDrawable)` + frame 번호가 맞는 `DrawableTimeout` 2개, 이후 정상 present, event 보관 상한 256(`drawable_timeout_emits_events_and_skips_frames`) | 통과 |
| device 없음 init → `Unsupported` + `DeviceError` event, 다음 init 성공; 깨진 shader → `PipelineFailure{Compile}` event(`missing_device_and_shader_failures_are_events`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 GPU 실행 오류(page fault·timeout·device removed)는 일부러 만들지 않았습니다. Apple GPU에서 반복
  fault는 process의 GPU 접근을 막고(`AccessRevoked`) 같은 test process의 다른 GPU 테스트를 깨뜨릴 수
  있으며, Apple Silicon 내장 GPU는 제거할 수 없기 때문입니다. 대신 GPU에서 정상 완료한 submission을 지정
  코드로 실패한 것처럼 보는 `#[cfg(test)]` 주입을 쓰고, 그 뒤 분류·event·lost 상태·재 init 복구는 실제
  device로 확인했습니다. 실제 `-error.code` 읽기 경로는 주입이 없을 때만 실행됩니다.
- drawable timeout도 창 밖 layer가 drawable을 제한 없이 주므로(XI.8.2 probe) 주입으로 재현했습니다.
- host(CPU) 메모리 pressure(dispatch memory pressure source)는 block 기반 API라 이 작업에서 연결하지
  않았습니다. GPU 예산 기반 pressure만 다룹니다.

## 남은 위험 / 한계

- 실제 command buffer 오류에서 `-error.code` 값이 표와 같은지는 실제 fault 환경(실제 창, 장시간 GPU
  작업)에서만 확인할 수 있습니다(XII.4·XI.11.6 conformance에서 관찰 필요).
- 예산은 backend가 만든 resource만 셉니다. drawable·command buffer·driver 내부 할당은 포함하지 않으므로
  실제 GPU 사용량보다 작습니다.
- 공통 계층 cache(texture·glyph atlas) purge 연결은 runtime 몫입니다. backend는 transient pool만 스스로
  회수합니다.

## 후속 수정 (리뷰 반영)

통합 리뷰 지적 두 개가 이 행의 "crash 없이 error event" 보장에 걸려 `Fix:` 커밋으로 고쳤습니다. 행 상태는
`[x]`로 둡니다(지적한 경로를 고쳤고 같은 DEV-MAC 실제 GPU 테스트로 확인).

- **render target 용도 없는 attachment(`d5d28f0`)**: sampling 전용 external texture를 color·depth
  attachment로 쓰면 backend가 그대로 `renderCommandEncoderWithDescriptor`에 넘겼습니다. 기본 실행은 정의되지
  않은 GPU 동작으로 `Ok`였고, Metal API validation(`MTL_DEBUG_LAYER=1`)에서는 `SIGABRT`로 process가
  끝났습니다(재현 확인). 이제 pass 계획 단계에서 `InvalidUsage`입니다
  (`attachments_without_render_target_usage_are_rejected`, validation 켠 실행에서도 통과).
- **texture upload staging 예산(`5b47e61`)**: `update_texture`의 staging buffer가 예산 검사·사용량 기록
  없이 쌓였습니다. 이제 만들기 전에 예산을 확인해 넘으면 `MemoryPressure` event와 `OutOfDeviceMemory`이고,
  GPU가 upload command buffer를 끝내면 사용량에서 뺍니다
  (`texture_upload_staging_counts_against_the_budget`). `read_texture`는 자기 blit만 기다리므로 그 뒤 첫
  `poll`·`wait_idle`까지 staging 몫이 사용량에 남습니다(보수적으로 큼).
- GPU 오류 코드와 drawable timeout은 여전히 `#[cfg(test)]` 주입으로 재현합니다. 이 행의 작업은 처리
  테스트 추가이고, 주입 뒤 분류·event·복구 경로는 실제 device로 확인합니다.

| 명령 (`DEV-MAC`, 리뷰 수정 후) | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal` | 54 passed, 0 failed |
| `MTL_DEBUG_LAYER=1 cargo test -p stellar-renderer-metal` | 54 passed, 0 failed |
