# 작업 보고서: XII.10.43 캔버스 편집 성능 replay

- WBS: XII.10.43
- 작업명: 캔버스 편집 성능 replay(2만 가시 node)
- 상태: 부분 완료 (`[~]`) — 코어 replay와 판정 완료, IME 조합 scenario·xtask analyzer 연결·기준 장비 판정 남음
- 결정: ADR-0117 결정 10
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max, release). 다른 작업 9개가 같은 장비에서 동시에 돌았습니다. REF-MAC 판정이 아닙니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/edit_replay.rs`(신규), `crates/runtime/src/canvas/edit_replay/tests.rs`(신규), `crates/runtime/src/canvas.rs`(mod 등록)

## 구현 요약

- scenario 닫힌 목록 `CanvasEditScenario`: hover, 클릭 선택, 핸들 표시, 마퀴, 스냅 포함 이동, flex 컨테이너 안 크기 조절, 회전, Path point 드래그, 팬, 줌, IME 조합 입력.
- 예산 `budget()`: hover·클릭 선택·핸들 표시는 목표 5ms·상한 8ms, 나머지는 10ms(p99 nearest-rank). `judge`는 sample 부족(`too_few_samples`, 1,000 미만), 상한 초과(`over_limit`), 연결 안 됨(`not_connected`, 통과 아님)을 구분합니다.
- 결과 JSON `stellar.canvas-edit-replay.v1`(`to_json`): scenario, sample 수, p50·p99·max µs, 목표·상한, 판정. 닫힌 이름과 숫자만 씁니다.
- 측정(`measure_canvas_edit_replay_20k`, release ignored): root 아래 Rectangle 19,900개 격자 + flex Frame 아래 100개 + Path. 측정 구간은 입력 하나의 코어 처리이고 제품 경로 함수를 그대로 씁니다(`point_stack`·`hover_target`, `resolve_click`, `HandleTarget::of`·`build_handles`, `Marquee::update`, `MoveDrag::update`(스냅 2만 형제), `ResizeGesture::update`, `RotateGesture::update`, `PointDrag::update`, `Navigator::apply`+`world_to_view`). 측정용 별도 경로는 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::edit_replay` | 통과(1, ignored 1): 예산·판정·nearest-rank p99·JSON 모양 |
| `STELLAR_CANVAS_REPLAY_OUT=<scratch>/canvas-edit-replay.json cargo test --release -p stellar-runtime --lib canvas::edit_replay::tests::measure_canvas_edit_replay_20k -- --ignored --nocapture` | 통과: scenario별 1,000 sample, 연결된 10개 모두 pass, IME는 `not_connected` |

측정값(p50 / p99 / max, µs, 1,000 sample):

| scenario | p50 | p99 | max | 판정 |
|---|---|---|---|---|
| hover | 1.5 | 23.8 | 36.9 | pass(목표 안) |
| click_select | 2.0 | 24.7 | 73.4 | pass(목표 안) |
| handle_display | 0.8 | 1.6 | 28.5 | pass(목표 안) |
| marquee | 61.4 | 298.9 | 802.0 | pass |
| snap_move | 23.3 | 162.3 | 602.4 | pass |
| flex_resize | 0.0 | 0.04 | 19.5 | pass |
| rotate | 0.0 | 0.04 | 0.2 | pass |
| path_point_drag | 0.04 | 0.04 | 0.6 | pass |
| pan | 0.0 | 0.04 | 51.8 | pass |
| zoom | 0.04 | 0.04 | 0.08 | pass |
| ime_composition | - | - | - | not_connected |

## 차이(작업 지시와 다른 점)

- IME 조합 입력 scenario는 Text 편집 경로가 scene 입력에 이어지지 않아(ADR-0096 결정 3과 같은 이유) `not_connected`로 결과에 남겼습니다. 측정 전용 경로를 만들면 이중 경로가 되어 만들지 않았습니다. 그래서 행 전체 판정은 통과가 아닙니다.
- 계획서의 "analyzer 판정"은 xtask analyzer(`stellar.performance-result.v2`)인데 runtime은 diagnostics·xtask에 의존하지 않아(ADR-0084 결정 8) 같은 예산 의미의 판정을 이 module에 두고 결과 JSON을 따로 냅니다. analyzer 연결(XVI.2.3·XVI.4.2)과 REF-MAC 측정은 남아 있습니다.
- "Present 예약 10ms"는 셸·render thread가 없어 코어 처리 시간으로 쟀습니다.
- 계획서 선행(XII.10.16 등)에 `[!]` 사슬이 있지만 코어 replay는 선행 없이 실행할 수 있어 만들었습니다.

## 남은 위험 / 한계

- 측정은 다른 작업 9개가 동시에 도는 개발 장비 값입니다. 판정은 REF-MAC에서 다시 해야 합니다.
- 결과 JSON을 저장소에 두지 않았습니다(측정 기록용, 실행 때 `STELLAR_CANVAS_REPLAY_OUT`으로 씀).
