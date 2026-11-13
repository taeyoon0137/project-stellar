# 작업 보고서: X.3.3 grow·shrink·basis와 잔여값 처리 순서

- WBS: X.3.3
- 작업명: grow, shrink, basis 구현, 분배 잔여값 처리 순서를 StableId가 아닌 child visual order로 고정
- 상태: 완료 (`[x]`)
- 선행: X.3.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/flex.rs`(신규) — `FlexItem`, `FlexBasis`, factor 변환·검증, main축 Fill 충돌 검증, canonical bytes, `resolve_flexible_lengths`(grow/shrink 분배 + min/max freeze 반복), 단위 테스트.
- `crates/layout/src/stack.rs` — 한 줄 자식 크기 결정을 `line_sizes`(base → 분배 → forced main으로 재해석)로 교체, reference fixture 테스트.
- `crates/layout/src/engine.rs` — `Prepared::flex`.
- `crates/layout/src/input.rs` — `LayoutNode::flex`, `with_flex`, Stack 자식 조합 검증, canonical bytes.
- `crates/layout/src/property.rs` — `LayoutProperty::Flex`.
- `crates/layout/src/lib.rs` — `pub mod flex;`.
- `docs/task-reports/X.3.3.md`(신규), 계획서(X.3.3 `[ ]`→`[x]`).

## 구현 요약

- **입력**: `grow`·`shrink`(비음수 `Scalar`, 1/64 단위 반올림), `basis`(`Auto` = main sizing, `Length`,
  `Percentage` = content main 기준). 기본값 grow 0·shrink 0·Auto. main축 `Fill` = grow 1·shrink 1·basis 0이며,
  main축 Fill과 기본값이 아닌 flex를 함께 쓰면 `InvalidSizing`.
- **분배(reference flex 축약)**: hypothetical = clamp(base) → 사용 가능(content main − gap 합)과 비교해 grow/shrink
  모드 결정 → factor 0 등은 고정 → 남은 공간을 grow factor 또는 scaled shrink factor(`shrink × base`) 비례 정수
  분배 → min/max 위반 고정 후 반복. main이 indefinite(Hug 측정)면 분배하지 않습니다.
- **잔여값 순서**: 내림 몫 뒤 남은 subpixel은 분배 대상 자식에게 **child visual order** 앞에서부터 1씩 줍니다
  (`distribute_weighted`). StableId는 쓰지 않으며 Reverse 배치에서도 visual order 기준입니다.
- 분배 뒤 각 자식을 정해진 main 크기(forced)로 다시 해석해 cross 크기를 확정합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 69 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| reference flex fixture(엔진 경유): grow 1/2/1 → 75/150/75, gap 10 → 70/140/70, shrink 1/1/2 → 75/75/50, Fill+Fixed+Fill → 150/100/150, basis 25% + grow 0.5/1.5 → 100/75/225, Hug container 분배 없음 70(`reference_flex_grow_shrink_basis_fixtures`) | 통과 |
| 잔여 subpixel이 StableId(5,7,9)가 아니라 visual order 첫 자식(id 9)에 감: Forward·Reverse 모두(`residual_follows_visual_order_not_stable_id`) | 통과 |
| main축 Fill + flex 충돌 거부, cross Fill + flex 허용, 음수 factor 거부, 3종 basis round-trip(`flex_validation_and_round_trip`) | 통과 |
| 분배 단위: grow 비례, scaled shrink, 음수 freeze 후 재분배(0/100), max freeze 재분배(20/40/40), 잔여 순서(`flex::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- 기본 shrink를 CSS(1)와 달리 0으로 두었습니다(고정 크기 자식은 명시하지 않으면 줄지 않음). reference fixture는
  factor를 모두 명시합니다.
- 자동 최소 크기(min-content)는 적용하지 않고 min 기본값 0을 씁니다. node min/max는 X.6.2에서 연결합니다.

## 남은 위험 / 한계

- factor를 1/64 단위로 반올림하므로 1/128 미만 차이의 factor는 같은 값으로 취급됩니다.
