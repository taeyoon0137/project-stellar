# ADR-0009: FFI C ABI 기준선

## Status

Accepted (소급, 2026-11-14). 이 ADR은 이미 구현된 결정을 소급 기록합니다. 아래 커밋은
보호 경로 `crates/ffi/`(I.3.5)를 변경했지만 같은 diff에 ADR이 없었습니다. 새 결정은 추가하지
않습니다.

## Context

- 기술 아키텍처 §8.2는 공통 코어와 셸 경계에 안정적인 C ABI 또는 생성된 FFI 바인딩을 쓰고, Rust
  내부 타입을 직접 노출하지 않으며, handle·POD 데이터·명시적 수명 관리를 사용하라고 요구합니다.
- panic 정책(II.1.3)은 release `panic = "abort"`와 FFI 경계에서 panic 미전파를 요구합니다.
- ADR 없이 `crates/ffi/`를 변경한 커밋은 다음과 같습니다.

| WBS | 커밋 | 내용 |
|---|---|---|
| II.2.7 | `6bfa4e1` | ffi crate 골격(core-types만 직접 의존) |
| II.4.1 | `aadc6e1` | runtime create/destroy/version C ABI |
| II.4.2 | `56847ee` | `StellarStatus`, handle별 last error |
| II.4.3 | `c5ec428` | 문자열 입력·출력 규칙 |
| II.4.4 | `b07b2c2` | 경계 `catch_unwind` |
| II.4.5 | `e5584a1` | Core→Shell pull 이벤트 큐 |
| II.4.6 | `ce9b66a` | C 헤더 생성과 drift 검사 |
| II.5.4 | `1a6c5ef` | drawable 크기·scale 전달 함수 |

## Decision

handle과 수명:

- handle은 opaque pointer가 아니라 정수 `uint64_t` registry handle입니다. `0`은 무효입니다. 전역
  registry가 handle → runtime을 소유합니다(II.4.1).
- `stellar_runtime_create(uint64_t *out_handle)`가 생성하고 `stellar_runtime_destroy(handle)`가
  파괴합니다. 알 수 없는 handle과 이중 파괴는 `STELLAR_INVALID_HANDLE`입니다(II.4.1, II.4.2).
- crate는 `lib`와 `staticlib`를 함께 생성합니다. macOS C 링크에는 `-framework CoreFoundation
  -framework Security`가 필요합니다(II.4.1).

상태 코드와 오류:

- 진단용 `stellar_runtime_live_count`를 뺀 모든 함수는 `StellarStatus`(repr `i32`)를 반환합니다.
  값은 `OK=0`, `INVALID_HANDLE=1`, `NULL_POINTER=2`, `BUFFER_TOO_SMALL=3`, `NO_ERROR=4`,
  `INTERNAL=5`, `INVALID_UTF8=6`, `INTERNAL_PANIC=7`이며 재사용·변경하지 않습니다(II.4.2~II.4.4).
- 오류 세부 정보는 전역이 아니라 각 runtime handle에 보관하고 `stellar_last_error(handle, …)`로
  읽습니다(II.4.2).

문자열:

- 입력은 UTF-8 `(ptr, len)`입니다. `len == 0`은 빈 문자열(ptr 무시), null + `len > 0`은
  `NULL_POINTER`, 비 UTF-8은 `INVALID_UTF8`입니다. NUL 종료에 의존하지 않으며 중간 NUL을
  보존합니다(II.4.3).
- 출력은 호출자 buffer `(buf, buf_len, out_written)`입니다. buffer가 작으면 `BUFFER_TOO_SMALL`,
  `buf = NULL`이면 필요한 길이만 보고합니다(II.4.1, II.4.3).

panic:

- 모든 `extern "C"` 함수를 `catch_unwind` guard로 감싸 panic을 `INTERNAL_PANIC`으로 바꿉니다.
  정상 경로는 panic 없이 상태를 반환합니다(II.4.4, release abort 정책과 정합).

이벤트:

- Core→Shell 이벤트는 handle별 FIFO 큐입니다. 셸은 `stellar_runtime_poll_event`로 pull합니다. 빈
  큐는 `NO_ERROR`, buffer 부족은 `BUFFER_TOO_SMALL`이며 이 경우 이벤트를 꺼내지 않습니다(II.4.5).
- callback 등록 API를 두지 않아 Core→Shell 재진입을 구조적으로 막습니다(II.4.5).
- 이벤트 push는 Rust 전용 API(`enqueue_event`)이며 C ABI가 아닙니다.

헤더:

- C 헤더 `crates/ffi/include/stellar_ffi.h`는 생성 파일입니다. 원본은 `tools/xtask/src/ffi_header.rs`의
  고정 템플릿이고 `cargo xtask generate-ffi-header`로 생성합니다. 외부 생성 도구는 쓰지 않습니다
  (승인 의존성 정책). CI는 재생성 후 Git diff가 없어야 통과합니다(II.4.6).
- 헤더는 C++ `extern "C"` guard를 포함합니다.

셸 전달:

- `stellar_runtime_set_drawable_size(handle, pixel_width, pixel_height, scale)`와
  `stellar_runtime_drawable_size(…)`는 셸이 계산한 backing 픽셀 크기와 scale을 전달·조회합니다.
  미설정 조회는 `NO_ERROR`입니다. 렌더링은 하지 않습니다(II.5.4).

정하지 않은 사항:

- 헤더에는 명시적 ABI 버전 심볼이 없습니다. §8.2의 버전형 메시지와 ABI 버전 협상 방식은 정하지
  않았습니다.
- `stellar_runtime_version`이 반환하는 문자열 형식은 이 ADR이 계약으로 고정하지 않습니다.

## Alternatives

- opaque pointer handle: 해제 후 사용·이중 해제를 셸 쪽에서 막기 어렵습니다. 정수 handle은 registry
  조회로 무효 handle을 상태 코드로 돌려줍니다. 미선택.
- 전역 last error(errno 방식): 여러 handle 사이에서 오류가 섞입니다(II.4.2 검증 기준). 미선택.
- callback 기반 이벤트: Core→Shell 재진입과 스레드 문제를 만듭니다. 미선택.
- 외부 헤더 생성 도구: 승인 의존성 목록 밖입니다. 미선택.

## Consequences

- 셸은 handle과 POD 인자만 다루고 Rust 타입 레이아웃에 의존하지 않습니다.
- 모든 호출은 상태 코드로 끝나므로 panic이 셸 프로세스를 종료시키지 않습니다.
- 헤더와 Rust 표면의 정합은 고정 템플릿, 함수명 노출 단위테스트, 검토로 보장합니다. 자동 생성보다
  수동 동기화 부담이 있습니다.

## Compatibility

- FFI: 함수 시그니처와 `StellarStatus` 숫자는 셸 바인딩이 의존하는 계약입니다. 함수 추가는 헤더
  재생성과 drift 검사로 진행하고, 기존 시그니처·숫자 변경은 새 ADR이 필요합니다.
- 파일 포맷: 영향 없음.

## Performance

- 각 호출은 registry 잠금과 handle 조회를 포함합니다. §8.2에 따라 대규모 문서 데이터는 FFI 왕복이
  아니라 Snapshot·Command 단위로 전달합니다. 성능 예산 파일은 변경하지 않습니다.

## Security

- pointer 인자는 역참조 전에 null을 검사합니다. 문자열 입력은 UTF-8 검증 후 사용합니다.
- buffer 출력은 `buf_len`을 넘지 않습니다.

## Migration

- 이미 구현된 상태를 기록하므로 마이그레이션은 없습니다.
- 기존 함수의 시그니처를 바꿔야 하면 새 이름의 함수를 추가하고, 기존 함수의 제거 시점은 새 ADR로
  정합니다.
