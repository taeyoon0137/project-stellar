# 작업 보고서: XII.10.3 클릭 선택

- WBS: XII.10.3
- 작업명: hit 결과를 편집 범위 직계 자식까지 올리는 클릭 선택, Shift 토글, 빈 곳 해제, ⌘A, 최상위 Escape 해제, Option primary
- 상태: `[x]` 완료 보고. 원장·계획서 marker는 통합 때 갱신합니다.
- 선행: XI.3.4, VII.3.2, XII.10.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/runtime/src/canvas/select.rs`(신규) — `SelectTree`, `EditingScope`, `SelectionChange`, `click_pick`,
  `resolve_click`, `select_all`, `escape`.
- `crates/runtime/src/canvas/hit.rs`(신규) — `PointStack`(커서 아래 node, 잠금 표시), `point_stack`(scene hit test 연결),
  `CanvasGeometry`(node world 상자, Fragment는 자손 합).
- `crates/runtime/src/canvas/select_fixtures.rs`(신규, 테스트 전용) — `canvas/select-scope-nested` 문서와 scene 생성.
- `crates/runtime/src/canvas/tool.rs` — 테스트의 save bytes 도우미를 fixture 하나로 모음.
- `crates/runtime/src/canvas.rs` — 모듈 등록.

## 구현 요약

- 클릭 규칙은 ADR-0104 결정 3.3입니다. 커서 아래 가장 위의 잠기지 않은 node(leaf)를 편집 범위 직계 자식까지 올립니다. leaf가
  범위 밖이면 가장 가까운 공통 조상으로 범위를 맞추고, 범위 자신·조상의 몸체를 누르면 그 node를 고르고 범위를 한 단계 올립니다.
  Fragment는 범위 직계 자식으로 묶음 선택되고, 인스턴스 안 scene node는 source가 인스턴스라 한 덩어리입니다.
- ⌘는 깊은 선택, Shift는 토글(범위는 선택 부모들의 공통 조상으로), Option은 다중 선택의 선택된 node를 primary로 지정합니다. 빈
  곳은 해제와 최상위 범위 복귀이고, ⌘A는 범위의 선택 가능 자식 전체입니다. 잠김·숨김·View root는 선택하지 않습니다(잠금은
  `commands::lock::NodeLock`, 숨김은 `SelectionPolicy` 판정을 그대로 씀).
- 결과는 `SelectionChange`(새 선택·범위)이고 기록 event는 `SetSelection` 하나(바뀌지 않으면 없음)입니다.
- Escape는 ADR-0104 결정 3.5 순서(제스처 취소 → 도구 복귀 → 범위 위로 → 최상위 해제)를 한 함수로 둡니다. 범위 위로 단계의
  검증은 XII.10.4에서 합니다.
- hit 입력은 scene narrow-phase hit test(XI.3.4)를 그대로 쓰고, 잠긴 node는 겹친 레이어 목록(XII.10.5)을 위해 표시만 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::` | 35 passed |

| 검사 | 결과 |
|---|---|
| 문서 → scene → 공간 인덱스로 만든 hit 목록에서 클릭 위치·범위·수식키별 선택과 범위 10건(`nested_fixture_click_matrix_from_scene_hits`) | 통과 |
| 인스턴스 한 덩어리(⌘에서도), 잠김·숨김·root 선택 불가, 잠긴 node 위 클릭 통과(`instance_is_one_block_and_hidden_locked_never_selected`) | 통과 |
| Shift 토글·Option primary·빈 곳 해제 event 1건·같은 선택 event 없음·⌘A·다른 깊이 Shift 추가의 범위(`shift_toggles_option_sets_primary_and_select_all_takes_scope_children`) | 통과 |
| 클릭 1회 history 1건, Escape 순서(제스처 취소 → 도구 복귀 → 해제), 해제 1건, 빈 선택 Escape 없음(`top_scope_escape_deselects_once_and_records_one_entry`) | 통과 |
| 잠긴 node를 위에서부터 표시하고 숨김 제외, Fragment bounds는 자손 합(`point_stack_lists_locked_nodes_topmost_first_and_skips_hidden`) | 통과 |

## 차이(작업 지시와 다른 점)

- 캔버스 명세 3.2·3.3·8장은 ADR-0098로 지워져 계획서 문구와 ADR-0104 결정 3으로 규칙을 정했습니다.
- 범위 자신·조상 몸체 클릭은 그 node를 고르는 규칙으로 정했습니다(View root 몸체만 빈 곳). 지운 명세에 다른 규칙이 있었다면
  UI 재설계 때 ADR로 바꿉니다.
- `canvas/select-scope-nested` fixture의 ComponentInstance는 component library가 없어 scene에 그려지지 않으므로, 인스턴스 hit은
  scene source 규칙(인스턴스 안 node의 source는 인스턴스)대로 만든 hit 목록으로 검증했습니다.

## 남은 위험 / 한계

- `app::canvas_gesture`의 옛 선택 경로(ADR-0071)는 소유 범위 밖이라 그대로 있습니다. 새 UI 연결 때 이 경로로 바꾸고 지웁니다
  (ADR-0104 결정 6).
- 창 session에 편집 범위 상태를 붙이는 연결은 셸 연결 때 합니다.
