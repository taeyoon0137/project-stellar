# 작업 보고서: XII.10.18 눈금자 가이드·Frame 레이아웃 그리드 저장 위치 ADR

- WBS: XII.10.18
- 작업명: 눈금자 가이드와 Frame 레이아웃 그리드의 저장 위치 결정
- 상태: 완료 (`[x]`)
- 결정: ADR-0117 결정 2
- 선행: 없음

## 필수 실행 환경

- `DEV-MAC` (문서 작업, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `docs/adr/ADR-0117-canvas-guides-grids-flow-drag-navigation-clipboard.md`(신규, Accepted)

## 구현 요약

- 가이드와 레이아웃 그리드는 둘 다 문서 데이터입니다(Git·Semantic Diff 대상). 창별 view 상태 안과 비교한 근거, ViewDefinition에 두지 않는 근거를 ADR에 적었습니다.
- 소유자는 Frame node입니다. 가이드는 View root Frame 또는 최상위 Frame local 좌표, 레이아웃 그리드는 어느 Frame에나 붙습니다.
- 값 모델(`stellar_document::guides`의 `RulerGuide`·`LayoutGrid`·`NodeGuides`·`GuideEdit`), canonical bytes와 텍스트 원본 표기, 상한을 정했습니다.
- 파일 포맷(선택 기록, chunk·record kind 예약은 보호 경로 개정 절과 함께), migration(없음, 기록이 없으면 가이드 없음), Semantic Diff 범주(`guides`), Semantic Merge 규칙(guide id별, grid 목록은 값 하나)을 함께 정했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo xtask validate-adr --base 0c226a06` | 종료 코드 0 |

## 차이(작업 지시와 다른 점)

- 없음. 결정을 실제 문서 모델·저장·command에 잇는 일은 ADR-0117 결정 2.4에 후속 범위로 적었습니다(XII.10.19·XII.10.20 보고서 참조).

## 남은 위험 / 한계

- 연결 단계에서 `core-types::reserved`에 chunk·record kind를 예약할 때 이 ADR에 개정 절을 더해야 합니다.
