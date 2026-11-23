# 작업 보고서: XII.10.8 변형 핸들 hit layer

- WBS: XII.10.8
- 작업명: 모서리·변 크기 핸들, 모서리 바깥 회전 영역, 모서리 안쪽 반경 핸들의 우선순위와 작은 선택의 핸들 숨김, 회전 node의 local 좌표 계산
- 상태: 완료 보고. 원장·계획서 marker는 통합 때 갱신합니다.
- 선행: XI.3.5, XII.4.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/runtime/src/canvas/handles.rs`(신규) — `CanvasHandle`, `HandleTarget`, `HandleLayer`, `HandleHit`, `build_handles`.
- `crates/runtime/src/canvas.rs` — 모듈 등록.

## 구현 요약

- 우선순위는 반경 > 크기 조절 모서리 > 크기 조절 변 > 회전 > 본체입니다(ADR-0104 결정 4). ADR-0074 결정 3으로 padding·gap 핸들 자리는
  없습니다. 반경 핸들은 XI.3.5 transient layer의 modifier control point로 넣어 같은 hit 규칙을 그대로 씁니다. 핸들이 없으면 선택 node
  world 사각형 안이 본체입니다.
- 단일 선택은 node local 사각형 꼭짓점을 world로 옮긴 상자라 회전한 node의 핸들이 local 축을 따릅니다. 다중 선택은 선택 bounds의 합이고
  상자 핸들은 primary를 조작합니다.
- 반경 핸들 위치는 local 모서리에서 안쪽으로 `max(실효 반경, 12pt를 local 단위로 바꾼 값)`이며 변 절반에서 멈춥니다. 실효 반경은
  Frame·Rectangle은 `RectangleParams::effective_corner_radius`(VI.5.2 평가 규칙)를 그대로 쓰고, Polygon은 단일 반경을 짧은 변 절반으로
  자른 값입니다.
- 숨김 기준(ADR-0104 결정 2 표): 화면 짧은 변 8pt 미만은 크기·반경 핸들 없음, 24pt 미만은 변 핸들 없음, 반경 핸들은 단일 선택이고 40pt
  이상일 때만. 길이 0인 축의 크기 핸들은 XI.3.5처럼 만들지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::handles` | 4 passed |

| 검사 | 결과 |
|---|---|
| 반경 핸들(12,12 안쪽)·모서리·변·회전·본체·바깥 순 hit, 큰 반경은 변 절반에서 멈춤, 우선순위 순서(`priority_order_is_radius_resize_corner_edge_rotate_body`) | 통과 |
| 40×40 상자의 줌별 숨김(40pt 전부, 39.6pt 반경 숨김, 23.6pt 변 숨김, 7.6pt 크기 핸들 숨김, 24pt·8pt 경계 포함), 다중 선택 상자와 primary, 반경 없는 node(`small_selections_hide_handles_by_screen_size`) | 통과 |
| 45도 회전 node의 상자 꼭짓점, 회전 모서리 hit, 반경 핸들 local (12,12)의 world 위치, 변 띠(`rotated_node_handles_follow_local_axes`) | 통과 |
| 문서·scene에서 만든 Frame·Rectangle·Fragment 대상, 핸들을 만든 뒤 scene semantic hash·node 수·save bytes 불변(`handles_come_from_the_document_and_stay_out_of_the_snapshot`) | 통과 |

## 차이(작업 지시와 다른 점)

- padding·gap 핸들 자리는 ADR-0074 결정 3에 따라 두지 않습니다(계획 문구의 우선순위 목록에서 그 단계만 뺌).
- Line 끝점 핸들(ADR-0071 UX.2.9)은 이 행 범위 밖이라 Line은 일반 상자 규칙(길이 0 축 핸들 없음)을 따릅니다.

## 남은 위험 / 한계

- 핸들을 overlay로 그리는 연결(XII.4.4 overlay에 반경 핸들 추가)과 핸들 드래그의 편집(XII.10.9 크기, XII.10.10 회전, XII.10.11 반경)은
  각 행의 몫입니다.
