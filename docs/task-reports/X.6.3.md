# 작업 보고서: X.6.3 aspect ratio constraint

- WBS: X.6.3
- 작업명: aspect ratio constraint를 한 축이 definite일 때만 적용하고 양 축 indefinite는 intrinsic size로 해결
- 상태: 완료 (`[x]`)
- 선행: X.6.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/sizing.rs` — `AspectRatio`, `PreparedRatio`(1/64 정수 비, 한 번 반올림 변환), 검증·canonical bytes, 정책 문서, ratio matrix 테스트.
- `crates/layout/src/engine.rs` — `Prepared::ratio`, 크기 해석에서 비율 적용(mode → definite 축 clamp → 비율 → 다른 축 clamp).
- `crates/layout/src/input.rs` — `LayoutNode::aspect_ratio`, `with_aspect_ratio`, 검증·canonical bytes.
- `crates/layout/src/property.rs` — `LayoutProperty::AspectRatio`.
- `docs/task-reports/X.6.3.md`(신규), 계획서(X.6.3 `[ ]`→`[x]`).

## 구현 요약

- **한 축만 definite**: definite 축 값(clamp 후)에서 다른 축을 `× h / w` 또는 `× w / h`로 정하고(정수 비율 곱, 한 번
  반올림) 그 축을 clamp합니다. definite에는 Fixed, 정해진 containing block의 Percentage, Fill 영역, 부모 forced 크기
  (Stack 분배 등)가 포함됩니다.
- **두 축 definite**: 비율을 적용하지 않습니다(각 축 크기 그대로).
- **두 축 indefinite**: 가로를 intrinsic(콘텐츠 extent + padding, clamp) 크기로 정해 definite로 만든 뒤 비율로 세로를
  정합니다(콘텐츠 높이는 쓰지 않음).
- 비율 값은 1/64 단위로 0보다 커야 합니다(0·음수·1/128 이하 → `InvalidSizing`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 110 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| ratio matrix(2:1): Fixed/Hug, Hug/Fixed, Fill/Hug, Percentage/Hug, Hug/Fill → 비율 적용, Fixed/Fixed → 미적용, Hug/Hug + 콘텐츠 30×40 → 30×15, Hug/Hug 콘텐츠 없음 → 0×0(`ratio_matrix_fixture`) | 통과 |
| 순서: max_width 60 → 60×30, max_height 20 → 100×20, 3:1 너비 100 → 높이 2133 subpixel(같은 테스트) | 통과 |
| Stack grow로 정해진 main 너비 120 → 세로 60, round-trip, 0·음수·너무 작은 비율 거부(`ratio_follows_stack_main_size_and_validates`) | 통과 |

## 차이(작업 지시와 다른 점)

- "양 축 indefinite는 intrinsic size로 해결"을 "가로 intrinsic 크기를 definite로 삼고 비율로 세로를 정한다"(CSS
  aspect-ratio의 inline 축 우선 규칙)로 해석했습니다. intrinsic 측정은 현재 입력의 고정 intrinsic 값이며, 측정 trait
  연결(X.6.4)은 선행 미완료로 이 lane에서 다루지 않았습니다.

## 남은 위험 / 한계

- 비율로 정해진 축이 min/max에 걸리면 비율이 깨질 수 있습니다(다른 축으로 다시 전파하지 않음).

## 후속 변경 (리뷰 minor: 가중치 양자화 규칙 통일)

- 1/128 미만 양수 비율은 이제 `InvalidSizing` 대신 flex factor·grid fr과 같은 `LayoutError::BelowResolution`입니다.
  0·음수 비율은 그대로 `InvalidSizing`입니다. 상세와 검증 결과는 `docs/task-reports/X.3.3.md` 후속 수정 절에 있습니다.
