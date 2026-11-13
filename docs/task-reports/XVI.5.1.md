# 작업 보고서: XVI.5.1 CPU/GPU memory budget provider와 정규화 bucket

- WBS: XVI.5.1
- 작업명: CPU/GPU memory budget provider interface를 정의하고 OS별 available memory 정보를 normalized bucket으로 반환
- 상태: 완료 (`[x]`)
- 선행: Phase XI, Phase XII (Phase 단위 선행은 완화 적용: provider는 렌더러·앱 셸 코드 없이 정의할 수 있고, GPU working set 상한은 renderer가 나중에 채울 자리로 둠)

## 필수 실행 환경

- `DEV-MAC` (macOS arm64 로컬 검증. Windows는 ADR-0006에 따라 보류).

## 변경 파일 목록

- `crates/diagnostics/src/memory.rs`(신규) — `MemoryBucket`, `MemoryPressure`, `MemoryReading`, `GpuMemory`,
  `MemorySnapshot`, `BucketThresholds`, `MemoryBudget`, `MemoryBudgetProvider`, `MockMemoryProvider`,
  `SystemMemoryProvider`, `MemoryQueryError`, `pressure_from_darwin_level`, macOS FFI(`macos` 내부 모듈).
- `crates/diagnostics/src/lib.rs`(수정) — `memory` 모듈 등록.
- `docs/task-reports/XVI.5.1.md`(신규), 계획서(XVI.5.1 `[ ]`→`[x]`).

## 구현 요약

- **interface**: `MemoryBudgetProvider::snapshot()`이 `MemorySnapshot { cpu, gpu, pressure }`를 돌려주고, 기본
  메서드 `budget(&BucketThresholds)`가 CPU·GPU 각각을 `MemoryBucket`(Low < Medium < High)으로 정규화합니다.
  캐시 관리자(XVI.5.2~)는 bytes가 아니라 bucket으로 정책을 고릅니다.
- **정규화 규칙**(`BucketThresholds::DEFAULT`): Low = 가용 15% 미만 또는 1 GiB 미만, High = 가용 40% 이상이고
  4 GiB 이상, 그 밖은 Medium. 비율은 내림한 ‰가 아니라 `available × 1000`과 `경계 × total`을 u128로 정확히
  비교합니다. OS 압박 단계는 bucket 상한으로만 작동합니다(Warning → 최대 Medium, Critical → Low; 올리지
  않음). 경계는 `validate`로 일관성(≤ 1000‰, Low ≤ High)을 확인합니다.
- **GPU**: `GpuMemory::Unified { working_set_limit }`(Apple Silicon 통합 메모리)는 CPU pool에서 GPU reading을
  유도합니다(전체 = min(CPU 전체, 상한), 가용 = min(CPU 가용, 그 전체), OS 압박 함께 적용).
  `GpuMemory::Dedicated`(전용 VRAM)는 OS 압박과 무관하게 판정합니다. Metal 권장 working set 상한은
  renderer가 알기 때문에 `SystemMemoryProvider::with_gpu_working_set_limit`로 받는 자리만 둡니다.
- **macOS 질의**(최소 FFI, libSystem, 선언은 macOS SDK 헤더 기준): 전체 = `sysctlbyname("hw.memsize")`,
  가용 = (`free_count` + `inactive_count`) × `host_page_size`(`host_statistics64(HOST_VM_INFO64)`), 압박 =
  `sysctlbyname("kern.memorystatus_vm_pressure_level")`. `vm_statistics64`는 rev1 prefix(152 bytes, 정렬 8,
  컴파일 타임 크기 검사)와 `HOST_VM_INFO64_REV1_COUNT`(38)로 읽어 이후 SDK에서 늘어난 field와 무관합니다.
  `mach_host_self` send right는 사용 후 `mach_port_deallocate`로 반환합니다. 가용이 전체를 넘으면 전체로
  자르고, page 곱 overflow·0 page·sysctl 크기 이상은 typed error입니다. 그 밖의 OS는 `Unsupported`입니다.
- **mock fixture**(기준 장비 M1 16 GiB, 통합 메모리): `low()` 512 MiB 가용 + Warning, `medium()` 6 GiB 가용
  (37.5%) + Normal, `high()` 12 GiB 가용(75%) + Normal.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-diagnostics` | 완료 |
| `cargo test -p stellar-diagnostics` | unit 8 passed, doctest 2 passed(compile_fail 1 포함), 0 failed |
| `cargo clippy -p stellar-diagnostics --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-diagnostics -- --check` | 통과 |
| `git diff --check` | 통과 |
| scratch probe(`SystemMemoryProvider::new().budget(DEFAULT)`, 저장소 밖 임시 crate) | total 38,654,705,664(`sysctl hw.memsize`와 동일), available 13,794,623,488(356‰), pressure Normal(`sysctl kern.memorystatus_vm_pressure_level: 1`과 일치), cpu·gpu = medium |
| `vm_stat` 대조 | 같은 시점 근처 (free 180,530 + speculative 3,371 + inactive 664,247) × 16,384 ≈ 13.9 GB로 provider 값과 같은 규모 |

| 검사 | 결과 |
|---|---|
| mock low/medium/high fixture가 CPU·GPU 모두 Low/Medium/High(`mock_fixtures_map_to_low_medium_high`) | 통과 |
| 15%·40% 경계 정확 판정(경계 −1 byte와 경계), 작은 장비 절대량 규칙, 큰 장비 비율 규칙(`thresholds_are_applied_at_exact_boundaries`) | 통과 |
| OS 압박이 bucket 상한으로만 작동, darwin level 1·2·4 매핑, 알 수 없는 값 무시(`os_pressure_caps_the_bucket`) | 통과 |
| 통합 메모리 working set 상한 적용·0 상한 무시·압박 공유, 전용 VRAM은 압박 무관(`gpu_reading_follows_unified_or_dedicated_memory`) | 통과 |
| 전체 0·가용 > 전체 거부, ‰ 계산 overflow 없음, 잘못된 경계 거부(`invalid_values_are_rejected`) | 통과 |
| 실제 macOS 질의: 전체 ≥ 1 GiB, 0 < 가용 ≤ 전체, working set 상한 반영(`macos_system_provider_reports_consistent_values`) | 통과 |

## 차이(작업 지시와 다른 점)

- GPU 전용 정보(Metal `recommendedMaxWorkingSetSize`)는 직접 질의하지 않았습니다. Objective-C runtime·Metal
  device가 필요해 diagnostics crate 경계를 넘기 때문에, renderer가 값을 넘겨주는 생성자만 두었습니다.
- bucket 경계 값(15%·1 GiB, 40%·4 GiB)은 이 작업에서 정한 설계 값입니다. 작업 범위(ALLOWED PATHS)에 ADR 경로가
  없어 별도 ADR을 쓰지 않고 이 보고서와 코드 문서에 근거를 남겼습니다. 성능 예산 파일
  (`config/performance-budgets.yaml`, ADR 보호 경로)로 옮길지는 XVI.5.3~XVI.5.4에서 정하는 것이 좋습니다.
- Windows provider는 ADR-0006에 따라 만들지 않았고 `Unsupported`를 돌려줍니다.

## 남은 위험 / 한계

- `kern.memorystatus_vm_pressure_level`의 값 1이 정상임은 이 장비에서 확인했지만, 2(warning)·4(critical)는 Darwin
  dispatch memory pressure 값 규약에 따른 매핑이며 실제 압박 상태를 재현해 확인하지는 않았습니다.
- 가용 메모리는 free + inactive의 보수적 추정입니다. purgeable·compressor로 회수 가능한 양은 반영하지 않아
  실제보다 낮게 나올 수 있습니다(bucket이 한 단계 낮게 나오는 쪽으로 치우침).
- 질의는 동기 호출입니다. 호출 빈도·스레드(UI thread 2ms 예산, XVI.5.4)는 budget manager 작업에서 정해야 합니다.
