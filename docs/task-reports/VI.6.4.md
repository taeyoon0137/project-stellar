# 작업 보고서: VI.6.4 path bounds(cubic extrema 포함)

- WBS: VI.6.4
- 작업명: Path bounds 계산 — cubic extrema 포함
- 상태: 완료 (`[x]`)
- 선행: VI.6.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/path.rs` — `path_bounds`, 내부 `cubic_extrema_t`/`cubic_at`.
- `docs/task-reports/VI.6.4.md`(신규), 계획서(VI.6.4 `[ ]`→`[x]`).

## 구현 요약

- **정확한 geometry bounds**: control point hull이 아니라 곡선 자체의 bounds입니다.
  각 cubic에서 축별 도함수 `a t² + b t + c`(`a = −p0+3p1−3p2+p3`, `b = 2(p0−2p1+p2)`,
  `c = p1−p0`)의 근 중 (0, 1) 구간 값을 극값으로 포함하고, 끝점을 포함합니다. 계산은
  f64로 수행합니다. 빈 path는 `None`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 106 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| analytic fixture: 아치 y 극값 75, 옆 볼록 x 극값 75(오차 1e-4)(`cubic_extrema_analytic_fixtures`) | 통과 |
| 비대칭 곡선 vs 10,001점 고밀도 샘플링 독립 기준(오차 1e-2), control hull보다 안쪽 | 통과 |
| 타원 path bounds = 타원 bounds, 직선 polygon, 빈 path | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. (작성 중 테스트 기대값의 계수 계산 오류를 발견해, 손 계산 대신 독립적인
  고밀도 샘플링 기준을 추가했습니다.)

## 남은 위험 / 한계

- stroke 두께·join이 반영된 visual bounds는 Stroke modifier(VI.9) 이후입니다.
