# 작업 보고서: X.1.1 레이아웃 입력·출력·오류 타입

- WBS: X.1.1
- 작업명: `layout`에 `LayoutInput`, `LayoutBox`, `LayoutResult`, `LayoutError` 정의
- 상태: 완료 (`[x]`)
- 선행: III.3.3(`[x]`), Phase IX(완화 — 아래 차이 참고)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/geometry.rs`(신규) — `Axis`, `LayoutSize`, `LayoutRect`(LayoutUnit 정수, 비음수 크기, union·translate·contains, finite `Rect` 변환).
- `crates/layout/src/property.rs`(신규) — 오류가 가리킬 `LayoutProperty`.
- `crates/layout/src/error.rs`(신규) — `LayoutError`(구조·값·계산·디코딩 오류, 한국어 Display).
- `crates/layout/src/input.rs`(신규) — `LayoutInput`, `LayoutNode`, `LayoutChild`, `ContainerLayout`, `SizingMode`, `LayoutContent`, `IntrinsicContent`, tree·값 검증, canonical bytes.
- `crates/layout/src/output.rs`(신규) — `LayoutBox`, `LayoutResult`, canonical bytes.
- `crates/layout/src/codec.rs`(신규, 내부) — LE writer/reader.
- `crates/layout/src/test_support.rs`(신규, 테스트 전용), `crates/layout/src/lib.rs` — 모듈 선언.
- `docs/task-reports/X.1.1.md`(신규), 계획서(X.1.1 `[ ]`→`[x]`).

## 구현 요약

- **입력**: `LayoutInput { root, nodes }`. node는 자식 참조(`StableId` + `OrderKey`), 배치 방식(이 작업은
  `Free`만 정의), padding(`Insets`), 축별 `SizingMode`(이 작업은 `Fixed`만 정의), 콘텐츠(`None` 또는
  intrinsic 크기·baseline)를 가집니다. 모든 float은 core-types finite 타입이라 NaN·±무한대를 담을 수 없습니다.
- **검증**: 중복 node, root 없음, 없는 자식, 다중 부모, 순환(root가 자식이거나 분리된 순환), 도달 불가,
  최대 깊이(`MAX_LAYOUT_DEPTH = 256`, 반복 DFS), intrinsic leaf의 자식, 음수 값, LayoutUnit 범위 초과를
  `LayoutError`로 거부합니다. 자식 visual order는 OrderKey 오름차순(같은 키는 StableId tie-break, III.5)입니다.
- **출력**: `LayoutBox`는 local rect(부모 local 좌표 border box), content rect(자신의 local 좌표), baseline
  (top 기준, 선택), overflow bounds(자신의 bounds를 반드시 포함)를 `LayoutUnit` 정수로 가집니다.
  `LayoutResult`는 pre-order box 목록과 StableId 조회를 제공하고 중복 box를 거부합니다.
- **canonical bytes**: 입력 `SLYI` v1, 결과 `SLYR` v1 little-endian 형식. 디코딩은 비유한 float(`NonFinite`),
  비정규 OrderKey, 음수 크기 rect, overflow 불변식 위반, 잘린·남는 bytes를 거부합니다. runtime·fixture
  비교용이며 저장 포맷이 아닙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 38 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 (layout → core-types만 의존) |
| `cargo run -q -p xtask -- validate-dependencies` | 종료 코드 0 (외부 의존성 추가 없음) |

| 검사 | 결과 |
|---|---|
| 입력 round-trip: encode → decode 동일, 재인코딩 bytes 동일(`input_round_trips_through_canonical_bytes`) | 통과 |
| 결과 round-trip과 subpixel 값 보존(`result_round_trips_through_canonical_bytes`) | 통과 |
| finite validation: bytes의 NaN·+∞·-∞ 거부, 생성 단계 비유한 거부, 결과 rect의 finite `Rect` 변환(`decode_rejects_non_finite_and_malformed_bytes`, `result_values_convert_to_finite_rects`, `to_rect_is_finite_px`) | 통과 |
| 값 검증(음수 width·padding, 2^25 px 범위 초과, baseline 음수 허용)(`value_validation_rejects_negative_and_out_of_range`) | 통과 |
| 구조 검증 오류 7종(`structure_validation_errors`), 깊이 제한(`depth_limit_is_enforced_without_recursion`) | 통과 |
| visual order가 삽입 순서가 아니라 OrderKey를 따름(`visual_order_follows_order_key_not_insertion`) | 통과 |
| box·결과 불변식(overflow 포함, 중복 box, 디코딩 음수 크기·flag 오류)(`box_and_result_invariants_are_enforced`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 "Phase IX"는 phase 단위 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다. 이 작업은 레이아웃
  crate 자체 타입만 정의하며 Phase IX 코드(component·variant)를 필요로 하지 않습니다.
- `ContainerLayout`은 `Free`, `SizingMode`는 `Fixed`만 정의했습니다. Stack·Grid·Overlay·Constraint와
  Fill·Hug·Percentage는 각 담당 작업(X.2~X.6)에서 같은 enum에 추가합니다.

## 남은 위험 / 한계

- canonical bytes 형식은 개발 중 확장됩니다(version 1 유지). 저장 포맷이 아니므로 호환성 약속은 없습니다.
- baseline은 intrinsic 콘텐츠가 준 값만 전달합니다. container baseline 규칙과 baseline 정렬은 X.3.6·X.10.4
  소관입니다.
