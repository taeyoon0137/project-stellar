# 작업 보고서: VIII.7.1 환경 입력 key registry

- WBS: VIII.7.1
- 작업명: environment key registry에 platform, OS/API version, color scheme, contrast, reduced motion, viewport, locale,
  density, interaction state 정의
- 상태: 완료 (`[x]`)
- 선행: VIII.1.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/environment.rs`(신규) — key 상수 11개, `ALL_KEYS`, 환경 enum 타입 4개와 variant 번호,
  `EnvironmentKeyInfo`, `key_info`, `key_by_name`, `EnvironmentTypes`와 golden 테스트.
- `crates/expressions/src/lib.rs` — `environment` 모듈 등록.
- `crates/expressions/src/ast.rs` — `EnvironmentKey` 문서 주석이 새 registry를 가리키도록 수정(코드 변경 없음).
- `docs/task-reports/VIII.7.1.md`(신규), 계획서(VIII.7.1 `[ ]`→`[x]`).

## 구현 요약

- **key**(numeric ID → 이름, 값 타입): 1 platform(Enum), 2 os_version(Number), 3 api_level(Number), 4 color_scheme(Enum),
  5 contrast(Enum), 6 reduced_motion(Boolean), 7 viewport_width(Length), 8 viewport_height(Length), 9 locale(String,
  BCP 47), 10 density(Number), 11 interaction_state(Enum). ID는 canonical AST 형식(VIII.2.5)에 기록되는 값이라 고정입니다.
- **환경 enum**: `0xE000_0001`~`0xE000_0004` 대역의 `EnumTypeId`와 variant 번호(platform: generic·web·apple·android,
  color_scheme: light·dark, contrast: standard·high, interaction_state: default·hover·pressed·focused·disabled).
  platform 계열은 Target Profile 종류(§17.1 Web·Apple·Android)에 "profile 없음"을 더했습니다.
- **타입 제공**: `EnvironmentTypes`가 환경 참조를 `Required(value_type)`으로 type 검사에 제공합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (125 unit + 9 integration + 3 doctest passed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **key numeric ID golden test**(`key_numeric_ids_match_golden`): 11개 key의 `ID 이름 타입 variant수` 표가 고정 golden과 일치, enum variant 번호·enum 타입 ID 고정 | 통과 |
| 미등록 key `None`, 이름 조회, ID 1부터 빈틈없음·이름 유일, 타입 제공자(`registry_lookup_and_types`) | 통과 |

## 차이(작업 지시와 다른 점)

- "OS/API version"을 `os_version`(2)과 `api_level`(3) 두 key로 나눴습니다(Apple OS 버전과 Android API level, §17.1).
- 새 numeric registry(key ID, 환경 enum 타입 ID 대역)를 정의합니다. `crates/expressions`는 ADR 보호 경로가 아니지만
  registry 결정 기록(ADR)은 lane 허용 경로(`docs/adr/` 제외) 밖이라 작성하지 않았습니다. 후속 ADR이 필요합니다.

## 남은 위험 / 한계

- missing key 기본값 정책은 VIII.7.2에서 이 registry에 추가합니다.
- 다른 enum registry(문서 enum 등)가 생길 때 `0xE000_0000` 대역 예약을 지켜야 합니다(현재 저장소에 다른 enum 타입 ID
  registry 없음).
