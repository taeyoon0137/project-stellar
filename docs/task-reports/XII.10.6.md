# 작업 보고서: XII.10.6 영역 선택(마퀴)

- WBS: XII.10.6
- 작업명: 빈 캔버스·Root Frame 빈 영역에서 시작하는 영역 선택, 수식키 4종(기본 걸침, Option 완전 포함, ⌘ 가장 깊은 node, Shift 더하기),
  transient preview와 pointer up의 SetSelection 하나
- 상태: 부분 완료 보고. 기능·history 기준은 통과, 2만 node 갱신 p99 8ms 기준은 preview 갱신 전체로는 미충족(아래).
- 선행: VII.3.3, XII.10.3(이 작업에서 구현), XI.3.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 성능은 release 빌드 측정입니다(다른 작업 여러 개가 같은 기계에서 동시에 돌던 상태).

## 변경 파일 목록

- `crates/runtime/src/canvas/select_area.rs`(신규) — `Marquee`(`begin`, `hits`, `update`, `finish`, `cancel`), `MarqueeResult`.
- `docs/adr/ADR-0104-canvas-tools-units-and-selection-core.md` — 성능 절에 preview 갱신 한계 기록.
- `crates/runtime/src/canvas.rs` — 모듈 등록.

## 구현 요약

- 규칙은 ADR-0104 결정 3.7입니다. 후보는 시작 때 한 번 만듭니다. 기본은 편집 범위의 선택 가능 직계 자식, ⌘는 범위 아래 선택 가능 자식이
  없는 가장 깊은 node입니다. 후보마다 world 사각형과 bounds를 미리 계산합니다.
- 기본은 겹침(bounds 겹침 뒤 world 사각형 분리축 판정이라 회전 node의 빈 모서리는 걸리지 않음), Option은 완전 포함, Shift는
  `AreaSelectionMode::Add`입니다.
- 진행 중 결과는 기존 `WindowViewState` 영역 선택 preview(VII.3.3)를 그대로 쓰고, hit가 직전과 같으면 preview를 다시 만들지 않습니다.
  pointer up은 `SetSelection` 하나(같으면 없음)와 선택에 맞춘 범위를 돌려주고, 취소는 event가 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::` | 43 passed, 1 ignored(측정) |
| `cargo test --release -p stellar-runtime --lib canvas::select_area -- --include-ignored --nocapture` | 4 passed |

| 검사 | 결과 |
|---|---|
| 중첩 fixture 수식키 matrix: 기본 [11, 12], Option [11], ⌘ [13, 17]과 범위 11, Shift 더하기 [13, 17, 18, 19], 마퀴마다 history 1건, 같은 선택·취소는 0건(`modifier_matrix_on_the_nested_fixture`) | 통과 |
| 45도 회전 상자: bounds만 겹치는 모서리 영역은 걸리지 않고 실제 사각형과 겹치면 걸림(`rotated_nodes_use_their_world_quad`) | 통과 |
| 1,000 move event 동안 history 0건, pointer up 뒤 1건(`thousand_moves_make_one_history_entry`) | 통과 |
| 2만 node 후보 판정 p50 53.8µs, p99 130µs, max 163µs(`measure_marquee_update_20k`, release) | 통과(8ms 이하) |
| 2만 node preview 갱신 전체 p50 57.7µs, p99 9.73ms, max 14.8ms(같은 측정) | 미충족(8ms 초과) |

## 차이(작업 지시와 다른 점)

- p99 8ms 기준은 이 모듈의 후보 판정으로는 충족하지만, preview 갱신 전체는 9.73ms입니다. 원인은 commands crate의
  `Selection::new`가 중복 제거를 선형 탐색으로 해서 원소 수의 제곱 비용이 드는 것입니다(`update_area_selection`이 부름). 1만 개 가까이
  덮는 영역에서 한 번에 수 ms가 듭니다. 그 파일은 이 작업의 소유 범위 밖이라 고치지 않았습니다.

## 남은 위험 / 한계

- `stellar_commands::selection::Selection::new`의 중복 제거를 정렬·hash 기반으로 바꾸면 preview 갱신도 기준 안에 들어올 것으로 보입니다
  (후보 판정 p99 130µs). 그 변경과 재측정이 이 행을 닫는 데 남은 일입니다.
- 렌더 thread가 `displayed_selection`을 overlay로 그리는 연결은 셸·렌더 연결 때 합니다.
