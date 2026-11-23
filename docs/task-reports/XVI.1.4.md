# 작업 보고서: XVI.1.4 trace dimension privacy

- WBS: XVI.1.4
- 작업명: interaction kind, document size bucket, visible node bucket, cache state만 dimension으로 허용
- 상태: 부분 완료 (`[~]`) — 2026-11-23 ledger-refresh 판정: 구현 판정 통과, 선행 미충족(계획 1.2 규칙 4): 선행 XVI.1.3(`[~]`)이 `[x]`가 아닙니다(XVI.1.2 사슬). 이전 기록: 완료 (`[x]`)
- 결정: ADR-0084 결정 8
- 선행: II.3.2(`[x]`), XVI.1.3(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/diagnostics/src/interaction.rs` — `TraceDimensions`, bucket, `TRACE_LINE_KEYS`.
- `docs/task-reports/XVI.1.4.md`(신규).

## 구현 요약

- dimension은 닫힌 enum 네 개입니다. 문자열 field가 없어 object 이름·text·path를 담을 수 없습니다(compile_fail doctest).
- bucket 경계는 아키텍처 §5.3 보장 규모(문서 200k, 가시 20k)를 기준으로 합니다.
- trace 줄은 허용 key만 쓰고, 값은 숫자이거나 고정 이름입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics` | lib 25 passed, doc 3 passed(compile_fail 포함), 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `trace_line_uses_only_allowed_keys`: 모든 key가 허용 목록 안이고 값 문자 집합 제한, 고정 줄 golden.
- `buckets_follow_the_guaranteed_scale`.
- `TraceDimensions` compile_fail doctest(문자열 field 추가 불가).

## 성능

- 해당 없음.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 없음.
