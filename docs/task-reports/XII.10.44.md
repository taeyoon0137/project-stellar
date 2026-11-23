# 작업 보고서: XII.10.44 Flip Horizontal·Flip Vertical

- WBS: XII.10.44
- 작업명: Flip Horizontal·Flip Vertical
- 상태: 부분 완료 (`[~]`) — 코어 완료, `scale` longhand 표현(IX.11.9)·메뉴 남음
- 결정: ADR-0105 결정 5
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/commands/src/flip.rs`(신규)
- `crates/commands/src/arrange/plan.rs`(`flip`)
- `crates/runtime/src/canvas/transform/arrange.rs`(`flip`, `can_flip`)

## 구현 요약

- 축 결정: world 축(선택 world 경계 중심을 지나는 선). 근거는 회전 node도 화면 방향 그대로 뒤집혀야 하고 다중 선택의 공통 축과 규칙이 하나여야 한다는 점입니다.
- 표현 결정: 위치 변환 선형부 부호 반전(`matrix(-1, 0, 0, 1, …)`). Frame·Shape·Path·Text·Image 같은 규칙이고 Path·Polygon geometry는 따로 반전하지 않습니다. 배치한 인스턴스·Fragment는 비활성입니다.
- 명령 하나가 CommandTransaction 하나입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --lib -- flip` | 통과(4): 중심 불변, 두 번 반전 원복, undo, 다중 선택·종류별(Frame·Rectangle·Ellipse·Line·Text), 회전 부모 공간 변환, 비활성 |
| `cargo test -p stellar-runtime --lib -- canvas::transform::arrange` | 통과: 단일·다중 선택 중심 불변·두 번 원복, 45도 회전 node world 거울상 |

## 차이(작업 지시와 다른 점)

- 완료 기준의 Frame `scale` 부호 반전은 `scale` longhand가 없어 선형부 반전으로 대신합니다. 위치·선형부는 지금 저장 위치(Frame `transform`, 그 밖은 위치 Transform modifier)에 씁니다. IX.11.9 step이 제품 목록에 없고 scene이 layout record 위치를 그리지 않기 때문입니다(ADR-0086 결정 3, ADR-0105 결정 1). step이 등록되면 `stellar_commands::arrange::place` 한 곳만 바꿉니다.
- migration 노드(`transform-origin: 0 0`) fixture는 저장 origin이 없어 만들 수 없습니다.
- 셸 UI·FFI·메뉴·`ShortcutPolicy` 부분은 사용자 방향(2026-11-23, ADR-0098)에 따라 하지 않았습니다. UI 재설계 때 합니다.

## 남은 위험 / 한계

- IX.11.9 뒤 Frame은 `scale` 부호 쓰기로 바꾸고 다시 판정해야 합니다.
