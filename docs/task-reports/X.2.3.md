# 작업 보고서: X.2.3 percentage position·size

- WBS: X.2.3
- 작업명: percentage position/size를 containing block content rect 기준으로 계산
- 상태: 완료 (`[x]`)
- 선행: X.2.1(`[x]`, 이 lane), III.3.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/rounding.rs` — `LayoutUnit::checked_mul_percent`(f32 백분율 정확 분해 + `i128` 1회 반올림), `decompose_f32`, 테스트.
- `crates/layout/src/free.rs` — `Offset::Percentage`, 백분율 offset 해석·인코딩, Hug 측정 규칙, 중첩 fixture.
- `crates/layout/src/engine.rs` — `SizingMode::Percentage` 크기 해석(containing block 기준, indefinite면 콘텐츠 크기).
- `crates/layout/src/error.rs` — X.2.1의 임시 `UnsupportedFeature` 오류 제거(백분율 구현으로 사용처 없음).
- `docs/task-reports/X.2.3.md`(신규), 계획서(X.2.3 `[ ]`→`[x]`).

## 구현 요약

- **기준**: 백분율 offset·크기는 containing block = 부모 content rect의 해당 축 크기 기준입니다(가로는 content
  너비, 세로는 content 높이). 중첩되면 각 단계의 content rect(자기 padding 반영)가 다음 단계 기준입니다.
- **정확 계산**: `checked_mul_percent`는 f32 백분율을 비트 분해한 정확한 유리수 `m·2^e`로 `raw·m·2^e/100`을
  정수 계산해 한 번만 반올림합니다(ties away from zero, X.1.5 규칙). float 곱의 중간 반올림이 없어 플랫폼 간
  같은 값이며, 범위 초과는 `OutOfRange`입니다.
- **Hug 부모**: containing block이 아직 없으므로 백분율 시작 anchor 자식은 Hug extent에 기여하지 않고, 백분율
  크기 자식은 콘텐츠 크기로 기여한 뒤 확정된 content rect 기준으로 다시 해석됩니다.
- canonical tag: offset `Length` 0, `Percentage` 1.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 51 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| nested percentage fixture: root(padding 20) → A 50%·50% at (10%, 25%) → B 25%·100% at (50%, 0) → C 33.333332% 너비 + 끝 10% anchor(subpixel 1451/853) · root의 음수 백분율 offset D, 입력 round-trip(`nested_percentage_fixture_uses_containing_block_content_rect`) | 통과 |
| Hug 부모에서 백분율 자식이 부모 확정 후 해석됨(`percentage_children_of_hug_parent_resolve_after_parent_size`) | 통과 |
| 백분율 곱 정확·1회 반올림(50%, 100/3 %, ±12.5%, 0.5 tie, 0.1%, subnormal, 1e30 범위 오류, MAX×100%)(`percent_multiplication_is_exact_and_rounds_once`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 백분율 값 자체의 범위는 검증하지 않고, 해석 결과가 LayoutUnit 범위를 넘을 때 계산 오류(`Arithmetic`)로
  보고합니다.
