# 작업 보고서: XII.10.26 생성 도구

- WBS: XII.10.26
- 작업명: Frame·Rectangle·Ellipse·Line·Polygon 생성 도구
- 상태: 부분 완료 (`[~]`) — 2026-11-24 통합 판정: 이 행의 구현은 끝났지만 선행 XII.10.9이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`) — 코어. 셸 pointer 연결은 UI 재설계 때
- 결정: ADR-0117 결정 6
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/create.rs`(신규), `crates/runtime/src/canvas/create/tests.rs`(신규), `crates/runtime/src/canvas.rs`(mod 등록)
- `crates/runtime/src/canvas/flow_drag.rs`(흐름 컨테이너 판정 `container_axes`와 삽입 key `flow_insert_key`를 생성·붙여넣기가 함께 쓰게 정리)

## 구현 요약

- `CreateGesture`(부모 local 순수 계산): 드래그는 영역, Shift는 상자 정비례(긴 변)·Line 45도(직각 배수는 정확한 0), Option은 시작점 중심, 드래그 중 Space는 크기를 지킨 채 시작점 이동. 끌기 시작 거리 3pt(ADR-0104) 미만은 클릭이고 기본 100×100(Line은 100px 가로선, Option이면 중심)입니다.
- `creation_parent`: pointer 아래 가장 위의 잠기지 않은 Frame(인스턴스 투영 행·숨김 제외, 잠금은 `commands::lock`), 없으면 View root.
- `commit`: 칠은 셸 앱 설정 `ShapePaint`(코어 기본 색 없음, Frame 배경은 `fill`). 흐름 컨테이너면 XII.10.24의 같은 삽입 index 규칙으로 OrderKey를 정하고 위치를 쓰지 않습니다(Frame transform 항등, 도형 Transform modifier 없음). `InsertNode`와 새 선택을 `insert_and_select`로 한 항목에 기록하고 도구 기계에 `CreationFinished`를 넣어 Move로 돌립니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::create` | 통과(5): 도구 다섯의 드래그·클릭·Shift·Option·Space, 배율에 따른 끌기 시작 판정, 도구별 생성 history +1·새 선택·Move 복귀·저장 위치, undo 한 번이 생성·선택을 함께 되돌림, flex 컨테이너 pointer 자리 삽입 index, 칠 없음·빈 크기 거부(history 0) |

## 차이(작업 지시와 다른 점)

- 앞선 셸 경로 `app::frame_insert`(ADR-0071)가 아직 있습니다. 소유 범위 밖이라 지우거나 합치지 않았고, 새 UI 연결 때 지웁니다(ADR-0117 결정 6.6). 두 경로의 저장 모양(Transform → Fill → Stroke 순서, Polygon 5변)은 같게 맞췄습니다. 다른 점은 Frame 배경으로, 옛 경로는 흰색 고정이고 새 경로는 앱 설정 `fill`입니다.

## 남은 위험 / 한계

- 셸 연결 전까지 도구 막대·pointer 입력에서 이 경로를 부르는 화면이 없습니다.
