# 작업 보고서: XII.3.2 macOS mouse·trackpad·keyboard 이벤트 → InputEvent 변환

- WBS: XII.3.2
- 작업명: macOS mouse/trackpad/keyboard 이벤트를 공통 `InputEvent`로 변환
- 상태: 완료 (`[x]`)
- 선행: II.5.4 (`[x]`), XII.3.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust 단위 테스트 + 로컬 Xcode Debug 빌드와 headless `--input-check`). 실제 HID 장치를 사람이
  조작하는 검사는 하지 않았습니다.

## 변경 파일 목록

- `crates/ffi/src/input_macos.rs`(신규) — `StellarMacEvent`(C 고정 구조체), `convert_mac_event`, virtual key code →
  HID usage 표(`mac_key_code`, 119개), `mac_modifiers`, flagsChanged 좌우 판정, scroll 단위·부호·phase, `MacEventError`, 테스트.
- `crates/ffi/src/lib.rs` — `input_macos` 등록, `stellar_runtime_submit_mac_event`, `stellar_runtime_poll_input`,
  상태 코드 `InvalidArgument`(8)·`QueueFull`(9), 입력 대기열(용량 4096)과 runtime별 `InputTimeDomain`, `Instant` 기반
  코어 단조 시계(기준점은 첫 runtime 생성), 헤더 drift·FFI 테스트.
- `crates/ffi/tests/fixtures/mac_events_v1.golden`(신규) — 원시 NSEvent 필드 15건 → InputEvent 바이트.
- `tools/xtask/src/ffi_header.rs`(헤더 템플릿), `crates/ffi/include/stellar_ffi.h`(`cargo run -p xtask -- generate-ffi-header`로 재생성).
- `apps/macos/StellarMac/CanvasView.swift` — mouse·right/other mouse·drag·move·scroll·key·flagsChanged 처리기,
  hover tracking area, `acceptsFirstResponder`, `CanvasInput.macEvent`(NSEvent 필드 복사, view 좌표 변환).
- `apps/macos/StellarMac/InputCheck.swift`(신규), `apps/macos/StellarMac/main.swift` — `--input-check` headless 검사.
- `scripts/macos/smoke.sh` — `--input-check` 실행 추가.
- `docs/adr/ADR-0014-ffi-shell-surface.md`(신규) — 이번 FFI ABI 추가 기록.
- `docs/task-reports/XII.3.2.md`(신규), 계획서(XII.3.2 `[ ]`→`[x]`).

## 구현 요약

- **경로**: `CanvasView`의 AppKit 처리기 → `CanvasInput.macEvent`가 NSEvent 원시 필드를 `StellarMacEvent`로 복사(종류별
  accessor만 호출) → `stellar_runtime_submit_mac_event` → Rust `convert_mac_event` → runtime 입력 대기열 →
  `stellar_runtime_poll_input`(InputEvent 바이트, pull).
- **coordinate**: 셸이 `convert(locationInWindow, from: nil)`로 flipped 캔버스 view 좌표(좌상단 원점, y 아래)로 옮깁니다.
  창 없는 이벤트(화면 좌표)는 화면→창→view로 옮깁니다. 코어는 `f64`→`f32` IEEE 반올림 후 유한성을 검사하고, drag 중
  view 밖 음수 좌표를 유지합니다.
- **modifiers**: Shift·Control·Option(→Alt)·Command(→Meta)·Caps Lock만 반영하고 numericPad·help·function·
  device-dependent bit는 무시합니다(물리 key 그대로, Control+click도 Primary + Control).
- **double click**: down/up의 `clickCount`를 그대로(255 포화) 쓰고 drag·move는 0입니다. down/up의 0 이하 값은 거부합니다.
- **button·device**: left/right/other(2=Middle, 3=Back, 4=Forward), 그 밖 button은 무시. tablet point subtype만 Pen.
- **trackpad·wheel**: `hasPreciseScrollingDeltas`면 Point, 아니면 Line. AppKit content 방향 delta를 반전해 공통 규칙
  (x 오른쪽·y 아래쪽 양수)으로 맞춥니다. phase는 began/changed·stationary/ended·cancelled/momentum/discrete로,
  `mayBegin`만 있으면 이벤트를 만들지 않습니다.
- **keyboard**: keyDown/keyUp은 Carbon `kVK_*` → HID usage 표, keyDown repeat 유지. flagsChanged는 device-dependent
  bit로 좌우 modifier key down/up을 판정(bit 없는 합성 이벤트는 device-independent bit), Caps Lock은 lock on/off.
  fn 등 표에 없는 key와 enter/exit·gesture 이벤트는 대기열에 넣지 않습니다(`out_queued = 0`).
- **시각**: runtime별 `InputTimeDomain`을 첫 제출의 `receive_uptime`(셸 수신 시각)과 코어 단조 시계로 고정하고,
  `NSEvent.timestamp`를 코어 domain으로 옮깁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ffi` | 43 passed, 0 failed |
| `cargo clippy -p stellar-ffi --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-ffi -- --check` | 통과 |
| `cargo test -p xtask` / `cargo clippy -p xtask --all-targets -- -D warnings` / `cargo fmt -p xtask -- --check` | 115 passed(+문서 테스트) / 경고 0 / 통과 |
| `cargo run -p xtask -- generate-ffi-header` 후 `cargo run -p xtask -- generate-ffi-header --check` | drift 없음(종료 0) |
| `cc -fsyntax-only -std=c11 -Wall -Werror` / `c++ -fsyntax-only -std=c++17 -Wall -Werror` (생성 헤더) | 통과(`_Static_assert`/`static_assert` 104 byte 포함) |
| `crates/ffi/tests/runtime_abi.c` 빌드·실행(staticlib 링크) | `ok: 1000 cycles, 0 leaks` |
| `cargo run -q -p xtask -- validate-crate-graph` / `validate-dependencies` | 종료 0 / 종료 0 |
| `scripts/macos/smoke.sh` | `BUILD SUCCEEDED`(Swift warning/error 0), smoke·lifecycle·canvas·text-input OK, `input ok: 57 events`, 종료 0 |
| `git diff --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **coordinate fixture**: Rust(view point 유지, 음수 좌표, f32 반올림, `-0.0` 정규화, 비유한·범위 밖 거부) + headless 합성 NSEvent 두 위치가 창 좌표(좌하단)→view point(좌상단)로 정확히 뒤집힘, 창 없는 scroll·other mouse 이벤트(화면 좌표) | 통과 |
| **modifiers fixture**: 5개 modifier 단독·조합, 무시 bit(numericPad·function·device bit)가 pointer·key·wheel 모두 같은 규칙 | 통과 |
| **double click fixture**: `clickCount` 1·2·3 down/up 기록, 1000→255 포화, drag는 0, 0·음수 거부 | 통과 |
| golden 15건(`mac_event_sequence_matches_golden_fixture`), single click이 XII.3.1 양 플랫폼 golden `pointer_down_primary`와 바이트 동일 | 통과 |
| key 표 대표값 24개(Rust)·Carbon `kVK_*` 상수 25개(headless), 표 단사·119개, flagsChanged 좌우 Shift·나머지 modifier·Caps Lock | 통과 |
| scroll: trackpad began(dx -3, dy 5)·momentum, wheel line(dy -2), phase·단위·부호 | 통과 |
| 시각: 코어 domain 간격 == native 간격(250ms, 오차 1µs 이내), 무시 이벤트는 domain 미갱신 | 통과 |
| FFI: FIFO poll, 무시 이벤트 0개, 규칙 위반 `InvalidArgument`+last error, null·잘못된 handle, 대기열 4096 상한 `QueueFull`, 작은 buffer 보존, `StellarMacEvent` 크기·offset | 통과 |
| 검사 민감도: `CanvasInput.viewPoint`가 창 좌표를 그대로 반환하도록 임시 변경 시 `--input-check`가 `coordinate 1: 좌표 (251.0, 580.0) != (10.0, 20.0)`로 실패(종료 1) 확인 후 원복 | 확인 |

## 차이(작업 지시와 다른 점)

- 이벤트 해석과 key code 표는 Swift 셸이 아니라 셸·코어 경계 crate(`stellar-ffi`)의 `input_macos` 모듈에 두었습니다.
  셸은 NSEvent 원시 필드를 고정 구조체로 복사만 합니다(ADR-0014). 변환 규칙을 Rust golden으로 검증하기 위해서입니다.
- 사람의 실제 mouse·trackpad·keyboard 조작 대신 AppKit 합성 `NSEvent`(`NSEvent.mouseEvent`/`keyEvent`, `CGEvent`
  경유 scroll·other button)를 캔버스 처리기에 전달하는 headless 검사로 확인했습니다. mouse 이벤트는 표시하지 않은 창에서
  `NSWindow.sendEvent`가 전달하지 않아, 창 hit test로 캔버스를 확인한 뒤 해당 처리기를 직접 호출합니다(key는 `sendEvent`).
- 코어 소비자(DocumentThread, XII.1.2)가 없어 입력은 runtime 대기열(용량 4096)에 머뭅니다. 읽기는 진단용 `poll_input`입니다.

## 남은 위험 / 한계

- 실제 trackpad pinch(magnify)·rotate·smart zoom gesture는 공통 `InputEvent` 집합(XII.3.1)에 없어 변환하지 않습니다.
  Zoom 인터랙션(XII.3.6)에서 별도 kind가 필요하면 schema 버전을 올려야 합니다.
- 소비자가 없는 동안 실행 중인 앱은 입력 4096개 이후 `STELLAR_QUEUE_FULL`로 버립니다(XII.3.4 coalescing·XII.1.2 소비자에서 해소).
- macOS 26 실제 HID 장치(ISO·JIS 배열 실물, pen tablet, 실제 momentum scroll 값)는 사람이 확인하지 않았습니다.
- keyDown은 아직 `interpretKeyEvents`로 넘기지 않아 캔버스에서 IME 문자 입력은 없습니다(XII.5.3~XII.5.4 소관).
- Windows 셸 변환(XII.3.3)은 ADR-0006으로 보류입니다.
