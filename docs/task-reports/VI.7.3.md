# 작업 보고서: VI.7.3 paragraph 속성

- WBS: VI.7.3
- 작업명: paragraph alignment, line height, letter spacing, wrapping mode 정의(실제 layout은 Phase X)
- 상태: 완료 (`[x]`)
- 선행: VI.7.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/text.rs` — `ParagraphStyle`, `TextAlignment`, `LineHeight`,
  `WrappingMode`, 경계 상수, `ParagraphStyleError`.
- `docs/task-reports/VI.7.3.md`(신규), 계획서(VI.7.3 `[ ]`→`[x]`).

## 구현 요약

- **정의**: 정렬(Start/Center/End/Justify), 줄 높이(Auto / 글꼴 크기 배수 / 고정 논리
  픽셀), 자간(em, 글꼴 크기 대비 비율), 줄바꿈(Wrap/NoWrap). 모든 float는
  `FiniteF32`라 NaN·무한대가 저장될 수 없습니다.
- **기본값**: Start, Auto, 자간 0, Wrap.
- **경계**: 배수 `0 < v ≤ 10`, 고정 `0 < v ≤ 10000`, 자간 `−1 ≤ v ≤ 10` em. 범위 밖은
  `LineHeightOutOfRange`/`LetterSpacingOutOfRange`로 거부합니다. 실제 줄 배치는
  layout 엔진(Phase X) 소관입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 118 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 기본값 validation(`paragraph_style_defaults_are_valid`) | 통과 |
| 경계값 validation(배수·고정·자간 상하한 포함/초과)(`paragraph_style_boundary_values`) | 통과 |

## 차이(작업 지시와 다른 점)

- 지시에 수치 경계가 없어 위 상한·하한을 정했습니다. 값 자체는 layout 측정(Phase X)
  이후 조정될 수 있으며, 변경 시 이 상수와 테스트를 함께 갱신합니다.

## 남은 위험 / 한계

- 자간을 em 비율로 고정했습니다. 픽셀 단위 입력 UI는 변환 계층에서 처리합니다.
