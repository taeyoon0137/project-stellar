# 작업 보고서: XII.3.1 공통 InputEvent와 native timestamp 단조 domain 변환

- WBS: XII.3.1
- 작업명: 공통 `InputEvent`(pointer, wheel, key, text composition, focus, resize) 정의와 native timestamp의 monotonic domain 변환
- 상태: 완료 (`[x]`)
- 선행: II.3.3 (`[x]`, `MonotonicClock`/`ManualClock`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust 단위 테스트 + 로컬 Xcode smoke 빌드). Windows 원천은 QPC 값 fixture로 검증했고 Windows 실기 실행은 하지 않았습니다.

## 변경 파일 목록

- `crates/ffi/src/input.rs`(신규) — `InputEvent`, `InputEventKind`, 종류별 payload, `Modifiers`, `KeyCode`, `TextRange`, `NativeTimestamp`, `InputTimeDomain`, `MonotonicNanos`, `InputEventError`, 직렬화/역직렬화, 단위 테스트.
- `crates/ffi/src/lib.rs` — `pub mod input;` 등록.
- `crates/ffi/tests/fixtures/input_events_v1.golden`(신규) — 양 플랫폼 공통 golden 바이트 13건.
- `docs/task-reports/XII.3.1.md`(신규), 계획서(XII.3.1 `[ ]`→`[x]`).

## 구현 요약

- **이벤트 집합**: `InputEventKind`는 `Pointer`/`Wheel`/`Key`/`TextComposition`/`Focus`/`Resize` 여섯 종류로 닫혀 있습니다.
  pointer는 phase(Down/Up/Move/Cancel)·button·device·click count·modifier·위치, wheel은 phase·단위(Point/Line)·위치·delta,
  key는 HID usage(page 0x07) 기반 `KeyCode`·repeat, composition은 phase(Start/Update/Commit/Cancel)·UTF-8 text·UTF-8 byte selection,
  resize는 논리 크기(`Size2`)·backing scale(`FiniteF32`)을 가집니다. 좌표·크기·scale은 stellar-core-types의 finite 타입만 씁니다.
- **검증**: `InputEvent::new`가 payload 규칙(down/up은 button·click count 필수, move/cancel은 click count 0, key up repeat 금지,
  HID usage 0 거부, selection 범위·문자 경계, cancel은 text 없음, scale > 0)을 검사합니다. decode도 같은 생성자를 거칩니다.
- **native timestamp**: `NativeTimestamp`는 macOS `NSEvent.timestamp`(초, 반올림), `mach_absolute_time`(tick×numer/denom),
  Windows QPC(tick×1e9/frequency)를 nanosecond로 변환합니다(NaN·음수·overflow·timebase 0 거부).
  `InputTimeDomain`은 같은 순간의 (native, `MonotonicClock::now_nanos`) 기준쌍으로 코어 단조 domain에 옮기고, 결과를
  `[0, u64::MAX]`·변환 순서 단조 비감소로 고정합니다. wall clock은 쓰지 않습니다.
- **텍스트 range**: macOS `NSRange`와 Windows TSF ACP의 UTF-16 range를 `TextRange::from_utf16`으로 UTF-8 byte range로
  변환합니다(surrogate pair 중간·범위 밖 거부).
- **직렬화**: 플랫폼 공통 versioned little-endian 레이아웃(header: version u8 | kind u8 | timestamp u64)을
  `encode`/`decode`로 제공합니다. unknown version/kind/tag, 잘림, 남는 바이트, 비유한 float, 음수 크기, bool 0/1 외 값,
  invalid UTF-8을 거부합니다.
- C ABI(`extern "C"`)는 추가하지 않았습니다. 셸별 native 이벤트 변환과 전달 경로는 XII.3.2/XII.3.3에서 연결합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ffi` | 25 passed, 0 failed |
| `cargo clippy -p stellar-ffi --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-ffi -- --check` | 통과 |
| `cargo run -p xtask -- generate-ffi-header` 후 `git diff --exit-code -- crates/ffi/include/stellar_ffi.h` | diff 없음(종료 0) |
| `cargo xtask validate-crate-graph` | 종료 0 |
| `scripts/macos/smoke.sh` | `BUILD SUCCEEDED`, smoke/lifecycle/canvas OK, 종료 0 |

| 검사 | 결과 |
|---|---|
| **양 플랫폼 event fixture schema 동일**: macOS(NSEvent 초)·macOS(mach tick)·Windows(QPC) 원천의 같은 논리 이벤트 13건이 golden 바이트와 모두 일치, golden decode 복원(`both_platforms_match_golden_fixture`) | 통과 |
| golden이 6개 kind를 모두 포함(`golden_covers_every_event_kind`), header 바이트 수기 검산(`golden_header_bytes_are_hand_checked`) | 통과 |
| native timestamp 변환·거부(NaN, ±무한대, 음수, overflow, timebase 0) | 통과 |
| 코어 시계 기준 매핑, 역행 timestamp 단조 고정, 기준점 이전 0 고정, 같은 순간의 플랫폼 간 동일 시각 | 통과 |
| UTF-16→UTF-8 range(한글·이모지·surrogate 중간·범위 밖·overflow), modifier unknown bit 거부 | 통과 |
| payload 규칙 위반 거부, 전 fixture round-trip, 손상 바이트 거부(`malformed_bytes_are_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- 공통 타입은 stellar-core-types가 아니라 셸·코어 경계 crate인 `stellar-ffi`의 Rust 모듈로 두었습니다(전역 registry 비변경 규칙).
- "양 플랫폼 fixture"는 Windows 셸 실행 없이 QPC tick 값을 입력으로 한 Rust fixture로 검증했습니다.

## 남은 위험 / 한계

- macOS virtual key code·Windows scan code → HID usage 변환표, 좌표계 변환(macOS view flip, Windows DPI)은 XII.3.2/XII.3.3 소관입니다.
- 셸에서 InputEvent를 코어로 넘기는 C ABI(고정 구조체 또는 ring buffer)는 아직 없습니다. 추가 시 `tools/xtask/src/ffi_header.rs` 템플릿 변경이 함께 필요합니다.
- native 시각 역행은 이전 결과로 고정하므로, 원천 간 순서가 크게 뒤섞이면 latency 측정값이 0에 가깝게 눌릴 수 있습니다.
