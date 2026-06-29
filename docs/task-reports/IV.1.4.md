# 작업 보고서: IV.1.4 scalar 암묵 타입 추론 제한

- WBS: IV.1.4
- 작업명: 스칼라 암묵 타입 추론 제한(YAML 1.1 혼동 차단), 엄격 타입 해석 primitive
- 상태: 완료 (`[x]`)
- 선행: IV.1.2(제한 loader)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/yaml/scalar.rs` (신규: `parse_bool`, `parse_u32` 엄격 해석기 + 테스트)
- `crates/project/src/yaml/mod.rs` (`scalar` 모듈 공개)
- `crates/project/src/manifest.rs` (`schema_version`을 `scalar::parse_u32`로 해석; end-to-end 혼동 fixture)
- `docs/task-reports/IV.1.4.md`(신규), 계획서(IV.1.4 `[ ]`→`[x]`)

## 구현 요약

- 제한 loader는 모든 스칼라를 문자열로 보존합니다(암묵 타입 없음, IV.1.2에서 확인). IV.1.4는
  타입이 필요한 곳에서 쓰는 **엄격 해석 primitive**를 추가하고, YAML 1.1 혼동을 차단합니다.
- `scalar::parse_bool`: 정확히 `true`/`false`만 불리언. `yes`/`no`/`on`/`off`/`y`/`n`·대문자
  변형은 `None`.
- `scalar::parse_u32`: canonical base-10만. leading-zero(`010`)·진법 접두(`0x`/`0o`/`0b`)·
  부호·`_`·소수점·공백·`yes`·범위 초과는 `None`(`0`은 허용).
- `manifest.rs`의 `schema_version` 해석을 `scalar::parse_u32`로 통일(엄격성 단일 출처).
- 실패 시 `None`을 반환해 호출부가 "문자열 보존" 또는 "명시 오류"를 선택할 수 있게 했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (34 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| YAML 1.1 혼동 fixture | 기대 처리 | 결과 |
|---|---|---|
| `id: yes` | 문자열 "yes"(불리언 아님) | 통과 |
| `name: 2020-01-01` | 문자열(날짜 아님) | 통과 |
| `id: 010` | 문자열 "010"(8진수/정수 아님) | 통과 |
| `platform: on` | 문자열 "on"(불리언 아님) | 통과 |
| `schema_version: yes/01/0x1/1_0/true/1.0` | `InvalidSchemaVersion` 명시 오류 | 통과 |
| `parse_bool` yes/no/on/off/True/TRUE/1/0 | `None` | 통과 |
| `parse_u32` 01/0x1F/+5/-5/1_000/1.0 | `None` | 통과 |

## 차이(작업 지시와 다른 점)

- 지시의 "문자열로만 허용하거나 명시 오류 처리"를 둘 다 구현했습니다: 문자열 필드는 혼동값을
  원문 보존(loader), 타입 필드(schema_version)는 혼동값을 명시 오류로 거부.
- `parse_bool`은 현재 매니페스트에 불리언 필드가 없어 사용처가 없지만, IV.1.4의 "yes/no" 차단을
  직접 구현한 primitive로서 추가했습니다(향후 불리언 필드가 동일 규칙을 재사용).

## 남은 위험 / 한계

- 부동소수·날짜 전용 타입 해석기는 아직 없습니다. 매니페스트에 그런 타입 필드가 생기면 동일한
  엄격 원칙으로 `scalar`에 추가합니다.
