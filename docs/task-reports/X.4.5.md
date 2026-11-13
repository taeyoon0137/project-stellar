# 작업 보고서: X.4.5 content-based track sizing의 intrinsic hook 연결

- WBS: X.4.5
- 작업명: content-based track sizing을 텍스트·이미지 intrinsic size hook으로 연결
- 상태: 완료 (`[x]`)
- 선행: X.4.2(`[x]`), X.7.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/grid_tracks.rs` — `TrackItem`의 min-content·max-content 두 기여, 경계별 기여 선택, span 여러 칸
  item 기여 선택, indefinite fr은 max-content 기여, 테스트.
- `crates/layout/src/grid.rs` — 열 기여를 `MaxContent`·`MinContent` 측정으로 구함(min-content는 `MinContent` 경계 열이
  있을 때만), 행 기여는 두 값 동일, 정책 문서, mixed content fixture.
- `crates/layout/src/engine.rs` — 측정 mode를 container 측정에서 자식 크기 해석으로 전달(`Engine::mode`, 크기 해석
  memo key에 mode 포함), `resolve_size_in`.
- `docs/task-reports/X.4.5.md`(신규), 계획서(X.4.5 `[ ]`→`[x]`).

## 구현 요약

- **두 기여**: Grid 열 item 기여는 intrinsic 측정 hook(X.6.4)으로 구한 min-content(텍스트: 가장 넓은 단어 폭에서
  줄바꿈, 이미지: natural)와 max-content(텍스트: 한 줄) 두 값입니다. `MinContent` 경계는 min-content, `MaxContent` 경계는
  max-content 기여를 씁니다. 따라서 `minmax(min-content, max-content)`는 min-content에서 시작해 가용 공간이 남으면
  max-content까지 늘어납니다(maximize).
- **span 여러 칸 item**: 대상 track 중 `MaxContent` 경계가 있으면 max-content, 없으면 min-content 기여로 부족분을
  분배합니다. indefinite fr 단위는 max-content 기여입니다.
- **mode 전달**: container item의 min-content는 자식 min-content의 합성입니다(예: 가로 Stack = 자식 min-content 합 +
  gap). container 측정 동안 엔진 mode를 그 측정의 mode로 두고 자식 크기 해석이 이를 따릅니다. 크기 해석 memo key에
  mode가 들어갑니다. 배치 단계는 항상 `MaxContent`입니다.
- **행 기여**: 열 너비가 정해진 높이는 mode와 무관하므로 두 기여가 같습니다. 텍스트 item을 열 너비로 줄바꿈하려면 item
  너비를 `Fill`로 둡니다(Hug item은 기존처럼 자기 콘텐츠 크기로 영역 시작에 놓임).
- 고정 intrinsic 콘텐츠는 두 기여가 같아 기존 fixture·X.11.1 corpus 결과는 바뀌지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 141 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| mixed content(너비 400): MinContent 열 = 가장 넓은 단어 50·세 줄 36, MaxContent 열 = 한 줄 110, minmax 열 40 → 85(한 줄), 고정 30 열 image 30×22.5, 행 높이 36. 너비 250 → minmax 60·두 줄, Hug 너비 → minmax 40, grid 230(`mixed_text_and_image_content_tracks`) | 통과 |
| MinContent 열의 Hug 가로 Stack(text 두 개, gap 4) min-content = 25 + 4 + 15 = 44(max-content 69 아님), 이미지 MinContent 열 = natural 20(`min_content_propagates_through_container_items_and_images`) | 통과 |
| track sizing 단위: MinContent 20·MaxContent 70·minmax 20(indefinite)/50(가용 50), span 2 item의 min·max 기여 선택, indefinite fr = max-content 기여(`min_and_max_content_contributions_feed_matching_breadths`) | 통과 |
| 기존 X.4.1~X.4.4 fixture·X.11.1 corpus golden | 통과(변경 없음) |

## 차이(작업 지시와 다른 점)

- 선행 X.7.2는 이 lane에서 먼저 완료했습니다. 실제 텍스트 엔진 결과는 X.10.4가 같은 hook(`ContentMeasurer`)으로
  연결합니다. 현재 텍스트 기여는 X.6.4 기본 intrinsic(단어 폭 모델)입니다.

## 남은 위험 / 한계

- 줄바꿈 Stack(`Wrap`)의 min-content는 indefinite main에서 한 줄로 측정합니다(CSS처럼 자식마다 줄을 바꾼 최댓값이
  아님).
- 중첩 Grid는 바깥 측정 mode와 무관하게 자기 track 종류로 item 기여를 정합니다(fr track의 min-content 제약 축소는 없음).
