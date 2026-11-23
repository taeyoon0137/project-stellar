# 작업 보고서: XII.10.12 비례 확대(Scale) 명령

- WBS: XII.10.12
- 작업명: 비례 확대(Scale) 명령
- 상태: 부분 완료 (`[~]`) — 크기·위치·modifier·반경 완료, 테두리 두께·글자 크기 command 없음
- 결정: ADR-0105 결정 4
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/commands/src/scale.rs`(신규)
- `crates/commands/src/arrange/plan.rs`(`scale`)
- `crates/runtime/src/canvas/transform/arrange.rs`(`scale`)

## 구현 요약

- 고른 node와 자손 전체의 크기·위치를 같은 비율로 바꿉니다(기준점 고정, 선형부 유지).
- Stroke 두께, StrokeExpand 폭, Corner·Frame·Rectangle·Polygon 반경, Shadow blur·offset longhand(0x1094·0x1095), Blur 반경을 같은 비율로 바꿉니다.
- 쓸 수 없는 값은 건너뛰고 이유 코드(`border-width`, `font-size`, `instance-size`, `path-geometry`)로 돌려줍니다.
- 한 transaction(Undo 한 번)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --lib -- scale` | 통과(2): 200% 크기·위치·반경·Stroke·Shadow, undo 1회 원복, 비율 범위 거부 |
| `cargo test -p stellar-runtime --lib -- canvas::transform::arrange::tests::scale` | 통과: 창 history 1건과 undo |

## 차이(작업 지시와 다른 점)

- Frame `border-*-width`는 longhand command가 없어(IX.11.10 남은 범위) 건너뜁니다. 묶음 key 0x0105는 쓰지 않습니다.
- Text `font-size`는 node 하나의 command가 없어(IX.11.11 남은 범위) 건너뜁니다. 완료 기준의 "text·Frame border 두께까지"는 미충족입니다.
- 위치·선형부는 지금 저장 위치(Frame `transform`, 그 밖은 위치 Transform modifier)에 씁니다. IX.11.9 step이 제품 목록에 없고 scene이 layout record 위치를 그리지 않기 때문입니다(ADR-0086 결정 3, ADR-0105 결정 1). step이 등록되면 `stellar_commands::arrange::place` 한 곳만 바꿉니다.
- 셸 UI·FFI·메뉴·`ShortcutPolicy` 부분은 사용자 방향(2026-11-23, ADR-0098)에 따라 하지 않았습니다. UI 재설계 때 합니다.

## 남은 위험 / 한계

- IX.11.10·IX.11.11 command가 생기면 `scale.rs`의 두 건너뜀을 쓰기로 바꿔야 합니다.
