# 작업 보고서: XI.3.4 node 종류별 narrow-phase hit test

- WBS: XI.3.4
- 작업명: Shape/Path/Text/Image별 narrow-phase hit test, alpha pixel 판정 제외, bounds·geometry 정책 명시
- 상태: 완료 (`[x]`)
- 선행: XI.3.3 (`[x]`, `033f4a1`), VI.5.6 (`[x]`), VI.6.4 (`[x]`), X.10.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/narrow_phase.rs`(신규) — `hit_test`, `hit_test_view_narrow`, `content_contains`,
  `clip_contains_precise`, `clip_stack_contains_precise`, `rounded_rect_distance`, `NarrowPhaseOptions`,
  `TextHitGeometry`, `LayoutBoxesOnly`, `FLATTEN_TOLERANCE`, `MAX_CUBIC_SEGMENTS`, 모듈 문서 정책 표.
- `crates/scene/src/narrow_phase/tests.rs`(신규) — node 종류별 fixture 테스트 11개.
- `crates/scene/src/hit_test.rs` — broad-phase 본체를 허용 거리를 받는 `broad_phase`로 분리(`hit_test_broad`는
  허용 거리 0으로 동작 동일), `HitStats`에 `path_clipped`·`content_missed` 추가, `to_local`·`linear_scale`
  crate 내부 공유.
- `crates/scene/src/lib.rs` — `narrow_phase` 모듈 등록.
- `docs/task-reports/XI.3.4.md`(신규), 계획서(XI.3.4 `[ ]`→`[x]`).

## 구현 요약

- **파이프라인**(`hit_test`): broad-phase(XI.3.3) 후보마다 ① clip stack에 path clip이 있으면 clip path 내부를
  fill rule로 정밀 판정(broad-phase는 control 껍질), ② content geometry 판정을 적용합니다. topmost 우선 순서는
  유지되고, 뺀 수는 `path_clipped`·`content_missed`로 셉니다. view 좌표 함수는 허용 거리를 view 단위로 받아
  viewport 축척으로 world 단위로 바꿉니다.
- **종류별 정책**(alpha pixel 판정 없음):

  | content | 원본 node | 정책 | 맞는 영역 |
  |---|---|---|---|
  | `Rect` | Frame | geometry | 실효 radius 둥근 사각형 전체(칠 유무 무관) + stroke가 상자 밖으로 나가는 띠(Center 폭/2, Outside 폭, Inside 0) |
  | `Path` | Shape(VI.5.6 평가 결과)·Path | geometry | fill layer가 있으면 fill rule 내부, stroke마다 정렬별 띠(Center 폭/2, Inside 내부∩폭, Outside 외부∩폭), paint layer가 없으면 내부 |
  | `Text` | Text | bounds | `TextHitGeometry`가 주는 line rect(X.10.3 selection rect)의 합집합, 없거나 비면 layout 상자 |
  | `Image` | Image | bounds | 배치 상자 전체(fit·crop·투명 픽셀 무관) |

- **geometry 세부**: cubic은 world 허용 오차 0.05를 node 평균 축척(`√|det|`)으로 나눈 local 허용 오차로 Wang 공식
  선분화하고 cubic당 256 선분으로 제한합니다(신뢰할 수 없는 큰 좌표에서도 command 수에 선형). fill은 열린
  subpath를 암묵적으로 닫고 winding number로 NonZero/EvenOdd를 판정합니다. 열린 subpath 끝은 StrokeCap(Butt 자름,
  Round 반원, Square 반폭 연장)을 따르고, join은 정의 전(XI.6.2)이라 round join으로 봅니다. 계산은 f64입니다.
- **허용 거리**(`NarrowPhaseOptions::with_tolerance`, finite·비음수, 기본 0): broad-phase 질의 영역·node bounds와
  모든 content 영역을 넓힙니다(가는 선 선택용). clip은 넓히지 않습니다.
- scene은 text crate에 의존하지 않으므로(II.2.8) line rect는 호출자가 `TextHitGeometry`로 넘깁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-scene` | 완료 |
| `cargo test -p stellar-scene` | unit 90, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |
| `cargo test -p stellar-renderer-core -p stellar-export` | export unit 55·doc 0, renderer-core unit 95·doc 3 passed, 0 failed |

| 검사(검증 기준: node kind별 fixture) | 결과 |
|---|---|
| Shape: Ellipse 평가 path(4 cubic)가 곡선 안만 맞고 bounds 모서리는 broad 후보였다가 narrow에서 제외, 해석적 타원 식 oracle과 격자 9,000점 이상 일치(`shape_ellipse_hits_curve_interior_not_bounding_box_corner`) | 통과 |
| Shape: stroke만 있는 원은 띠(반폭 2)만 맞고 내부는 아래 Frame으로 통과(`stroke_only_shape_hits_band_and_passes_interior_to_lower_node`) | 통과 |
| Path: 열린 선 Butt/Round/Square cap 경계, 닫힌 사각형 round join(`open_path_caps_follow_stroke_cap`) | 통과 |
| Path: EvenOdd 구멍·NonZero 같은 방향 채움·역방향 구멍, stroke Center/Inside/Outside 띠, paint 없는 path는 내부(`path_fill_rules_and_stroke_alignment`) | 통과 |
| Frame: 칠 없는 상자 전체, 둥근 모서리 밖 제외, Inside 테두리는 넓히지 않고 Outside·Center는 넓힘, 모서리 거리 수치(`frame_uses_rounded_box_and_outset_strokes`) | 통과 |
| Text: layout 상자 정책, line rect 정책(짧은 둘째 줄 오른쪽 통과), 빈 line rect는 layout 상자로 복귀(`text_uses_line_rects_or_layout_box`) | 통과 |
| Image: fit 3종 모두 배치 상자 전체, Inside stroke는 넓히지 않음(`image_uses_placement_box_without_alpha`) | 통과 |
| path clip 정밀 판정: control 껍질 안·삼각형 밖 점은 broad 후보였던 자식이 `path_clipped`로 빠지고 부모가 topmost(`path_clip_is_tested_precisely`) | 통과 |
| 허용 거리: 2배 확대 node의 면적 0 선이 world 2 이내만 맞음, view 4배 확대에서 view 8 = world 2, 음수·NaN 거부(`tolerance_widens_hairlines_in_local_and_view_units`) | 통과 |
| 무작위 fixture 400점: narrow 결과가 broad 결과의 순서 유지 부분 집합이고 뺀 수 = 통계(`narrow_hits_are_ordered_subset_of_broad_on_random_fixture`) | 통과 |
| ±1e30 cubic도 선분 256개 상한, 허용 오차 0·NaN·∞도 상한, 축척 0 node 미적중, MoveTo만/길이 0 선분은 Round cap만 점(`huge_or_degenerate_geometry_stays_bounded`) | 통과 |

- **변이 확인**: Butt cap을 Round처럼, EvenOdd를 NonZero처럼, path clip 정밀 판정 생략, Frame Outside outset 절반,
  cubic 선분화 1개로 바꾼 5개 변이가 각각 해당 테스트를 실패시켰고, 원본으로 복원했습니다.

## 차이(작업 지시와 다른 점)

- 작업 지시의 Shape/Path/Text/Image 외에 Frame(`Rect` content)과 path clip 정밀 판정도 함께 정했습니다. Shape는
  VI.5.6 평가로 Path geometry가 되어 들어오므로 Path와 같은 geometry 정책을 씁니다(Shape만의 별도 분기 없음).
- Text glyph 윤곽 판정은 하지 않습니다(bounds 정책). X.10.3 selection rect는 text layout cache가 scene 밖에 있어
  `TextHitGeometry` 주입으로 받습니다.

## 남은 위험 / 한계

- stroke join(XI.6.2)이 정해지면 miter join 끝 판정을 맞춰야 합니다. 현재 round join이라 뾰족한 miter 끝은 맞지
  않을 수 있습니다. dash도 반영하지 않습니다(전체 선으로 판정).
- 허용 거리와 선분화 허용 오차의 local 변환은 평균 축척 근사라 비균일 축척 node에서는 방향에 따라 다릅니다.
- 경계 위 점은 부동소수 오차 범위에서 어느 쪽이든 될 수 있습니다(oracle은 경계 0.1 이내를 비교에서 제외).
- `TextHitGeometry`를 실제 text layout cache(X.10.5)에 연결하는 것은 runtime 통합 작업 몫입니다.
- 지연(p99) 측정은 XI.3.6 benchmark 범위입니다.
