# 작업 보고서: XII.4.4 selection overlay와 handle의 transient scene layer

## WBS

- WBS: XII.4.4
- 작업명: selection overlay와 handles를 transient scene layer로 렌더합니다.
- 상태: 완료 (`[x]`) — 2026-11-23 재판정(plan-revision). 선행 XII.4.3이 `[x]` 판정을 받아 [차단 보고서](XII.4.4-blocked.md)의 재개 조건 중 선행 조건이 풀렸습니다. 이미 들어와 있던 코어 구현을 행 지시·검증 열 기준으로 다시 판정했습니다.
- 선행: XI.3.5 (`[x]`), XII.4.3 (`[x]`, 2026-11-23 ledger-verification 재판정)

## 변경 파일

- 이 판정은 코드를 바꾸지 않았습니다. 판정 대상 코드는 이미 main에 있습니다(`4d54410b`).
  - `crates/scene/src/overlay.rs` — 문서 scene과 분리된 선택 외곽선·모서리 handle layer(`SelectionOverlay::layer`)와 transient hit-test layer(`hit_layer`), `build_selection_overlay`.
  - `crates/runtime/src/window_session.rs` — `ProjectWindowSession::selection_overlay`(창 camera와 화면 선택으로 overlay 생성).
- `docs/task-reports/XII.4.4.md`(신규), 계획서(XII.4.4 `[!]` → `[x]`).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-scene --lib overlay` | 0 (3 passed) |
| `cargo test -p stellar-runtime --lib selection_overlay` | 0 (1 passed) |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 없음. 기준 커밋 `main` `d2bf97b3`, branch `lane/plan-revision`.

## 테스트

- `overlay::tests::document_scene_never_receives_overlay_nodes`: overlay를 만들어도 문서 scene의 semantic hash와 node 수가 그대로이고, overlay layer는 별도 snapshot입니다.
- `window_session::tests::selection_overlay_renders_as_a_transient_layer_outside_the_project_snapshot`(검증 열 "project snapshot에 overlay node 없음"): 선택·확대 뒤 overlay layer(외곽선 1 + handle 4)를 공통 pass compiler(`compile_frame`)로 draw 5개 이상으로 컴파일하고, 문서 scene hash·문서 record·저장 chunk bytes·`save_now` 결과(`written: false`)·디스크 tree가 모두 그대로임을 확인합니다.
- 남은 경고 수: 0(이 판정에서 코드를 바꾸지 않음).

## 성능

- 해당 없음. 대규모 선택의 비용(source 표 O(N log N), 중복 검사 최악 O(S²))은 측정하지 않았습니다(아래 남은 위험).

## 차이

- 행 지시의 "렌더"는 transient layer를 렌더 가능한 scene snapshot으로 만들고 공통 pass compiler가 받는 데까지로 판정했습니다. 실제 CanvasHost 화면에 그리는 일은 XII.4.1(Metal 연결)의 범위이며 이 행의 선행이 아닙니다. XII.4.3을 `[x]`로 판정한 기준(렌더 연결은 XII.4.1 범위)과 같습니다.

## 남은 위험

- 셸이 넘길 overlay 모양(`OverlayStyle`의 색·handle 크기)과 캔버스 overlay 디자인 명세는 UI 재설계 때 정합니다(2026-11-23 사용자 방향: 지금은 엔진만).
- 대규모 선택의 overlay 생성 비용은 성능 replay(XII.10.43, XVI.2)에서 측정해야 합니다.
