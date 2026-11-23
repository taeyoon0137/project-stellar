# 작업 보고서: XVI.5.4 memory pressure 증분 제거

- WBS: XVI.5.4
- 작업명: memory pressure event에서 낮은 priority부터 증분 eviction하고 단일 UI thread 구간 2ms를 넘지 않게 chunking
- 상태: 완료 (`[x]`)
- 선행: XVI.5.3
- 결정 기록: ADR-0108 결정 6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, release 측정 포함).

## 변경 파일 목록

- `crates/runtime/src/cache/mod.rs` — `PressureLevel`, `CacheLimits`(단계별 목표), `Clock`·`SystemClock`, `ChunkDeadline`(2 ms
  chunk), `PressureEvictor`(pressure event → pass, UI tick마다 `run_chunk`), `ChunkReport`.
- `crates/runtime/src/cache/tests.rs`(신규), `crates/runtime/src/lib.rs`(module 설명).
- `crates/renderer-core/src/cache.rs` — `BudgetedCache::take_released`, `ReleasedResources`, `CacheBudgetManager::take_released`.
- `crates/renderer-core/src/cache_adapters.rs` — `ReleaseQueue`(지운 값을 고정 크기 묶음으로 보관), tessellation·mipmap cache가
  지운 값을 바로 해제하지 않고 보관.
- `crates/runtime/src/cache/thumbnail.rs` — 썸네일 cache도 지운 값을 보관.

## 구현 요약

- **목표**: `CacheLimits::evictable_bytes`를 압박이 없을 때 지울 수 있는 cache 상한으로 두고, Normal은 그 값, Warning은 절반,
  Critical은 0까지 줄입니다. Document/CurrentScene 사용량은 목표에 더해 둡니다(지우지 않음). 압박 단계는 진단 crate의
  `MemoryPressure`(XVI.5.1)와 같은 뜻이고, 앱 연결 코드가 그대로 옮겨 넘깁니다(runtime → diagnostics 직접 의존은 crate graph에
  없으므로 만들지 않음).
- **증분 제거**: `on_pressure`·`on_usage_changed`는 목표만 정하고 아무것도 지우지 않습니다. UI thread tick마다 `run_chunk`가
  manager의 `evict_chunk`(XVI.5.2 순서: 낮은 priority부터)를 2 ms 예산으로 부릅니다. `ChunkDeadline`은 다음 단위를 지우기 전에
  "지금까지 쓴 시간 + 단위 하나의 최대 비용(처음 100 µs, 관측값이 더 크면 그 값)"이 2 ms 안인지 봅니다.
- **해제는 pass 끝에 UI thread 밖에서**: 지운 resource의 메모리 반환(큰 buffer 해제)은 chunk 안에서 하지 않습니다. cache는 지운
  값을 보관하고, evictor는 pass 동안 모았다가 pass가 끝나는 chunk 보고(`ChunkReport::released`)로 한 번 내줍니다. 호출자는 그
  묶음을 worker thread에서 버립니다. 사용량은 지운 즉시 줄어든 것으로 봅니다. 목표가 바뀌어 pass가 사라져도 모은 묶음은 다음
  `run_chunk`로 한 번 나옵니다.

## 측정 (`DEV-MAC`, release)

`cargo test -p stellar-runtime --release --lib cache::tests::measure -- --ignored --nocapture`. 썸네일 200,000개(각 1 KiB)를 Critical에서
모두 지우며 실제 시계로 chunk 시간을 잽니다. 다른 작업 12개가 동시에 도는 부하 상태(load average 10\~50)에서 잰 값입니다.

| 해제 방식 | chunk 수 | chunk당 단위 | chunk p50 | chunk 최대 | 2 ms 초과 chunk | 단위 하나 최대 |
|---|---:|---:|---:|---:|---:|---:|
| chunk 안에서 바로 해제(첫 구현) | 56\~81 | 2,469\~3,571 | 1.90 ms | 5.98\~43.5 ms | 2\~23 | 5.1\~5.9 ms |
| pass 도중 worker가 계속 해제 | 39\~43 | 4,651\~5,128 | 1.90 ms | 5.1\~6.2 ms | 2\~8 | 4.9\~5.8 ms |
| pass 끝에 한 번 worker로 넘김(채택) | 36\~42 | 4,761\~5,555 | 1.90 ms | 1.90\~2.83 ms | 0\~1 | 1.1\~1.3 ms |

- 첫 두 방식의 수 ms 단위는 매 run 같은 위치(지운 양 약 85 MB, 177 MB 지점)에서 나왔습니다. 해제를 아예 하지 않고 재면 사라져,
  allocator가 메모리를 OS에 돌려주는 동안의 경합으로 판단하고 위 채택안으로 바꿨습니다.
- 채택안에서 남은 초과 chunk(최대 2.83 ms, run당 0\~1개)는 단위 하나가 1.1\~1.3 ms 걸린 경우입니다. 평소 단위 비용(약 0.4 µs)의
  수천 배라 부하 상태의 OS 선점으로 봅니다. deadline은 단위 사이에서만 멈출 수 있어 선점(아키텍처 6.3이 따로 분류하는 통제
  불가 지연)은 막지 못합니다. 기준 장비(M1 16 GB) 무부하 측정은 하지 않았습니다.

## 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib cache::` | 6 passed, 1 ignored(release 측정) |
| `cargo test -p stellar-runtime --release --lib cache::tests::measure -- --ignored --nocapture` | 위 표(채택안), 통과(중앙값 ≤ 2 ms 판정) |
| `cargo test -p stellar-renderer-core --lib cache` | 통과 |
| `cargo clippy -p stellar-runtime -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-runtime -p stellar-renderer-core -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| simulated pressure: 가짜 시계(단위 비용 5\~100 µs 결정적 난수)로 Normal → Warning → Critical을 차례로 주면 모든 chunk가 2 ms 이하, 한 번에 끝내지 않고 여러 chunk로 나뉨, 지우는 priority가 낮은 것부터 단조 증가, Normal·Warning 목표 도달, Critical 뒤 Document/CurrentScene만 남음(`simulated_pressure_evicts_incrementally_within_ui_chunk_budget`) | 통과 |
| 목표 아래에서는 일을 만들지 않음, 단계별 목표(`pressure_below_target_does_not_schedule_work`) | 통과 |
| 느린 단위를 관측하면 단위 비용 추정이 올라가 다음 단위를 멈춤(`slow_units_raise_the_unit_cost_estimate`) | 통과 |
| 해제 묶음은 pass 도중 내주지 않고, pass가 사라져도 한 번 내줌(`released_resources_are_handed_over_once_when_the_pass_ends`) | 통과 |
| 썸네일 cache를 evictor로 줄이고 해제 묶음이 한 번 나옴(`pressure_evictor_drives_thumbnail_eviction`) | 통과 |

## 남은 한계

- 앱 연결(OS memory pressure 알림 → `on_pressure`, UI tick → `run_chunk`, 해제 묶음 → worker pool `CacheGeneration`)은 UI 재설계
  때 합니다. 압박이 풀릴 때 glyph atlas page 상한을 되돌리는 호출(`BudgetedGlyphAtlas::restore_page_budget`)도 그때 연결합니다.
- 해제를 pass 끝까지 미루므로 물리 메모리 반환은 pass 길이(위 측정에서 36\~42 chunk, 60 Hz tick 기준 약 0.7 초)만큼 늦습니다.
- 무부하 기준 장비 측정이 필요합니다.
