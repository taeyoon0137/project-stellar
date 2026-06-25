# 작업 보고서: II.4.5 Core→Shell 이벤트 큐

- WBS: II.4.5
- 작업명: pull 방식 `stellar_runtime_poll_event`, callback 재진입 금지
- 상태: 완료 (`[x]`)
- 선행: II.4.2 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/ffi/src/lib.rs` (handle별 이벤트 큐 + `enqueue_event`/`stellar_runtime_poll_event`)
- `docs/task-reports/II.4.5.md`, 계획서(II.4.5 `[ ]`→`[x]`)

## 구현 요약

- handle별 `VecDeque<(u32 kind, Vec<u8> payload)>` 큐. 코어 내부 Rust API `enqueue_event(handle, kind, payload)`로 push.
- `stellar_runtime_poll_event`로 FIFO pull. 빈 큐는 `NoError`, buffer 부족은 `BufferTooSmall`(이벤트 보존, pop 안 함), null buf는 길이 조회(out_written에 필요 길이).
- callback API를 두지 않아 Core→Shell 재진입이 구조적으로 불가능.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 빈 큐 → NoError | 통과 |
| 단일 이벤트 pull | 통과 |
| 여러 이벤트 FIFO 순서 | 통과 |
| buffer 부족 → BufferTooSmall(이벤트 보존) | 통과 |
| `cargo test -p stellar-ffi` / fmt --check / clippy --locked -D warnings | 통과(10 tests) |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- "callback 재진입 금지"는 callback 등록 API를 두지 않고 pull 전용으로 설계해 구조적으로 보장했습니다.
