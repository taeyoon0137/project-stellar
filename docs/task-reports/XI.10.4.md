# 작업 보고서: XI.10.4 clip·mask·opacity·shadow·blur와 기능별 image fixture

- WBS: XI.10.4
- 작업명: clip, mask, opacity, shadow, blur를 순차적으로 구현하고 각 기능마다 독립 image fixture를 추가
- 상태: 완료 (`[x]`, macOS 부분 — D3D12 비교는 Windows 재개 때, ADR-0006)
- 선행: XI.4.4 (`[x]`), XI.10.1 (`[x]`, 이 lane)
- 결정 기록: ADR-0102 결정 5

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon GPU, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/effects/blur.rs`(신규) — `shadow_sigma`(σ = b/2), `layer_blur_sigma`(σ = r), `kernel_radius`, `kernel`, `valid_sigma`,
  `MAX_SIGMA`.
- `crates/renderer-core/src/effects/fixture.rs` — `LayerOp`의 opacity·shadows·blur·clip·mask, `ShadowSpec`, `ClipSpec`(scissor·stencil·mask),
  `CoverageShape`, 기능 표시 7개, layer 검증.
- `crates/renderer-core/src/effects/raster.rs` — 효과 순서대로 그리는 layer, 분리 blur, coverage mask, CPU 효과 의미 테스트 3개.
- `crates/renderer-core/src/effects/lower.rs` — shadow·blur·mask pass, scissor·stencil clip 합성, 불투명도·mask uniform, `Step::Scissor`.
- `crates/renderer-core/src/effects/inventory.rs` — `effect_fixtures`(기능별 24개), `effect`.
- `crates/renderer-core/src/effects/mod.rs` — 모듈 등록.
- `crates/renderer-metal/src/shaders.rs` — `GaussianBlur` fragment(`BlurUniforms` 48 byte)와 program layout.
- `crates/renderer-metal/src/tests/shaders.rs` — `GaussianBlur` 구현에 맞춰 짧은 uniform 거부 사례로 바꿈.
- `crates/renderer-metal/src/tests/conformance.rs` — 효과 그룹 GPU 테스트.

## 구현 요약

- 효과 순서: 내용 → shadow(내용 아래, 앞 shadow가 아래) → layer blur → clip → mask → opacity → blend(CSS `filter → clip-path → mask →
  opacity → mix-blend-mode`와 같음).
- blur: `R = ceil(3σ)`, 가중치 `exp(−i²/(2σ²))` 정규화, 가로 → 세로, 밖은 투명. shadow는 1단계에서 alpha를 모든 채널로 퍼뜨리고 2단계에서
  shadow 색(working)을 곱한 뒤 정수 offset quad로 쌓습니다.
- clip: scissor(`SetScissor`), path stencil(`ClipStencil` 증가 → `TestEqual` 합성 → 감소로 되돌림), mask clip·mask(`R8Unorm` coverage를 slot 1에
  bind, `DrawUniforms.mode = 1`). opacity는 합성 tint입니다.
- CPU 기준은 같은 순서와 저장 정밀도(layer half, mask 8-bit, target 8-bit)를 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core --lib effects` | 26 passed |
| `cargo test -p stellar-renderer-metal` | 62 passed |
| `cargo clippy -p stellar-renderer-core -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 기능별 fixture — opacity 4(0.5·0.25·0·중첩), scissor 2(안·밖), stencil 3(둥근 사각형·별·중첩), mask clip 2(soft·겹침 합집합), mask 2(띠·불투명도와 함께), shadow 5(blur·hard·두 개·반투명 내용·곡선), blur 4(σ 1·3·가장자리·불투명도), 조합 1(shadow+blur+stencil+opacity+multiply) — GPU image가 CPU 기준과 class 허용 오차 안(`effect_fixtures_match_cpu_reference_per_feature`) | 통과 |
| 그룹 불투명도는 겹친 영역을 한 번 합성(`group_opacity_composites_once_over_overlap`), blur alpha 합 보존과 blur 뒤 clip(`blur_preserves_energy_and_runs_before_clip`), shadow alpha·offset·색과 내용이 위(`shadow_follows_content_alpha_offset_and_color`) | 통과 |
| CSS 반경 대응과 kernel 모양(대칭·정규화·`w₁/w₀ = exp(−1/(2σ²))`) | 통과 |

## 차이(작업 지시와 다른 점)

- "기능별 Metal/D3D12 비교"는 Metal만 했습니다. D3D12는 같은 fixture·lowering을 쓰도록 만들었고 backend는 Windows 재개 때 더합니다.
- shadow offset은 정수 pixel만 harness가 씁니다(Nearest 1:1). 소수 offset은 제품 frame encoder가 linear sampling으로 다룹니다.

## 남은 위험 / 한계

- Frame `box-shadow`는 fill alpha와 무관하지만 이 구현은 `drop-shadow()` 의미(내용 alpha)입니다. 반투명 Frame fill의 shadow는 CSS
  `box-shadow`보다 옅습니다. document 쪽 note(SHADOW_NOTE)와 맞추는 일은 export·document 소유 작업입니다.
- (해결, ADR-0116 결정 3) 제품 pass compiler가 shadow·layer blur·clip mask 효과 pass와 합성 program(`resolve_or_normal`)을 만듭니다.
  아래 "후속: 제품 pass compiler 연결" 절을 봅니다.

## 후속: 제품 pass compiler 연결 (lane render-pass-png, ADR-0116 결정 3)

- `crates/renderer-core/src/pass_compiler/effects.rs`(신규): `EffectStep`(shadow 원본·blur·clip mask), `EffectPass`,
  `DrawEffects`(합성 program·shadow·mask), `LayerEffects`(layer blur), `Composite::resolve`(`blend::resolve_or_normal`),
  device σ·offset 계산과 진단(`render.effect.anisotropic_blur`, `render.effect.blur_too_large`).
- `crates/renderer-core/src/pass_compiler.rs`: `FramePlan`에 `effect_passes`·`draw_effects`·`layer_effects`·`blend_diagnostics`·
  `effect_diagnostics`, graph에 효과 pass와 read 연결. 효과가 없는 scene의 graph는 그대로입니다.
- `crates/renderer-core/src/pass_compiler/effects/tests.rs`(신규), `pass_compiler/tests.rs`(ordering fixture의 shadow pass 수).

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core --lib` | 218 passed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| layer blur: 배율 2에서 σ 4·4, blur pass 2개(가로 입력 = layer 출력, 세로 입력 = 가로 출력), 부모는 blur 결과만 읽음(`layer_blur_adds_two_blur_passes_and_parent_reads_the_result`) | 통과 |
| blend: layer 경계 Multiply → `BlendComposite(Multiply)`·`Replace`·경계 불투명도 0.5, 접힌 Screen은 내용 draw 합성 mode, 진단 0(`blend_composites_resolve_program_and_state`) | 통과 |
| shadow: 원본 1 + shadow마다 blur 2, 축별 배율 (2, 3)에서 offset·σ 변환, 1단계 alpha 퍼뜨림·2단계 색 곱, target이 결과를 읽음(`shadows_get_source_and_blur_passes_in_device_units`) | 통과 |
| 기울임 변환은 근사 진단, 회전 + 균일 배율은 진단 없음, σ 100은 상한 64와 진단(`skewed_or_huge_blur_is_reported`) | 통과 |
| MSAA 없는 둥근 clip은 `R8Unorm` mask pass와 draw read(`mask_clip_draws_get_a_mask_pass`) | 통과 |
