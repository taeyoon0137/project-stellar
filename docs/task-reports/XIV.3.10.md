# 작업 보고서: XIV.3.10 diff inverse

- WBS: XIV.3.10
- 작업명: diff inverse 구현
- 상태: 완료 (`[x]`)
- 선행: XIV.3.9 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/apply.rs` — `SemanticChange::inverse`, `inverse`, `SemanticDiff::inverse`.
- `docs/task-reports/XIV.3.10.md`(신규), 계획서(XIV.3.10 `[ ]`→`[x]`).

## 구현 요약

- **규칙**: AddObject ↔ RemoveObject, AddToken ↔ RemoveToken(객체 전체 유지), Move/Reorder는
  from ↔ to, 속성 변경 4종은 old ↔ new. 종류 분류와 출처는 유지합니다. 모든 변경이 이전·이후
  상태를 담고 있어(XIV.3.2) 정보 손실이 없습니다.
- **정렬**: 역변경 목록은 canonical 순서로 다시 정렬한 `SemanticDiff`입니다.
- 역변경 적용도 `apply`의 stale 검증을 그대로 받습니다(target 기준 이전 상태 확인).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | 45 passed, 0 failed |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 200 seed × 3쌍(무관 문서, 문서↔변형): `apply(inverse(diff(A,B)), B) == A`, `inverse(inverse(d)) == d`, `inverse(diff(A,B))`와 `diff(B,A)` bytes 동일(`apply_inverse_restores_base`) | 통과 |
| 변경별 역 kind(Reorder/SetProperty/Remove→Add), 이중 역 동일·출처 유지, 역변경을 base에 적용하면 stale 거부(`inverse_swaps_each_change`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 인스턴스의 정의 참조와 override를 동시에 바꾼 diff는 출처의 컴포넌트가 "이후 값 우선"
  규칙이라 `inverse(diff(A,B))`와 `diff(B,A)`의 출처 컴포넌트가 다를 수 있습니다(적용 결과는
  같음). 출처는 표시 정보입니다.
