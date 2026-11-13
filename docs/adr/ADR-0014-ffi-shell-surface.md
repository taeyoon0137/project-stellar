# ADR-0014: 셸↔코어 FFI 표면 확장 기록

## Status

Accepted (2026-11-14, 구현 기록)

## Context

기술 아키텍처 §8.2는 셸과 코어 경계를 다음으로 고정합니다: 안정적인 C ABI, Rust 내부 타입 비노출,
handle·POD 데이터·버전형 메시지와 명시적 수명 관리, "고빈도 입력은 작은 고정 구조체 또는 ring buffer로
전달". II.4.1~II.4.6은 정수 registry handle, 모든 함수의 `StellarStatus` 반환, handle별
`stellar_last_error`, UTF-8 ptr+len 입력·호출자 buffer 출력, `catch_unwind` guard, pull 방식 이벤트
큐, 고정 템플릿 헤더 생성(`cargo xtask generate-ffi-header`)을 정했습니다.

구현 계획 1.2의 3번 규칙은 작업 지시 없이 FFI 시그니처를 추가하지 않도록 하고, I.3.5 정책은
`crates/ffi/` 변경에 같은 diff의 ADR을 요구합니다. 이 ADR은 Phase XII 셸 통합 작업이 추가하는
C ABI를 작업별로 기록합니다. 기존 함수·상태 코드의 의미는 바꾸지 않습니다.

## Decision

### XII.3.2 — macOS 입력 이벤트 전달

XII.3.1이 정의한 공통 `InputEvent`(Rust)를 macOS 셸이 코어에 넘기는 경로입니다.

1. **고정 구조체 `StellarMacEvent`(104 byte, `#[repr(C)]`)**: AppKit `NSEvent` 원시 필드(event type,
   keyCode, subtype, isARepeat, hasPreciseScrollingDeltas, modifierFlags, timestamp, clickCount,
   buttonNumber, scrollingDelta, phase, momentumPhase)와 셸 수신 시각(`receive_uptime`), 캔버스 view
   좌표(`x`, `y`)를 담습니다. 첫 필드 `version`(`STELLAR_MAC_EVENT_VERSION` = 1)이 다르거나 `reserved`가
   0이 아니면 거부합니다. pointer 필드는 없습니다. 크기·offset은 Rust 테스트와 헤더의
   `_Static_assert`/`static_assert`로 고정합니다.
2. **변환은 코어, 복사는 셸**: 셸은 AppKit 값을 가공하지 않고 복사하며, 위치만 AppKit view 변환으로
   캔버스 view 좌표(flipped view, 좌상단 원점, y 아래로 증가)로 옮깁니다. 이벤트 종류·button·modifier·
   scroll phase 해석과 virtual key code → HID usage 변환은 `crates/ffi/src/input_macos.rs`가 합니다.
   변환 규칙은 그 모듈 문서가 기준입니다(공통 집합 밖 이벤트는 대기열에 넣지 않음).
3. **`stellar_runtime_submit_mac_event(handle, const StellarMacEvent *event, uint32_t *out_queued)`**:
   변환 결과를 runtime 입력 대기열에 넣고 넣은 수(0 또는 1)를 씁니다. null `event`는
   `STELLAR_NULL_POINTER`, 규칙 위반은 `STELLAR_INVALID_ARGUMENT`(last error에 세부)입니다.
4. **`stellar_runtime_poll_input(handle, buf, buf_len, out_written)`**: 대기열의 다음 이벤트를
   `InputEvent::encode` 바이트로 읽습니다. 빈 대기열 `STELLAR_NO_ERROR`, `buf == NULL`이면 길이만 보고,
   작은 buffer는 `STELLAR_BUFFER_TOO_SMALL`(꺼내지 않음) — `stellar_runtime_poll_event`(II.4.5)와 같은
   pull 규칙입니다. 셸 검증·진단과 이후 코어 소비자를 위한 읽기 경로입니다.
5. **상태 코드 추가(끝에 덧붙임)**: `STELLAR_INVALID_ARGUMENT = 8`, `STELLAR_QUEUE_FULL = 9`.
   기존 값 0~7은 그대로입니다.
6. **대기열 용량**: `STELLAR_INPUT_QUEUE_CAPACITY` = 4096 이벤트. 가득 차면 새 입력을 버리고
   `STELLAR_QUEUE_FULL`을 반환합니다. 소비자(DocumentThread, XII.1.2)가 생기기 전에도 메모리가 무한히
   늘지 않게 하는 상한이며, UI→DocumentThread queue의 overflow 정책은 XII.1.2가 정합니다.
7. **시간 domain**: runtime마다 `InputTimeDomain`을 두고 첫 제출 이벤트의 `receive_uptime`(셸이 받은
   시각, `NSEvent.timestamp`와 같은 축)과 코어 단조 시계 현재값으로 기준점을 잡습니다. 코어 단조 시계는
   `Instant` 기반이며 기준점을 첫 `stellar_runtime_create`에서 고정합니다(runtime 생성 뒤 발생한 입력이
   0으로 눌리지 않게). wall clock은 쓰지 않습니다(II.3.3).

### XII.3.4 — 입력 대기열 coalescing

1. **끝 항목과만 병합**: runtime 입력 대기열(`crates/ffi/src/input_queue.rs`)은 새 이벤트를 아직 꺼내지 않은
   마지막 항목과만 병합합니다. pointer down/up·cancel, key, text composition, focus, resize는 병합하지 않으므로
   이들의 상대 순서와 앞뒤 move/wheel과의 순서는 바뀌지 않습니다.
2. **pointer move**: 둘 다 `Move`이고 button·device·modifier가 같으면 최신 이벤트(위치·시각)만 남깁니다.
3. **wheel**: 같은 phase(`Changed`·`Momentum`·`Discrete`)·단위·modifier면 delta를 더하고 위치·시각은 최신 값을
   씁니다. `Began`·`Ended`는 gesture 경계라 병합하지 않고, `f32` 합이 비유한이면 병합하지 않습니다.
4. **`out_queued` 의미**: 시그니처는 그대로이고, 끝 항목에 병합된 이벤트도 "대기열에 반영"으로 보아 1을 씁니다
   (무시된 이벤트만 0). 헤더 주석을 이에 맞췄습니다.
5. **용량**: `STELLAR_INPUT_QUEUE_CAPACITY`는 항목 수입니다. 가득 찬 상태에서도 끝 항목과 병합되는 move/wheel은
   받고, 병합할 수 없는 이벤트만 `STELLAR_QUEUE_FULL`로 버립니다.

## Alternatives

- **Swift에서 `InputEvent` 바이트를 직접 인코딩**: 인코더가 두 벌이 되고 변환 규칙을 Rust 테스트·CI에서
  검증할 수 없습니다. 채택하지 않습니다.
- **필드별 scalar 인자 함수**: 인자가 많고 버전을 붙일 곳이 없어 §8.2의 버전형 메시지 원칙에 맞지
  않습니다. 채택하지 않습니다.
- **공유 메모리 ring buffer**: §8.2가 허용하지만 소비자 thread(XII.1.x)가 없는 지금은 이점이 없습니다.
  DocumentThread 도입 시 재검토합니다.
- **셸이 코어 callback을 받아 push**: II.4.5가 callback 재진입을 금지합니다. 셸→코어 방향은 동기
  호출로 충분합니다.
- **대기열 전체를 훑어 같은 종류를 병합(XII.3.4)**: 사이에 낀 down/up·key를 건너뛰어 병합하면 순서가
  바뀝니다. 끝 항목 병합만 채택합니다.
- **wheel도 최신 값만 유지(XII.3.4)**: scroll 거리가 사라집니다. delta 합을 채택합니다.

## Consequences

- 긍정: macOS 이벤트 해석이 순수 Rust 함수라 golden fixture로 결정론적으로 검증되고, Windows 셸이
  같은 대기열·`poll_input`을 재사용할 수 있습니다.
- 긍정: 구조체 크기·버전 검사로 셸과 코어의 레이아웃 불일치를 실행 전에 잡습니다.
- 부정: 소비자가 없는 동안 실행 중인 앱의 대기열은 병합할 수 없는 이벤트(click·key 등)가 4096개 쌓이면
  `STELLAR_QUEUE_FULL`로 입력을 버립니다. 연속 move/wheel은 병합되어 상한에 닿지 않습니다(XII.3.4).
- 부정: 병합된 move의 중간 위치와 개별 시각은 남지 않습니다. 입력 지연(T0) 측정은 XII.3.5에서 병합 전
  수신 시각 기준으로 정합니다.
- 부정: macOS 전용 구조체가 공통 헤더에 들어갑니다. Windows 재개 시 별도 구조체를 추가합니다.

## Compatibility

- 추가만 있습니다. 기존 함수 시그니처·상태 코드 값·헤더 선언 순서는 바꾸지 않았습니다.
- `StellarMacEvent` 레이아웃이 바뀌면 `STELLAR_MAC_EVENT_VERSION`을 올리고 이전 버전은 거부합니다.
- 입력 바이트 레이아웃은 XII.3.1의 `INPUT_EVENT_SCHEMA_VERSION`(1)을 그대로 따릅니다.
- 파일 포맷·저장 구조에는 영향이 없습니다.

## Performance

- 제출 1회는 registry mutex 1회, 104 byte 복사, O(1) 변환과 끝 항목 1개 비교(병합)입니다. 입력 전달 단계 예산(§6.2, 목표
  0.15ms)에 대한 측정은 T0/T1 trace 작업(XII.3.5)에서 합니다.

## Security

- 구조체의 모든 필드를 검증합니다(버전, 예약 0, 0/1 bool, 유한 좌표·delta·timestamp, click count,
  scroll phase bit). 실패는 상태 코드와 handle별 last error로만 보고하고 panic은 `ffi_guard`가 막습니다.
- 구조체에 pointer가 없어 셸이 코어에 임의 메모리 참조를 넘기지 않습니다.

## Migration

- 새 ABI이므로 기존 데이터·코드 마이그레이션은 없습니다. 셸은 재생성한 `stellar_ffi.h`를 사용합니다.
