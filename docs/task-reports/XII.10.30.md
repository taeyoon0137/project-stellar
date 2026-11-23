# 작업 보고서: XII.10.30 Boolean modifier geometry 평가

- WBS: XII.10.30
- 작업명: Boolean modifier geometry 평가를 결정론적 순수 함수로 구현하고 scene builder에 연결
- 상태: 완료 (`[x]`)
- 결정: ADR-0084 결정 7(Accepted)
- 선행: VI.9.4(`[x]`), VI.5.6(`[x]`), XI.2.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/scene/src/boolean.rs`(신규), `boolean/tests.rs`(신규) — `evaluate_boolean`, `signed_area`.
- `crates/scene/tests/fixtures/boolean_v1.golden`(신규) — 결과 path golden.
- `crates/scene/src/narrow_phase.rs` — 선분화 함수를 crate 안에서 공유.
- `crates/scene/src/incremental.rs`·`source.rs` — Boolean host 결과 path 연결.
- `crates/runtime/src/canvas/scene_source.rs` — operand 흡수, 문서 순서 operand, host local 좌표 변환, operand → host 의존.
- `docs/task-reports/XII.10.30.md`(신규).

## 구현 요약

- ADR로 다음을 정했습니다.
  - operand node는 결과에 흡수되어 단독 표시하지 않습니다.
  - operand 순서는 host geometry 다음 참조 operand를 문서 pre-order(OrderKey 순)로 둡니다.
  - Subtract는 첫 operand(host) 기준입니다.
  - 결과는 host paint로 칠합니다.
- 평가 순서는 다음과 같습니다.
  - 1/64 px 정수 격자로 옮깁니다.
  - `i128` orientation으로 교차 분할하고, 반올림 뒤 새 교차가 없을 때까지 반복합니다.
  - 선분 양쪽 operand별 winding으로 경계 선분만 남깁니다.
  - 남은 선분을 NonZero loop로 잇습니다.
- 평가가 실패하면(좌표 ±32,768 px·선분 20,000 초과) scene은 host geometry를 그리고 수를 셉니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | lib 151 passed, 통합 3 passed(1 ignored), 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo test -p stellar-runtime --lib scene_source` | 6 passed, 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- 사각형 4개 연산 넓이 정확(175·25·75·150).
- 두 원 analytic 넓이 1% 이내(4연산).
- Subtract 순서 의존, Union·Intersect 순서 무관.
- operand transform, 변 공유·떨어진 operand, EvenOdd 구멍, 범위 밖·빈 입력 거부.
- 같은 입력 3회 bit 동일(canonical bytes).
- 결과 path golden 일치.
- scene 연결: operand 미표시, host path 넓이 75, 실패 시 host geometry와 fill rule 유지.
- 문서 연결: operand 흡수·의존 dirty.

## 성능

- 교차 분할은 x 정렬 sweep 쌍 검사(최악 O(선분²))입니다. 일반 도형은 선분 수백 개 이하라 ms 미만입니다.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 곡선은 선분화 결과(허용 오차 0.05 px)로 path를 냅니다. 원래 cubic을 보존하지 않습니다. export 굽기에서 곡선 보존이 필요하면 후속입니다.
- host의 Boolean 앞뒤 modifier 순서 의미는 Boolean 항목을 뺀 stack 평가로 단순화했습니다.
