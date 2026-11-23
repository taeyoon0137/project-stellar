# 작업 보고서: XII.10.5 select-through

- WBS: XII.10.5
- 작업명: 선택 bounds 안 누름은 선택 유지(끌면 이동, 놓으면 클릭 규칙), 잠금·숨김 hit 통과, 겹친 레이어 목록
- 상태: 완료 보고. 원장·계획서 marker는 통합 때 갱신합니다.
- 선행: XII.10.3(이 작업에서 구현), XI.3.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/runtime/src/canvas/select_through.rs`(신규) — `PressPlan`, `plan_press`, `OverlapEntry`, `overlapping_layers`.
- `crates/runtime/src/canvas/select_fixtures.rs` — 잠금·숨김 지정 도우미를 다른 테스트가 쓰게 공개.
- `crates/runtime/src/canvas.rs` — 모듈 등록.

## 구현 요약

- 규칙은 ADR-0104 결정 3.6입니다. 누른 점이 선택 bounds(선택 node world bounds의 합, Fragment는 자손 합) 안이거나 고를 node가 이미
  선택되어 있으면 `KeepSelection`(끌면 선택 이동, 놓으면 XII.10.3 클릭 결과)입니다. 밖이면서 고를 node가 있으면 `SelectAndDrag`
  (누름에서 선택, 끌면 그 선택 이동), 없으면 `Marquee`입니다.
- 잠긴 node는 scene에서 `pointer-events: none`이라 클릭이 통과하고, 숨긴 node는 hit 목록에 없습니다. 겹친 레이어 목록은
  `PointStack` 전체(잠금 포함, 위에 그린 것부터, View root 제외)입니다.
- 누름에서 바꾼 선택의 기록은 제스처 끝에 한 번 합니다(XII.10.40 `GestureRecorder`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::select` | 10 passed |

| 검사 | 결과 |
|---|---|
| 3단 겹침(40, 41, 잠긴 42) scene에서 선택 bounds 안 누름은 선택 유지·놓으면 41, bounds 밖은 41 즉시 선택 후 끌기, 잠긴 node만 있는 곳은 마퀴, 목록은 42(잠김)·41·40, 잠긴 42는 어떤 클릭으로도 선택 안 됨(`three_layer_overlap_drag_click_and_list`) | 통과 |

## 차이(작업 지시와 다른 점)

- 캔버스 명세 4.3절이 지워져 목록 항목 형식(node와 잠금 표시)을 ADR-0104에서 정했습니다. 문맥 메뉴 연결은 XII.10.39 몫입니다.

## 남은 위험 / 한계

- 잠긴 node의 목록 판정은 local hit bounds와 clip 기준(broad-phase 정밀도)이라, 곡선 path의 bounds 안 빈 곳도 목록에 들어갈 수
  있습니다(ADR-0104 결과).
- 선택 이동 자체(위치 longhand 편집)는 XII.10.13 몫입니다.
