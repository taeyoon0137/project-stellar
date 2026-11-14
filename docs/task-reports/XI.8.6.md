# 작업 보고서: XI.8.6 Metal GPU timestamp query

- WBS: XI.8.6
- 작업명: Metal GPU timestamp query를 연결해 pass별 GPU 시간 측정
- 상태: 완료 (`[x]`)
- 선행: XI.8.5 (이 lane에서 `0c22214`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max, stage boundary counter sampling 지원 확인, 실제 GPU 측정).

## 변경 파일 목록

- `crates/renderer-metal/src/timestamps.rs`(신규) — `timestamp` counter set 탐지, `MTLCounterSampleBuffer`
  pool, render pass descriptor sample 부착(`attach_samples`), resolve·GPU tick → ns 환산, 결과 보관,
  공개 `gpu_timing_line`·`MAX_TIMED_PASSES`·`MAX_TIMING_RESULTS`.
- `crates/renderer-metal/src/encode.rs` — `encode_pass`가 pass index별 timestamp sample을 붙임.
- `crates/renderer-metal/src/backend.rs` — init에서 지원 판정(capability `timestamp_query`)과
  `DeviceConfig::timestamps`에 따른 활성화, frame별 sample buffer 할당·반환(discard·encode 실패·GPU
  오류 시 반환), 완료 시 해석, `query_timestamps`(Pending·Ready·Unsupported).
- `crates/renderer-metal/src/lib.rs` — 모듈 등록·공개.
- `crates/renderer-metal/src/tests/timestamps.rs`(신규), `tests/mod.rs`, `tests/smoke.rs`(주석 갱신).
- `docs/task-reports/XI.8.6.md`(신규), 계획서(XI.8.6 `[ ]`→`[x]`).

## 구현 요약

- **지원 판정**: `supportsCounterSampling:MTLCounterSamplingPointAtStageBoundary`이고 `timestamp` counter
  set이 있으면 `DeviceCapabilities::timestamp_query = true`(이 Mac: 참). `DeviceConfig::timestamps`가
  참일 때만 측정합니다.
- **기록**: frame마다 pool에서 sample buffer 하나(128 sample)를 꺼내 graph pass `i`의 render pass에
  vertex 시작(sample `2i`)·fragment 끝(`2i+1`) timestamp를 붙입니다. pass가 64개를 넘거나 buffer가 8개
  모두 사용 중이면 그 frame은 측정하지 않습니다(렌더는 그대로).
- **해석**: `poll`(또는 `query_timestamps`)이 GPU 완료를 확인하면 sample을 resolve합니다. GPU tick →
  ns는 init·resolve 시점 `sampleTimestamps:gpuTimestamp:` 쌍의 비율로 환산하고(probe에서 M3 Max는
  1:1), 오류 값이나 역전된 sample은 0 ns로 보고합니다. submission 전체 시간은 command buffer
  `GPUStartTime`·`GPUEndTime`입니다. 결과는 최근 64개만 보관하고 buffer는 pool로 돌아갑니다.
- **query 의미**: GPU 미완료 = `Pending`, 측정 결과 있음 = `Ready(pass별 시간, 실행 순서)`, 측정하지
  않는 설정·device·frame 또는 보관 범위 밖 = `Unsupported`.
- **보고**: `gpu_timing_line`이 `renderer.metal.gpu submission=.. total_ns=.. pass<id>_ns=..` 고정
  형식 한 줄을 만듭니다(정수·정적 이름만).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal` | 41 passed, 0 failed |
| `cargo test -p stellar-renderer-metal timestamps -- --nocapture` | 5 passed; 보고 줄 `renderer.metal.gpu submission=1 total_ns=414500 pass0_ns=39583 pass1_ns=17166` |
| `cargo clippy -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-metal -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| simple scene(256×256 transient layer에 반투명 full-screen quad 32장 → composite): 제출 직후 Pending 또는 Ready, 완료 뒤 Ready, pass 순서 = graph 실행 순서, 모든 pass > 0 ns, total > 0, 모두 1초 미만(`simple_scene_reports_gpu_duration_per_pass`) | 통과 |
| 3 frame 동시 제출 후 모두 Ready(sample buffer 여러 개), 64개 보관 상한을 넘긴 가장 오래된 submission은 Unsupported, 최신은 Ready(`frames_in_flight_reuse_sample_buffers_and_keep_recent_results`) | 통과 |
| `timestamps = false` config는 Unsupported(`timestamps_not_requested_are_unsupported`), 보고 줄 형식(`timing_line_is_fixed_format`) | 통과 |

## 차이(작업 지시와 다른 점)

- submission 전체 시간은 pass sample의 최소·최대가 아니라 command buffer `GPUStartTime`·`GPUEndTime`
  입니다. 그래서 total에는 pass 사이 GPU scheduling 시간이 포함되어 pass 합보다 큽니다(측정 예: 414 µs
  vs pass 합 57 µs).

## 남은 위험 / 한계

- tick → ns 비율은 두 시점 쌍의 평균입니다. GPU clock이 frame 중 바뀌는 device에서는 오차가 생길 수
  있습니다(Apple Silicon에서는 probe상 1:1).
- 가상화 macOS 등 counter sampling이 없는 GPU에서는 `timestamp_query = false`이고 테스트는 메시지를
  출력하고 건너뜁니다.
- 아키텍처 §6.3 "인터랙티브 GPU 실행 시간 p99 4ms" 판정은 benchmark(XI.11.4)에서 이 값을 모아야
  합니다.
