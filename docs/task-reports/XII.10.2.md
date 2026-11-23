# 작업 보고서: XII.10.2 이동·크기 단위와 수식키 결정

- WBS: XII.10.2
- 작업명: 화살표·크기 조절 키 단위, 드래그 수식키, 재부모화 방지 수식키, 스냅 허용 오차, 줌 단계·상하한, 핸들 숨김 기준 ADR
- 상태: 완료 보고. 원장·계획서 marker는 통합 때 갱신합니다.
- 선행: 없음

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `docs/adr/ADR-0104-canvas-tools-units-and-selection-core.md`(신규) — 결정 2 표(단위·허용 오차·줌·핸들 수치)와 수식키 표.
- `crates/runtime/src/canvas/tool_units.rs`(신규) — ADR 값의 코어 상수와 줌 단계·허용 오차 변환 함수.
- `crates/runtime/src/canvas.rs` — 모듈 등록.

## 구현 요약

- 열린 결정 O10을 ADR-0104 결정 2로 닫았습니다. 화살표 1 px, Shift 10 px, ⌘-화살표 크기 조절 1 px·⌘⇧ 10 px(오른쪽·아래 변),
  드래그 수식키(Shift 축 고정·비율·15도, Option 복제·중심 기준·완전 포함, ⌘ 깊은 선택과 재부모화 방지, Space 일시 Hand·시작점
  이동), 스냅 허용 오차 5 pt, 줌 2%\~25600%와 단계 14개, 픽셀 그리드 800% 이상, 핸들 숨김 8 pt·24 pt, 반경 핸들 40 pt 이상입니다.
- 줌 상하한과 끌기 시작 거리는 기존 정의(`view_tabs::MIN_ZOOM`·`MAX_ZOOM`, `InteractionConfig::DEFAULT_DRAG_THRESHOLD`)를 다시
  내보내 같은 값을 두 곳에 두지 않았습니다.
- 가이드 12장은 ADR-0098로 지워져 ADR-0104 표를 기록 원본으로 삼습니다. 테스트가 ADR 파일의 표 행을 읽어 코어 상수와 비교합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib tool_units` | 3 passed |
| `cargo xtask validate-adr --base 682c7b0c` | 통과(오류 출력 없음) |

| 검사 | 결과 |
|---|---|
| ADR 표 행마다 코어 상수 값이 들어 있고 ADR이 Accepted(`constants_match_the_adr_table`) | 통과 |
| 줌 단계가 viewport 범위를 오름차순으로 덮고 단계 이동·픽셀 그리드 경계가 맞음(`zoom_steps_cover_the_viewport_range_in_order`) | 통과 |
| 줌 25%·400%에서 화면 허용 오차 동일(`snap_tolerance_is_constant_on_screen`) | 통과 |

## 차이(작업 지시와 다른 점)

- "캔버스 명세 3·5·7·16장 제안과 비교"와 "가이드 12장 기록"은 두 문서가 ADR-0098로 지워져, 계획서 문구, ADR-0071(셸 임시 값),
  ADR-0074(간격 핸들 없음)와 비교했고 기록은 ADR-0104 표로 대신했습니다.

## 남은 위험 / 한계

- 수치는 사용성 측정 전 값입니다. 새 UI 설계 때 바꾸면 ADR 표와 상수를 함께 고칩니다(테스트가 불일치를 잡습니다).
