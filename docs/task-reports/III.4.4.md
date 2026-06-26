# 작업 보고서: III.4.4 Shadow와 blur parameter 타입

- WBS: III.4.4
- 작업명: `BlurRadius`(음수·비정상 거부)·`Shadow` 정의, 경계값 검증
- 상태: 완료 (`[x]`)
- 선행: III.3.1(`FiniteF32`), III.4.1(`Color`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/shadow.rs` (신규: `BlurRadius`, `BlurError`, `Shadow`, `ShadowError`, 경계값·round-trip 테스트)
- `crates/core-types/src/value.rs` (`Vec2`에 `PartialOrd, Ord` 파생 추가 — canonical 비교용)
- `crates/core-types/src/typed_value.rs` (`TypedValue::Shadow` placeholder → 정식 `Shadow` 교체, `ValueDecodeError::Shadow`)
- `crates/core-types/src/lib.rs` (`pub mod shadow;`)
- `docs/task-reports/III.4.4.md`(신규), 계획서(III.4.4 `[ ]`→`[x]`)

## 구현 요약

- `BlurRadius`: blur·확산용 radius parameter. **음수와 비유한 값을 거부**하고 `0`은 허용
  (`-0.0`→`0.0` 정규화). 경계값은 `0`(허용)과 음수(거부).
- `Shadow`: `offset`([`Vec2`], 음수 허용) + `BlurRadius` + `Color`. 고정 29바이트
  (offset 8 + blur 4 + color 17) round-trip. `from_bytes`는 비유한 offset, 음수·비유한 blur,
  잘못된 색상을 거부.
- `TypedValue::Shadow`를 III.3.5 opaque placeholder에서 정식 `Shadow`로 교체. type tag
  `0x0B` 유지(고정 길이라 Color처럼 길이 프리픽스 없이 인라인). 디코드 실패는
  `ValueDecodeError::Shadow`.
- `Vec2`가 `Ord`를 갖지 않아 `Shadow`/`TypedValue`의 `Ord` 파생이 불가능했습니다. `Vec2`
  성분이 `FiniteF32`(전순서)이므로 `(x, y)` 사전식 전순서를 파생하도록 `PartialOrd, Ord`를
  추가했습니다(canonical 비교 일관성).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-core-types` (93 unit + 8 doctest) | 0 |
| `cargo fmt -p stellar-core-types -- --check` | 0 |
| `cargo clippy -p stellar-core-types --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| BlurRadius 경계값: `0`·`-0.0` 허용 | 통과 |
| BlurRadius 음수(`-0.001`,`-1.0`)·비유한(NaN·Inf) 거부 | 통과 |
| Shadow round-trip(음수 offset 포함), `0` blur round-trip | 통과 |
| Shadow from_bytes: 음수 blur·비유한 blur·비유한 offset·잘못된 색상 거부 | 통과 |
| `TypedValue::Shadow` 정식 타입 round-trip(tag `0x0B`) 및 음수 blur payload 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- III.4.1~III.4.3과 동일하게 `TypedValue::Shadow` placeholder를 정식 `Shadow`로 교체했습니다
  (tag `0x0B` 유지). 이제 남은 placeholder는 Typography(VI.7.2/X.8.1)뿐입니다.
- `Vec2`에 `Ord` 파생을 추가했습니다. III.3.3의 기존 Vec2 동작(생성·접근·round-trip)은
  바뀌지 않으며, 비교 가능성만 확장됩니다(전체 테스트 통과 확인).
- spread radius·그림자 종류(inner/outer)는 본 작업 범위에 포함하지 않았습니다(지시는 blur
  parameter와 radius 거부에 한정). 필요 시 후속 작업에서 확장합니다.

## 남은 위험 / 한계

- `Shadow`는 단일 그림자 값입니다. 다중 그림자·inner shadow·spread는 상위(effect/scene)
  타입에서 다룹니다.
- blur radius의 상한(비정상적으로 큰 값)은 두지 않았습니다. "비정상"은 비유한·음수로 한정해
  거부하며, 과대값은 유한한 한 허용합니다(렌더 단계에서 clamp 가능).
