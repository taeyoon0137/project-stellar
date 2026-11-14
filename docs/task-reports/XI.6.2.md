# 작업 보고서: XI.6.2 stroke tessellation(width, join, cap, dash)

- WBS: XI.6.2
- 작업명: stroke tessellation에 폭·join·cap·dash를 적용하고 fill과 같은 tessellation cache를 공유
- 상태: 완료 (`[x]`)
- 선행: XI.6.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/stroke.rs`(신규) — `StrokeStyle`(`StrokeWidth`, `StrokeJoin`, `MiterLimit`,
  `DashPattern`, 허용 오차), `StrokeTessellator` trait, `ReferenceStrokeTessellator`, `tessellate_stroke`,
  내부 평탄화·dash·join·cap·점 cap 기하, 상한 상수.
- `crates/renderer-core/src/stroke/tests.rs`(신규) — reference bounds fixture와 속성 테스트.
- `crates/renderer-core/src/tessellation.rs` — `TessellationStyle::Stroke`, `TessellationKey::stroke`,
  `TessellationCache::stroke`, `FillMesh::bounds`, `TessellationError::{InvalidStroke, TooComplex}`.
- `crates/renderer-core/src/lib.rs` — `stroke` 모듈 등록.
- `docs/task-reports/XI.6.2.md`(신규), 계획서(XI.6.2 `[ ]`→`[x]`).

## 구현 요약

- **style**: 폭(유한·비음수, 0은 빈 mesh), join(Miter 기본·Round·Bevel), miter limit(≥ 1, SVG 기본 4),
  cap(scene `StrokeCap` 재사용), dash(간격 최대 16개 고정 배열·`Copy`, 홀수 개는 두 번 반복, offset 음수
  허용), 평탄화 허용 오차. `StrokeStyle::canonical`이 mesh가 같은 style(폭 0, Miter가 아닌 join의 limit,
  간격 합 0인 dash)을 하나의 key로 모읍니다. `StrokeStyle::from_layer`는 scene `StrokeLayer`의
  `Inside`/`Outside`를 폭 2배 중심 stroke로 바꿉니다(clip은 pass 계획 소관).
- **기하**: path를 subpath polyline으로 평탄화한 뒤 segment마다 사각형, 꺾인 점마다 바깥 쐐기 join,
  열린 끝에 cap을 붙입니다. Miter는 miter 비율 `2/|a+c| = 1/sin(θ/2)`가 limit을 넘거나 180° 되돌아가면
  Bevel로 대체합니다. 닫힌 subpath는 cap 없이 시작점에도 join을 둡니다. 길이 0 subpath는 document Line 점
  cap 규칙(VI.5.4)을 따릅니다.
- **곡선 오차**: cubic은 2차 차분 상한식으로 균일 분할하고, stroke offset 오차(`반폭 · sin(반각)`, 양 끝은
  정확한 접선과 chord 사이 각)가 허용 오차를 넘으면 분할을 최대 4회 늘립니다(Jordan 부등식으로 보수적 배수).
- **dash**: offset으로 pattern 시작 상태를 정하고 polyline을 따라 on 구간을 열린 dash로 자릅니다. 닫힌
  subpath의 시작·끝이 모두 on이면 두 dash를 이어 시작점에도 join을 둡니다. 길이 0 on 간격은 진행 방향 점
  cap이며, subpath 끝점에서 시작하는 dash는 만들지 않습니다.
- **결정론·무 panic**: 원호(round join·cap·점)는 단위 벡터 이등분(정규화한 합)으로 나눠 삼각함수 없이
  사칙연산·`sqrt`·`floor`·`ceil`만 씁니다. 평탄화 점 2^21, 정점 2^22, dash 2^16, dash 진행 2^22, 원호 깊이 10,
  cubic 1024 분할 상한을 넘으면 `TooComplex`이고, f32 범위를 넘는 정점은 `Failed`입니다. 재귀가 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-renderer-core` | 종료 코드 0 |
| `cargo test -p stellar-renderer-core` | 108 unit passed(신규 13) + 3 doctest passed, 0 failed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 수평선 Butt/Square/Round cap bounds `[0,-1,10,1]`/`[-1,-1,11,1]`/`[-1,-1,11,1]`(`caps_reference_bounds`) | 통과 |
| 내각 60° V: Miter 끝 x=12, Round 11, Bevel 10.5, limit 1.5→Bevel 10.5, limit 2→Miter 12, 양 끝 Butt 모서리(`joins_reference_bounds`) | 통과 |
| 닫힌 정사각형 모든 join×cap bounds `[-1,-1,11,11]`, cap 무영향, miter 시작 모서리 꼭짓점, 삼각형 16(`closed_square_has_joins_but_no_caps`) | 통과 |
| 반지름 10 사분원 cubic: 허용 오차 0.25/0.05/0.01에서 bounds·최대 반지름이 허용 오차 이내, 폭 20도 이내, 분할 증가(`cubic_quarter_circle_bounds_within_tolerance`) | 통과 |
| dash `[3,2]`·offset 1·`[3,7]`·offset −2·Square·홀수 `[2]` bounds와 삼각형 수, 합 0 dash = 실선(`dash_reference_bounds`) | 통과 |
| 길이 0 dash 점 cap(Round 점 2개, Butt 없음, 대각선 Square 회전)(`zero_length_dash_draws_dots_along_direction`) | 통과 |
| 닫힌 dash seam 병합: dash 4개, 시작 모서리 miter 꼭짓점, 삼각형 24, 전체 on = 실선(`closed_dash_merges_seam_into_join`) | 통과 |
| 길이 0 subpath·`M p Z` 점 cap, MoveTo 단독·빈 path·폭 0은 빈 mesh(`zero_length_subpaths_follow_line_dot_policy`) | 통과 |
| 180° 되돌아감 Miter→Bevel, Round 반원(`reversal_joins_are_bounded`) | 통과 |
| style 검증 오류, tag round-trip, 아주 짧은 dash→`TooComplex`, f32 범위 초과→`Failed`(`style_validation_and_limits`) | 통과 |
| `StrokeLayer` 정렬·cap 변환(`from_layer_maps_alignment_and_cap`) | 통과 |
| 같은 path/style hit·같은 `Arc`, fill/stroke key 분리, 값별 miss, 정규화 key 동일(`stroke_cache_hits_and_key_separation`) | 통과 |
| 무작위 300 path(cubic·중복점·닫힘·dash): 결정론, 모든 정점이 control bounds + `outset` 안(`random_paths_stay_within_outset_bounds_and_are_deterministic`) | 통과 |

## 차이(작업 지시와 다른 점)

- scene `StrokeLayer`에는 join·dash 필드가 없어(scene은 이 작업의 수정 범위 밖) renderer 쪽
  `StrokeStyle`에 정의했습니다. `from_layer`는 join·dash를 기본값으로 둡니다.
- fill tessellation 알고리즘(삼각분할)은 이 작업 범위가 아니어서 구현하지 않았습니다.

## 남은 위험 / 한계

- mesh 삼각형은 겹칠 수 있어 renderer가 stroke coverage를 합집합(stencil/coverage mask)으로 그려야
  합니다(Metal backend XI.8.x에서 처리).
- scene `StrokeLayer::outset`은 miter 확장을 반영하지 않습니다. culling·hit test bounds는
  `StrokeStyle::outset` 또는 mesh bounds를 써야 합니다(scene 연결 작업에서 반영 필요).
- cusp처럼 분할해도 꺾임이 줄지 않는 곡선은 1024 분할 상한에서 허용 오차를 넘을 수 있습니다.
- 계산은 IEEE-754 기본 연산과 `sqrt`만 사용해 ADR-0006 판정 규칙상 플랫폼 간 동일 구성이지만, Windows
  재개 시 golden bounds를 한 번 더 확인하는 것이 좋습니다.
