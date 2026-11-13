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
