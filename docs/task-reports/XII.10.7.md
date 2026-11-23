# 작업 보고서: XII.10.7 hover 미리보기

- WBS: XII.10.7
- 작업명: 지금 클릭하면 선택될 node(수식키 반영)의 hover 윤곽, 캔버스·사이드바 hover 동기화, history·저장 제외
- 상태: 완료 보고. 원장·계획서 marker는 통합 때 갱신합니다.
- 선행: XII.10.3(이 작업에서 구현), XII.4.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 성능은 release 빌드 측정입니다(다른 작업 여러 개가 같은 기계에서 동시에 돌던 상태).

## 변경 파일 목록

- `crates/runtime/src/canvas/select_hover.rs`(신규) — `HoverState`, `HoverSource`, `hover_target`, `hover_outline`.
- `crates/runtime/src/canvas.rs` — 모듈 등록.

## 구현 요약

- hover 대상은 클릭 규칙의 고를 node 계산(`click_pick`)을 그대로 씁니다(ADR-0104 결정 3.8). ⌘는 깊은 선택이고 Shift·Option은 고를 node를
  바꾸지 않습니다. 따로 판정하는 경로가 없어 hover와 클릭이 갈라질 수 없습니다.
- 캔버스 pointer와 사이드바 레이어 행이 같은 `HoverState` 하나를 바꾸며, 어디서 정했는지(`HoverSource`)를 함께 둡니다.
- hover 윤곽은 `CanvasGeometry`의 world 사각형(회전 포함, Fragment는 자손 bounds)인 transient 값입니다. history·저장·scene snapshot에
  들어가지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::select_hover` | 2 passed, 1 ignored(측정) |
| `cargo test --release -p stellar-runtime --lib canvas::select_hover -- --include-ignored --nocapture` | 3 passed |

| 검사 | 결과 |
|---|---|
| 무작위 tree 40개 × hit 목록 40개 × 수식키 16조합(잠김·숨김·Fragment, 무작위 범위·선택)에서 hover 대상과 클릭 결과가 일치(`hover_target_matches_the_click_result_for_every_modifier_combination`) | 통과 |
| 캔버스 hover(⌘ 깊은 node 포함)와 사이드바 hover가 같은 상태, 윤곽 사각형, history 0건·save bytes·scene semantic hash 불변(`canvas_and_sidebar_share_one_hover_without_history_or_save`) | 통과 |
| 2만 가시 node scene에서 hit test + hover 갱신 p50 2.4µs, p99 3.5µs, max 10.9µs(`measure_hover_20k`, release) | 통과(목표 5ms, 상한 8ms 이하) |

## 차이(작업 지시와 다른 점)

- 없음. 윤곽을 그리는 overlay layer 연결(XII.4.4 overlay에 hover 사각형 추가)은 렌더·셸 연결 때 합니다.

## 남은 위험 / 한계

- 측정은 Frame만 있는 2만 node fixture입니다. path·text가 많은 문서의 narrow-phase 비용은 XII.10.43 replay가 측정합니다.
