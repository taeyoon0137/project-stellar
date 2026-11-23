# 작업 보고서: IX.11.9 Frame 위치와 transform 분리

- WBS: IX.11.9
- 작업명: Frame 위치와 transform 분리 migration
- 상태: 부분 완료 (`[~]`) — 레코드 변환, CSS 순서 합성, rotate(O13) 규칙은 끝났습니다. 2026-11-24 layout-render-wire: scene이 layout 결과 위치와 저장 transform을 CSS 순서로 합성해 그립니다(step 등록의 scene 조건 충족). 다음이 남았습니다.
  - 제품 step 등록(`crates/project` `PRODUCT_STEPS`)
  - 캔버스 배치 속성 저장 위치
  - `transform-origin` longhand
  - binding·variant 참조 보고
  - 인스펙터 경로
- 결정: ADR-0024 결정 8, ADR-0086 결정 3, ADR-0125 결정 3
- 선행: IX.11.6, IX.11.21, IX.11.22, X.1.2, V.8.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/project/src/migration/steps/{mod,frame_position}.rs`(신규), `crates/project/src/migration/mod.rs`(등록 한 줄).
- `crates/layout/src/transform.rs`: `css_node_transform`, `rotate_matrix`.
- 2026-11-24 layout-render-wire:
  - `crates/scene/src/document/layout.rs`, `crates/scene/src/document/layout/tests.rs`(신규): 문서 → layout 입력 → 배치 규칙.
  - `crates/scene/src/document.rs`: 변환이 layout 결과로 위치·크기를 정합니다.
  - `crates/layout/src/view.rs`: 증분 경로용 Fragment 펼치기(`flatten_children`).
  - `crates/scene/Cargo.toml`, `Cargo.lock`, `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`: scene → layout 의존 등록.
  - `docs/adr/ADR-0125-scene-draws-layout-results.md`(신규).

## 구현 요약

- 부모가 있는 Frame은 이동 성분을 layout record `position: absolute; left; top`으로 옮깁니다. 선형 부분은
  `matrix(a, b, c, d, 0, 0)` bit 그대로입니다.
- root Frame 이동 성분은 캔버스 배치로 보고서에 남깁니다. 이미 위치 longhand가 있으면 건너뜁니다(두 번 적용해도
  같음).
- O13 결정: 직각 배수(0·90·180·270도, ±360k)는 sin·cos를 정확한 0·±1로 둡니다. 그 밖은 f64 `sin_cos`를 f32로
  반올림합니다.
  - 근거: 직각 배수는 플랫폼 무관하게 bit가 같아야 하는 흔한 입력입니다.
  - 그 밖의 각은 libm 차이가 남을 수 있어 저장 형식은 `matrix()`로 둡니다.
- scene 합성(2026-11-24): `stellar_scene::document::layout::DocumentLayout`이 View subtree의 layout을 증분 계산하고, Layout
  규칙 node(흐름 컨테이너 자식, record에 `position`을 명시한 node)의 scene `NodeInput.transform`을
  `css_node_transform(layout 결과 원점, origin 0 0, 저장 transform)`으로 만듭니다. 크기는 layout 결과 크기입니다.
  - migration 결과 모양(이동 성분 → `left`·`top`, 선형 부분만 transform)은 옛 transform과 같은 화면 위치에 그려집니다.
  - 흐름 부모 입력의 이동 성분은 layout 위치 뒤 `translate`로 더해집니다. step의 flow 부모 분기와 같은 뜻입니다.
  - 위치 원본이 아직 저장 transform인 자유 배치 node는 이전처럼 그립니다(layout 입력에서는 흐름에 참여하지 않는
    `position: absolute; left: 0; top: 0`).
  - `transform-origin`은 아직 저장하지 않아 합성 origin은 `0 0`입니다(migration이 명시 저장할 값과 같음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project frame_position` | 통과. 선형 부분 bit 동일, 이동 → left·top, root 보존, 멱등. 합성 결과가 옛 transform과 같습니다. |
| `cargo test -p stellar-layout transform` | 통과. 0·90·180·270·-90·450도 bit 결과, origin 고정점. |
| `cargo test -p stellar-scene --lib document` (2026-11-24) | 통과(33, 성능 측정 1개는 ignored). migration 모양 node(회전 행렬 + `left: 40px; top: 25px`)의 scene transform이 옛 transform과 같음, `right`·`bottom` anchor, 흐름 자식 위치·`translate` 합성, flex-grow 결과 크기, 증분 = 전체 계산(크기·gap·`display`·표시·순서·재부모화·삭제·삽입), Fragment 펼치기, 계산 실패 시 Layout 규칙 node만 skip, react-native 세트, Text 줄 너비, 줄바꿈 측정 hook, View Variant 표시 patch |
| `cargo test -p stellar-scene --release --lib -- --ignored flow_document_edit_performance --nocapture` (2026-11-24) | 2만 node 흐름 문서: 처음 변환 321 ms, 편집 한 번 `scene_edits` 중앙값 0.34 ms(최대 1.4 ms), Upsert 15건 |

## 차이(작업 지시와 다른 점)

- step을 `PRODUCT_STEPS`에 넣지 않았습니다. scene이 아직 layout 결과 위치로 그리지 않아, 켜면 옮긴 Frame이 원점에
  그려집니다.
  - (2026-11-23 style-defaults-finish 재확인) `crates/scene`은 여전히 `stellar-layout`에 의존하지 않습니다. 등록 조건과
    1→2 묶음 순서는 ADR-0115 결정 5에 적었습니다.
  - (2026-11-24 layout-render-wire) scene이 layout 결과로 그리게 되어 scene 조건은 충족됐습니다. 등록은
    `crates/project` 소유 작업이라 이 작업에서 하지 않았습니다.

## 남은 위험 / 한계

- `cargo xtask test-migrations` 대상 step이 아직 없습니다.
- flow 부모 입력의 `translate` 분기, View·ComponentDefinition root의 캔버스 배치 longhand가 남았습니다.
- runtime의 옛 캔버스 행 목록(`CanvasScene`)은 저장 transform으로 그립니다. step을 등록하면 그 행 목록에서 옮긴 Frame이
  부모 원점에 보입니다. 화면 경로를 scene source로 바꾸는 일은 UI 재설계 몫입니다(ADR-0125 결정 9).

## 근거 커밋

- `61f00f59` Feat: scene 문서 변환이 layout 결과 위치·크기로 그리기
