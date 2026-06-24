# 작업 보고서: II.3.1 ErrorCode·StellarError

- WBS: II.3.1
- 작업명: core-types에 `ErrorCode` enum과 `StellarError { code, message, context }`
- 상태: 완료 (`[x]`)
- 선행: II.2.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/core-types/src/error.rs` (신규)
- `crates/core-types/src/lib.rs` (`pub mod error;`)
- `docs/task-reports/II.3.1.md`, 계획서(II.3.1 `[ ]`→`[x]`)

## 구현 요약

- `ErrorCode`(`#[repr(u32)]`, 안정적 숫자 1~6) + `as_u32`/`from_u32`(round-trip).
- `StellarError { code, message(사용자), context(개발자) }` — 사용자 문자열과 개발자 context 분리.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| numeric code golden(1~6) | 통과 |
| code round-trip / 알 수 없는 값 None | 통과 |
| 사용자/개발자 필드 분리 | 통과 |
| `cargo test -p stellar-core-types` / fmt / clippy --locked | 통과(3 tests) |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 외부 serde 미사용(부트스트랩 정책)으로 직렬화는 숫자 코드 기반 수동 round-trip으로 구현했습니다.
