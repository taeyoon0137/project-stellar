# 작업 보고서: VIII.4.4 color·string·수치 함수 evaluator

- WBS: VIII.4.4
- 작업명: color·string·수치 함수 evaluator 구현
- 상태: 완료 (`[x]`)
- 선행: VIII.4.2(`[x]`), VIII.1.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/function_eval.rs`(신규) — `apply_function`(16개 함수 의미), sRGB↔Display P3 선형 변환 행렬, 색 변환·혼합,
  reference fixture 테스트.
- `crates/expressions/src/eval.rs` — `EvalErrorKind::{InvalidArgument, StringTooLong}`, `ArgumentIssue`, `MAX_STRING_BYTES`,
  `numeric` crate 공개, 단계용 `UnsupportedNode` 제거.
- `crates/expressions/src/bytecode.rs` — `Instr::Call { function, argc }`, Call 컴파일, `CompileError::UnsupportedNode` 제거.
- `crates/expressions/src/vm.rs`·`crates/expressions/src/ast_eval.rs` — Call 실행, VIII.4.1 미지원 검사 테스트를 Call 지원 검사로 전환.
- `crates/expressions/src/lib.rs` — 모듈 선언.
- `docs/task-reports/VIII.4.4.md`(신규), 계획서(VIII.4.4 `[ ]`→`[x]`).

## 구현 요약

- **수치**: `min`/`max`/`clamp`/`abs`/`round`/`floor`/`ceil`/`lerp`/`remap`은 단위 타입을 유지하고 f32 기본 연산과 정확 연산만 씁니다.
  `clamp` 하한 > 상한은 `InvalidArgument(InvertedRange)`, `remap` 빈 입력 범위는 `DivisionByZero`, overflow는 `NonFiniteResult`.
  `round`는 0.5를 0에서 먼 쪽으로 반올림합니다.
- **색상**: 선형광에서 계산합니다(아키텍처 §10.3). `mix`는 `to`를 `from` 색공간으로 변환한 뒤 premultiplied alpha로 보간하고
  t = 0/1 끝점은 입력을 그대로 돌려줍니다(t는 [0, 1]). `with_alpha`는 alpha [0, 1]. `to_srgb`/`to_display_p3`는 III.4.2 transfer
  decode → D65 primaries 행렬 → encode이며 gamut 밖 값을 자르지 않습니다.
- **문자열**: `concat`, `replace`(겹치지 않는 일치 왼쪽부터, 빈 검색어 거부), `format_number`(자릿수 0~9 정수, 정확한 2진 값의
  10진 반올림, `-0` 표기 제거). 결과는 `MAX_STRING_BYTES`(1 MiB) 이하이며, 할당 전에 결과 길이를 계산해 초과를 `StringTooLong`으로 거부합니다.
- 모든 함수 오류는 호출 node StableId를 가리키고, VM과 AST 기준 evaluator가 같은 `apply_function`을 씁니다. 이로써 미지원 node가 없어졌습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (111 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **수치 reference fixture**(`numeric_reference_values`): 21건(5개 단위 타입, clamp 경계, round 반올림 경계 `±2.5`·`0.49999997`·`-0.4`, floor/ceil 음수, lerp 외삽, remap 단위 변경·역방향 범위) 정확 일치 | 통과 |
| 수치 오류(`numeric_argument_errors`): 역전 범위, remap 0 범위, lerp overflow | 통과 |
| **색상 reference fixture**(`color_reference_values`): with_alpha, mix 끝점 정확, 검정·흰색 중간 = 선형광 0.5 인코딩(≈0.735357), premultiplied 혼합(빨강+투명 → 반투명 빨강), 다른 색공간 혼합 결과 색공간 | 통과(허용 1e-5) |
| **색공간 변환 reference fixture**(`color_space_conversion_reference_values`): f64 독립 기준 구현과 6건 일치(허용 1e-5 = `color.transfer_abs_tolerance`), 공개 근사값(sRGB 빨강 → P3 `0.9175, 0.2003, 0.1386`, P3 빨강 → sRGB `1.0931, -0.5434, -0.2538`, 허용 1e-3), 무채색 유지, 같은 색공간 항등, sRGB→P3→sRGB 왕복 11건 | 통과 |
| 색상 인자 오류(t·alpha 범위 밖)(`color_argument_errors`) | 통과 |
| **문자열 reference fixture**(`string_reference_values`): concat(한글·빈 문자열), replace 3건, format_number 8건(반올림·`-0` 제거·큰 값·9자리) | 통과 |
| 문자열 오류·한도(`string_argument_errors_and_limits`): 빈 검색어, 비정수·범위 밖 자릿수, 한도 정확히 허용·1 byte 초과 거부, 치환 확장 초과 | 통과 |
| 공유 concat 40단(2^41 bytes 규모)이 메모리 폭증 없이 `StringTooLong`(`shared_concat_chain_hits_string_limit_instead_of_exploding`) | 통과 |
| 모든 fixture에서 의미 함수 = VM = AST 기준 evaluator | 통과 |

## 차이(작업 지시와 다른 점)

- VIII.4.1에서 Call을 예로 쓴 "미지원 node 거부" 테스트는 미지원 node가 없어졌으므로 "Call 컴파일·평가 성공" 검사로 바꿨습니다.
- 문자열 결과 한도(`MAX_STRING_BYTES` = 1 MiB)를 새로 정했습니다. 공유 node로 결합을 반복하는 표현식이 메모리를 폭증시키지
  않게 하기 위한 값이며, 설정 파일 관리 대상으로 옮길지는 후속 결정입니다.

## 남은 위험 / 한계

- 색 transfer는 core의 `powf`(libm 초월함수)를 사용합니다. 결과는 허용 오차 안에서 검증했지만 플랫폼 간 bit 동일성은
  보장되지 않습니다(VIII.4.6에서 ADR-0006 규칙 4로 다룹니다).
- D65 primaries 행렬은 이 crate에 상수로 있습니다. renderer의 색공간 변환(Phase XI)이 같은 상수를 쓰도록 공용화가 필요합니다.
- `format_number`는 Rust 표준 float 포맷(플랫폼 독립 구현)을 사용합니다. 지역화(소수점 기호 등)는 지원하지 않습니다.
