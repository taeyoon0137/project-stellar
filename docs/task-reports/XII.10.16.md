# 작업 보고서: XII.10.16 스마트 가이드와 측정 overlay

- WBS: XII.10.16
- 작업명: 스마트 가이드와 측정 overlay
- 상태: 부분 완료 (`[~]`) — 자료·transient layer 완료, ShellTheme 값·셸 그리기 남음
- 결정: ADR-0105 결정 3(C2, C5)
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/snap/guides.rs`(신규)

## 구현 요약

- `smart_guides`: 정렬선(움직인 상자와 같은 좌표의 후보를 덮는 선), 가장 가까운 형제 거리 라벨, 같은 간격·같은 크기 표시.
- `measure`: 선택 후 Option-hover 거리(떨어지면 마주 보는 가장자리, 품으면 네 변 안쪽).
- `build_guide_layer`: 화면 좌표 scene snapshot(선·라벨 배경)과 라벨 문자열 자리. 문서·저장 경로 밖입니다.
- C2 결정: 라벨 glyph는 셸 시스템 UI 글꼴이고 코어는 문자열(CSS 길이 표기)과 자리만 냅니다. 근거는 라벨이 도구 UI이고 접근성 설정을 셸이 따라야 한다는 점입니다.
- C5 결정: Text 측정은 layout 상자 가장자리 기준이고 baseline 거리는 내지 않습니다. 근거는 측정값이 CSS `margin`·`gap`·inset 값이 되어야 한다는 점입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib -- canvas::snap::guides` | 통과(4): 정렬선·거리 라벨, 같은 간격·크기, Option-hover, layer 노드·라벨 자리와 모양 차이 |

## 차이(작업 지시와 다른 점)

- 대비 높이기 on/off golden은 ShellTheme(XII.9.3) 값이 없어 서로 다른 `GuideStyle` 두 개로 layer가 달라지는 것만 확인했습니다.
- overlay가 project snapshot에 없음은 layer가 별도 `SceneSnapshot`이고 문서 command를 만들지 않는다는 것으로 확인했습니다.
- 셸 UI·FFI·메뉴·`ShortcutPolicy` 부분은 사용자 방향(2026-11-23, ADR-0098)에 따라 하지 않았습니다. UI 재설계 때 합니다.

## 남은 위험 / 한계

- ShellTheme 값이 정해지면 golden fixture를 더해야 합니다.
