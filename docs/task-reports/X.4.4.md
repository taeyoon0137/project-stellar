# 작업 보고서: X.4.4 grid auto placement (row-major deterministic)

- WBS: X.4.4
- 작업명: auto placement를 row-major deterministic order로 구현
- 상태: 완료 (`[x]`)
- 선행: X.4.3(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/grid.rs` — 점유 격자(`Occupancy`), 3단계 배치(`place_items`), 문서, fixture 테스트.
- `crates/layout/src/error.rs` — 사용처가 없어진 임시 `UnsupportedFeature` 제거.
- `docs/task-reports/X.4.4.md`(신규), 계획서(X.4.4 `[ ]`→`[x]`).

## 구현 요약

- **순서**: 모든 단계는 child visual order(OrderKey, 같은 키는 StableId tie-break)로 처리합니다. 입력 삽입 순서는
  결과에 영향을 주지 않습니다.
- **단계**: ① 열·행 명시 item을 그대로 놓음(겹침 허용) ② 행만 명시된 item을 그 행의 첫 빈 열에 ③ 나머지를
  row-major cursor(sparse)로 — 완전 자동은 cursor부터 열을 늘리며 빈 칸을 찾고 줄 끝이면 다음 행 첫 열, 열만
  명시된 item은 그 열이 cursor 열보다 앞이면 다음 행부터 그 열이 비는 첫 행. 놓은 뒤 cursor = (행, 끝 열).
- 자동 item은 점유 칸을 피하고 행은 `implicit_row` 크기 암시 행으로 늘어납니다. 열은 늘리지 않으므로 행 고정 item이
  들어갈 열이 없으면 `InvalidGrid`("자동 배치 공간이 없습니다")입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 89 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 삽입 순서 id 2..6 / OrderKey 5,3,1,4,2 → visual order 4,6,3,5,2가 row-major (0,0),(1,0),(2,0),(0,1),(1,1)에 배치, 삽입 순서 반전 시 결과 bytes 동일, 결과 box 순서 = visual order(`auto_placement_follows_visual_order_key_not_insertion`) | 통과 |
| 명시 item 점유 회피, span 2 다음 행 이동, sparse cursor(앞 빈 칸 유지), 행 고정, 열 고정(다음 행부터), 3칸 item 행 이동, 암시 행 5개 Hug 50(`auto_placement_skips_occupied_cells_row_major`) | 통과 |
| 기존 X.4.1~X.4.3 grid fixture 회귀 | 통과 |

## 차이(작업 지시와 다른 점)

- dense packing(앞 빈 칸 재사용)은 지원하지 않고 sparse만 구현했습니다.

## 남은 위험 / 한계

- 자동 배치 행 탐색은 `u16` line 범위(65535행)로 제한합니다.

## 후속 수정 (리뷰 major: grid 자동 배치·점유 비용 무제한)

- **결함**: 자동 배치·점유 비용이 행 × 열 × span에 비례하고 상한이 없었습니다. 1열 grid의 마지막 행 명시 item과
  행 span 65535 자동 item 하나로 `compute_layout`이 수 초 탐색한 뒤 "공간 없음"을 반환했고, 열이 많은 grid의 마지막 행
  명시 item은 행 × 열 bool 배열을 GB 단위로 할당할 수 있었습니다. 자동 item은 명시 item 검증의 `u16` line 범위를 넘어
  암시 행을 만들 수 있었습니다.
- **수정** (`crates/layout/src/grid.rs`, `grid_tracks.rs`, `error.rs`):
  - 새 오류 `LayoutError::LimitExceeded { node, limit: LayoutLimit }`와 `LayoutLimit::{GridLines, GridCells, GridWork}`.
  - **line 범위**: 한 축 track은 `MAX_GRID_TRACKS`(= 65534, line 1~65535)개까지입니다. 명시 track 수·명시 item 끝
    line은 검증에서, 자동 item 끝 line은 배치 중에 검사합니다. 자동 item이 범위 안에 자리가 없으면 "공간 없음"이 아니라
    `GridLines`입니다. 끝 line이 65536이 되는 `Span { start: 65535, span: 1 }`도 이제 거부합니다(이전 검사는 끝 index
    65535까지 허용).
  - **cell 수**: 열 × 행 ≤ `MAX_GRID_CELLS`(2^20). 명시 track·명시 item은 검증에서, 자동 배치가 늘린 행은 점유 시점에
    `GridCells`로 거부합니다(할당 전).
  - **작업량**: grid 계산 한 번의 cell 검사·점유·track sizing 반복을 `MAX_GRID_WORK`(2^24) 단위 결정적 예산으로 셉니다.
    넘으면 `GridWork`.
  - 자동 배치 탐색은 점유 cell을 만나면 그 cell을 포함하는 시작 위치를 건너뜁니다(결과는 한 칸씩 탐색과 동일 — 기존
    배치 fixture 그대로 통과). track sizing의 span 1 기여는 item 목록을 한 번만 훑어 모으고, fr 판정은 bool mask로 바꿔
    track × item, track² 비용을 없앴습니다. 영역 크기는 track offset으로 O(1)에 계산합니다.

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 122 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 리뷰 재현(1열, 마지막 행 명시 item + 행 span 65534 자동 item) → 즉시 `GridLines`, 끝 line이 u16을 넘는 명시·자동 item 검증 거부, 끝 line 65535까지 허용·자동 item이 범위 끝에 꼭 맞으면 배치(`auto_placement_past_u16_line_range_is_limit_error`) | 통과 |
| 열 20000 + 행 65534 명시 item → 검증 `GridCells`, 명시 track 수·cell 수 상한, 자동 배치가 늘린 행의 cell 상한(`grid_cell_and_track_counts_are_bounded`) | 통과 |
| 겹친 전면 명시 item 17개 → `GridWork`, 12열 자동 item 2000개는 정상 계산(`grid_work_is_bounded`) | 통과 |
| 예산 소모·초과 결정성과 track sizing `WorkLimit`(`work_budget_is_deterministic_and_bounded`) | 통과 |
| 기존 X.4.1~X.4.4 fixture와 X.11.1 corpus golden | 통과(변경 없음) |

- **남은 한계**: 상한값(2^20 cell, 2^24 작업 단위)은 일반 UI grid보다 충분히 크게 잡은 고정값입니다. 행 6만여 개 grid에서
  maximize·fr freeze가 한 바퀴에 track 하나씩만 고정되는 극단 입력은 `GridWork`로 거부될 수 있습니다.
