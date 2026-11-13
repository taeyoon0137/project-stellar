# 작업 보고서: X.5.2 Anchor target 정의와 StableId 참조

- WBS: X.5.2
- 작업명: Anchor target을 parent edge, sibling edge, center, baseline로 정의하고 참조는 StableId 사용
- 상태: 완료 (`[x]`)
- 선행: III.2.3(`[x]`), X.1.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/constraint.rs`(신규) — `AnchorEdge`, `AnchorTarget`, `Constraint`, 검증, canonical bytes, sibling target 해석(`resolve_targets`), 테스트.
- `crates/layout/src/output.rs` — `LayoutDiagnostic::MissingAnchorTarget`(비치명 진단).
- `crates/layout/src/input.rs` — `ContainerLayout::Constraint`(container tag 4), `LayoutNode::constraints`, `with_constraint`, 검증·canonical bytes.
- `crates/layout/src/engine.rs` — Constraint solver 연결(X.5.4) 전까지 `UnsupportedFeature` 명시 오류.
- `crates/layout/src/error.rs` — `LayoutError::InvalidConstraint`, `LayoutError::UnsupportedFeature`.
- `crates/layout/src/property.rs` — `LayoutProperty::Constraints`.
- `crates/layout/src/lib.rs` — `pub mod constraint;`(엔진 연결 전 비테스트 dead code 허용).
- `docs/task-reports/X.5.2.md`(신규), 계획서(X.5.2 `[ ]`→`[x]`).

## 구현 요약

- **edge**: 가로 `Left`·`Right`·`CenterX`, 세로 `Top`·`Bottom`·`CenterY`·`Baseline`(tag 0~6).
- **target**: `Parent(edge)`(부모 content box edge, center 포함, baseline 불가) 또는
  `Sibling { node: StableId, edge }`(같은 부모의 다른 자식, sibling baseline은 첫 baseline·없으면 bottom).
  참조는 StableId만 쓰고 index·순서를 쓰지 않습니다.
- **Constraint**: `edge = target + offset`(offset 음수 허용). 자기 edge와 target edge의 축이 다르거나 부모
  baseline target이면 `InvalidConstraint`.
- **missing sibling = 진단**: sibling target이 같은 부모의 표시되는 형제 목록에 없으면 그 constraint를 제외하고
  `LayoutDiagnostic::MissingAnchorTarget { node, target }`을 남깁니다(오류 아님).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 95 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| parent edge·sibling edge·center·baseline target 해석과 StableId → 형제 index, offset subpixel(`targets_cover_parent_sibling_center_and_baseline`) | 통과 |
| 없는 sibling → constraint 제외 + `MissingAnchorTarget` 진단, 나머지 constraint 유지(`missing_sibling_is_diagnostic_not_error`) | 통과 |
| 축 불일치·부모 baseline 거부, offset 범위, 7 edge round-trip과 tag golden(`constraint_validation_and_round_trip`) | 통과 |

## 차이(작업 지시와 다른 점)

- 진단 타입과 target 해석까지 구현했고, 결과(`LayoutResult`)에 진단을 싣는 것은 solver를 엔진에 연결하는 X.5.4에서 합니다.

## 남은 위험 / 한계

- graph 분류(X.5.3)와 solver(X.5.4) 전까지 Constraint container 계산은 `UnsupportedFeature` 오류입니다.
