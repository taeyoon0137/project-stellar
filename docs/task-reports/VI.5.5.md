# 작업 보고서: VI.5.5 Polygon

- WBS: VI.5.5
- 작업명: Polygon side count≥3, rotation, corner radius
- 상태: 완료 (`[x]`)
- 선행: VI.5.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/shape.rs` — `PolygonParams`(`new`/`sides`/`vertices`/
  `to_path`), `MAX_POLYGON_SIDES = 1000`,
  `ShapeError::{InvalidSideCount, InvalidCornerRadius}`.
- `docs/task-reports/VI.5.5.md`(신규), 계획서(VI.5.5 `[ ]`→`[x]`).

## 구현 요약

- **검증 생성자**: 변 수 3..=1000, 회전은 `Angle`(도 단위, 기존 `TypedValue::Angle`
  사용례와 동일), corner radius는 비음수 유한 값만 허용합니다.
- **꼭짓점**: bounds에 내접하는 타원 위, 회전 0°에서 첫 꼭짓점은 위쪽 가운데,
  y-down 기준 시계 방향. 계산은 f64로 수행해 다각형이 커도 오차를 줄입니다.
- **path**: radius 0이면 MoveTo + LineTo + Close. radius > 0이면 각 꼭짓점에서
  인접 변 위의 접점(거리 `t = r / tan(내각/2)`)을 잇는 원호를 cubic(핸들
  `4/3·tan(φ/4)·r`)으로 근사합니다. `t`가 인접 변 절반을 넘으면 clamp하고 저장값은
  원본을 유지합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 95 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 2 sides 거부(+ 상한 초과·음수 radius)(`two_sides_are_rejected`) | 통과 |
| 3·4·100 sides path fixture(command 수, 꼭짓점이 원 위, 시작점, 다이아몬드 좌표, 결정론)(`three_four_and_hundred_side_path_fixtures`) | 통과 |
| 45° 회전 정사각형, 둥근 모서리 command 구조, 과도 radius clamp | 통과 |

## 차이(작업 지시와 다른 점)

- 파일 입력 방어용 상한 `MAX_POLYGON_SIDES = 1000`을 추가했습니다(지시에는 상한
  없음). 비정상 파일이 거대한 path를 만들지 못하게 하기 위함입니다.

## 남은 위험 / 한계

- polygon star(내부 반경) 변형은 범위 밖입니다.
