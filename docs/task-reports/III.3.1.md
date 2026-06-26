# 작업 보고서: III.3.1 FiniteF32

- WBS: III.3.1
- 작업명: `FiniteF32`로 NaN·±Infinity를 생성·직렬화 단계에서 거부, `-0.0`을 `0.0`으로 정규화
- 상태: 완료 (`[x]`)
- 선행: G1 (Phase II 게이트, II.6/II.7.2/II.7.3 연기 — 사용자 지시로 Phase III 우선)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/value.rs` (`FiniteF32`, `NonFiniteError`, 단위테스트)
- `crates/core-types/src/lib.rs` (`pub mod value {}` → `pub mod value;`)
- `docs/task-reports/III.3.1.md`, 계획서(III.3.1 `[ ]`→`[x]`)

## 구현 요약

- `FiniteF32(f32)`. `new`가 `is_finite()` 검사로 NaN·±Inf를 `NonFiniteError`로 거부, `value == 0.0`(±0 모두 참)일 때 `+0.0`으로 정규화.
- 직렬화: `to_le_bytes`/`from_le_bytes`. 복원 단계에서도 `new`를 거쳐 비유한 비트 패턴 거부.
- NaN이 없고 -0.0이 정규화되어 `Eq`/`Ord`(전순서)/`Hash`(to_bits, Eq와 일관)를 직접 구현.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| NaN/+Inf/-Inf 생성 거부 | 통과 |
| -0.0 → +0.0 정규화(값·비트·Eq·Hash) | 통과 |
| 정상 값(1.5, -2.5) 보존 | 통과 |
| 전순서 정렬 | 통과 |
| 직렬화 round-trip + 비유한 바이트(NaN/Inf) 복원 거부 | 통과 |
| `cargo test -p stellar-core-types`(37 unit + 6 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 거부는 `Result<_, NonFiniteError>`로 표현했습니다("거부" 의미를 명시).
