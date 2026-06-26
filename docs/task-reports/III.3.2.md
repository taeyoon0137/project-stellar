# 작업 보고서: III.3.2 단위 newtype

- WBS: III.3.2
- 작업명: `Length`, `Percentage`, `Angle`, `Duration`, `Scalar`를 별도 newtype으로 구현(잘못된 단위 산술 금지)
- 상태: 완료 (`[x]`)
- 선행: III.3.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/value.rs` (`define_unit!` 매크로 + 5개 단위 newtype, `saturating_finite`, compile_fail doctest, 단위테스트)
- `docs/task-reports/III.3.2.md`, 계획서(III.3.2 `[ ]`→`[x]`)

## 구현 요약

- 5개 단위는 각각 `FiniteF32`를 감싸는 별도 newtype(`define_unit!` 매크로로 정의). `new`/`get`/`raw`.
- `Add`/`Sub`는 **같은 단위끼리만** 구현 — 다른 단위 산술·대입은 컴파일 실패. 오버플로(±무한대)는 `saturating_finite`로 유한 범위(`f32::MAX`/`MIN`)에 고정(finite±finite는 NaN 불가).
- compile_fail doctest 2건: (1) `Length + Percentage`, (2) `Length`→`Angle` 대입.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 같은 단위 Add/Sub(Length 10±2.5) | 통과 |
| 비유한 값 거부, -0.0 정규화 | 통과 |
| 오버플로 saturate(MAX+MAX=MAX, 유한) | 통과 |
| 잘못된 단위 산술·대입 compile-fail(2 doctest) | 통과 |
| `cargo test -p stellar-core-types`(40 unit + 8 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 같은 단위 `Add`/`Sub`의 오버플로는 saturate(문서화)로 처리해 `FiniteF32` 불변식을 유지했습니다.
