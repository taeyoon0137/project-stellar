# 작업 보고서: X.4.3 grid item start/end·span·gap

- WBS: X.4.3
- 작업명: grid item start/end, span과 gap 구현, invalid span은 오류
- 상태: 완료 (`[x]`)
- 선행: X.4.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/grid.rs` — `GridPlacement`(`Auto`·`Span`·`Lines`), `GridItem`, item 검증·canonical bytes, track 변환, Grid `measure`·`place`(열 → 행 track sizing, 영역 배치), fixture 테스트.
- `crates/layout/src/input.rs` — `LayoutNode::grid`, `with_grid`, Grid 부모 자식 검증, canonical bytes.
- `crates/layout/src/engine.rs` — Grid container 분기 연결.
- `crates/layout/src/property.rs` — `LayoutProperty::GridPlacement`.
- `crates/layout/src/lib.rs` — X.4.2 임시 dead code 허용 제거.
- `docs/task-reports/X.4.3.md`(신규), 계획서(X.4.3 `[ ]`→`[x]`).

## 구현 요약

- **위치**: 축마다 `Span { start, span }` 또는 `Lines { start, end }`(line 1부터). `Auto`는 X.4.4 자동 배치이며
  이 작업 시점에는 계산 시 `UnsupportedFeature` 명시 오류입니다.
- **invalid span = 오류**: span 0, line 0, 끝 ≤ 시작, 열 범위가 명시 열을 넘음, 자동 span > 열 수 →
  `InvalidGrid`(입력 검증 단계). 행 범위가 명시 행을 넘으면 `implicit_row` 크기 암시 행을 만듭니다.
- **겹침 허용 정책**: 명시 위치 item 영역이 겹쳐도 오류가 아니며 그대로 겹쳐 놓습니다(z-order = visual order).
- **gap**: 이웃 track 사이에만 들어가며 여러 칸 영역은 걸친 track 합 + 사이 gap입니다.
- **sizing 연결**: 열 track은 item 측정 크기(containing block indefinite)를 기여로, 행 track은 열 영역 너비가
  정해진 측정 높이를 기여로 X.4.2 `size_tracks`에 넘깁니다. item은 영역 시작에 놓이고 `Fill` 축은 영역을
  채우며 `Percentage`는 영역 크기 기준입니다. Hug Grid는 track 합 + gap입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 87 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| start/span·start/end·gap fixture: 3×2 고정 track(gap 10·5, padding 4)에서 1칸, 2×2 span 영역 140×55, Lines 영역 item과 겹침 허용, Hug 크기 208×63, visual order(`start_end_span_gap_and_overlap_fixture`) | 통과 |
| invalid fixture 7종(span 0, line 0, 끝=시작, 끝<시작, 열 범위 초과, 자동 span > 열, 자동 행 span 0) 모두 `InvalidGrid`(`invalid_spans_are_errors`) | 통과 |
| 암시 행(implicit 15) 위치, [MaxContent, 1fr]에서 intrinsic 70이 열 1을 정하고 fr 130, 영역 기준 50% 너비·Fill 높이(`implicit_rows_content_tracks_and_percentage_items`) | 통과 |
| GridItem bytes golden과 round-trip(`grid_item_round_trips`) | 통과 |

## 차이(작업 지시와 다른 점)

- item 정렬(justify/align-self)은 두지 않았고 영역 시작 정렬로 고정했습니다(`Fill`로 영역 채움).

## 남은 위험 / 한계

- 자동 배치(`Auto`)는 X.4.4에서 구현합니다.
- 열 방향 암시 track은 만들지 않습니다(열 범위 초과는 오류).
