# 작업 보고서: III.3.4 문자열 값과 정규화 경계

- WBS: III.3.4
- 작업명: 문자열 UTF-8 + NFC 정규화 적용 위치 분리(사용자 텍스트 보존, 식별자·경로만 정규화)
- 상태: 완료 (`[x]`)
- 선행: G1 (Phase II 게이트, II.6/II.7.2/II.7.3 연기 — 사용자 지시로 Phase III 우선)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/text.rs` (`Normalizer`, `IdentityNormalizer`, `UserText`, `NormalizedText`, 단위테스트)
- `crates/core-types/src/lib.rs` (`pub mod text;`)
- `docs/task-reports/III.3.4.md`, 계획서(III.3.4 `[ ]`→`[x]`)

## 구현 요약

- 모든 문자열 값은 UTF-8(`String`).
- 정규화 적용 위치를 타입으로 분리: `UserText`는 입력을 그대로 보존(정규화기를 구조적으로 받지 않음), `NormalizedText`는 생성 시 주입된 `Normalizer`로 정규화(식별자·파일 경로용).
- `Normalizer` trait가 NFC 알고리즘의 주입 지점. core-types는 std 전용이라 NFC 테이블을 포함하지 않으며, 실제 NFC 정규화기는 상위 계층에서 주입.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 사용자 텍스트 보존(분해형 시퀀스 그대로, code point 수 유지) | 통과 |
| 정규화 텍스트가 정규화기 적용(e+◌́ → é) | 통과 |
| 분리 검증(같은 입력: 사용자 보존 vs 경로 정규화 결과 상이) | 통과 |
| IdentityNormalizer no-op | 통과 |
| `cargo test -p stellar-core-types`(48 unit + 8 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 위험 / 한계

- 실제 Unicode NFC 알고리즘은 core-types에 포함하지 않았습니다(std 전용 정책 + NFC 테이블 미포함). NFC는 `Normalizer` trait로 상위 계층(추후 승인된 정규화 의존성)에서 주입합니다. 본 작업은 plan의 "정규화를 적용할 **위치를 분리**"에 해당하며, 알고리즘 구현은 주입 지점으로 위임합니다.

## 작업 지시와 다른 점

- 위 한계대로 NFC 알고리즘 자체는 구현하지 않고 주입 지점(`Normalizer`)으로 분리했습니다. 테스트는 NFC 한 사례를 흉내내는 stub 정규화기로 적용 동작을 검증했습니다.
