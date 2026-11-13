# 작업 보고서: VI.6.5 Shape→Path 파괴 변환 계획

- WBS: VI.6.5
- 작업명: Shape→Path 파괴 변환 계획 — 사용자 명시 command에서만 node kind 변경, StableId 유지
- 상태: 완료 (`[x]`)
- 선행: VI.5.6, VI.6.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/path.rs` — `ShapeToPathPlan`, `plan_shape_to_path`.
- `docs/task-reports/VI.6.5.md`(신규), 계획서(VI.6.5 `[ ]`→`[x]`).

## 구현 요약

- **순수 계획**: `plan_shape_to_path(stable_id, geometry, next_id)`는 문서를 바꾸지
  않고 계획만 반환합니다. 적용은 사용자 명시 command `ConvertShapeToPath`(VII.2.7)만
  수행합니다 — 렌더·hit test는 계속 VI.5.6 순수 평가를 쓰며 node kind를 바꾸지
  않습니다.
- **동일성**: 계획의 path는 VI.5.6 평가 결과 그대로라 변환 전후 렌더 geometry가 같고,
  StableId는 유지됩니다. 새 point·handle에는 VI.6.3 id를 부여합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 108 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 변환 전 렌더 geometry 동일(command·bounds 일치, Rectangle/Ellipse/Line) + StableId 유지(`conversion_keeps_id_and_render_geometry`) | 통과 |
| 계획 생성이 원본을 바꾸지 않음(`planning_is_pure`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Line은 중심선 path로 변환되므로 stroke modifier(VI.9)를 함께 옮겨야 시각이 같습니다.
  command 구현(VII.2.7)에서 modifier stack을 그대로 승계합니다.
