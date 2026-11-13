# 작업 보고서: X.5.5 constraint 변경 영향 범위를 연결 요소로 제한

- WBS: X.5.5
- 작업명: constraint 변경 영향 범위를 connected component로 제한
- 상태: 완료 (`[x]`)
- 선행: X.5.4(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/constraint.rs` — `ConstraintSystem`(축 단위 증분 풀이), `ConstraintSibling`, `ConstraintPlacement`, 형제별 풀이 횟수, 테스트, 모듈 문서.
- `docs/task-reports/X.5.5.md`(신규), 계획서(X.5.5 `[ ]`→`[x]`).

## 구현 요약

- **근거**: constraint 참조는 같은 부모 형제 사이에만 있으므로 한 형제의 변경은 그 형제의 무방향 연결 요소 밖의 해를
  바꾸지 않습니다(X.5.3 연결 요소).
- **`ConstraintSystem`**: 한 축의 형제 입력(constraint, Fill 여부, 크기, baseline)과 해를 보관합니다.
  `set_constraints(id, …)`는 변경 **전·후** 연결 요소 중 그 형제를 포함한 것의 합집합만 해를 지우고 X.5.4
  `solve_axis(only = 영향 형제)`로 다시 풉니다. 다른 요소의 해는 재사용합니다.
- **측정**: `solve_counts()`가 형제별 누적 풀이 횟수를 줘서 무관 요소의 재계산이 0임을 직접 확인합니다.
- 실패(순환 등)한 변경은 복사본에서 계산하므로 시스템 상태를 바꾸지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 105 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 연결 요소 {A,B}·{C,D}·{E}에서 C 변경 → {C,D}만 재풀이, 풀이 횟수 [1,1,2,2,1](무관 요소 재계산 0), 무관 해 불변(`unrelated_components_are_not_recomputed`) | 통과 |
| 요소 병합(E→B: {E} ∪ {A,B,E})과 분리(B 끊김: 이전 요소 {A,B,E}) 시 영향 범위, 증분 결과 = 전체 재계산 결과(`merging_and_splitting_components_resolve_both_sides`) | 통과 |
| 순환을 만드는 변경은 오류이며 상태 불변, 없는 형제 오류(`failed_update_leaves_system_unchanged`) | 통과 |

## 차이(작업 지시와 다른 점)

- 증분 API는 constraint container 하나·축 하나 단위의 독립 구조입니다. 문서 command → layout dirty set 연결(X.7.1)과
  증분 엔진 전체(X.7.2~X.7.3)는 선행 미완료(VII.1.4, IX.2.2)로 이 lane에서 연결하지 않았습니다. `compute_layout`은
  여전히 전체를 계산합니다.

## 남은 위험 / 한계

- 영향 판정은 constraint 변경만 다룹니다. 형제 크기(콘텐츠) 변경 시에도 같은 연결 요소 규칙이 적용되지만 API는 아직 없습니다.
