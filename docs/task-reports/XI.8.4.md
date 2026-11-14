# 작업 보고서: XI.8.4 pipeline state와 shader library cache

- WBS: XI.8.4
- 작업명: pipeline state와 shader library cache 구현, shader compile 실패는 구조화 오류로 반환
- 상태: 완료 (`[x]`)
- 선행: XI.8.3 (이 lane에서 `9885be0`로 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple M3 Max, 실제 GPU offscreen render-to-texture pixel readback, Metal 런타임 MSL
  compiler).

## 변경 파일 목록

- `crates/renderer-metal/src/pipeline.rs` — `ShaderLibraryCache`(소스 digest + 소스 비교, 실패 memo),
  `PipelineCache`((`PipelineKey`, depth attachment format) → pipeline state, `(DepthMode, StencilMode)` →
  depth-stencil state, `SamplerDesc` → sampler state), 공개 `ShaderError`/`ShaderErrorStage`.
- `crates/renderer-metal/src/shaders.rs` — program별 shader 변형 표(`ProgramLayout`)와 MSL 추가:
  `Texture`·`LayerComposite`(sample × tint), `GlyphMask`(tint × coverage), `ClipStencil`(color write 없음),
  `ClipMask`(coverage).
- `crates/renderer-metal/src/commands.rs` — `BindTexture`(slot·sampled·single sample·color format 검사,
  sampler cache), sampling program의 texture 필수 검사, depth attachment pass의 depth-stencil state bind,
  `DecrementWhereEqual` reference + 1 보정.
- `crates/renderer-metal/src/encode.rs` — pass format에 depth format 자리(XI.8.5에서 채움).
- `crates/renderer-metal/src/backend.rs` — `last_shader_error`, `shader_compile_count`, 테스트용
  `use_shader_source`(`#[cfg(test)]`), command 문맥에 texture table.
- `crates/renderer-metal/src/objc.rs` — `rust_string`을 제품 코드에서 사용(compiler log).
- `crates/renderer-metal/src/lib.rs` — `ShaderError`·`ShaderErrorStage` 공개.
- `crates/renderer-metal/src/tests/shaders.rs`(신규), `tests/mod.rs` — XI.8.4 테스트.
- `docs/task-reports/XI.8.4.md`(신규), 계획서(XI.8.4 `[ ]`→`[x]`).

## 구현 요약

- **shader library cache**: MSL 소스마다(FNV-1a 64 bucket + 소스 문자열 비교로 충돌 배제) compile
  결과를 한 번 보관합니다. **실패도 보관**해 깨진 소스를 frame마다 다시 compile하지 않습니다. 내장
  library는 처음 draw가 있을 때 compile합니다.
- **pipeline cache**: key는 (`PipelineKey`, pass depth·stencil attachment format)입니다. pass에
  depth·stencil attachment가 있으면 그 pass의 모든 pipeline이 format을 선언해야 하는 Metal 규칙 때문입니다.
  depth·stencil key를 depth attachment 없는 pass에서 쓰면 `InvalidUsage`입니다. depth-stencil state는
  depth(`Less`, write 여부)·stencil(`Equal` 비교 + Increment/DecrementClamp/Keep, mask 0xFF) 조합마다,
  sampler는 `SamplerDesc`마다 하나씩 보관합니다.
- **구조화 오류**: compile·entry point·pipeline state 실패는 `ShaderError { stage, program, log }`로
  `last_shader_error()`에 남기고, 공통 계층에는 `BackendErrorKind::PipelineCreation`(정적 detail, 복구
  정책 Disable)을 돌려줍니다. log는 `NSError.localizedDescription`(최대 4096 byte, UTF-8 경계 절단)입니다.
  실패는 encoder를 열기 전 계획 단계에서 나므로 frame이 열리지 않습니다.
- **program**: `SolidFill`, `Texture`, `LayerComposite`, `GlyphMask`, `ClipStencil`, `ClipMask`를
  구현했습니다. `BlendComposite`(XI.10.2)·`GaussianBlur`(XI.10.4)는 `Unsupported`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-metal` | 29 passed, 0 failed |
| `cargo clippy -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-metal -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| known shader fixture: `Texture` program이 XI.7.4 Missing placeholder tile(16×16)을 Nearest + Repeat로 40×24에 그린 readback이 `rasterize_placeholder_reference` CPU 기준과 byte 단위로 같음(`known_shader_fixture_matches_placeholder_cpu_reference`) | 통과 |
| `GlyphMask`: R8 coverage [0,255,128,64] × green → 정확히 (0,c,0,c); `ClipMask`: R8 mask target에 사각형 안쪽만 255(`glyph_mask_and_clip_mask_programs_render_coverage`) | 통과 |
| 6 frame(두 blend) 동안 library compile 1회, pipeline 2개, shader 오류 없음(`library_and_pipelines_are_compiled_once_per_key`) | 통과 |
| 깨진 MSL: 두 번 모두 `PipelineCreation`(복구 불가)·frame 미개방, `ShaderError` stage Compile·log에 "error"·4096 byte 이하, compile 1회만(실패 memo), clear frame은 계속 렌더(`shader_compile_failure_is_a_structured_error`) | 통과 |
| entry point 없는 MSL: stage MissingEntryPoint, program SolidFill, log = 함수 이름(`missing_entry_point_is_a_structured_error`) | 통과 |
| texture 없는 sampling draw·sampling 불가 texture·slot 4·depth key without depth attachment `InvalidUsage`, graph texture source·GaussianBlur `Unsupported`, 오류 뒤 정상 frame(`texture_binding_and_program_rules_are_checked`) | 통과 |

## 차이(작업 지시와 다른 점)

- shader는 runtime MSL 소스 compile(`newLibraryWithSource:`)입니다. 빌드 시 `.metallib` precompile은
  Xcode toolchain 의존이 생겨 이 작업에서 하지 않았습니다. 첫 draw 시 compile 비용(수십 ms)은 warm-up
  대상입니다.
- `BlendComposite`는 scene 타입 `SceneBlendMode`를 담아 crate graph상 테스트에서 값을 만들 수 없으므로
  `Unsupported` 경로는 `GaussianBlur`로만 확인했습니다(두 program은 같은 match arm).
- graph resource sampling(`TextureSource::Graph`)은 transient resource와 함께 XI.8.5 범위라 이 작업에서는
  `Unsupported`입니다.

## 남은 위험 / 한계

- pipeline state 생성 실패(`ShaderErrorStage::PipelineState`)는 내장 shader로 재현할 방법이 없어 실제
  경로를 테스트하지 못했습니다(compile·entry point 실패만 실제 확인).
- 첫 frame shader compile·pipeline 생성이 frame 예산을 넘습니다. 앱 시작 시 warm-up(XII.4)이 필요합니다.
- `ClipStencil`·depth-stencil state의 실제 렌더 결과는 depth·stencil attachment를 지원하는 XI.8.5에서
  확인합니다.
