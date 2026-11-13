# 작업 보고서: XI.4.5 draw item batching key와 순서 보존 batching

- WBS: XI.4.5
- 작업명: draw item batching key를 pipeline, texture/atlas, clip state, blend state로 정의, z-order를 바꾸는 batch 금지
- 상태: 완료 (`[x]`)
- 선행: XI.4.2 (이 lane에서 `ee94123`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/batching.rs`(신규) — `BatchKey`, `TextureKey`, `ClipKey`, `TargetInfo`,
  `Batch`, `PassBatches`, `batch_key`, `batch_pass`, `batch_frame`, `batch_count`, `target_for`.
- `crates/renderer-core/src/batching/tests.rs`(신규) — key 구성 fixture, layer target, 무작위 순서 보존.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.4.5.md`(신규), 계획서(XI.4.5 `[ ]`→`[x]`).

## 구현 요약

- **batch key** = `PipelineKey`(XI.5.1) + `TextureKey` + `ClipKey` + 합성 `SceneBlendMode`.
  - pipeline: program(사각형·path 단색, image texture, text glyph mask, layer 합성), 고정 기능 blend
    (opaque `Replace`, 그 밖 `PremultipliedOver`), depth(opaque pass test+write, transparent pass test,
    layer pass 없음), stencil(stencil clip이면 `TestEqual`), target format·sample(main은 surface,
    layer는 `Rgba16Float`·1x).
  - texture/atlas: 이미지 `SceneResourceRef`, glyph atlas(page는 XI.6.5에서 추가), offscreen layer, 없음.
  - clip state: `ClipPlan`(XI.4.3)에서 scissor·stencil reference·mask scissor.
  - blend state: draw에 접힌 blend(XI.4.4), layer 합성은 경계 node 자기 blend 우선.
- **순서 보존**: pass draw 목록에서 **연속한** 같은 key draw만 묶습니다. 떨어진 같은 key draw를 당겨 묶지
  않으므로 z-order가 바뀌는 batch는 만들어지지 않습니다. depth test로 순서가 무관한 opaque pass도 같은
  규칙입니다. 추가로 Normal이 아닌 blend draw(backdrop을 읽음)는 key가 같아도 묶지 않고, stencil·mask
  clip draw는 clip stack 값이 같을 때만 묶습니다(scissor·reference가 같아도 clip 모양이 다를 수 있음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 92 passed, 0 failed; doctest 3 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| key 구성 fixture 18 node: 같은 key 연속 묶음, scissor 변경·glyph atlas·texture 변경에서 분리, multiply는 같은 key여도 분리, 같은 둥근 clip은 묶고 모양이 다른 clip은 분리, 떨어진 같은 key는 분리(A,B,…,A), opaque pass `Replace`·`TestAndWrite`(`batches_split_on_every_key_component_and_never_reorder`) | 통과 |
| layer pass key는 layer format·1x·depth 없음, 합성 draw는 layer texture key로 분리, screen 경계 합성은 자기 blend를 key에 담음(`layer_passes_use_layer_target_and_composites_use_layer_textures`) | 통과 |
| 무작위 scene 400개: batch가 빈틈·겹침 없는 연속 구간, batch 안 draw key 동일, 펼친 순서가 compile 순서와 같음, 펼친 plan이 painter 기준과 같음, 전체 batch 수 < draw 수(`batching_preserves_render_order_on_random_scenes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XI.4.2는 이 lane에서 완료했습니다(명시 row-ID). 완화 적용은 없습니다.
- key가 같아도 묶지 않는 두 경우(Normal 외 blend, 모양이 다른 stencil·mask clip)를 추가했습니다. 둘 다
  key만으로는 결과가 같음을 보장할 수 없기 때문입니다.

## 남은 위험 / 한계

- batch를 backend command(`DrawCommand`)로 바꾸는 encode는 Metal backend(XI.8.5)에서 합니다. 이 작업은 key와
  묶음 범위까지입니다.
- glyph atlas page·gradient 같은 key 구성 요소는 해당 기능(XI.6.5·XI.10.3)이 생기면 늘어납니다.
- 겹치지 않는 draw를 재정렬해 batch를 키우는 최적화는 z-order 보존 규칙 때문에 하지 않았습니다.
