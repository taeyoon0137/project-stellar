# 작업 보고서: VI.8.3 crop rect 범위 정책

- WBS: VI.8.3
- 작업명: crop rect는 normalized 0..1로 clamp 평가, 저장값 validation에서는 범위 밖 거부
- 상태: 완료 (`[x]`)
- 선행: VI.8.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/image.rs` — `CropRect::from_components`(엄격 검증),
  `CropRect::clamped`(평가용), `ImageError::CropOutOfRange`.
- `docs/task-reports/VI.8.3.md`(신규), 계획서(VI.8.3 `[ ]`→`[x]`).

## 구현 요약

- **저장값(엄격)**: `x, y ≥ 0`, `width, height > 0`, `x + width ≤ 1`,
  `y + height ≤ 1`. 범위 밖은 clamp하지 않고 `CropOutOfRange`로 거부합니다 — 파일의
  잘못된 값이 조용히 고쳐져 다시 저장되지 않게 합니다. `ImagePayload::decode`도 같은
  검증을 거칩니다.
- **평가값(clamp)**: `clamped`는 런타임 값(애니메이션·표현식 결과)을 0..1로 clamp하고
  width/height를 남은 범위로 제한합니다. 비유한 값은 전체 영역 기준값으로 대체합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 128 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 경계값 fixture: 경계 포함 허용, 범위 밖 5종 거부, clamp 평가, NaN·∞ 대체, decode 검증(`crop_boundary_fixtures`) | 통과 |

## 차이(작업 지시와 다른 점)

- 크기 0 crop(`width` 또는 `height` = 0)을 저장값에서 거부합니다. 빈 crop은 의미가
  없고 렌더 경로에서 0 나눗셈을 만들 수 있어서입니다.

## 남은 위험 / 한계

- 없음.
