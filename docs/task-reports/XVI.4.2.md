# 작업 보고서: XVI.4.2 p50/p95/p99 analyzer

- WBS: XVI.4.2
- 작업명: sample에서 p50/p95/p99와 통제 가능 10ms 초과 count를 계산하는 analyzer
- 상태: 완료 (`[x]`)
- 결정: ADR-0084 결정 11
- 선행: XVI.4.1(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/analyzer.rs`(신규).
- `docs/task-reports/XVI.4.2.md`(신규).

## 구현 요약

- 통제 가능 sample의 nearest-rank 백분위(정렬 후 `ceil(p·n/100)`번째)와 최댓값을 냅니다. `controllable_max_us`(10ms) 초과 수도 셉니다. 평균은 판정에 쓰지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `reference_statistics`: 1\~100ms에서 p50 50, p95 95, p99 99, max 100, 10ms 초과 90. 작은 표본 nearest-rank, 빈 표본 0.

## 성능

- 정렬 O(n log n).

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 없음.

## 2026-11-23 interaction-trace 후속: 1,000 sample reference와 10ms 경계

| 테스트 | 확인 |
|---|---|
| `thousand_sample_reference_and_limit_boundary` | 1\~1,000µs 1,000개에서 p50 500µs·p95 950µs·p99 990µs·max 1ms, 10ms와 같은 값은 초과 아님, 1ns 넘으면 초과, 입력 순서 무관 |

### 근거 커밋

- `e1b2454f` Feat: 성능 sample analyzer·제외 코드·release gate 추가 (XVI.4.2\~XVI.4.4)
- 이 커밋(`Task(XVI.4.2)`): 1,000 sample reference와 경계 검사.
