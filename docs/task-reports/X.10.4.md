# 작업 보고서: X.10.4 Text intrinsic 크기·baseline의 layout intrinsic hook 연결

- WBS: X.10.4
- 작업명: Text intrinsic width/height와 baseline을 layout intrinsic hook에 연결
- 상태: 부분 완료 (`[~]`) — layout 쪽 hook·전파·검증은 끝났고, `stellar-text` 구현체 연결은 crate graph 결정이 남았습니다
  (아래 "남은 위험"). 상태 표기는 통합 때 ledger가 정합니다.
- 결정: ADR-0076 결정 6(X.10.4·X.3.6 항목)
- 선행: X.10.2(`[x]`), X.6.4(`[x]`). 이전 차단 보고서(`X.10.4-blocked.md`)의 첫 원인(X.6.4 미완료)은 해소되었습니다.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/intrinsic.rs` — `TextLayoutHook`(min-content 너비, 너비 제약 줄 배치 = 너비·높이·첫 baseline,
  revision), `TextLineLayout`, `TextHookMeasurer`(hook → `ContentMeasurer`), 모듈 문서.
- `crates/layout/src/intrinsic/text_hook_tests.rs`(신규) — hook 연결·container baseline 전파·revision cache 테스트.
- `docs/task-reports/X.10.4.md`(신규).
- container 첫 baseline 규칙(block·flex·grid)은 IX.11.4·IX.11.5 커밋의 `block.rs`·`flex.rs`·`grid.rs`에 있습니다.

## 구현 요약

- **hook 모양**: 텍스트 엔진이 이미 내는 값(X.10.2 `TextLayout::width`·`height`·`first_baseline`, 1/64 px `TextUnit` =
  `LayoutUnit`)을 그대로 받습니다. `min_content_width(node, text)`, `layout(node, text, width)`(너비 없음 = 줄바꿈 없음),
  `revision()`입니다.
- **측정 연결**: `TextHookMeasurer`는 가용 너비가 있으면 그 너비로, max-content는 너비 없이, min-content는
  `min_content_width`로 줄 배치합니다. 엔진은 Text leaf의 `auto` 너비를 `fit-content`(min·max-content 사이), 높이를 정한
  너비에서의 줄 배치 높이, baseline을 첫 줄 baseline + 위쪽 padding으로 씁니다. 다른 콘텐츠 종류는 기본 구현에
  위임합니다.
- **baseline 전파**: block은 baseline을 가진 첫 in-flow 자식, flex는 첫 item(없으면 border box 아래 변 합성), grid는 첫
  행 첫 item에서 container 첫 baseline을 정합니다. 결과 `LayoutBox::baseline`에 실립니다.
- **cache**: hook revision이 측정기 revision이라 폰트 로드 등으로 revision이 바뀌면 계산 간 cache를 쓰지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-layout --lib text_hook` | 3 passed |
| `cargo test -p stellar-layout` | unit 104 포함 전부 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| hook 너비·높이·baseline 사용(fit-content 53×23, baseline 18, 너비 30에서 세 줄, `min-content` keyword) | 통과 |
| container 첫 baseline: flex row 첫 item text 15, 첫 item 무 baseline이면 아래 변 30, column flex 20, block 45, grid 15, 중첩 block→flex→text 15 | 통과 |
| hook revision 같으면 hook 호출 0, 바뀌면 다시 측정 | 통과 |
| Stack baseline fixture(Chromium 기대값, baseline 정렬) | X.3.6 커밋의 `flex/baseline-*` fixture로 확인(공유 fixture) |

## 차이(작업 지시와 다른 점)

- `stellar-text`의 `TextLayout`으로 `TextLayoutHook`을 구현하는 adapter는 넣지 않았습니다(아래).

## 남은 위험 / 한계

- `tools/xtask/src/crate_graph.rs` 정책상 `stellar-layout`·`stellar-text`를 함께 의존할 수 있는 crate가 없습니다. hook
  구현체를 둘 crate(예: runtime이 layout에도 의존하거나 새 상위 crate)를 정하는 crate graph 변경은 이 lane 소유 경로
  밖입니다. 결정이 나면 그 crate에서 `TextLayoutHook`을 `TextLayout`으로 구현하면 됩니다(값 단위가 같아 변환 없음).
