# 작업 보고서: XI.11.3 Frame·Shape·Path·Text·Image·Modifier·clip·P3 fixture 100개 이상

- WBS: XI.11.3
- 작업명: Frame, Shape, Path, Text, Image, Modifier, clip, P3 fixture를 최소 100개 구성
- 상태: 구현 완료. 선행 XI.7.4가 `[~]`(GPU 출력 판정 보류)라 marker는 통합 때 정합니다.
- 선행: XI.10.4 (`[x]`, 이 lane), XI.7.4 (`[~]`), XI.6.6 (`[x]`)
- 결정 기록: ADR-0102 결정 6

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon GPU, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/effects/inventory.rs` — `frame_fixtures`(12), `shape_fixtures`(13), `path_fixtures`(12), `text_fixtures`(11),
  `image_fixtures`(13), `p3_fixtures`(8), 기존 `gradient_fixtures`(14)·`blend_fixtures`(14)·`effect_fixtures`(24 → 25), `inventory`,
  `check_inventory`·`InventoryReport`, `MIN_TOTAL`(100)·`MIN_PER_CATEGORY`(8)·`ALL_FEATURES`.
- `crates/renderer-core/src/effects/fixture.rs`, `raster.rs`, `lower.rs` — 이미지 uv wrap(clamp·repeat).
- `crates/renderer-metal/src/tests/conformance.rs` — inventory 전체 Metal 비교 테스트.

## 구현 요약

- inventory 121개: Frame 23, Shape 14, Path 12, Text 11, Image 13, Modifier 30, clip 8, P3 10(분류는 fixture 하나에 하나). 기능 표시 14종이 모두
  하나 이상의 fixture를 가집니다.
- Text는 4×4 supersampling한 합성 glyph coverage(막대·고리·사선)로 glyph atlas 합성 경로를 봅니다. Image는 placeholder missing·decoding
  tile(XI.7.4 공통 data, repeat)을 포함합니다.
- `check_inventory`는 전체·분류별 최소 수, 기능 포함, 이름 유일·형식(소문자·숫자·`-`), fixture 계약(`validate`), 허용 오차 class 존재,
  기능 표시 유무를 검사해 문제 목록을 돌려줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 204 passed |
| `cargo test -p stellar-renderer-metal --lib tests::conformance` | 6 passed(inventory 121개 GPU 비교 포함, 약 5초) |
| `cargo clippy -p stellar-renderer-core -p stellar-renderer-metal --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| inventory 검사 문제 0, 전체 121 ≥ 100, 분류마다 8개 이상, 모든 기능 포함, 모든 fixture CPU 기준 생성(`inventory_check_passes_with_required_categories_and_features`) | 통과 |
| 문제 종류별 보고(이름 형식·중복, 없는 class, 기능 표시 없음, 계약 위반, 전체·분류 수 부족)(`inventory_check_reports_each_problem_kind`) | 통과 |
| inventory 121개를 Metal로 그려 CPU 기준과 class 허용 오차 안, reference raw·PNG 121쌍 기록(`full_inventory_matches_cpu_reference_on_metal`) | 통과 |

## 차이(작업 지시와 다른 점)

- Text fixture는 실제 font glyph가 아니라 합성 coverage입니다. renderer-core에는 font rasterizer가 없어 glyph 모양 자체는 text crate 몫입니다.
- 선행 XI.7.4가 `[~]`입니다. placeholder data는 같은 공통 tile을 쓰고 Metal 출력이 CPU 기준과 같음을 이 inventory가 확인합니다(XI.7.4의 남은
  판정에 쓸 수 있는 근거).

## 남은 위험 / 한계

- fixture는 32×32 크기라 대형 scene 경로(타일·batching)는 보지 않습니다. 대형 경로는 XI.11.4 benchmark가 봅니다.
