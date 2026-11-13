# 작업 보고서: X.3.4 wrap과 multi-line cross alignment

- WBS: X.3.4
- 작업명: wrap과 multi-line cross alignment 구현
- 상태: 완료 (`[x]`)
- 선행: X.3.3(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/stack.rs` — `StackWrap`, `LineAlignment`, `StackLayout::{wrap, cross_gap, line_alignment}`, wrap canonical bytes(`encode_stack_wrap`/`decode_stack_wrap`), 줄 단위 배치(`build_lines`, `line_offsets`)로 measure·place 재구성, fixture 테스트.
- `crates/layout/src/input.rs` — Stack container bytes에 wrap 필드 연결.
- `docs/task-reports/X.3.4.md`(신규), 계획서(X.3.4 `[ ]`→`[x]`).

## 구현 요약

- **줄 나누기**: `Wrap`이고 content main이 definite면 visual order로 hypothetical main 크기(+ gap)를 누적하다
  content main을 넘기 직전에 줄을 바꿉니다(줄마다 최소 한 자식, 단독 넘침 허용). main이 indefinite(Hug)면 한 줄.
- **줄별 분배**: grow·shrink·basis를 줄마다 따로 분배합니다(X.3.3).
- **줄 cross 크기**: 그 줄 자식 cross 최댓값(cross Fill·Stretch 자식은 콘텐츠 크기로 기여). `NoWrap`은 한 줄이
  content cross 전체입니다.
- **align-content(`line_alignment`)**: Start/Center/End/SpaceBetween/SpaceAround/SpaceEvenly는 main 정렬과 같은
  분배 규칙(잔여 subpixel은 줄 순서, 넘침 시 fallback 동일), `Stretch`는 남는 cross 공간을 줄에 균등 분배합니다.
  줄 사이 간격은 `cross_gap`. 줄 cross 확정 뒤 cross Fill·Stretch 자식을 그 크기로 다시 해석합니다.
- 자식은 자기 줄 안에서 `cross_alignment`를 따르고, Reverse는 줄마다 main 축 거울상입니다(줄 순서 유지).
- canonical bytes: 기존 Stack bytes 뒤에 `wrap u8, cross_gap f32, line_alignment u8`(X.3.1 golden 유지).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 73 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| line break fixture: 너비 100·gap 10에서 [40,40] / [40,30] / [100](정확히 맞음) / [120](단독 넘침), cross gap 5 위치와 Hug 높이 63, main Hug면 한 줄(`line_break_fixture`) | 통과 |
| align-content fixture: 높이 90·남음 30에서 7종 정렬의 줄 위치(7.5 px 분배 포함), Stretch 줄 cross 35/45와 cross Fill 자식 높이, 줄 안 Center 정렬(`align_content_fixture`) | 통과 |
| 줄별 grow 분배(50/50, 100)와 wrap + Reverse 줄별 거울상(`wrapped_lines_flex_independently_and_reverse`) | 통과 |
| wrap 필드 bytes golden `[1, 00 00 a0 40, 6]`, line alignment tag, round-trip, 음수 cross gap 거부(`wrap_fields_bytes_golden_and_validation`) | 통과 |
| 기존 X.3.1~X.3.3 fixture 회귀 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `wrap-reverse`(줄 순서 반전)는 지원하지 않습니다.
- 줄 바꿈 판단은 hypothetical main(base를 min 0으로 clamp)으로 하며, min/max 연결은 X.6.2에서 이 값에 반영됩니다.
