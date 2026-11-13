# 작업 보고서: XI.5.4 device lost·out-of-memory·error 분류와 복구 가능 여부

- WBS: XI.5.4
- 작업명: device lost/out-of-memory/error 분류와 복구 가능 여부 구조화
- 상태: 완료 (`[x]`)
- 선행: XI.5.1 (이 lane에서 `11a70c9`로 `[x]`), II.3.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/backend/recovery.rs`(신규) — `Recovery`, `RecoveryAction`,
  `RecoveryTracker`, 상수(`TIMEOUT_ESCALATION`, `SURFACE_LOST_ESCALATION`, `MAX_DEVICE_RECREATIONS`),
  `BackendErrorKind`의 `ALL`/`recovery`/`is_recoverable`/`code`/`from_code`/`core_error_code`/
  `user_message`, `BackendError::recovery`/`is_recoverable`, mock error matrix 테스트.
- `crates/renderer-core/src/backend/call_order.rs` — device 재생성 오류(`DeviceLost`·`Internal`)를
  `Lost` 상태로 처리(기존: `DeviceLost`만).
- `crates/renderer-core/src/backend/mod.rs` — 모듈 등록, 호출 순서 문서 갱신.
- `docs/task-reports/XI.5.4.md`(신규), 계획서(XI.5.4 `[ ]`→`[x]`).

## 구현 요약

- **분류표**: 9개 `BackendErrorKind`마다 기본 복구(`Recovery`)와 복구 가능 여부를 고정했습니다.

  | 종류 | 기본 복구 | 복구 가능 |
  |---|---|---|
  | `Timeout` | 다음 frame 재시도 | 예 |
  | `SurfaceLost` | surface 재구성(`resize`) | 예 |
  | `OutOfDeviceMemory`·`OutOfHostMemory` | cache purge 후 한 번만 재시도 | 예 |
  | `DeviceLost`·`Internal` | device 재생성(`init`) | 예 |
  | `InvalidUsage` | frame skip + 보고 | 예 |
  | `Unsupported`·`PipelineCreation` | renderer 비활성화 + 보고 | 아니요 |

- **escalation(`RecoveryTracker`)**: 성공 frame 없이 같은 오류가 이어지면 단계를 올립니다. timeout
  3회 → surface 재구성, surface lost 2회 → device 재생성, device 재생성 3회 초과 → 비활성화. 메모리
  부족은 purge 후 재시도가 정확히 한 번이고 재시도도 실패하면 frame skip입니다(반복 loop 없음,
  XVI.5.6 "retry count=1"과 같은 규칙). 비활성화는 이후 성공 frame으로 풀리지 않습니다.
- **코드**: 렌더러 오류 코드 `0x0B01`~`0x0B09`를 golden으로 고정했습니다(진단·telemetry는 이 코드만
  사용, 아키텍처 §39). II.3.1 `ErrorCode` 숫자 분류는 모두 `Internal`(1)이고, 사용자 메시지는 내용·경로
  없는 정적 문구입니다. runtime이 이 값으로 `StellarError`를 만듭니다(renderer-core는 core-types를 직접
  import할 수 없음, II.2.8).
- **validator 연동**: device 재생성이 필요한 오류(`DeviceLost`·`Internal`)는 `CallOrderValidator`를
  `Lost`로 바꿔 다음 호출을 `init`으로 제한합니다. matrix 테스트가 `Internal` 오류 뒤 살아 있는 device
  재 init이 거부되는 틈을 찾아내 이렇게 고쳤습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 67 passed, 0 failed; doctest 3 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| mock error matrix 13 호출 × 9 오류 = 117 경우: 오류 종류 그대로 전달, device 재생성 오류만 `Lost`, 어떤 오류 뒤에도 frame이 열린 채 남지 않음, 분류된 조치 수행 후 복구 가능 오류는 다음 frame 제출 성공, 복구 불가 오류만 비활성화, panic 없음(`mock_backend_error_matrix`) | 통과 |
| 코드·복구·복구 가능 여부 golden, 코드 역변환, `ErrorCode` 분류, 사용자 메시지(`classification_and_codes_are_stable`) | 통과 |
| 메모리 부족 재시도 정확히 1회 후 frame skip, 다음 작업은 다시 1회(`memory_errors_retry_exactly_once`) | 통과 |
| timeout·surface lost·device lost escalation, 성공 frame 초기화, 비활성화 유지(`repeated_errors_escalate`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XI.5.1은 이 lane에서 완료, II.3.1은 `[x]`입니다. 완화 적용은 없습니다.
- `StellarError` 변환 자체는 crate 의존 정책상 runtime 몫이라, 여기서는 변환에 필요한 숫자 분류·코드·
  사용자 메시지만 제공합니다.
- `Internal`을 device 재생성 대상으로 분류했습니다. driver 내부 오류 뒤 device 상태를 신뢰할 수 없기
  때문입니다. 이에 따라 XI.5.1 validator가 `Internal`도 `Lost`로 처리합니다.

## 남은 위험 / 한계

- escalation 임계값(3·2·3)은 초기값입니다. 실제 Metal 오류 빈도(XI.8.7)를 보고 조정할 수 있습니다.
- cache purge 자체(어떤 cache를 비울지)는 XVI.5 cache priority 정책이 정합니다. 여기서는 조치 종류만
  정합니다.
