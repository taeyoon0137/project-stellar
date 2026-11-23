# 작업 보고서: XVI.5.6 GPU OOM 직전 cache purge 후 한 번 재시도

- WBS: XVI.5.6
- 작업명: GPU OOM 직전 캐시 purge 후 재시도를 한 번만 수행하고 반복 loop를 금지
- 상태: 완료 (`[x]`)
- 선행: XI.5.4, XVI.5.4
- 결정 기록: ADR-0108 결정 7

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/renderer-core/src/cache.rs` — `CacheKind::gpu_resident`, `CacheBudgetManager::purge_gpu`, `OomRetry`, `with_oom_retry`.
- `crates/renderer-core/src/cache/tests.rs` — mock OOM 시험 2개. `crates/renderer-core/src/lib.rs`(module 설명).

## 구현 요약

- **판단은 기존 정책 그대로**: XI.5.4 `RecoveryTracker`가 이미 메모리 부족(`OutOfDeviceMemory`·`OutOfHostMemory`)을
  `PurgeCachesAndRetry` → (재시도도 실패하면) `SkipFrame`으로 정합니다. `with_oom_retry`는 그 판단을 실제 작업 실행에 붙인
  것입니다. 첫 메모리 부족이면 `purge_gpu` 뒤 작업을 한 번 더 하고, 그것도 메모리 부족이면 tracker에 알려(`SkipFrame`, 재시도
  대기 해제) 오류를 돌려줍니다. 재시도 횟수는 변수로도 0 또는 1로 막아, tracker 상태와 무관하게 반복하지 않습니다.
- **메모리 부족이 아닌 오류**는 재시도하지 않고 그대로 돌려줍니다(복구는 호출자가 같은 tracker로 정함). 성공하면
  `operation_succeeded`를 알려 다음 작업이 다시 한 번의 재시도를 받습니다.
- **purge 대상**: GPU에 올라가는 resource를 가진 cache(이미지 mipmap, tessellation, glyph atlas)의 지울 수 있는 단위 전부입니다.
  Document/CurrentScene과 GPU 밖 cache(썸네일)는 남깁니다. glyph atlas는 atlas 규칙대로 현재 frame·GPU 진행 중 page와 마지막
  page를 남깁니다. purge 직후 다시 할당하므로 지운 값은 XVI.5.4처럼 미루지 않고 그 자리에서 해제합니다.
- "OOM 직전"은 Metal backend가 예산을 넘는 할당을 Metal에 요청하기 전에 `OutOfDeviceMemory`로 거부하는 경로(XI.8.7
  `MemoryBudget`)와 Metal 자체 거부를 모두 같은 오류 종류로 받으므로 같은 함수로 처리됩니다.

## 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | unit 171 passed, doctest 3 passed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| mock OOM(항상 메모리 부족): 작업 호출 2회(처음 + 재시도 1회), `retries` = 1, 오류 반환, GPU cache만 비고 Document/CurrentScene·썸네일·atlas 마지막 page는 남음, purge byte = 줄어든 사용량, 다음 작업도 다시 한 번만 재시도(`mock_gpu_oom_retries_exactly_once`) | 통과 |
| purge 뒤 성공(재시도 1회), Host 메모리 부족도 같은 정책, 처음부터 성공하면 purge 없음, 메모리 부족이 아닌 오류(`DeviceLost`)는 재시도 없음(`oom_retry_succeeds_once_and_non_memory_errors_are_not_retried`) | 통과 |

## 남은 한계

- Metal backend의 실제 할당 호출(`create_texture`·`create_buffer`·upload)을 `with_oom_retry`로 감싸는 연결은 render thread가
  manager를 소유하게 될 때(UI 재설계와 함께) 합니다. 지금 renderer-metal은 오류를 돌려주고 event만 남깁니다.
