# 작업 보고서: X.5.1 Overlay layout

- WBS: X.5.1
- 작업명: Overlay layout에서 모든 child가 같은 containing block을 사용하고 z-order는 OrderKey를 따르게 함
- 상태: 완료 (`[x]`)
- 선행: X.1.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/overlay.rs`(신규) — `OverlayLayout`, `OverlayAlignment`, canonical bytes, `measure`·`place`, fixture 테스트.
- `crates/layout/src/input.rs` — `ContainerLayout::Overlay(OverlayLayout)`(container tag 3), 검증·인코딩 연결.
- `crates/layout/src/engine.rs` — Overlay 분기 연결.
- `crates/layout/src/lib.rs` — `pub mod overlay;`.
- `docs/task-reports/X.5.1.md`(신규), 계획서(X.5.1 `[ ]`→`[x]`).

## 구현 요약

- **공유 containing block**: 모든 자식이 부모 content rect 하나를 containing block으로 씁니다(`Fill` = content rect,
  `Percentage` = content rect 기준). 자식끼리 위치·크기에 영향을 주지 않습니다.
- **정렬**: container 축별 `Start`/`Center`/`End`(Center는 ties away from zero)로 content rect 안에 놓습니다.
- **z-order**: 결과 box 순서(뒤가 위)가 child visual order(OrderKey)입니다. 입력 node 순서·삽입 순서·StableId와 무관합니다.
- **Hug**: 축별 자식 크기 최댓값.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 92 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| overlay fixture: content 100×60에서 Fill(전체)·고정 40×20·50%×25% 자식을 Start/Center/End로 배치(Center 22.5 px subpixel 포함)(`overlay_fixture_shares_containing_block`) | 통과 |
| z-order: 삽입 2,3,4 / OrderKey 3,1,2 → 결과 순서 3,4,2, 같은 원점에 겹침, node 목록 반전 시 결과 동일(`z_order_follows_order_key`) | 통과 |
| Hug overlay 60×40, End/Center 정렬, round-trip, bytes golden `[2, 1]`(`hug_overlay_and_round_trip`) | 통과 |

## 차이(작업 지시와 다른 점)

- 자식별 정렬(self alignment)은 두지 않고 container 정렬 하나로 고정했습니다.

## 남은 위험 / 한계

- 없음.
