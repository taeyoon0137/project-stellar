# 작업 보고서: VI.5.6 Shape → Path 순수 평가

- WBS: VI.5.6
- 작업명: Shape를 Path geometry로 평가하는 순수 함수(node를 Path로 변환하지 않음)
- 상태: 완료 (`[x]`)
- 선행: VI.5.2~VI.5.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/shape.rs` — `ShapeGeometry`(4종), `evaluate_shape_path`,
  `RectangleParams::to_path`(실효 radius 기반 둥근 사각형).
- `docs/task-reports/VI.5.6.md`(신규), 계획서(VI.5.6 `[ ]`→`[x]`).

## 구현 요약

- **순수 평가**: `evaluate_shape_path(&ShapeGeometry)`는 종류별 path(VI.5.2~VI.5.5,
  VI.6.1 command)를 반환할 뿐 node·geometry 값을 바꾸지 않습니다. 파괴 변환은
  VI.6.5 명시 command의 몫입니다.
- **Rectangle path**: VI.5.2 실효 radius를 사용해 위 변 왼쪽에서 시계 방향으로 그리고,
  radius 0 모서리는 직각, 양수는 κ cubic 1/4 원호입니다.
- **bit-stability**: 같은 입력을 100회 평가해 canonical bytes(VI.6.1 인코딩)가
  동일함을 4개 종류 모두 확인했습니다. 사칙연산만 쓰는 Rectangle은 golden command로
  고정했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 98 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 동일 입력 bit-stable path commands(4종 × 100회)(`evaluation_is_bit_stable`) | 통과 |
| Rectangle golden(직각·clamp된 둥근 모서리)(`rectangle_path_golden`) | 통과 |
| 평가가 node/geometry를 변환하지 않음(`evaluation_does_not_convert_the_node`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Polygon 꼭짓점은 `sin/cos`(f64 → f32 반올림)를 사용합니다. 같은 플랫폼에서는
  bit-stable이지만, macOS·Windows libm의 마지막 ulp 차이가 f32 반올림 경계에 걸리면
  플랫폼 간 차이가 날 수 있습니다. 양 플랫폼 golden 비교(VI.10.4, CI-WIN)에서
  확인하고 필요하면 결정론적 삼각함수로 교체합니다.
