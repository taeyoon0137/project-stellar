# 작업 보고서: XII.10.20 Frame 레이아웃 그리드와 픽셀 그리드

- WBS: XII.10.20
- 작업명: Frame 레이아웃 그리드(열·행·폭·gutter)와 픽셀 그리드 overlay, 레이아웃 그리드의 문서 저장 연결
- 상태: 부분 완료 (`[~]`) — 값·트랙 계산·스냅 선·표시 계산과 문서 모델·command·저장·텍스트 원본·Semantic Diff·Merge·CLI·MCP 연결
  완료. overlay 그리기(셸·renderer transient layer)는 UI 재설계 때 연결합니다.
- 결정: ADR-0117 결정 2·4, ADR-0126
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple Silicon). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

1차(값·트랙·스냅·표시 계산):

- `crates/runtime/src/canvas/layout_grid.rs`, 값 모델 `crates/document/src/guides.rs`(`LayoutGrid`, `tracks`)

2차(문서 저장 연결, ADR-0126): 가이드와 같은 기록(`NodeGuides`)이라 XII.10.19 보고서의 변경 파일과 같습니다. 그리드에 해당하는 부분:

- `crates/document/src/guides.rs`: `LayoutGrid::to_line`·`GuideLine::parse`(그리드 줄을 검사·정규화), `GuideEdit::SetGrids` bytes
- `crates/commands/src/guides.rs`: `SetNodeGuides { edit: SetGrids(..) }`(그리드 목록 교체 = history 한 항목)
- `crates/semantic-diff/src/guides.rs`: 그리드 목록 = `[0xED02]` 속성 하나(순서 있는 값)
- `crates/ops/src/guides.rs`: 조회 결과에 Frame 크기로 `LayoutGrid::tracks`가 계산한 트랙

## 구현 요약

- 그리드는 Frame의 `NodeGuides` 기록 안 순서 있는 목록입니다. 저장(subtree 확장 절), 텍스트 원본(`guides` 목록의 `grid …` 줄),
  command(`SetGrids`), 외부 반영(tag 9), Semantic Diff(목록 값 하나)를 가이드와 같은 경로로 지납니다.
- 그리드는 layout 입력이 아니므로 command 무효화가 비어 있고 layout·scene 결과를 바꾸지 않습니다(ADR-0074, ADR-0117 결정 4.4).
- 그리드 목록은 영속 id가 없고 순서가 그리는 순서라 Merge에서 값 하나입니다: 양쪽이 다르게 바꾸면 값 충돌, 같게 바꾸면 충돌 없음.
- `stellar guides`·MCP `document.guides`는 그리드 줄, 표시 여부, Frame 크기로 계산한 열·행 트랙(CSS 길이 문장)을 냅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::layout_grid` | 1차 기준 통과(4): 12열 그리드 snap, 숨긴 그리드 스냅, 표시 상자, 줌 경계 |
| `cargo test -p stellar-commands --lib guides` | 통과: `SetGrids`를 포함한 편집 연쇄의 역과 bytes 왕복 |
| `cargo test -p stellar-semantic-merge --test guides_merge` | 통과: `grid_lists_changed_on_both_sides_conflict_as_one_value` |
| `cargo test -p stellar-cli --test guides` | 통과: 12열 그리드 줄과 트랙 12개 조회 |

## 차이(작업 지시와 다른 점)

- overlay를 실제로 그리는 일(셸·renderer transient layer)은 셸 재설계 때 합니다. 코어는 그릴 상자·선 값까지 냅니다.
- "그리드 유무에 따른 layout 결과 동일"은 그리드가 layout 입력이 아니라는 구조와 빈 무효화로 보장합니다.

## 남은 위험 / 한계

- 정사각 그리드는 축당 10,000 트랙에서 자릅니다. 아주 큰 Frame의 작은 칸은 앞쪽만 스냅 후보가 됩니다.
- Frame 복제·붙여넣기는 그리드를 옮기지 않습니다(ADR-0126 Consequences).

## 후속 수정 (2026-11-24 instance-integrity)

- 위 "Frame 복제·붙여넣기는 그리드를 옮기지 않습니다"를 해소했습니다. 레이아웃 그리드는 가이드와 같은 Frame 기록이라
  복제·붙여넣기·삭제 undo에서 layout record와 같은 규칙으로 따라갑니다(ADR-0133 결정 4). 변경·검증은 XII.10.19 보고서의
  같은 날짜 절에 적었습니다.
