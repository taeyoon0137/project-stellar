# 작업 보고서: II.4.3 FFI 문자열 규칙

- WBS: II.4.3
- 작업명: UTF-8 ptr+length 입력, 호출자 buffer 출력, NUL 종료 비의존
- 상태: 완료 (`[x]`)
- 선행: II.4.2 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/ffi/src/lib.rs` (decode_utf8_owned + set_label/label, StellarStatus::InvalidUtf8)
- `docs/task-reports/II.4.3.md`, 계획서(II.4.3 `[ ]`→`[x]`)

## 구현 요약

- 입력: `(ptr, len)` UTF-8. `len==0`이면 빈 문자열(ptr 무시), null+len>0이면 `NullPointer`, 비UTF-8이면 `InvalidUtf8`. NUL 종료에 의존하지 않음(중간 NUL 보존).
- 출력: 호출자 buffer(`buf, buf_len, out_written`), 작으면 `BufferTooSmall`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| embedded NUL round-trip(len 5) | 5바이트 보존 |
| 빈 문자열(len 0, null ptr) | Ok, 0바이트 |
| 잘못된 UTF-8 | InvalidUtf8 |
| cargo test / fmt / clippy --locked | 통과(6 tests) |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 문자열 입력/출력 규칙 검증을 위한 대표 함수로 set_label/label을 추가했습니다.
