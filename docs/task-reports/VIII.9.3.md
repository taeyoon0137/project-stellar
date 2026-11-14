# 작업 보고서: VIII.9.3 양 플랫폼 evaluator golden corpus

- WBS: VIII.9.3
- 작업명: 양 플랫폼 evaluator golden corpus를 CI에 추가(모든 result bytes 동일)
- 상태: 부분 완료 (`[~]`) — corpus·golden·macOS 검증 완료. Windows 실행은 미완료(ADR-0006, `libm` 60 case)이고, 엄격 선행
  VIII.4.6이 `[~]`라 이 행은 `[x]`가 될 수 없습니다(아래 "선행 판단")
- 선행: VIII.4.6(`[~]`, Windows 비교만 ADR-0006으로 보류 — **엄격 선행 미충족**, 아래 "선행 판단"), VIII.8.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증, debug·release 두 profile). 행에 환경 표기가 없어 필수 환경은 `DEV-MAC`입니다(계획서 §1.3
  적용 규칙 2). 제품 crate 테스트는 CI workflow에 추가하지 않고 통합 시 로컬 `cargo xtask check-core`로 검증합니다(계획서
  개정 기록 2026-11-14 "제품 테스트 실행 위치"). `CI-WIN` 비교는 ADR-0006으로 보류.

## 변경 파일 목록

- `crates/expressions/tests/evaluator_golden_corpus.rs`(신규) — 연산·함수 registry 전체를 표로 훑는 corpus 생성기,
  3경로(bytecode·canonical 복원 후 bytecode·AST 기준) 동일성 검사, golden 파일 bytes 비교, 범위·손 계산 검사.
- `crates/expressions/tests/fixtures/evaluator_golden_corpus_v1.golden`(신규, case 907줄, 57,874 bytes).
- `crates/bindings/tests/fallback_golden_corpus.rs`(신규) — 안전값과 orphan 기본값(VIII.8.2·VIII.8.3) 열기 결과 golden.
- `crates/bindings/tests/fixtures/fallback_golden_corpus_v1.golden`(신규, case 80줄).
- `docs/task-reports/VIII.9.3.md`(신규), 계획서(VIII.9.3 `[ ]`→`[~]`).

## 구현 요약

### expression evaluator corpus (907 case)

- **생성 규칙**(손으로 나열하지 않고 registry 표에서 만듦):
  - 산술: `ARITHMETIC_RULES` 33개 × 피연산자 6쌍(일반·음수 분수·반올림 `0.1+0.2`·0 나누기·overflow·subnormal) = 198,
    부호 반전 5 타입 × 4값.
  - 비교 6종 × 타입 10종 × 값 4쌍 중 type 규칙을 통과하는 조합(160) + nullable 동등 비교 8.
  - 논리 and/or 진리표·오른쪽 0 나누기 short-circuit, not, if(short-circuit·`T`/`Optional<T>` 공통 타입), match
    (Boolean·Number·String·Enum 대상의 일치 arm·기본 branch·선택 안 된 arm 오류).
  - 함수 16종 전부: 수치 함수는 수치 타입 5종마다(`remap`은 입력·출력 타입 25조합), 색 함수는 sRGB·P3·투명·gamut 밖 색,
    문자열 함수는 빈 문자열·한글·emoji·제어 문자·1 MiB 초과, `format_number`는 9값 × 자릿수 3 + 잘못된 자릿수 3.
  - 참조 입력: 값 누락·Required null·타입 불일치·환경 입력. DAG 공유(같은 node 두 번 참조) 사슬 20·30·130단계.
- **줄 형식**: `<이름> <분류> <입력 digest> <결과>`. 분류는 AST의 함수 호출로 계산(`libm` = 색 transfer `powf` 사용
  `mix`·`to_srgb`·`to_display_p3`, 그 밖 `ieee`). 입력 digest는 canonical AST bytes + 참조 입력(대상·타입·값 bytes·누락
  여부)의 FNV-1a 64라 case 정의가 바뀌면 golden이 달라집니다. 결과는 `value:<to_bytes hex>` | `null` |
  `error:<EvalErrorKind 공백 제거>@<오류 node 번호>`로 오류 종류뿐 아니라 오류 node까지 고정합니다.
- **검사**: case마다 bytecode 2회·canonical bytes 복원(구조 동일 확인) 후 재컴파일·AST 기준 evaluator 결과가 오류 node까지
  같아야 하고, 렌더링한 전체 파일이 golden 파일과 byte 단위로 같아야 합니다. 갱신은
  `STELLAR_UPDATE_EVALUATOR_CORPUS=1`일 때만 씁니다.

### orphan 기본값·안전값 corpus (80 case)

- 값 타입 13종의 안전값, 필수 13종 + Optional 3종 × 저장된 기본값 4상태(정상·없음·bytes 절단·타입 불일치)로
  `open_with_stored_defaults`를 연 orphan target 값과 진단 종류, 토큰을 참조하는 기본값, orphan target을 읽는 live
  binding 값, 진단 수(48). 같은 입력으로 두 번 열어 값·진단이 같은지와 입력이 수정되지 않았는지도 확인합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` / `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `STELLAR_UPDATE_EVALUATOR_CORPUS=1 cargo test -p stellar-expressions --test evaluator_golden_corpus every_result` | 첫 실행 실패(`ref.missing` case가 타입 없는 참조라 컴파일 불가) → 타입은 주고 값만 빼는 `withheld` 입력으로 고친 뒤 종료 코드 0, golden 생성 |
| `STELLAR_UPDATE_FALLBACK_CORPUS=1 cargo test -p stellar-bindings --test fallback_golden_corpus fallback_results` | 종료 코드 0, golden 생성 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (unit 128 + `evaluation_memory_bound` 3 + `evaluator_determinism` 3 + `evaluator_golden_corpus` 3 + doctest 3 passed, 0 failed) |
| `cargo test -p stellar-bindings` | 종료 코드 0 (unit 51 + `fallback_golden_corpus` 2 + `token_change_benchmark` 3 passed, 0 failed) |
| `cargo test -p stellar-expressions --release --test evaluator_golden_corpus --test evaluator_determinism` | 종료 코드 0 (3 + 3 passed) — release profile에서도 golden bytes 동일 |
| `cargo test -p stellar-bindings --release --test fallback_golden_corpus` | 종료 코드 0 (2 passed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` / `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |
| (리뷰 후 재검증, XVI.6.4까지 쌓인 HEAD) `cargo test -p stellar-expressions -p stellar-bindings --locked` | 종료 코드 0 (bindings unit 51 + `fallback_golden_corpus` 2 + `token_change_benchmark` 3, expressions unit 129 + `evaluation_memory_bound` 3 + `evaluator_determinism` 3 + `evaluator_golden_corpus` 3 + `expression_fuzz` 3 + doctest 3 passed, 0 failed) |
| (리뷰 후 재검증) `cargo clippy -p stellar-expressions -p stellar-bindings --all-targets --locked -- -D warnings` | 종료 코드 0 (경고 0) |
| (리뷰 후 재검증) `cargo fmt -p stellar-expressions -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 907 case × 3경로 결과(오류 node 포함) 동일, 렌더링 파일 = golden bytes(`every_result_matches_golden_bytes`) | 통과(macOS debug·release) |
| node 종류 7·이항 연산 13·단항 2·함수 16 전부 포함, 이름 유일, ieee 847·libm 60, value·null·error 결과 모두 포함, 산술 198 case 전부 type 통과(`corpus_covers_every_operator_function_kind_and_outcome`) | 통과 |
| 손 계산: `12.5+4=16.5`, `12.5%4=0.5`, `0.1+0.2=0x3e99999a`, 0 나누기·overflow 오류, `round(-2.5)=-3`, `round(0.49999997)=0`, `2^130` 사슬 overflow, and/or short-circuit, with_alpha 채널 bytes, 한글 concat, 1 MiB 초과(`hand_checked_golden_entries`) | 통과 |
| 안전값·orphan 기본값 80 case = golden bytes, 재열기 결정적, 입력 무수정(`fallback_results_match_golden_bytes`) | 통과(macOS debug·release) |
| 손 계산: 정상 기본값 = 표본 값, 손상 3종 = 안전값 + 정확한 진단, Optional 손상 = null, `base 10px + 2px = 12px`, `12.5px + 1px = 13.5px`, 진단 48(`golden_semantics_are_hand_checked`) | 통과 |
| **CI에 corpus 등록** | CI workflow 미등록(개정 기록상 제품 crate 테스트는 CI에 추가하지 않음). 두 corpus는 `#[ignore]`·feature 조건이 없는 integration test라 로컬 통합 검증 `cargo xtask check-core`의 `cargo test --workspace --locked` 단계에 포함됩니다 |
| **Windows 결과 bytes 동일** | 미실행(ADR-0006 보류) |

## 차이(작업 지시와 다른 점)

- **선행 판단**(통합 리뷰 지적 후 정정): 행 선행 VIII.4.6은 `[~]`(macOS 부분 완료, Windows 비교만 ADR-0006으로 보류)입니다.
  계획서 §1.2 규칙 4는 "선행 작업이 모두 `[x]`가 아니면 코드를 수정하지 않고 blocked 보고서만 작성"하라고 정하므로, 이 작업은
  **엄격한 행 선행 충족이 아닙니다**. ADR-0006은 `[x]`·`[~]` 판정 규칙과 "양 플랫폼 비교가 들어 있는 행은 macOS 부분을
  구현한다"(결정 4)만 정하고 선행 규칙을 완화한다고 쓰지 않습니다. 처음 보고서의 "ADR-0006으로 충족 처리"는 근거 없는
  표현이라 정정합니다.
  - 그래도 test·golden을 유지한 이유: 이 행은 ADR-0006 결정 4가 macOS 부분 구현을 지시하는 "양 플랫폼" 비교 행이고,
    VIII.4.6에 남은 것은 이 행에도 똑같이 남는 Windows 증거뿐입니다(VIII.4.6이 `[x]`가 되려면 Windows 재개가 필요해, 규칙 4를
    엄격히 적용하면 Windows 재개 전에는 이 행의 macOS 부분도 착수할 수 없습니다). 변경은 integration test 2개와 golden
    fixture 2개뿐이고 제품 코드는 바꾸지 않습니다. 같은 선행(VIII.4.6 `[~]`)을 둔 VIII.5.4·VIII.6.4도 구현되어 있고,
    XVI.6.7(선행 VII.6.3 `[~]`)도 코드를 두고 `[~]`로 남긴 같은 처리입니다.
  - 결과: 상태는 `[~]`가 상한입니다. VIII.4.6이 `[x]`가 되기 전에는 이 행을 `[x]`로 올리지 않습니다.
  - **필요한 결정**: "Windows 증거만 보류된 `[~]` 선행"을 규칙 4의 충족으로 볼지는 제품 소유자 결정 또는 ADR-0006 개정이
    필요합니다(VIII.5.4·VIII.6.4에도 소급 적용됨). 엄격 적용으로 결정되면 이 작업의 test·golden 추가 커밋을 `git revert`로
    되돌리고 이 보고서를 `docs/task-reports/VIII.9.3-blocked.md`로 바꿉니다.
- **CI 등록**(정정): 처음 보고서는 CI job 미등록을 `[~]` 사유로 적었지만, 계획서 개정 기록(2026-11-14 "제품 테스트 실행
  위치")은 제품 crate 테스트를 CI에 추가하지 않고 통합 시 로컬 `cargo xtask check-core`로 검증한다고 정합니다. 두 corpus는
  기본 `cargo test`에 포함되는 integration test라 check-core의 `cargo test --workspace --locked` 단계에서 실행되므로, CI
  workflow 미등록은 `[~]` 사유가 아닙니다. `[~]` 사유는 아래 ADR-0006 `libm` 판정과 위 선행 판단 두 가지입니다.
- ADR-0006 규칙 4: `ieee` 847 case는 구성상 플랫폼 간 같아야 하지만, `libm` 60 case는 플랫폼 `powf` 구현에 의존하므로
  macOS 증거만으로 `[x]`를 줄 수 없습니다(VIII.4.6과 같은 판정).
- VIII.4.6 corpus(14 case)는 그대로 두고 별도 파일로 확장했습니다. 오류 결과는 VIII.4.6과 달리 오류 node 번호까지
  고정합니다.

## 남은 위험 / 한계

- **CI 실행 위치**: 지금은 통합 시 로컬 `cargo xtask check-core`가 corpus를 실행합니다. 제품 테스트를 CI에서 실행하기로
  결정이 바뀌면 macOS CI job에 `cargo test -p stellar-expressions --test evaluator_golden_corpus --test
  evaluator_determinism --locked`와 `cargo test -p stellar-bindings --test fallback_golden_corpus --locked`를 추가합니다.
- **선행 규칙 결정 대기**: 위 "선행 판단"의 결정이 엄격 적용이면 이 작업의 test·golden을 되돌리고 blocked 보고서로 바꿔야
  합니다.
- **Windows 재개 시**: 같은 테스트를 `CI-WIN`에서 실행해 golden과 bytes 단위로 같은지 확인합니다. `libm` 60 case
  (`mix`·`to_srgb`·`to_display_p3`)가 다르면 transfer 함수를 결정적 구현으로 바꾸거나 해당 case를 허용 오차 비교로
  분리하는 결정이 필요합니다.
- golden은 생성 규칙(값 표·case 이름·digest 형식)과 오류 Debug 표기를 고정합니다. 오류 enum의 필드 이름이나 Debug
  형식을 바꾸면 golden을 의도적으로 갱신해야 합니다.
- subnormal 입력·결과 case가 있어 flush-to-zero가 켜진 환경에서는 실패합니다(의도한 감지 대상).
