# 작업 보고서: X.2.1 Free layout 기본 배치

- WBS: X.2.1
- 작업명: Free layout에서 자식 local position·size를 그대로 사용하고 parent padding·content origin 적용
- 상태: 완료 (`[x]`)
- 선행: X.1.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/engine.rs`(신규) — `compute_layout` 진입점, `ChildConstraint`, 공통 크기 해석(`resolve_size`, 계산 내 memo), top-down 배치(`arrange`), overflow bounds·baseline 조립.
- `crates/layout/src/free.rs`(신규) — `FreePosition`, Free 배치(`place`)와 Hug 측정(`measure`), fixture 테스트.
- `crates/layout/src/input.rs` — `LayoutNode::position`, `with_position`, 검증·canonical bytes.
- `crates/layout/src/property.rs` — `LayoutProperty::Position`.
- `crates/layout/src/error.rs` — `LayoutError::UnsupportedFeature`.
- `crates/layout/src/lib.rs` — `pub mod engine; pub mod free;`.
- `docs/task-reports/X.2.1.md`(신규), 계획서(X.2.1 `[ ]`→`[x]`).

## 구현 요약

- **엔진**: 입력 검증 → 길이를 `LayoutUnit`으로 한 번 변환 → root 크기 해석 → top-down 배치. 결과는
  pre-order(부모 다음 자식, 자식은 OrderKey visual order) `LayoutBox` 목록입니다. 같은 node·같은 제약의 크기
  해석은 계산 안에서 memo합니다.
- **Free 배치**: 자식 rect origin = `(padding.left + x, padding.top + y)`(부모 content box 원점 기준 local
  position). 자식 크기는 sizing 그대로: `Fixed` 값, `HugContent` 콘텐츠 크기(+ padding), `Fill`은 position부터
  content box 끝까지(0 미만은 0).
- **Hug 측정**: Free 부모가 Hug인 축의 content extent = 자식 `position + size` 최댓값(0 이상). 이때 자식
  `Fill`은 콘텐츠 크기로 기여하고, 부모 크기 확정 후 그 크기를 채웁니다(X.6.1 명시 규칙).
- **결과 값**: content rect는 padding inset, overflow bounds는 자신과 자손 bounds의 union, baseline은
  intrinsic 콘텐츠 baseline + padding top(그 외 `None`).
- `Percentage` 크기는 X.2.3 범위라 이 작업에서는 `UnsupportedFeature` 오류로 명시 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 45 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 기본 위치 fixture: padding (10,4,6,20) root에서 (5,7)·(-10,50)·(0,0) 자식 rect, content rect, 아래 넘침 overflow, pre-order·visual order(`basic_position_fixture_applies_padding_and_content_origin`) | 통과 |
| Hug root extent(고정 + intrinsic Hug 자식), intrinsic baseline + padding, Fill = content − position, content 밖 position의 Fill 0(`hug_fill_and_intrinsic_in_free_layout`) | 통과 |
| Hug 부모 안 Fill 자식이 콘텐츠 크기로 측정되고 확정 크기를 채움(`hug_parent_measures_fill_child_by_content`) | 통과 |
| 삽입 순서를 뒤집어도 결과 bytes 동일, 1/3 px·0.1 px subpixel 값(`layout_is_deterministic_and_independent_of_insertion_order`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. (Percentage 크기는 X.2.3에서 구현하며 그 전까지 명시 오류입니다.)

## 남은 위험 / 한계

- 레이아웃 재귀 깊이는 `MAX_LAYOUT_DEPTH`(256)로 제한됩니다(X.1.1 검증).
- 증분 재계산(X.7)은 아직 없으며 `compute_layout`은 매번 전체 tree를 계산합니다.
