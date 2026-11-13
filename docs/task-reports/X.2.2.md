# 작업 보고서: X.2.2 absolute anchor 해석 순서와 과다 제약 오류

- WBS: X.2.2
- 작업명: absolute anchors left/right/top/bottom과 fixed size 조합의 해석 순서 정의, over-constrained 입력은 오류
- 상태: 완료 (`[x]`)
- 선행: X.2.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/free.rs` — `Offset`, `AxisAnchors`, `FreePosition { horizontal, vertical }`, anchor 해석(크기 → 위치 순서), Hug 측정 기여 규칙, 조합 검증, canonical bytes, matrix·오류 fixture.
- `crates/layout/src/input.rs` — `with_anchors`, Free 부모 자식의 anchor 조합 검증 연결, position 인코딩 위임.
- `crates/layout/src/engine.rs` — 변환된 anchor(`PreparedAnchors`) 사용.
- `crates/layout/src/error.rs` — `LayoutError::OverConstrained { node, axis }`.
- `docs/task-reports/X.2.2.md`(신규), 계획서(X.2.2 `[ ]`→`[x]`).

## 구현 요약

- **anchor 모델**: 축마다 시작(left/top) `s`, 끝(right/bottom) `e` offset을 선택적으로 가집니다(부모 content
  box 기준). 기존 `with_position(x, y)`는 시작 anchor입니다.
- **해석 순서(축별, content 크기 `C`)**: ① 크기 — `Fill`이면 `C − (s|0) − (e|0)`(0 미만은 0), 그 밖은 자기
  크기. ② 위치 — `s`가 있으면 `s`, `e`만 있으면 `C − e − 크기`, 둘 다 없으면 0.
- **과다 제약**: 양 끝 anchor + definite 크기(`Fixed`·`HugContent` 등 `Fill`이 아닌 mode)는 우선순위로 해소하지
  않고 입력 검증 단계에서 `OverConstrained { axis }`로 거부합니다(`compute_layout`도 같은 오류).
- **Hug 측정**: 끝 anchor가 있는 자식은 부모 끝 기준이라 Hug extent에 기여하지 않습니다.
- anchor는 Free 부모의 자식에서만 해석하고 다른 배치 방식에서는 무시합니다(모듈 문서에 명시).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 48 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 가능한 조합 matrix: anchor 4종(없음·시작·끝·양끝) × 크기 3종(Fixed·Hug·Fill) × 축 2 = 24 case의 위치·크기, 양끝+Fixed/Hug는 `OverConstrained`(검증·계산 모두)(`anchor_combination_matrix_and_errors`) | 통과 |
| 교차 anchor(시작 60 + 끝 30 > 80)의 Fill 크기 0, Hug 부모에서 끝 anchor 자식 비기여·넘침 overflow(`fill_between_crossing_anchors_is_zero_and_end_anchors_skip_hug`) | 통과 |
| anchor canonical round-trip, 범위 밖 offset `OutOfRange`(`anchors_round_trip_and_validate_ranges`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- anchor 사이가 음수가 되는 Fill은 오류가 아니라 0으로 자릅니다(min/max clamp는 X.6.2).
- percentage offset은 X.2.3에서 추가합니다.
