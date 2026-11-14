# 작업 보고서: XI.10.6 HDR 명시적 미지원 상태와 확장 값 비조용 clamp

- WBS: XI.10.6
- 작업명: HDR path는 명시적 unsupported 상태로 유지하고 extended value를 silent clamp하지 않음
- 상태: 완료 (`[x]`)
- 선행: XI.10.5 (`[x]`, 같은 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/output_color.rs` — `DynamicRange`, `OutputColorPlan::negotiate_output`·`requested_range`·
  `output_range`, `GamutStatus::ExtendedRange`, `SurfaceColor::unclamped_linear`, `OutputColorWarning::{HdrUnsupported,
  ExtendedRangeClamped}`, `GamutReport`의 확장 값 집계와 `warnings()`, float drawable SDR 전환, 모듈 문서 HDR 절.
- `crates/renderer-core/src/output_color/tests.rs` — HDR fixture 테스트 3개 추가.
- `crates/renderer-core/src/lib.rs` — 모듈 설명.
- `docs/task-reports/XI.10.6.md`(신규), 계획서(XI.10.6 `[ ]`→`[x]`).

## 구현 요약

- **명시적 미지원**: 출력 dynamic range는 항상 SDR(`output_range`)입니다. HDR 요청(`negotiate_output(_, Hdr, _)`)은 SDR로
  렌더링하고 `HdrUnsupported`(code `render.color.hdr_unsupported`) 경고를 냅니다. `surface_config`는 확장 범위 float
  drawable(`Rgba16Float`)을 8-bit SDR drawable(`Bgra8Unorm`)로 바꿔 EDR 경로가 몰래 켜지지 않게 합니다.
- **확장 값 판정**: 지원하는 가장 넓은 SDR gamut(Display P3 `[0, 1]`, ε 1e-5)으로 옮겨도 범위 밖이면
  `GamutStatus::ExtendedRange`입니다(sRGB 1.5 흰색, 음수 채널 등). P3 안이지만 sRGB surface 밖인 색(확장 sRGB로 적은 P3
  색 등)은 gamut 문제로 `Clipped`이며 확장 값과 구분합니다.
- **비조용 clamp**: 표시 값(`linear`·`encoded`)은 `[0, 1]`로 자르지만 판정과 자르기 전 값(`unclamped_linear`)을 함께
  돌려주고, scene 검사(`scan`)는 확장 값 색·node 수·node ID(최대 64)를 모아 `ExtendedRangeClamped`
  (code `render.color.extended_range_clamped`) 경고를 냅니다. scene·문서 저장 값은 바꾸지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-renderer-core` | 종료 코드 0 |
| `cargo test -p stellar-renderer-core` | 145 unit passed(신규 3) + 3 doctest passed, 0 failed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| HDR 요청: P3 지원 display `[HdrUnsupported]`, 미지원 `[DisplayP3Unsupported, HdrUnsupported]`, 출력 SDR, SDR 요청 무경고, `Rgba16Float`→`Bgra8Unorm`·다른 format 유지(`hdr_request_is_explicitly_unsupported`) | 통과 |
| sRGB 1.5 → `ExtendedRange`·표시 1·`unclamped_linear` = decode(1.5) > 2, P3 surface에서도 확장, 음수 채널 확장, sRGB 1.05 빨강은 P3 surface `InGamut`·sRGB surface `Clipped`, 범위 안 색 unclamped = clamped(`extended_values_are_flagged_not_silently_clamped`) | 통과 |
| HDR fixture scene(sRGB 2.0 fill, P3 1.2 stroke): 확장 색 2·node 2·ID `[1, 3]`, gamut 잘림 0, 경고 `[HdrUnsupported, ExtendedRangeClamped]`, scene 저장 값 2.0 유지, sRGB fallback에서도 확장 경고(`hdr_fixture_scene_emits_warnings`) | 통과 |
| XI.10.5 기존 테스트 5개(capability·gamut·scan) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. HDR을 실제로 지원할 때는 `DynamicRange::Hdr`의 출력 경로와 float drawable 허용을 별도 작업으로 추가합니다(스키마는
  이미 확장 값을 저장할 수 있음, 아키텍처 §10.3).

## 남은 위험 / 한계

- 이미지 pixel의 확장 값은 8-bit decode(XI.7.1)라 생기지 않습니다. 16-bit·HDR 이미지 decode는 초기 범위 밖입니다.
- 경계 판정은 core-types `powf`(libm) transfer에 의존합니다. Windows 재개 시 경계 fixture를 다시 확인해야 합니다(ADR-0006
  규칙 4).
