# 작업 보고서: VI.5.4 Line

- WBS: VI.5.4
- 작업명: Line start/end point와 cap style, zero-length line을 점 cap으로 명시
- 상태: 완료 (`[x]`)
- 선행: VI.5.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/shape.rs` — `LineCap`(Butt/Round/Square, tag 0~2),
  `LineParams`(`is_zero_length`, `stroked_bounds`, `to_path`).
- `docs/task-reports/VI.5.4.md`(신규), 계획서(VI.5.4 `[ ]`→`[x]`).

## 구현 요약

- **파라미터**: start, end(finite `Vec2`), cap. 선 두께는 Stroke modifier(VI.9)의
  속성이므로 bounds 계산은 두께를 인자로 받습니다.
- **cap별 bounds**: Butt = 선분 양쪽 반폭 사각형, Square = 진행 방향으로 반폭 연장한
  사각형, Round = 끝점 bbox를 반폭만큼 확장(반원 끝).
- **zero-length 정책**: 방향이 없으므로 점 cap — Round는 지름 = 두께인 원, Square는
  한 변 = 두께인 축 정렬 정사각형, Butt는 면적이 없어 그리지 않음(`None`).
- **path**: 중심선 MoveTo/LineTo(Stroke 확장은 modifier 단계).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 92 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| cap별 bounds fixture(수평·대각 × 3 cap)(`cap_bounds_fixtures`) | 통과 |
| zero-length 점 cap 정책(`zero_length_line_uses_point_cap_policy`) | 통과 |
| 중심선 path | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- miter/join은 polyline이 아닌 단일 선분이라 해당 없음. Stroke modifier(VI.9)의
  stroke alignment(inside/center/outside)가 정해지면 bounds 계산에 반영합니다.
