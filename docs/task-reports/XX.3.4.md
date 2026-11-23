# 작업 보고서: XX.3.4 SVG로 정확히 표현 가능한 gradient·clip·mask·opacity의 vector 기록

- WBS: XX.3.4
- 작업명: SVG로 정확히 표현 가능한 gradient, clip, mask, opacity를 vector로 기록
- 상태: 부분 완료 (`[~]`) — 2026-11-23 lane 검토 판정: clip·mask·opacity·Outside stroke vector 기록과 golden·결정론 test는 끝났습니다. 선행 XI.10.4가 `[ ]`이고(계획 1.2 규칙 4), 행 범위의 gradient는 scene `Paint`에 값이 없어 기록하지 않았으며, browser render 확인은 수동 Quick Look 한 번이라 자동 fixture가 아닙니다. 보고 당시 기록: 완료 (`[x]`, scene이 표현하는 범위).
- 선행: XI.10.4 (`[ ]`). 효과 적용 순서를 고정하는 렌더 행이 아직 없어, 이 행은 SVG 표기가 scene 값만으로
  정확히 정해지는 기능(clip·mask·opacity)만 vector로 둡니다. blur·shadow·blend는 부분 래스터화(XX.4.2) 대상입니다.
- 결정: ADR-0091 결정 1.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust. browser render 확인은 macOS Quick Look(WebKit)).

## 변경 파일 목록

- `crates/export/src/svg/element.rs`: `StrokeMask`(None·InsideClip·OutsideMask), `write_stroke`가 정렬별 자르기 수단을
  받음, Outside stroke용 `write_outside_mask_def`와 `OUTSIDE_MASK_MARGIN`, 단위 테스트.
- `crates/export/src/svg/writer.rs`: Outside stroke를 mask로 기록, `SvgDocument::mask_definitions`.
- `crates/export/src/svg/id.rs`: Outside mask id `<node id>-m`.
- `crates/export/src/svg/mapping.rs`: `OutsideStroke` row를 vector로, Rasterize 지원 상태 설명 갱신, 지원 상태 test 갱신.
- `crates/export/src/svg/writer/tests.rs`, `crates/export/src/svg/writer/tests/vector.rs`(신규): golden·결정론 테스트.
- `crates/export/tests/fixtures/vector_effects_v1.svg`(신규): golden SVG.

## 구현 요약

- **Outside stroke**: 폭을 두 배로 그리고 `<mask maskUnits="userSpaceOnUse">`로 geometry 안쪽 절반을 지웁니다. mask는
  영역 전체 흰색 사각형 위에 geometry를 자기 채우기 규칙(even-odd 포함)으로 검게 칠합니다. 흰색·검은색 luminance는
  색공간과 무관하게 1·0이라 mask 값이 정확합니다. mask 영역은 geometry 경계 상자를 node의 가장 넓은 Outside stroke
  폭 × 4(SVG 기본 miter limit 4에서 join이 닿는 최대 거리)만큼 넓혀 그린 stroke 전체를 덮습니다. node당 mask 하나를
  여러 Outside stroke가 공유합니다. 좌표는 stroke 요소의 user space(`transform` 적용 전 node local)라 Inside clip과
  같은 규약입니다.
- **clip**: 기존 vector 기록(Frame 직각·둥근·path clip, 중첩 clip chain, Inside stroke 자기 clip)을 그대로 씁니다.
- **opacity**: subtree 합성 opacity는 node 그룹 `opacity`, 색 알파는 `fill-opacity`·`stroke-opacity`로 씁니다(기존).
- **gradient**: scene `Paint`에 gradient variant와 기하(방향·중심)가 없습니다(core-types `Gradient`는 stop 목록만).
  기록할 scene 값이 없어 mapping row는 `NotInScene`으로 둡니다. scene에 gradient 기하가 생기면 row를 vector로 바꿉니다.
- 지원 상태가 `Deferred`인 기능이 더는 없습니다. 남은 비vector 기능은 Rasterize(image 색공간 override, drop shadow,
  layer blur, blend)와 NotInScene(gradient 2종)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-export --locked` | 통과(이 작업 신규·갱신: `vector_effects_match_golden`, `vector_effects_are_byte_identical`, `outside_masks_cover_the_stroke_and_cut_the_geometry`, `strokes_center_inside_and_outside`, 지원 상태 inventory) |
| `qlmanage -t -s 480 vector_effects_v1.svg`(WebKit) 후 pixel 표본 비교(허용 오차 채널당 12) | 11개 표본 모두 기대값과 일치 |

browser render 표본(golden SVG, 배율 1.6, sRGB 합성 기대값):

| 위치(user 좌표) | 기대 | 결과 |
|---|---|---|
| Frame 배경 (60,80) | (242,242,230) | (242,242,230) |
| 삼각형 안 (40,28) | 노랑 (255,204,0) | 일치 |
| 삼각형 위 Outside stroke (40,17) | 50% 파랑 over 배경 (121,121,242) | (121,121,243) |
| 삼각형 안쪽 가장자리 (40,22) | 노랑(mask가 안쪽 절반을 지움) | 일치 |
| group opacity 0.5 초록 (72,22) | (121,197,115) | (121,198,115) |
| 중첩 clip 안 반투명 빨강 in group (90,30) | (197,140,115) | (198,140,115) |
| 중첩 clip 밖 (110,30) | 배경 | 일치 |
| even-odd 고리 구멍 (115,80) | 검정(안쪽 contour Outside stroke 6) | 일치 |
| 고리 몸체 (105,80) | 보라 (128,0,128) | 일치 |
| 고리 바깥 stroke (97,80) | 검정 | 일치 |

## 차이(작업 지시와 다른 점)

- gradient: scene `Paint`에 gradient 값이 없어 기록 대상이 없습니다. mapping row는 `NotInScene`으로 둡니다.
- 선행 XI.10.4가 미완료라 효과 적용 순서가 걸린 blur·shadow·blend는 vector로 두지 않고 부분 래스터화로 넘깁니다.

## 남은 위험 / 한계

- gradient는 scene 값이 없어 기록하지 않습니다. scene `Paint` gradient가 생길 때 mapping row와 writer를 함께 바꿔야
  합니다.
- 반투명·opacity 합성은 SVG 렌더러가 sRGB에서, Stellar 렌더러가 선형 공간에서 합니다(아키텍처 §10.3). 구조는 같고 색
  차이는 렌더 행(XI.10.x)의 허용 오차가 정합니다.
- browser render 확인은 Quick Look 한 엔진이며 자동 test가 아닙니다.
