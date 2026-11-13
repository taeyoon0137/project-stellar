# 작업 보고서: VI.5.3 Ellipse

- WBS: VI.5.3
- 작업명: Ellipse는 bounds로 정의, start/end angle 미지원(파일에 있으면 unknown field 정책)
- 상태: 완료 (`[x]`)
- 선행: VI.5.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/shape.rs` — `EllipseParams`(bounds), `RawField`,
  `ELLIPSE_FIELD_BOUNDS`, `CIRCLE_KAPPA`, `to_fields`/`from_fields`/`to_path`,
  `ShapeError::{UnknownMandatoryField, MissingField}`.
- `docs/task-reports/VI.5.3.md`(신규), 계획서(VI.5.3 `[ ]`→`[x]`).

## 구현 요약

- **정의**: 타원은 shape local 좌표의 외접 `Rect` bounds 하나로만 정의합니다.
  호·부채꼴(start/end angle)은 초기 범위에서 지원하지 않습니다.
- **unknown field 정책(R6)**: 저장 계층이 파싱한 tagged field(`RawField`)를
  해석할 때, 모르는 optional field(예: 미래 버전의 start angle)는 무시하고 모르는
  mandatory field는 `UnknownMandatoryField`로 거부합니다. document crate는 crate
  그래프상 storage codec에 의존하지 않으므로 field 의미 해석만 담당합니다.
- **path 평가**: `to_path`는 오른쪽 끝에서 시작해 4개 cubic(κ = 4(√2−1)/3)으로
  전체 타원을 그리고 Close합니다(VI.6.1 command만 사용).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 89 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| full ellipse path reference(200×100, 13점 손 계산 일치, 결정론)(`full_ellipse_path_matches_reference`) | 통과 |
| unknown optional 무시·mandatory 거부·bounds 누락(`ellipse_unknown_field_policy`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- κ 근사의 최대 반경 오차(약 0.027%)는 렌더 허용 오차 정책(Phase XI)과 함께
  재확인합니다.
