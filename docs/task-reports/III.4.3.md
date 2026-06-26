# 작업 보고서: III.4.3 Gradient stop과 저장/시각 순서 분리

- WBS: III.4.3
- 작업명: `GradientStop`(안정 ID·position·color)·`Gradient` 정의, 저장 순서와 시각 순서 분리
- 상태: 완료 (`[x]`)
- 선행: III.1.1(`StableId`), III.4.1(`Color`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/gradient.rs` (신규: `GradientStop`, `Gradient`, `GradientError`, round-trip·순서 테스트)
- `crates/core-types/src/typed_value.rs` (`TypedValue::Gradient` placeholder → 정식 `Gradient` 교체, `ValueDecodeError::Gradient`)
- `crates/core-types/src/lib.rs` (`pub mod gradient;`)
- `docs/task-reports/III.4.3.md`(신규), 계획서(III.4.3 `[ ]`→`[x]`)

## 구현 요약

- `GradientStop`: 안정 `StableId` + `position`(`FiniteF32`, 유한) + `Color`. 고정 37바이트
  직렬화(id 16 + position 4 + color 17).
- `Gradient`: stop의 **저장 순서**(`stops()`, 입력 `Vec` 순서)를 보존. **시각 순서**
  (`visual_order()`)는 저장 순서와 분리되어 `(position, 안정 ID)` 기준으로 정렬됩니다.
- 동일 position stop은 안정 ID로 tie-break 되어 순서가 고정(결정론). 저장 순서를 바꿔도
  시각 순서는 변하지 않습니다.
- 직렬화: stop 개수(u32 BE) + 각 stop. `from_bytes`는 부족·잉여 bytes, 비유한 position,
  잘못된 색상을 거부하며 bogus 큰 count도 입력 길이로 검증(미리 할당하지 않음).
- `TypedValue::Gradient`를 III.3.5 opaque placeholder에서 정식 `Gradient`로 교체. type tag
  `0x0A` 유지, 프레이밍은 길이 프리픽스 그대로. 디코드 실패는 `ValueDecodeError::Gradient`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-core-types` (85 unit + 8 doctest) | 0 |
| `cargo fmt -p stellar-core-types -- --check` | 0 |
| `cargo clippy -p stellar-core-types --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| stop·gradient round-trip(저장 순서 보존) | 통과 |
| 시각 순서가 저장 순서와 독립(position 기준 동일) | 통과 |
| 동일 position stop의 안정 순서(안정 ID 오름차순, 저장 순서 무관 결정론) | 통과 |
| 부족·잉여 bytes, 비유한 position 거부 | 통과 |
| `TypedValue::Gradient` 정식 타입 round-trip(tag `0x0A`) 및 잘못된 payload 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- III.4.1과 동일하게, III.3.5에서 예고한 대로 `TypedValue::Gradient` placeholder를 정식
  `Gradient`로 교체했습니다(tag `0x0A` 유지). 남은 placeholder는 Shadow(III.4.4)·
  Typography(VI.7.2/X.8.1)입니다.
- "저장 순서와 시각 순서 분리"를 시각 순서가 저장 순서에 의존하지 않도록 `(position, 안정 ID)`
  정렬로 해석했습니다(stable sort가 아닌 안정 ID tie-break). 동일 position의 순서가 저장
  순서와 무관하게 고정되어 결정론을 보장합니다.

## 남은 위험 / 한계

- `position`은 유한만 검증하고 `[0,1]` 범위를 강제하지 않았습니다(저작 보존·범위 정책 선점
  회피). 범위·정규화 정책이 필요하면 후속 작업에서 추가합니다.
- gradient의 종류(linear/radial/conic)·축·보간 색공간은 본 작업 범위 밖이며 scene/렌더 단계
  타입에서 다룹니다. 본 작업은 stop 집합과 순서 의미에 한정합니다.
