# 작업 보고서: X.3.6 baseline 정렬과 bottom edge fallback

- WBS: X.3.6
- 작업명: baseline alignment hook을 추가하고 Text baseline이 없는 node는 bottom edge fallback을 사용
- 상태: 완료 (상태 표기는 통합 때 ledger가 정합니다)
- 결정: ADR-0076 결정 6(X.10.4·X.3.6 항목)
- 선행: X.3.2(`[x]`), X.10.4(이 lane 커밋 `55a2edb1`, layout 쪽 hook·전파 완료. `stellar-text` 구현체 연결은 X.10.4
  보고서의 남은 항목)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 기대값 생성만 로컬 Google Chrome 157.0.8103.79(headless)를 썼습니다.

## 변경 파일 목록

- `crates/layout/src/flex.rs` — `align-items`·`align-self: baseline`(row container), line 공유 baseline·line cross 크기,
  container 첫 baseline, column container에서는 시작 쪽.
- `crates/layout/src/input.rs` — flex의 baseline 정렬 수락(grid item baseline은 계속 `Unsupported`).
- `crates/layout/tests/css/{definitions.py,expected.json}` — baseline fixture 10개(`flex/baseline-*`).
- `docs/task-reports/X.3.6.md`(신규).

## 구현 요약

- row container의 line마다 baseline 정렬 item의 첫 baseline(없으면 border box 아래 변 = bottom edge fallback)을 공유
  baseline에 맞춥니다. line cross 크기는 다른 item의 바깥 cross 크기와 최대 ascent + 최대 descent 중 큰 값입니다
  (CSS Flexbox §9.4 8, §9.6).
- container 첫 baseline은 첫 줄에 baseline 정렬 item이 있으면 공유 baseline, 없으면 첫 item의 baseline(없으면 아래 변)입니다.
  중첩 container(block·flex·grid item)의 baseline이 바깥 정렬로 이어집니다.
- column container의 cross 축(가로)에는 가로 baseline이 없어 `baseline`은 시작 쪽입니다(Chromium 157 fixture로 확인).
- Text baseline은 측정 hook(X.10.4)에서, 고정 intrinsic 콘텐츠 baseline은 입력에서 옵니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `python3 crates/layout/tests/css/generate.py` | fixture 197개(flex 100, grid 56, positioning 33, block 8) 생성 |
| `cargo test -p stellar-layout` | 전부 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| mixed text/image fixture(`flex/baseline-mixed-text-image`: baseline 없는 이미지·box는 아래 변) | Chromium과 일치 |
| Stack baseline fixture(`flex/baseline-text-items`, X.10.4와 공유) | 일치 |
| 일부 item만 baseline(`flex/baseline-align-self-subset`), item padding, 중첩 flex·block·grid item, wrap 두 줄, container baseline의 바깥 전파, column 시작 쪽 | 10개 모두 일치 |
| 기존 fixture 187개 | 계속 일치 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `last baseline`, `wrap-reverse` container의 baseline 정렬(마지막 baseline 의미), grid item baseline은 받지 않거나
  첫 baseline으로만 다룹니다(grid는 `Unsupported`).
