# 작업 보고서: X.1.5 레이아웃 rounding 정책(logical subpixel)과 pixel snapping 분리

- WBS: X.1.5
- 작업명: 레이아웃 수치 rounding 정책을 logical subpixel 단위로 고정하고 pixel snapping을 렌더 단계로 분리
- 상태: 완료 (`[x]`)
- 선행: III.3.1(`FiniteF32`, `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. `planning/task-status.yaml`의 required_environments는 `DEV-MAC`만 지정).

## 변경 파일 목록

- `crates/layout/src/rounding.rs`(신규) — 정책 문서, `SUBPIXELS_PER_PX`, `LayoutUnit`, `LayoutUnitError`, 정확 반올림 helper, 1/3·배율 1.5 golden fixture 테스트.
- `crates/layout/src/lib.rs` — `pub mod rounding;` 선언.
- `docs/task-reports/X.1.5.md`(신규), 계획서(X.1.5 `[ ]`→`[x]`).

## 구현 요약

- **logical subpixel 고정**: 레이아웃 수치는 logical px의 `1/64` 단위 정수 `LayoutUnit(i32)`로
  계산·확정합니다(범위 약 ±33,554,431 px).
- **반올림 규칙**: float 입력(`from_px`/`from_length`), 정확한 유리수 입력(`from_ratio_px`), 비율 곱
  (`checked_mul_ratio`), 정수 나눗셈(`checked_div_round`)은 모두 가장 가까운 subpixel로 반올림하고 정확히
  중간이면 0에서 먼 쪽으로 보냅니다(양·음 대칭). 덧셈·뺄셈·정수배는 정확합니다. 범위 초과는
  `LayoutUnitError::OutOfRange`, 0 나눗셈은 `DivisionByZero`로 반환하며 조용히 saturate하지 않습니다.
- **플랫폼 독립 계산**: float→subpixel 변환은 2의 거듭제곱 곱셈(정확), Rust가 정의한 `as i64` 절삭,
  정확한 소수부 뺄셈·비교만 사용하고 플랫폼 수학 라이브러리 `round`에 의존하지 않습니다. 유리수·비율·
  나눗셈 경로는 `i128` 정수 연산입니다.
- **pixel snapping 분리**: 이 모듈의 어떤 API도 device scale을 받지 않으며 device pixel 격자에 맞추지
  않습니다. snapping은 scene 값을 받는 렌더 단계(`renderer-core`) 책임으로 정책 문서에 명시했습니다.
  `1/64` 격자는 2의 거듭제곱이라, 렌더 단계가 logical 값에 배율 1.5(=3/2)를 곱해도 약 87,000 px
  이하에서 f32 반올림 오차가 없어 snapping 입력도 플랫폼 간 동일합니다.

## 실행한 명령 / 검증 (`DEV-MAC`, Apple Silicon arm64)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 종료 코드 0 (23 unit passed + 2 doctest passed, 0 failed) |
| `cargo test -p stellar-layout --release rounding` | 종료 코드 0 (8 passed; 최적화 빌드에서도 같은 bit 결과) |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **1/3·배율 1.5 golden fixture**(`one_third_and_scale_1_5_fixture_is_bit_exact`): 8개 입력의 f32 입력 bits → 기대 subpixel raw → 기대 logical px f32 bits → 배율 1.5 렌더 입력 f32 bits를 고정하고, 마지막 값이 정확한 유리수 `raw·3/128`과 같음을 확인 | 통과 |
| fixture 값: 1/3→21(0.328125 px), -1/3→-21, 2/3(배율 1.5의 device px 1개)→43, 100/3→2133, 0.1→6, 0.5 subpixel→1, 2.5 subpixel→3(최근접 짝수 규칙이면 2), -2.5→-3 | 통과 |
| 정확한 유리수 1/3·2/3·100/3 경로가 float 경로와 같은 값, 0 나눗셈 오류 | 통과 |
| 레이아웃 값이 배율 1.5 device 격자에 snap되지 않음(1/3 px → 0.328125 px, device 0.4921875) | 통과 |
| 정수 산술 반올림(6400/3→2133, 21×3/2→32, ±5/2→±3, 12/5→2) | 통과 |
| 범위 초과 오류(float 경계 2^25 px, `f32::MAX`, add/sub/mul/ratio/div overflow), `MIN` 경계 허용 | 통과 |
| `-0.0`→0, `Length` 왕복, 격자 값 f32·f64 왕복 정확 | 통과 |

## 차이(작업 지시와 다른 점)

- "양 플랫폼 동일"은 로컬(DEV-MAC)에서 golden bit 표와 정확 연산 구성으로 보장했습니다. Windows
  실행 증거는 이 작업에서 만들지 않았습니다. 같은 테스트가 workspace 테스트에 포함되어 있어 CI-WIN에서
  같은 표를 검증할 수 있습니다.
- device pixel snapping 함수 자체는 구현하지 않았습니다. `renderer-core`는 `scene`만 의존할 수 있어
  layout crate의 snapping 함수를 사용할 수 없고, 작업 지시가 snapping을 렌더 단계로 분리하라고 하므로
  layout에는 정책·격자·배율 독립성만 두었습니다.

## 남은 위험 / 한계

- `checked_div_round`는 몫만 반올림합니다. 여러 자식에 공간을 나눌 때의 잔여 subpixel 분배 순서는
  X.3.3(visual order 기준) 소관입니다. 예: 100 px / 3 → 2133 subpixel × 3 = 6399로 1 subpixel이 남습니다.
- `to_px_f32`는 `|raw| > 2^24`(약 262,144 px 초과)에서 f32 정밀도 때문에 정확하지 않을 수 있습니다
  (결정적인 Rust 캐스트 규칙을 따름). 정확한 값이 필요하면 `to_px_f64`를 사용합니다.
- 렌더 단계 snapping 규칙(반올림 방향, hairline 처리)은 renderer 작업에서 별도로 고정해야 합니다.
