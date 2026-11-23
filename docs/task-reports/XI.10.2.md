# 작업 보고서: XI.10.2 alpha 합성과 지원 blend mode 수식

- WBS: XI.10.2
- 작업명: alpha compositing과 지원 blend mode의 수식을 고정하고 초기 지원 목록 밖 mode는 diagnostic으로 처리
- 상태: 완료 (`[x]`)
- 선행: XI.10.1 (`[x]`, 이 lane)
- 결정 기록: ADR-0102 결정 3

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon GPU, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/blend.rs`(신규) — `source_over`, `blend_channel`, `composite`, `BlendDiagnostic`(`render.blend.unsupported_mode`),
  `resolve_blend_tag`, `resolve_or_normal`, `composite_program`, `composite_blend_state`, `SceneBlendMode` 재노출.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `crates/renderer-core/src/effects/fixture.rs` — `LayerOp`, `Op::Layer`, `Feature::Blend`, `SceneBlendMode` 재노출.
- `crates/renderer-core/src/effects/raster.rs` — layer 합성, `blend` 수식 사용(자체 source-over 제거), canvas 저장 정밀도(8-bit 양자화·half).
- `crates/renderer-core/src/effects/lower.rs` — layer pass(`LAYER_FORMAT` transient)와 합성 draw, graph version texture.
- `crates/renderer-core/src/effects/inventory.rs`(신규) — `blend_fixtures`(14개).
- `crates/renderer-core/src/effects/tolerances.conf`, `compare.rs` — `blend` class.
- `crates/renderer-metal/src/shaders.rs` — `LayerComposite` mask slot, `BlendComposite(mode)` framebuffer fetch fragment 6개, `ProgramLayout`
  (`mask_slot`, `uniform_size`, `shader_blend`).
- `crates/renderer-metal/src/commands.rs` — slot 1 자동 bind, `BlendComposite` + `PremultipliedOver` 거부, program uniform 크기 검사.
- `crates/renderer-metal/src/tests/conformance.rs`, `tests/shaders.rs` — blend 기준 image 그룹 테스트와 규칙 위반 사례.

## 구현 요약

- source-over와 W3C separable 일반식(`Cs·(1−αb) + Cb·(1−αs) + αs·αb·B(Cb/αb, Cs/αs)`)을 working 값에 적용합니다. 지원 6개
  (normal, multiply, screen, overlay, darken, lighten). 목록 밖 tag는 진단을 남기고 normal로 그립니다.
- Metal: normal은 `LayerComposite`(고정 기능 premultiplied over), 나머지는 `BlendComposite(mode)`가 framebuffer fetch로 backdrop을 읽어 같은
  식으로 합성하고 blend state는 `Replace`입니다.
- CPU 기준은 GPU attachment처럼 쓸 때마다 저장 format 정밀도로 맞춥니다(8-bit target 양자화, layer는 half float). 그래야 blend가 읽는
  backdrop 값이 같습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 194 passed |
| `cargo test -p stellar-renderer-metal` | 60 passed |
| `cargo clippy -p stellar-renderer-core -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| mode 6개 × 불투명·반투명(투명 clear) backdrop, 중첩 multiply/screen, 선형 target overlay — GPU image가 CPU 기준과 `blend` class(채널 2) 안(`blend_reference_images_match_cpu_formula`) | 통과 |
| 기준 image pixel을 `blend::composite`로 다시 계산해 일치, 빈 열은 backdrop 그대로(`blend_reference_pixels_follow_blend_formula`) | 통과 |
| separable 함수 W3C 값, 불투명 합성 = B, 투명 source·backdrop 항등, multiply 반투명 손 계산, 목록 밖 tag 진단, program·blend state 표 | 통과 |
| `BlendComposite` + `PremultipliedOver` → `InvalidUsage`, 짧은 uniform 범위 거부(`texture_binding_and_program_rules_are_checked`) | 통과 |

## 차이(작업 지시와 다른 점)

- 지원 목록 밖 mode는 document·scene이 표현하지 못하므로(6개 enum) renderer가 받는 지점은 tag 해석입니다. CSS keyword 해석은 document
  registry가 맡습니다.

## 남은 위험 / 한계

- 처음에 8-bit 반올림 경계(sRGB 0.5 = 127.5)에 걸린 backdrop 색이 GPU(127)와 CPU(128)에서 갈라져 blend 뒤 2 LSB 차이가 났습니다. fixture
  색을 경계 밖으로 옮기고 `blend` class 채널 허용값을 2로 두었습니다.
- (해결, ADR-0116 결정 3·4) 제품 pass compiler가 모든 draw의 합성 mode를 `resolve_or_normal`로 풀어 `composite_program`·
  `composite_blend_state`를 `FramePlan::draw_effects`에 두고, 목록 밖 tag 진단을 `FramePlan::blend_diagnostics`에 남깁니다. scene
  frame encoder(`effects::scene`)가 그 mode로 layer를 합성하고 Metal offscreen target이 `BlendComposite`로 그립니다(XX.2.1 보고서).
