# 작업 보고서: X.6.2 모든 layout mode에 같은 순서의 min/max clamp

- WBS: X.6.2
- 작업명: min/max clamp를 모든 layout mode의 최종 size에 동일 순서로 적용
- 상태: 완료 (`[x]`)
- 선행: X.6.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/sizing.rs` — `SizeLimits`, `AxisLimits::clamp`, 검증·변환(`prepare_limits`), canonical bytes, 정책 문서, boundary fixture 테스트.
- `crates/layout/src/engine.rs` — `Prepared::limits`, 단일 크기 해석(`resolve_size`)에서 forced·mode·Hug 값 모두 같은 clamp 적용.
- `crates/layout/src/stack.rs`, `crates/layout/src/flex.rs` — flex 분배 입력에 자식 main 축 min/max 연결, hypothetical·indefinite 경로도 같은 clamp.
- `crates/layout/src/input.rs` — `LayoutNode::limits`, `with_limits`, 검증·canonical bytes.
- `docs/task-reports/X.6.2.md`(신규), 계획서(X.6.2 `[ ]`→`[x]`).

## 구현 요약

- **순서(모든 mode 공통)**: ① sizing mode 또는 부모가 정한 forced 크기(Stack 분배·Stretch, Grid 영역, Overlay·Free
  Fill 영역, Constraint 해) → ② `max` 먼저, `min` 나중: `max(min, min(값, max))` → ③ Hug 축은 콘텐츠 extent +
  padding 뒤 같은 clamp. definite 축은 clamp한 값으로 content box를 정해 자식을 측정합니다.
- clamp는 엔진의 단일 크기 해석 함수 한 곳에서만 하므로 Free·Stack·Grid·Overlay·Constraint와 root가 같은 규칙을
  씁니다. Stack grow·shrink는 같은 min/max를 freeze 반복에 넣어 분배 결과가 이미 clamp된 상태입니다.
- **오류**: `min > max` → `InvalidSizing { axis }`, 음수 → `NegativeValue`. `min == max`는 허용.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 108 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| min>max 오류(가로·세로), 음수 min 오류, min == max 허용과 round-trip(`min_greater_than_max_and_negative_limits_are_rejected`) | 통과 |
| boundary fixture(Free·root): root max 80, Fixed 50/max 40 → 40, Fill 영역 20/min 30 → 30, Hug 70/max 50 → 50, 경계값 그대로(`boundary_fixture_in_free_and_root`) | 통과 |
| boundary fixture(Stack grow max → 20/40/40, shrink min → 70/30, Stretch max → 30, Grid 영역 max 45/min 70, Overlay Fill → 80×70, Constraint Fill 100 → max 70)(`boundary_fixture_in_stack_grid_overlay_and_constraint`) | 통과 |
| 기존 fixture 회귀(min/max 미지정 시 동작 불변) | 통과 |

## 차이(작업 지시와 다른 점)

- 계획 순서상 X.6.1 바로 다음이지만, "모든 layout mode"에 적용됨을 검증하려고 Free·Stack·Grid·Overlay·Constraint를
  구현한 뒤 수행했습니다(X.6 섹션 내부 순서 유지).

## 남은 위험 / 한계

- Constraint에서 Fill 해 크기가 clamp되면 위치는 clamp 전 해(시작 edge 기준)를 유지합니다(끝·가운데 등식은 다시 풀지
  않음).
- 자동 최소 크기(min-content)는 적용하지 않습니다(min 기본 0).
