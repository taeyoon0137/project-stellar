# 작업 보고서: X.4.1 Grid track 타입

- WBS: X.4.1
- 작업명: Grid track 타입 Fixed, Fraction, MinContent, MaxContent, MinMax 정의
- 상태: 완료 (`[x]`)
- 선행: X.1.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/grid.rs`(신규) — `TrackSize`, `TrackBreadth`, `GridLayout`, track·grid 검증, canonical bytes, 테스트.
- `crates/layout/src/input.rs` — `ContainerLayout::Grid(GridLayout)`(container tag 2), 검증·인코딩 연결.
- `crates/layout/src/engine.rs` — Grid 배치 구현(X.4.3) 전까지 `UnsupportedFeature` 명시 오류.
- `crates/layout/src/error.rs` — `LayoutError::InvalidGrid`, `LayoutError::UnsupportedFeature`.
- `crates/layout/src/lib.rs` — `pub mod grid;`.
- `docs/task-reports/X.4.1.md`(신규), 계획서(X.4.1 `[ ]`→`[x]`).

## 구현 요약

- **track 타입**: `Fixed(Length)`, `Fraction(Scalar)`, `MinContent`, `MaxContent`, `MinMax(TrackBreadth, TrackBreadth)`.
  `TrackBreadth`는 `Fixed`·`Fraction`·`MinContent`·`MaxContent`입니다.
- **type validation**: Fixed 비음수·LayoutUnit 범위, Fraction 비음수(1/64 단위 정수 변환), MinMax 최솟값 fr 금지,
  양쪽 Fixed면 min ≤ max, 열 track 하나 이상, 열·행 gap 비음수, 암시 행 track도 같은 검증.
- **GridLayout**: 열·행 track, `column_gap`, `row_gap`, `implicit_row`(자동 배치가 명시 행을 넘을 때, 기본 MaxContent).
- **canonical bytes**: breadth tag Fixed 0 / Fraction 1 / MinContent 2 / MaxContent 3, MinMax tag 4 + 두 breadth.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 78 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 7종 track(MinMax 3형태 포함) 열·행 검증 통과와 입력 round-trip, MinMax bytes golden(`valid_tracks_pass_and_round_trip`) | 통과 |
| 음수 Fixed·fr, minmax(fr, …), minmax(20, 10), 범위 초과, 열 없음, 음수 gap, 잘못된 암시 행, 미지 tag 거부(`invalid_tracks_are_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- track sizing은 X.4.2, item 배치는 X.4.3·X.4.4에서 구현합니다. 그 전까지 Grid container 계산은
  `UnsupportedFeature` 오류입니다.
