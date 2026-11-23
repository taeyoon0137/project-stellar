# 작업 보고서: XXX.3.2 timing function evaluator

- WBS: XXX.3.2
- 작업명: cubic-bezier 고정 반복 해법, spring 고정 step 적분, Color premultiplied 보간, IEEE 기본 연산만.
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 VIII.4.6이 `[x]`가 아닙니다. 선행이 끝나면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`)
- 결정: ADR-0094 결정 6.2
- 선행: XXX.1.1, VIII.4.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/prototype/src/timing.rs`(신규), `timing_corpus.txt`(신규).
- `crates/prototype/src/lib.rs`: module 등록.

## 구현 요약

- ADR-0094 결정 6.2: bezier 64회 이분법, CSS steps 공식, spring 1ms semi-implicit Euler(선형 보간, 끝에서 정확히 1), 지연 동안 시작값.
- `interpolate`는 같은 타입 끝값만 보간하고 Color는 premultiplied encoded 채널 선형 보간(결과 alpha 0이면 채널 0)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-prototype` | 통과(lib 13, 통합 test 11개 파일 24개). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `timing::tests::reference_corpus_is_byte_identical`: 6개 timing × 13 tick의 진행률·Color 보간 bits golden.
- `cubic_bezier_boundaries`, `steps_follow_css_easing`, `spring_settles_and_ends_exactly`, `color_interpolates_premultiplied`(alpha 0 fixture 포함), `linear_and_boundaries`.
- `inventory.rs::evaluation_path_has_no_transcendental_calls`(XXX.6.3 commit).

## 차이(작업 지시와 다른 점)

- corpus는 같은 장비(DEV-MAC) 재실행 동일성만 확인했습니다. 다른 플랫폼 비교는 Windows 보류(ADR-0006)로 하지 않았습니다.

## 남은 위험 / 한계

- `f64::floor`는 정확한 연산이지만 컴파일러가 libm 호출로 낮출 수 있습니다. 결과는 정확하므로 플랫폼 차이가 없습니다.
