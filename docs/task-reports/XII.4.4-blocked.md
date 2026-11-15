# 차단 보고서: XII.4.4 selection overlay와 handle의 transient scene layer

## WBS

- WBS: XII.4.4
- 작업명: selection overlay와 handles를 transient scene layer로 렌더합니다.
- 상태: `[!]`
- 선행: XI.3.5 `[x]`, XII.4.3 `[~]`

## 실패 선행조건

- XII.4.3이 `[~]`입니다. 코어 camera 변환은 있지만 scene camera를 실제 CanvasHost 렌더가 소비하지 않습니다.
- 계획 1.2 규칙 4는 선행이 모두 `[x]`가 아니면 진척을 올리지 않습니다. 2026-11-14 통합에서 이 행을 `[~]`로 올렸고,
  2026-11-15 검토에서 `[!]`로 고쳤습니다.

## 재현 명령

- `rg -n '^\| (XII\.4\.3|XII\.4\.4) ' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` — XII.4.3 `[~]`, XII.4.4 `[!]`.
- `cargo test -p stellar-scene --lib overlay`와 `cargo test -p stellar-runtime --lib window_session` — 이미 있는 코어 경로의
  검사입니다. 2026-11-15 DEV-MAC의 `cargo test --workspace`(종료 0)에 포함되어 통과했습니다.

## 필요한 결정과 환경

- `DEV-MAC`. 화면 렌더 연결은 CanvasHost Metal 연결(XII.4.1)과 XI.8.2에 달려 있습니다.
- 셸이 넘길 overlay 모양(`OverlayStyle`의 색·handle 크기)과 캔버스 overlay 디자인 명세가 필요합니다.

## 변경되지 않은 파일

- 선행 완료 전에 들어온 코어 구현은 유지합니다: `crates/scene/src/overlay.rs`(문서 scene과 분리된 선택 외곽선·모서리
  handle layer와 hit test), `ProjectWindowSession::selection_overlay`(`4d54410b`).
- 실제 캔버스 화면에 overlay를 그리는 셸 연결은 없습니다. 대규모 선택의 성능(source 표 O(N log N), 중복 검사 최악
  O(S²))도 측정하지 않았습니다.

## 재개 조건

- XII.4.3이 `[x]`가 된 뒤 overlay layer를 실제 CanvasHost 렌더가 소비하고, 검증 기준(project snapshot에 overlay node 없음)을
  다시 실행해 보고서를 완료 형식으로 씁니다.
