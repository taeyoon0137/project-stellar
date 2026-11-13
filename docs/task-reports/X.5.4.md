# 작업 보고서: X.5.4 초기 constraint solver(필수 등식만)

- WBS: X.5.4
- 작업명: 초기 constraint solver는 equality와 priority 없는 필수 제약만 지원, 지원하지 않는 priority 필드는 스키마에 넣지 않음
- 상태: 완료 (`[x]`)
- 선행: X.5.3(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/constraint.rs` — `AxisInput`, `AxisSolution`, `solve_axis`(의존 순서 정확 정수 풀이), Constraint container `measure`·`place` 엔진 연결, solution fixture 테스트, solver 문서.
- `crates/layout/src/output.rs` — `LayoutResult::with_diagnostics`/`diagnostics`, 결과 bytes에 진단 목록, `LayoutDiagnostic::UnderdeterminedConstraint`.
- `crates/layout/src/engine.rs` — 계산 중 진단 수집, Constraint container 분기 연결.
- `crates/layout/src/rounding.rs` — crate 내부 `div_round`(같은 ties-away 규칙).
- `crates/layout/src/error.rs` — `LayoutError::ConstraintCycle`, 사용처가 없어진 임시 `UnsupportedFeature` 제거.
- `crates/layout/src/lib.rs` — X.5.2 임시 dead code 허용 제거.
- `docs/task-reports/X.5.4.md`(신규), 계획서(X.5.4 `[ ]`→`[x]`).

## 구현 요약

- **스키마**: `Constraint { edge, target, offset }`만 있으며 priority·부등식·strength 필드는 두지 않았습니다(필수 등식만).
- **풀이**: 축마다 X.5.3 분류·의존 순서를 쓰고, 각 형제 등식을 2배 단위 정수(`a·start + b·size = rhs`)로 세워
  크기가 정해진 형제는 등식 하나로 시작 위치를, `Fill` 형제는 독립 등식 두 개(Cramer)로 시작·크기를 구한 뒤 한 번만
  반올림합니다(ties away from zero). Fill 크기가 음수면 0. 가로를 먼저 풀고, 정해진 너비로 높이를 측정한 뒤 세로를 풉니다.
- **분류 처리**: `Cycle` → `ConstraintCycle`, `Overdetermined` → `OverConstrained` 오류. `Underdetermined` →
  `UnderdeterminedConstraint` 진단 + fallback(미정 크기 = 콘텐츠 크기, 미정 위치 = 첫 등식 또는 0).
- **진단**: `MissingAnchorTarget`(X.5.2)과 함께 `LayoutResult::diagnostics()`에 배치 순서로 실리고 결과 bytes로
  round-trip합니다.
- **Hug 측정**: Constraint container의 indefinite 축 extent는 자식 크기 최댓값입니다(위치는 부모 크기에 의존).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 102 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| deterministic solution fixture: 고정 A(Left·Top) + Fill B(A.Right+8 ~ parent.Right−12, CenterY = A.CenterY) → A (20,15,50,20), B (78,10,200,30), visual order가 의존 순서와 달라도 같은 해, 재계산 bytes 동일(`deterministic_solution_fixture`) | 통과 |
| CenterX 정확 해: 6401 subpixel 중심의 시작 1600.5 → 1601(ties away), baseline 정렬(top −8)과 baseline 없는 형제의 bottom fallback(top 2)(`center_tie_and_baseline_solutions_are_exact`) | 통과 |
| overdetermined → `OverConstrained`, 순환 → `ConstraintCycle`, underdetermined·missing sibling 진단 순서와 fallback 위치·크기, 진단 bytes round-trip(`errors_and_diagnostics_from_classification`) | 통과 |
| 기존 X.5.2·X.5.3 단위 테스트 회귀 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Constraint container Hug 측정은 위치를 무시한 근사(자식 크기 최댓값)입니다.
- 부등식·priority(선호 제약)는 지원하지 않습니다(작업 지시대로 스키마에도 없음).
