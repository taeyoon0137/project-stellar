# 작업 보고서: XII.10.14 키보드 이동·크기 조절

- WBS: XII.10.14
- 작업명: 키보드 이동·크기 조절
- 상태: 부분 완료 (`[~]`) — 코어 완료, 단위 값(XII.10.2)·키 배정·셸 연결 남음
- 결정: ADR-0105 결정 2
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/transform/keyboard.rs`(신규)

## 구현 요약

- 화살표 이동은 기존 부모 공간 이동 경로(반복 grouping)입니다.
- 흐름 자식의 주축 화살표는 흐름 순서를 한 칸 바꾸고(교차축은 그대로), 선택이 없으면 camera를 팬합니다.
- 크기 조절 키는 오른쪽·아래 가장자리를 옮기고(최소 1px) 같은 node·속성 반복을 VII.4.4 규칙으로 한 항목에 합칩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib -- canvas::transform::keyboard` | 통과(4): 크기 키 20회 history 1건, 화살표 이동, 선택 없음 revision 불변, 흐름 자식 재정렬 |

## 차이(작업 지시와 다른 점)

- Rectangle 화살표 이동이 바꾸는 값은 위치 Transform modifier입니다(item record 위치 longhand 아님). 위치·선형부는 지금 저장 위치(Frame `transform`, 그 밖은 위치 Transform modifier)에 씁니다. IX.11.9 step이 제품 목록에 없고 scene이 layout record 위치를 그리지 않기 때문입니다(ADR-0086 결정 3, ADR-0105 결정 1). step이 등록되면 `stellar_commands::arrange::place` 한 곳만 바꿉니다.
- 단위(`step`)와 키 배정은 XII.10.2(ADR-0104) 값을 호출자가 넘깁니다.
- 셸 UI·FFI·메뉴·`ShortcutPolicy` 부분은 사용자 방향(2026-11-23, ADR-0098)에 따라 하지 않았습니다. UI 재설계 때 합니다.

## 남은 위험 / 한계

- 화살표 반복 grouping은 실제 시계(0.5초) 기준이라 부하가 크면 항목이 나뉠 수 있어 test는 상한(2)만 봅니다.
