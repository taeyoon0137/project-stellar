# 작업 보고서: X.6.4 content intrinsic measurement trait

- WBS: X.6.4
- 작업명: content intrinsic measurement trait를 Shape, Text, Image, ComponentInstance별로 정의
- 상태: 완료 (`[x]`)
- 선행: Phase VI(phase 단위 — 완화, 아래 차이 참고), IX.2.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/intrinsic.rs`(신규) — `ContentKind`, `IntrinsicMode`, `MeasureRequest`, `IntrinsicSize`,
  `MeasureError`, `IntrinsicMeasure` trait, 외부 hook `ContentMeasurer`(`Sync`, `revision`)와 `DefaultMeasurer`,
  종류별 기본 intrinsic `ShapeIntrinsic`·`TextIntrinsic`·`ImageIntrinsic`·`ComponentInstanceIntrinsic`,
  `MeasuredContent`, 검증·canonical bytes, 테스트.
- `crates/layout/src/input.rs` — `LayoutContent::Measured(MeasuredContent)`(content tag 2), `LayoutContent::is_leaf`,
  검증·인코딩. `LayoutContent`는 텍스트 단어 목록 때문에 `Copy`가 아닙니다.
- `crates/layout/src/engine.rs` — `compute_layout_with(input, measurer)`(`compute_layout`은 `DefaultMeasurer`),
  `PreparedContent`, 측정 콘텐츠 측정·memo(`measure_content`), baseline 계산 통합(`border_baseline`).
- `crates/layout/src/constraint.rs` — sibling baseline을 `border_baseline`으로 읽음(측정 콘텐츠 baseline 포함).
- `crates/layout/src/error.rs` — `LayoutError::ContentMeasure { node, kind, reason }`.
- `crates/layout/src/lib.rs` — `intrinsic` 모듈.
- `docs/task-reports/X.6.4.md`(신규), 계획서(X.6.4 `[ ]`→`[x]`).

## 구현 요약

- **trait**: `IntrinsicMeasure::measure(&MeasureRequest) -> IntrinsicSize`. 요청은 node, 종류, 축별 가용 content 크기
  (definite 축만 `Some`), indefinite 축 기준(`MinContent`/`MaxContent`), ComponentInstance 자식 extent입니다. 결과는
  content 크기와 첫 baseline입니다.
- **종류별 기본 intrinsic**(입력에 담긴 결정적 값으로 측정):
  - Shape: geometry bounds 그대로(가용 크기·mode 무관, baseline 없음).
  - Text: 단어 폭 목록을 가용 너비에서 greedy 줄바꿈(단어를 쪼개지 않음). 가용 너비가 없으면 max-content는 한 줄,
    min-content는 가장 넓은 단어 폭에서 줄바꿈. 높이 = 줄 수 × line height(빈 텍스트도 한 줄), baseline = ascent.
  - Image: natural 크기. 한 축만 definite면 natural 비율로 다른 축(한 번 반올림). natural에 0이 있으면 그대로.
  - ComponentInstance: 자식(instance virtual subtree)을 node의 container 배치로 측정한 extent. 자식을 가질 수 있는
    유일한 종류입니다.
- **외부 hook**: `ContentMeasurer`를 `compute_layout_with`에 넘기면 측정 콘텐츠를 그것으로 측정합니다. 처리하지 않는
  종류는 `MeasuredContent::measure`(기본 구현)에 위임합니다. 깊은 tree는 전용 stack thread에서 계산하므로 `Sync`입니다.
  측정 실패·음수 결과는 `LayoutError::ContentMeasure`입니다. `revision()`은 측정 규칙 변경을 cache key에 넣기 위한
  값입니다(X.7.2).
- **엔진**: Hug 등 콘텐츠 의존 축의 content extent를 측정기로 구하고, 같은 (node, 가용 크기, mode) 요청은 한 계산에서
  한 번만 측정기에 보냅니다. baseline은 content 너비가 정해진 측정의 baseline + 위쪽 padding입니다. 기존
  `LayoutContent::Intrinsic`(고정 값)은 그대로입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 132 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| Shape 기본: Hug 30×20 + padding 2 → 34×24, baseline 없음, 너비 고정이어도 높이 bounds(`shape_default_intrinsic_is_geometry_bounds`) | 통과 |
| Text 기본: max-content 55×12, baseline = padding + ascent, 너비 30 → 세 줄, 40 → 두 줄, 5 → 단어 단위 넘침, 빈 텍스트 0×12, min-content 20×36(`text_default_intrinsic_wraps_words_greedily`) | 통과 |
| Image 기본: natural 64×48, 너비 32 → 높이 24, 높이 24 → 너비 32, 3:1 비율 한 번 반올림(2133 subpixel), natural 0 포함 시 비율 없음(`image_default_intrinsic_keeps_natural_ratio`) | 통과 |
| ComponentInstance 기본: Free·Stack subtree extent, 빈 instance 0×0, Shape·Text·Image leaf 자식은 `ChildrenOnContentLeaf`(`component_instance_default_intrinsic_is_subtree_extent`) | 통과 |
| 외부 측정기가 Text만 대체·나머지 위임, 측정 실패·음수 결과 `ContentMeasure`(`external_measurer_replaces_defaults_and_errors_are_typed`) | 통과 |
| 네 종류 canonical bytes round-trip·재인코딩 동일, kind tag golden, 알 수 없는 tag·음수 단어 폭 거부(`measured_content_round_trips_and_validates`) | 통과 |
| 기존 fixture·X.11.1 corpus golden | 통과(변경 없음) |

## 차이(작업 지시와 다른 점)

- 선행 "Phase VI"는 phase 단위 선행이라 작업 지시에 따라 완화했습니다. `layout`은 `core-types`만 의존하므로 문서 node
  종류를 직접 읽지 않고, 같은 네 종류를 `ContentKind`로 정의했습니다. 문서 node → 레이아웃 입력 변환(X.1.2)은 선행
  IX.7.6 미완료로 이 lane에서 다루지 않았습니다.
- Text 기본 intrinsic은 실제 shaping이 아니라 입력에 담긴 단어 폭 기반 결정적 모델입니다. 실제 텍스트 엔진 연결은
  X.10.4가 `ContentMeasurer`로 합니다.
- 이 작업에서 엔진은 indefinite 축을 항상 `MaxContent`로 측정합니다. `MinContent` 측정 경로(Grid MinContent track)는
  X.4.5가 연결합니다.

## 남은 위험 / 한계

- baseline은 측정 콘텐츠·고정 intrinsic leaf에서만 나옵니다. container baseline 규칙과 Stack baseline 정렬은 X.3.6
  (선행 X.10.4 미완료) 소관입니다.
- 측정기 결과의 결정성은 구현자 책임입니다. 엔진은 같은 요청을 한 계산에서 한 번만 보냅니다.
