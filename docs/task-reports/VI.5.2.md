# 작업 보고서: VI.5.2 Rectangle 파라미터와 radius clamp

- WBS: VI.5.2
- 작업명: Rectangle size·개별 corner radius 정의, radius를 bounds에 clamp하는 평가 함수(저장값은 원본 유지)
- 상태: 완료 (`[x]`)
- 선행: VI.5.1, III.3.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/shape.rs` — `RectangleParams`,
  `RectangleParams::effective_corner_radius`.
- `docs/task-reports/VI.5.2.md`(신규), 계획서(VI.5.2 `[ ]`→`[x]`).

## 구현 요약

- **파라미터**: `size: Size2`(비음수) + `corner_radius: CornerRadius`(VI.3.1과 같은
  비음수 4귀퉁이 타입 재사용).
- **평가 clamp**: 각 변에서 인접 두 radius 합이 변 길이를 넘으면 모든 radius에
  `f = min(변 길이 / 인접 합)`을 곱합니다(형태 비율 보존). clamp는 평가에서만
  일어나고 **저장값은 원본 그대로** — 크기를 다시 키우면 원래 둥글기가
  돌아옵니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 84 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| oversize radius reference fixture(100×50/r40 → 25, 200×100/tl150 → 100) + 원본 유지(`oversize_radius_reference_fixture`) | 통과 |
| 맞는 radius 불변, 크기 0 → 실효 0·원본 유지 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Rectangle → path 평가(VI.5.6)가 이 실효 radius를 사용합니다.
