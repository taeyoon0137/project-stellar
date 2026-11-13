# 작업 보고서: XI.4.4 blend/opacity/filter 의존 분석과 최소 offscreen 경계

- WBS: XI.4.4
- 작업명: blend/opacity/filter dependency를 분석해 필요한 최소 offscreen subtree 경계 계산
- 상태: 완료 (`[x]`)
- 선행: VI.9.2 (`[x]`), XI.4.2 (이 lane에서 `ee94123`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/offscreen.rs`(신규) — `plan_boundaries`, `BoundaryPlan`, `EffectDecision`,
  `BoundaryReason`, `draws_as_single_composite`, `MAX_DISTRIBUTED_DRAWS`.
- `crates/renderer-core/src/offscreen/tests.rs`(신규) — nested opacity fixture, 무작위 의미 비교, pixel 겹침·
  상한·가시성 테스트.
- `crates/renderer-core/src/pass_compiler.rs` — `EffectBoundaries::Minimal`(기본값), 접힌 효과 stack
  (scope별 `Fold`), `PlannedDraw::blend`, draw·layer 합성에 접힌 opacity·blend 반영, 접힌 효과가 걸린 draw는
  transparent.
- `crates/renderer-core/src/pass_compiler/tests.rs` — 모사에 접힌 blend 반영, opaque pass draw의
  opacity·blend 검사, blur를 원본 변환으로 먼저 적용하도록 모사 수정.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.4.4.md`(신규), 계획서(XI.4.4 `[ ]`→`[x]`).

## 구현 요약

- **분석(아래에서 위로)**: 효과 node마다 subtree가 경계 결정 뒤 남기는 draw 단위(unit: 내용 draw 또는
  안쪽 layer 합성 draw)를 계산합니다. 다음을 모두 만족하면 layer 없이 효과를 접습니다(`Folded`).
  1. layer blur 없음(`Blur`) 2. unit ≤ 8개(`TooManyDraws`) 3. 모든 unit이 한 번에 합성되는 단일 draw:
  paint layer ≤ 1·shadow 없음(사각형·path), paint layer 없는 image. text는 glyph 겹침 가능성 때문에
  제외(`CompoundDraw`) 4. 안쪽에 접힌 Normal 외 blend나 blend 경계 합성이 없음. 접으면 안쪽 blend의
  backdrop이 바뀌기 때문(`NestedBlend`) 5. unit들의 device pixel 영역(render bounds를 pixel 경계로
  바깥 정수화)이 서로 겹치지 않음(`OverlappingDraws`). 아무것도 그리지 않으면 `NothingToDraw`입니다.
- **결과 반영**: 경계 node는 부모 쪽에 단일 합성 unit 하나를 남기고(자기 blend가 Normal이 아니면 blend
  표시), 접힌 node는 unit을 그대로 넘깁니다. compiler는 접힌 효과를 scope별 stack에 쌓아 draw
  `opacity`(곱)와 `blend`로 넣고, 경계를 감싼 접힌 효과는 layer 합성 draw에 넣습니다. 예:
  `A(0.5) ⊃ B(0.5) ⊃ 사각형` → layer 0개, 사각형 opacity 0.25. 분석과 compile 모두 재귀가 없습니다.
- **모사 수정**: 기존 painter 모사는 blur를 opacity와 묶어 hash해 "blur layer를 opacity로 합성"과 "blur 합성에
  opacity 접기"를 다른 값으로 봤습니다. 실제로는 같으므로(blur는 원본 변환, 그 뒤 opacity·blend 합성) 모사가
  blur를 원본에 먼저 적용하도록 고쳤습니다. 이 수정 전에는 무작위 비교가 그 경우에서 실패했고, 수정 후
  XI.4.2·XI.4.3 테스트를 포함해 모두 통과합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 89 passed, 0 failed; doctest 3 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| nested opacity fixture 22 node: node별 결정(접힘·겹침·blur·nested blend·compound·nothing) 13건이 기대값, 경계 6개, pass graph(effect pass 6개 순서·opaque 비어 있음·transparent draw의 opacity·blend)가 기대값, 보수적 compile은 layer 12개, 두 plan 모두 painter 기준과 같음(`nested_opacity_fixture_expected_pass_graph`) | 통과 |
| 무작위 scene 400개: 최소 경계 plan이 painter 기준과 모든 cell에서 같음, scene마다 layer 수 ≤ 보수적, 전체 layer 967 / 1659(42% 감소), 접힌 draw > 100(`minimal_boundaries_preserve_painter_order_on_random_scenes`) | 통과 |
| 위 비교의 검출력: `NestedBlend` 검사나 겹침 검사를 끈 분석이 이 테스트에서 실패함을 임시 변경으로 확인(변경은 되돌림) | 확인 |
| 소수 위치로 pixel을 나눠 덮으면 겹침, 정수 경계 맞닿음은 접힘, zoom 반영(`fractional_neighbours_overlap_at_pixel_level`) | 통과 |
| unit 상한 8 경계, 그리지 않는 node는 unit 아님, 숨김 효과 node 미결정, image + fill layer는 compound(`distribution_limit_and_visibility`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 VI.9.2는 `[x]`, XI.4.2는 이 lane에서 완료했습니다. 완화 적용은 없습니다.
- `EffectBoundaries` 기본값을 `Minimal`로 바꿨습니다. `Conservative`는 비교·진단용으로 남겼습니다.
- painter 모사의 blur 모델을 고쳤습니다(위 구현 요약). 제품 코드 결함이 아니라 모사 충실도 문제였습니다.

## 남은 위험 / 한계

- 모사는 node 하나 안의 fill·stroke 겹침을 모델링하지 않아 `CompoundDraw` 규칙은 fixture 결정으로만
  고정합니다.
- 접기 판단은 clip culling 전 가시성으로 unit을 셉니다. clip에 잘려 실제로 그리지 않는 unit도 세므로 경계가
  필요 이상 남을 수 있습니다(보수적, 정확성 문제 아님).
- blur leaf의 analytic 처리, 겹치는 group opacity의 layer 크기 축소(bounds 크기 offscreen)는 하지
  않았습니다. layer는 surface 크기입니다(XI.10.4·성능 작업에서 검토).
