# 작업 보고서: XXII.2.2 layout record의 CSS layout 속성 출력

## WBS

- WBS: XXII.2.2
- 작업명: IX.11.6 layout record의 CSS 기반 layout 속성을 같은 이름의 CSS 속성으로 출력
- 상태: 부분 완료 (`[~]`) — Web 출력과 변환·진단 규칙은 끝났습니다. 완료 기준의 "IR과 같음"은 아직 확인할 수 없습니다.
  출력이 CodegenIR(XXII.1.1, 차단 상태)을 거치지 않고 저장 문서의 layout record에서 바로 나오기 때문입니다.
  그래서 record와의 동일성으로 대신 확인했습니다.
- 결정: ADR-0089 결정 3·4·6, ADR-0086, 속성 모델 3장·5장 border 행
- 선행: Phase X, XXII.2.1, IX.11.6, IX.11.19

## 변경 파일

- IX.11.30과 같은 커밋입니다(`docs/task-reports/IX.11.30.md` 변경 파일 절).
- 이 행에 해당하는 부분은 다음과 같습니다.
  - `crates/ops/src/export_css.rs`의 `plan_element`: 배치·명시 값·border 보정.
  - `crates/ops/src/web_frame.rs`의 연결.
  - `crates/codegen/src/web/frame.rs`의 paint 분리.
- `docs/task-reports/XXII.2.2.md`(신규).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-ops --lib export_css` | 0 (7 통과) |
| `cargo test -p stellar-cli --test export_styles --test web_frame` | 0 (3 + 7 통과) |
| `cargo xtask validate-adr --base 33ba9b2f` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 없음.

## 테스트

- 같은 이름·값: `export_web_rules_carry_the_resolved_explicit_values`.
  - record와 payload 크기의 명시 longhand가 export-web 요소 규칙에 같은 이름·정규 값으로 있습니다.
  - 예외: 배치 규칙이 정하는 위치 longhand, root의 item longhand.
- 변환 규칙: `stellar_border_is_converted_without_moving_layout_boxes`. Stellar border(공간 없음) 보정은 다음과 같습니다.
  - `box-sizing: border-box`
  - content-box 크기 + padding 합(`108px`, `calc(50% + 8px)`)
  - padding − border(`1px`, `0px`, `4px`)
- 진단: padding이 border보다 작은 흐름 부모의 `approximated-border-padding` 3건.
- 배치: `placements_follow_parent_display_and_explicit_position`.
  - flex 부모 → flow, `display`를 명시한 block 부모 → flow, 명시 inset → inset, 숨김 → `display: none`.
  - 흐름 item의 transform 이동은 matrix에 남습니다.
- cascade 계산 대조: `computed_cascade_matches_document_resolution_for_both_sets`(IX.11.30 보고서 참고).

## 성능

- 해당 없음.

## 차이

- 작업 행은 IR(XXII.1.1)을 입력으로 가정합니다. XXII.1.1이 차단 상태라 출력은 저장 문서의 layout record와 문서 해석
  함수에서 바로 나옵니다. IR이 layout record를 받게 되면 같은 계획을 IR lowering으로 옮겨야 합니다.
- IX.11.9 전이라 저장 transform 이동을 위치로 쓰는 연결 규칙(Free 배치)을 두었습니다(ADR-0089 결정 4).

## 남은 위험

- "변환 규칙 없는 속성의 이름·값이 IR과 같음" 기준은 IR 경로가 생긴 뒤 다시 확인해야 합니다.
- Text·Image·Path 등 export가 아직 받지 않는 node 종류의 layout record는 출력되지 않습니다(기존처럼 실패).
- 불가능한 constraint 진단은 border 보정의 근사와 효과 없는 inset만 다룹니다. grid container 안 absolute의 명시
  line 배치처럼 엔진이 거부하는 조합(ADR-0076 결정 7)은 export에서 따로 진단하지 않습니다.
