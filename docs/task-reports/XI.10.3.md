# 작업 보고서: XI.10.3 gradient 렌더링과 stop tie 순서

- WBS: XI.10.3
- 작업명: gradient 렌더링을 구현하고 stop tie-order를 Phase III 규칙대로 적용
- 상태: 완료 (`[x]`)
- 선행: III.4.3 (`[x]`), XI.10.1 (`[x]`, 이 lane)
- 결정 기록: ADR-0102 결정 4

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon GPU, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/effects/gradient.rs`(신규) — `GradientStop`, `GradientRamp`(`resolve`, `evaluate`, `domain`, `ramp_coordinate`,
  `ramp`), `GradientGeometry`(선형·원형 `parameter`), `GradientError`, `RAMP_WIDTH`.
- `crates/renderer-core/src/effects/fixture.rs` — `Paint::Gradient`, `linear_gradient`, `Feature::Gradient`, stop 순서 검증.
- `crates/renderer-core/src/effects/raster.rs` — pixel 중심 `t`의 정확한 gradient 값.
- `crates/renderer-core/src/effects/lower.rs` — ramp texture(`Rgba16Float` 1024 texel) + 정점 uv 경로, union mesh용 stencil 되돌리기
  정점, `LowerError::Unsupported`(원형 gradient).
- `crates/renderer-core/src/effects/inventory.rs` — `gradient_fixtures`(14개), `stop`.
- `crates/renderer-core/src/effects/tolerances.conf` — `gradient` class.
- `crates/renderer-core/src/color.rs` — test 도우미 모듈 공개 범위(crate 안).
- `crates/renderer-metal/src/tests/conformance.rs` — gradient 그룹 GPU 테스트.
- `benchmarks/fixtures/Cargo.toml`, `benchmarks/fixtures/tests/gradient_stop_order.rs`(신규), `Cargo.lock` — core-types 시각 순서 → renderer
  연결 테스트.

## 구현 요약

- renderer는 stop을 core-types `Gradient::visual_order`(position, 안정 ID)가 준 순서 그대로 쓰고 다시 정렬하지 않습니다. position이 줄면
  `OutOfOrder`입니다.
- 값: 첫 stop 앞은 첫 색, 마지막 stop부터는 마지막 색, 사이는 `pᵢ ≤ t < pᵢ₊₁`인 마지막 `i`의 선형 보간(working premultiplied). 같은
  position은 `t = p`에서 뒤 stop이 이깁니다. 길이 0 gradient는 마지막 stop 단색입니다.
- GPU: stop 범위와 `[0, 1]`의 합집합을 1024 texel ramp로 만들고, 선형 gradient의 `t`(위치의 affine 함수)를 정점 uv.x로 보간해 `Texture`
  program이 linear filter·clamp로 읽습니다. 원형 gradient는 CPU 수식만 있고 GPU 경로는 gradient 전용 program과 함께 넣습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 198 passed |
| `cargo test -p stellar-renderer-metal --lib tests::conformance` | 4 passed |
| `cargo test -p stellar-bench-fixtures --test gradient_stop_order` | 1 passed |
| `cargo clippy -p stellar-renderer-core -p stellar-renderer-metal -p stellar-bench-fixtures --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 선형 2·3 stop, 대각선, tie hard edge, 같은 position 3 stop, `[0, 1]` 밖 stop, pad, 투명 stop, 길이 0, 타원·둥근 사각형 안, P3 surface, P3 → sRGB fallback, 선형 target — GPU image가 CPU 기준과 `gradient` class 안(`gradient_pixel_fixtures_match_cpu_formula`) | 통과 |
| core-types `Gradient` 저장 순서를 바꿔도 같은 position stop이 안정 ID 순서로 렌더되고 기준 image byte가 같음, hard edge 양쪽 색(`equal_position_stops_render_in_stable_id_order_regardless_of_storage_order`) | 통과 |
| 값 규칙(pad·보간·tie·투명 premultiplied), 순서·빈 목록·비유한 오류, 선형·대각·원형·길이 0 기하, ramp 범위 | 통과 |

## 차이(작업 지시와 다른 점)

- scene paint에는 gradient가 아직 없습니다(scene은 lane 소유 밖). 수식·GPU 경로·fixture는 renderer에 있고 scene paint 연결은 후속입니다.
- 원형 gradient GPU 경로는 넣지 않았습니다(결정 4).

## 남은 위험 / 한계

- 보간 공간은 선형 working 값입니다(아키텍처 §10.3). CSS 기본(인코딩 sRGB 보간)과 중간 색이 다르므로 web export는 이 차이를 근사·래스터
  경계로 다뤄야 합니다.
- ramp 해상도(1024)보다 좁은 stop 간격(폭 0.1% 미만)은 texel 하나 안에서 섞입니다.
