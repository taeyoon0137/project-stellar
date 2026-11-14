# 작업 보고서: XI.3.5 selection handle·modifier control point transient hit-test layer

- WBS: XI.3.5
- 작업명: selection handle와 modifier control point를 별도 transient hit-test layer로 추가, project 저장 미포함
- 상태: 완료 (`[x]`)
- 선행: XI.3.3 (`[x]`, `033f4a1`), VII.3.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/transient.rs`(신규) — `TransientLayer`(`push`, `push_selection_box`, `hit`, `clear`),
  `TransientHandle`, `HandleKind`(`priority`), `HandleShape`(`circle`, `square`, `segment`, `outside_corner`),
  `HandleMetrics`, `Corner`, `Edge`, `ControlPointId`, `TransientHit`, `CanvasHit`, `hit_test_canvas`.
- `crates/scene/src/transient/tests.rs`(신규) — 우선순위 fixture 등 테스트 5개.
- `crates/scene/src/lib.rs` — `transient` 모듈 등록.
- `docs/task-reports/XI.3.5.md`(신규), 계획서(XI.3.5 `[ ]`→`[x]`).

## 구현 요약

- **별도 layer**: handle은 창 상호작용 상태(선택은 `WindowViewState`, VII.3.1)에서 만든 일시 객체라
  `SceneSnapshot`·공간 인덱스·scene codec 어디에도 들어가지 않는 `TransientLayer`로 둡니다. layer에는 직렬화
  경로가 없어 project 저장·semantic hash에 포함될 수 없습니다. scene은 commands crate에 의존하지 않으므로(II.2.8)
  선택 상자 꼭짓점과 control point 위치는 호출자가 world 좌표로 넘깁니다.
- **좌표**: 위치는 world, 크기는 view px입니다. hit test 때 viewport로 위치만 view로 옮겨 zoom과 무관하게 화면에서
  같은 크기로 잡힙니다. 회전한 선택 상자(임의 사각형 꼭짓점)도 그대로 받습니다.
- **handle 종류와 영역**: 크기 조절 모서리(view 정사각형), 크기 조절 변(선분 띠), 회전(모서리 반지름 안이면서
  상자 **밖**), modifier control point(원, 식별자 `ControlPointId`). `push_selection_box`가 회전 4·변 4·모서리 4를
  한 번에 추가합니다.
- **우선순위**: handle은 항상 scene node보다 먼저 판정합니다. handle끼리는 종류 우선순위(control point 4 > 모서리 3 >
  변 2 > 회전 1), 같으면 view 거리(원·선분은 유클리드, 정사각형은 Chebyshev)가 가까운 쪽, 그것도 같으면 나중에 추가한
  handle이 이깁니다. `hit_test_canvas`는 handle이 없으면 narrow-phase scene hit test(XI.3.4) topmost를 돌려줍니다.
- 크기 값은 finite·비음수만 받고(`SceneValueError`), 면적 0 상자는 회전 영역 판정에서 상자 밖으로 봅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-scene` | 완료 |
| `cargo test -p stellar-scene` | unit 95, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |

| 검사(검증 기준: handle 우선순위 fixture) | 결과 |
|---|---|
| 200×100 선택 상자 + control point 2개: 꼭짓점 위는 모서리(변보다 앞섬, 상자 경계는 회전 아님), 변 띠, 모서리 바깥 회전, control point가 모서리와 겹치고 더 멀어도 우선, 상자 안 모서리 근처는 회전 아님 → node, 아무 영역도 아니면 Empty, handle이 node보다 먼저(`handle_priority_fixture`) | 통과 |
| 6×6 px로 축소된 상자에서 겹친 모서리 넷: 같은 거리면 나중 handle, 가까우면 거리 우선, 완전히 같은 handle은 나중 것, `clear` 후 미적중(`overlapping_handles_resolve_by_priority_then_distance_then_order`) | 통과 |
| 4배 확대·1/4 축소에서 handle이 view px 크기 유지, 45° 회전 상자의 회전 영역은 상자 밖에서만(`handles_keep_view_size_under_zoom_and_rotation`) | 통과 |
| layer 생성·canvas hit test 후 snapshot canonical bytes·semantic hash·index 크기·node 수 불변, 빈 layer면 narrow-phase topmost와 같음(`transient_layer_is_not_part_of_scene_or_save`) | 통과 |
| 음수·NaN·∞ 크기 거부, 공개 생성자와 `push_selection_box` 결과 일치, 면적 0 상자 회전 가능(`handle_sizes_are_validated`) | 통과 |

- **변이 확인**: 모서리 우선순위를 회전 아래로, control point 우선순위를 0으로, 동률 시 나중 handle 우선 제거,
  회전 영역의 상자 밖 조건 제거, canvas에서 handle 판정 생략 — 5개 변이가 각각 테스트를 실패시켰고 원본으로
  복원했습니다.

## 차이(작업 지시와 다른 점)

- handle 배치(크기·회전 영역 반지름)와 control point 목록 생성은 호출자(interaction 계층, XII.3.6·XII.4.4)가
  정합니다. 이 작업은 판정 layer와 우선순위 규칙까지입니다.

## 남은 위험 / 한계

- 우선순위 수치(control point > 모서리 > 변 > 회전)는 이 작업에서 정한 기본값입니다. 도구별로 달라야 하면
  interaction state machine(XII.3.6)에서 handle 구성을 바꿔야 합니다.
- overlay 렌더(XII.4.4)는 이 layer를 그리지 않습니다. 같은 handle 목록으로 그리도록 연결해야 합니다.
