# 작업 보고서: XII.4.4 selection overlay와 handle을 transient scene layer로 렌더

> 아래 초기 구현·검증 기록은 선별 통합 원본 범위 `3892c5e3..2636e8ce`에서 가져왔습니다.
> 현재 기준 `eac4531b` 위의 재검증·수정·남은 범위는 [세션 통합 보고서](../status/session-takeover-integration.md)를 따릅니다.

## WBS

- WBS: XII.4.4
- 작업명: selection overlay와 handles를 transient scene layer로 렌더합니다.
- 상태: 부분 완료 (`[~]`) — 코어 계산·scene layer 생성까지 통합. 실제 CanvasHost 렌더 연결 미완료.
- 선행: XI.3.5 (`[x]`), XII.4.3 (`[~]`, 코어 camera 입력만 통합)

## 변경 파일

- `crates/scene/src/overlay.rs` (신규) — `build_selection_overlay`(문서 scene·선택·camera → view 좌표 외곽선 path와 primary 모서리
  handle frame으로 된 별도 scene layer, 같은 상자의 hit test `TransientLayer`), `SelectionOverlay`, `OverlayTarget`,
  `OverlayStyle`(색·크기는 셸이 넘김), `OverlayError`, 단위 테스트 3개.
- `crates/scene/src/lib.rs` (수정) — 모듈 공개.
- `crates/runtime/src/window_session.rs` (수정) — `ProjectWindowSession::selection_overlay`(화면 선택 + camera), 검증 테스트.
- `docs/task-reports/XII.4.4.md`(신규), 계획서(XII.4.4 `[ ]`→`[~]`)

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-scene --lib overlay` | 0 (3 passed) |
| `cargo test -p stellar-runtime --lib window_session` | 0 (14 passed) |
| `cargo xtask check-core` | 0 (lane 마지막 실행, 아래 lane 보고 참고) |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이므로 없음

## 테스트

- 검증 기준 "project snapshot에 overlay node 없음":
  - `window_session::tests::selection_overlay_renders_as_a_transient_layer_outside_the_project_snapshot`: 저장 프로젝트 창에서 node 11을
    선택·×2 확대하고 overlay를 만들면 외곽선 1 + 모서리 handle 4의 transient layer와 hit test handle 12개가 생기고, 그 layer를 공통
    pass compiler(`compile_frame`, 항등 화면 view)로 draw 5개 이상으로 계획할 수 있음. 문서 scene의 semantic hash·node 수, 작업 문서
    레코드, 직렬화 chunk, Save Now(`written == false`), `design/` 디스크 bytes가 모두 그대로. active 탭이 없으면 overlay 없음. 통과.
  - `overlay::tests::document_scene_never_receives_overlay_nodes`: overlay를 만들어도 문서 scene hash·node 수 불변, overlay layer는
    별도 hash, 빈 선택은 빈 layer, 음수 두께는 구조화 오류. 통과.
  - `overlay_outlines_every_selected_node_and_handles_the_primary`: scene에 없는 선택 제외·선택 순서 유지, 자식 node world 상자,
    camera 적용 view 상자, layer z-order 1부터, handle 크기·위치(view px), 모서리 hit test가 `ResizeCorner(BottomRight)`. 통과.
  - `handles_keep_their_screen_size_at_any_zoom`: 25%·100%·800%에서 handle 크기가 같은 view px. 통과.
- 남은 경고 0.

## 성능

- 문서 scene 전체를 순회해 source 표를 만듭니다(O(N log N)). 선택 중복 검사도 최악 O(S²)이므로 대규모 선택의 성능 검증은 남아 있습니다. 문서 scene 자체는 복사하지 않습니다.

## 차이

- 계획 1.2 규칙 5: 새 module이라 구현 전 테스트는 컴파일 실패로만 재현됩니다.
- "렌더"는 overlay를 공통 scene primitive layer로 만들고 공통 pass compiler가 draw로 계획하는 데까지 확인했습니다. 셸 캔버스 화면에 GPU로
  그리는 단계는 CanvasHost Metal 연결(XII.4.1, 차단)에 달려 있습니다.
- 외곽선·handle 색과 크기는 코어가 정하지 않고 `OverlayStyle`로 받습니다(셸 디자인 명세에 캔버스 overlay 모양이 아직 없어 임의 모양을
  넣지 않음). 그리는 handle은 모서리 크기 조절 4개이고, 변 띠·회전 영역은 hit test 영역으로만 둡니다(XI.3.5).
- 문서 → scene 입력 변환(binding·layout 평가 결과를 `SceneBuilder` 입력으로)은 아직 어느 행에도 구현되지 않아, 테스트는 문서 node id로
  만든 Frame scene을 씁니다.

## 남은 위험

- 셸이 넘길 `OverlayStyle`(운영체제 강조색, HIG handle 크기)과 캔버스 overlay 디자인 명세가 필요합니다.
- 다중 선택은 각 node 외곽선과 primary handle만 그립니다. 다중 선택 전체 상자·회전 표시는 캔버스 편집 명세가 정해지면 바꿉니다.
