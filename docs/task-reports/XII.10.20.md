# 작업 보고서: XII.10.20 Frame 레이아웃 그리드와 픽셀 그리드

- WBS: XII.10.20
- 작업명: Frame 레이아웃 그리드(열·행·폭·gutter)와 픽셀 그리드 overlay
- 상태: 부분 완료 (`[~]`) — 값·트랙 계산·스냅 선·표시 계산 완료, 문서 모델·command·저장 연결과 overlay 그리기 남음
- 결정: ADR-0117 결정 2·4
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/layout_grid.rs`(신규), `crates/runtime/src/canvas.rs`(mod 등록)
- 값 모델은 XII.10.19 커밋의 `crates/document/src/guides.rs`(`LayoutGrid`, `tracks`)입니다.

## 구현 요약

- `LayoutGrid`(문서 값): `columns`·`rows`·`square`, `count`, `size`(`auto` 또는 px), `gap`(gutter), `margin`, `align`(`stretch`·`start`·`center`·`end`), `visible`. 이름은 CSS 어휘이고 layout 입력에 들어가지 않습니다(ADR-0074).
- `tracks(width, height)` 하나가 트랙을 계산합니다. `auto`는 늘림(양쪽 margin과 gap을 빼고 나눔), 고정 크기는 정렬 기준으로 놓습니다.
- `grid_snap_lines`: 트랙 시작·끝 가장자리를 스냅 엔진 `SnapLines::grid`로 냅니다. 표시 여부와 무관합니다.
- `grid_bands`: 레이아웃 그리드 토글과 그리드 `visible`이 켜졌을 때만 화면에 그릴 트랙 상자를 냅니다.
- `pixel_grid_lines`: 토글이 켜졌고 배율 800% 이상(ADR-0104 표)일 때만 가시 영역의 정수 world 선을 화면 좌표로 냅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::layout_grid` | 통과(4): 12열 그리드 snap 결과(224→222, 후보 종류 GridLine), 숨긴 그리드도 같은 스냅 선, 토글·원점에 따른 표시 상자, 줌 경계 표시(799%는 없음, 800%는 8pt 간격 선), 그리드 계산 전후 scene·save bytes 동일 |
| `cargo test -p stellar-document --lib guides` | 통과(5) |

## 차이(작업 지시와 다른 점)

- 그리드를 문서에 저장하고 command로 편집하는 연결은 XII.10.19와 같은 이유로 ADR-0117 결정 2.4의 후속 범위입니다.
- overlay를 실제로 그리는 일(셸·renderer transient layer)은 셸 재설계 때 합니다. 코어는 그릴 상자·선 값까지 냅니다.
- "그리드 유무에 따른 layout 결과 동일"은 그리드가 layout 입력이 아니라는 구조로 보장하고, test는 그리드 계산 전후의 게시 scene과 save bytes가 같은지로 확인했습니다. layout crate 결과 비교는 runtime이 layout crate에 의존할 수 없어 하지 않았습니다.

## 남은 위험 / 한계

- 정사각 그리드는 축당 10,000 트랙에서 자릅니다. 아주 큰 Frame의 작은 칸은 앞쪽만 스냅 후보가 됩니다.
