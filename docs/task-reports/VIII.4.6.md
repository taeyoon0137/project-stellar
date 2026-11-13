# 작업 보고서: VIII.4.6 evaluator 결정론 테스트

- WBS: VIII.4.6
- 작업명: 동일 input environment와 bytecode가 bit-stable TypedValue를 반환하는 결정론 테스트를 양 플랫폼에서 실행
- 상태: 부분 완료 (`[~]`) — macOS 부분 완료, Windows는 ADR-0006으로 보류
- 선행: VIII.4.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 행의 Windows 비교(`CI-WIN`)는 ADR-0006에 따라 보류.

## 변경 파일 목록

- `crates/expressions/tests/evaluator_determinism.rs`(신규) — 14 case 결정론 corpus와 5경로 비교, golden 비교, 분류 검사.
- `crates/expressions/tests/fixtures/evaluator_determinism_v1.golden`(신규) — case별 분류(`ieee`/`libm`)와 결과 bytes hex.
- `docs/task-reports/VIII.4.6.md`(신규), 계획서(VIII.4.6 `[ ]`→`[~]`).

## 구현 요약

- **corpus**: 산술(공유 참조), 음수 나머지, 0 나누기·overflow 오류, 수치 함수 9종 조합, match·if·Optional/null, 비교·논리
  short-circuit, 문자열 함수(한글·format_number), 정확 10진 포맷, with_alpha, 색 혼합, sRGB↔P3 변환(gamut 밖) — 값·null·오류
  결과를 모두 포함합니다.
- **5경로 동일성**: case마다 ① 같은 program 반복 평가 3회, ② 재컴파일(program 구조 동일), ③ canonical AST bytes에서 복원 후
  컴파일, ④ 참조 값 제공 순서 정방향·역방향(선형 탐색 제공자), ⑤ AST 기준 evaluator — 결과 bytes(`TypedValue::to_bytes`)가 모두 같아야 합니다.
- **golden**: 결과를 고정 golden 파일과 bytes 단위로 비교합니다. 분류는 AST의 함수 호출로 다시 계산해 golden 표기와 일치를
  확인합니다(`libm` = 색 transfer `powf` 사용 함수 포함). with_alpha 결과는 연산 없이 정해지므로 기대 bytes를 손으로 조립해 확인했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (116 unit + 3 integration + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **bit-stable 결과**(`evaluator_results_are_bit_stable_and_match_golden`): 14 case × 5경로 결과 bytes 동일, macOS 결과 = golden bytes | 통과(macOS) |
| golden 구성(`golden_covers_both_classes_and_all_outcomes`): ieee 11·libm 3, value·null·error 결과 모두 포함 | 통과 |
| 손 계산 golden(`golden_header_bytes_are_hand_checked`) | 통과 |
| 수동 검산: `arith_length_ratio` = 14.6/4.3 ≈ 3.3953488(`0x40594d65`), `angle_remainder_negative` = −90.75, `numeric_functions` = lerp(33%, 42%, 0.375) = 36.375%(`0x42118000`), `to_srgb_out_of_gamut` 채널 ≈ (−2.906, 1.0183, −0.5462) | 일치 |
| **macOS/Windows fixture bytes 동일** | 미실행(Windows 보류) |

## 차이(작업 지시와 다른 점)

- ADR-0006(Windows 지원 보류)에 따라 macOS 부분만 실행했습니다. 판정 규칙 4에 따라 상태는 `[~]`입니다. corpus에 libm 초월함수
  (`powf`)를 쓰는 색 변환 결과가 포함되어 있어, Windows 결과가 같다는 것을 구성만으로 주장할 수 없기 때문입니다.
- 행의 "input environment"는 `EvaluationEnvironment`(VIII.7.2, 미구현) 대신 참조 값 제공자(`ReferenceValues`)로 구성했습니다.

## 남은 위험 / 한계

- **Windows 재개 시 필요한 검증**: 같은 integration test를 `CI-WIN`에서 실행해 14 case의 결과 bytes가 golden과 같은지 확인해야
  합니다. `ieee` 11 case는 IEEE 기본·정확 연산만 써서 같아야 하고, `libm` 3 case(`mix_linear_light`, `to_display_p3`,
  `to_srgb_out_of_gamut`)는 플랫폼 `powf` 구현 차이로 마지막 bit가 다를 수 있습니다. 다르면 transfer 함수를 결정적 구현으로 바꾸거나
  해당 결과를 허용 오차 비교로 분리하는 결정이 필요합니다.
- 결정론 corpus를 CI golden으로 등록하는 작업은 VIII.9.3 소관입니다(선행 VIII.8.3 미완료).
