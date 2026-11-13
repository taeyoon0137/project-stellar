# 작업 보고서: XI.2.2 node 종류별 scene builder

- WBS: XI.2.2
- 작업명: Frame, Shape, Path, Text, Image를 scene primitive로 바꾸는 builder를 종류별 별도 함수로 구현
- 상태: 완료 (`[x]`)
- 선행: XI.1.2 (이 lane에서 완료: `363b5ad`), Phase VI (phase 수준 선행 — 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/builder.rs`(신규) — `SceneBuilder`(`begin_frame`/`finish_frame`, `build_shape`,
  `build_path`, `build_text`, `build_image`, `root_scope`, `finish`), `SceneScope`, `FrameScope`, 종류별
  입력(`FrameInput`, `ShapeInput`, `PathInput`, `TextInput`, `ImageInput`, 공통 `NodeInput`,
  `ModifierPaint`, `StrokeInput`, `BorderInput`, `FrameClipInput`), `SceneBuildError`.
- `crates/scene/src/builder/tests.rs`(신규) — golden·구조 테스트.
- `crates/scene/tests/fixtures/scene_builders_v1.golden`(신규) — 종류별 golden(semantic hash + canonical bytes hex).
- `crates/scene/src/stack.rs` — `PaintStack::layers` 문서에 content geometry 칠하기 규칙 명시.
- `crates/scene/src/lib.rs` — `builder` 모듈 등록.
- `docs/task-reports/XI.2.2.md`(신규), 계획서(XI.2.2 `[ ]`→`[x]`).

## 구현 요약

- **종류별 별도 함수**: Frame은 `begin_frame`(자식보다 먼저 z 발급, 자식 scope에 clip 설정) +
  `finish_frame`, leaf는 `build_shape`·`build_path`·`build_text`·`build_image`입니다.
- **primitive 매핑**: Frame → 둥근 사각형(`RectPrimitive`, 크기 상자), Shape → path(VI.5.6 평가 결과,
  NonZero), Path → path(document winding 유지), Text → text layout key + 상자, Image → asset·fit·crop·
  sampling·색공간.
- **공통 규칙**(모듈 문서): world transform = `local.then(parent_world)`; z는 호출 순서(pre-order);
  자신·조상 숨김 → Hidden, 자신·조상 잠금 → pointer None; paint는 payload 고유(Frame 배경 → 테두리)
  다음 modifier 순서; stroke 정렬은 상자형(Frame·Image) Inside, 선형(Shape·Path·Text) Center, Shape
  stroke cap은 Line의 document cap; Frame clip(bounds/path)은 자식 scope에만 겁니다.
- **입력 독립성**: scene은 document를 import하지 않으므로 입력은 document payload·modifier 평가 값의
  의미를 옮긴 독립 타입입니다. 실패(비유한 world transform, 음수 stroke 폭)는 구조화 오류이며 실패한
  node는 scope에 들어가지 않고 z도 소비하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | unit 49 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| frame·shape·path·text·image·nested 6개 golden: canonical bytes와 semantic hash 일치, golden bytes decode 결과가 build 결과와 동일(`every_node_kind_matches_golden_scene_snapshot`) | 통과 |
| Frame: 상자 rect, paint 순서(배경 → 테두리 Inside → modifier fill → modifier stroke Inside), shadow·opacity·blend, 자기 clip 없음(`frame_builds_rect_with_payload_paint_then_modifiers_and_clips_children`) | 통과 |
| 중첩: world transform 합성, bounds clip → path clip 누적·공유, 잠금 상속·숨김 자기 한정, pre-order z 1~6(`nested_scope_composes_transforms_clips_visibility_and_z_order`) | 통과 |
| leaf: Shape NonZero + Round cap, Path EvenOdd, Text resource·Center, Image field·Inside(`leaf_kinds_map_to_their_primitives`) | 통과 |
| 비유한 world transform·음수 stroke 오류, 실패 node 미추가(`invalid_inputs_are_structured_errors`) | 통과 |

golden 생성: 임시 출력 test로 6줄을 만든 뒤 frame header·rect bytes를 수기 확인(`STSN`, 버전 1,
revision 1, `id(1)`, rect tag, 100.0=`0x42c80000`, 50.0, radius 4.0)하고 임시 test를 제거했습니다.
값은 IEEE-754 사칙연산과 정수 연산만으로 정해집니다.

## 차이(작업 지시와 다른 점)

- 선행 "Phase VI"는 phase 수준이라 완화했습니다. 실제로 쓰는 VI.3(Frame)·VI.5(Shape)·VI.6(Path)·
  VI.7(Text)·VI.8(Image) 행은 b703259에서 모두 `[x]`이고, VI.9.6·VI.9.7·VI.10은 `[ ]`입니다(modifier
  평가 결과는 입력 값으로 받습니다).
- builder는 document node가 아니라 **평가된 값 입력**을 받습니다. document node → 입력 변환은
  document와 scene을 모두 보는 계층에서 해야 하며 현재 crate graph에는 그런 crate가 없습니다.
- stroke 정렬·text/image에 대한 modifier paint 의미는 document에 정의가 없어 위 규칙으로 정했습니다.

## 남은 위험 / 한계

- document payload ↔ builder 입력 변환과 의미 동등성(winding tag, cap, fit 등)은 두 crate를 보는 통합
  계층에서 교차 fixture로 검증해야 합니다.
- Text glyph 윤곽·이미지 픽셀 위에 layer를 칠하는 규칙은 renderer 구현(XI.4·XI.10)에서 지켜야 합니다.
- golden은 macOS에서만 생성·확인했습니다. 산술이 사칙연산뿐이라 ADR-0006 규칙 4에 따라 플랫폼
  무관으로 판단했습니다.
