# 작업 보고서: III.1.1 StableId (UUIDv7)

- WBS: III.1.1
- 작업명: `core-types`에 128비트 `StableId`(UUIDv7 생성 + 16바이트 직렬화, 표준 소문자 hyphen UUID 문자열)
- 상태: 완료 (`[x]`)
- 선행: G1 (Phase II 게이트) — 주: II.6/II.7.2/II.7.3(Windows 의존)은 환경 대기로 연기됨. 사용자 지시로 Phase III를 우선 진행.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/ids.rs` (`StableId`, `ParseStableIdError`, 단위테스트)
- `crates/core-types/src/lib.rs` (`pub mod ids {}` → `pub mod ids;`)
- `docs/task-reports/III.1.1.md`, 계획서(III.1.1 `[ ]`→`[x]`)

## 구현 요약

- `StableId([u8; 16])`. UUIDv7(RFC 9562) 레이아웃: 상위 48비트 ms timestamp, version 7, variant 0b10. 상위가 timestamp라 `Ord`(바이트 순)가 곧 시간 순 정렬.
- 외부 의존성 없이(std 전용) 구현. `new_v7(unix_ts_ms, rand_a, rand_b)`가 version/variant 비트를 항상 RFC에 맞게 설정하고 timestamp(48비트)·rand_a(12비트)·rand_b(62비트)를 마스킹.
- 직렬화: `from_bytes`/`to_bytes`(16바이트), `Display`(소문자 8-4-4-4-12), `FromStr`(대소문자 입력 허용·소문자 출력). 접근자 `version`/`variant`/`timestamp_ms`.
- 결정적 생성기(III.1.2)가 timestamp·random을 주입할 수 있도록 생성자를 입력 기반으로 분리.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| RFC 형식(version 7, variant 0b10, 36자·hyphen 위치, version/variant nibble) | 통과 |
| 시간 순 정렬(이른 timestamp < 늦은 timestamp, sort) | 통과 |
| binary round-trip(from_bytes/to_bytes) | 통과 |
| string round-trip(parse(to_string)), 대문자 입력 파싱, 잘못된 입력 거부 | 통과 |
| timestamp 보존·48비트 마스킹 | 통과 |
| `cargo test -p stellar-core-types`(13 unit + 2 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- "UUIDv7 생성"은 시스템 시계·엔트로피를 직접 끌어오는 대신 입력 기반 `new_v7`로 구현했습니다(core-types의 std 전용 정책 + III.1.2 결정적 생성기 주입을 위해). 시스템 시간/엔트로피를 쓰는 생성기는 상위 계층(또는 III.1.2)에서 이 생성자를 호출합니다.
