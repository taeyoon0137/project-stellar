# 작업 보고서: XXX.1.5 그래프 node 희소 배치 hint

- WBS: XXX.1.5
- 작업명: 사용자가 고정한 node 위치만 StableId→좌표로 저장하고 Semantic Diff에서 표현 전용 변경으로 분류.
- 상태: 부분 완료 (`[~]`) — hint 모음·형식·표현 전용 분류·선택 적용은 끝났습니다. semantic-diff crate 변경 목록과 Git stage 경로 연결, 프로젝트 열기·저장 연결이 남았습니다.
- 결정: ADR-0094 결정 5
- 선행: XXX.1.3, XIV.3.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/prototype/{layout_hint,source}.rs`(신규), `source_golden.txt`(신규).
- `crates/document/src/prototype/mod.rs`: module 등록.

## 구현 요약

- `LayoutHints`는 StableId 순 희소 map이고 빈 모음은 binary·텍스트 모두 0 bytes입니다.
- `diff_hints`는 모든 hint 변경을 `ChangeClass::Presentation`으로 분류하고, `apply_hint_changes`로 일부만 골라 적용(stage)할 수 있습니다.
- 프로토타입 텍스트 원본 v1(`PrototypeSource`)이 소유자의 MotionSpec과 hint를 CSS `transition-*` 줄로 씁니다. 정규형이 아닌 입력을 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 통과(lib 460개 포함 전체). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `graph_without_hints_saves_zero_bytes`, `hints_round_trip`, `hint_change_is_presentation_and_stages_separately`.
- `prototype::source::tests::{text_round_trip_is_byte_identical, golden_text, empty_owner_writes_no_file, non_canonical_text_is_rejected, condition_values_round_trip}`.

## 차이(작업 지시와 다른 점)

- semantic-diff crate는 이 작업의 소유 범위 밖이라 분류 API를 document에 두었습니다.

## 남은 위험 / 한계

- semantic-diff 변경 목록에 `Presentation` 분류를 싣고 Git stage가 따로 고르게 하는 연결이 필요합니다.
- `design/prototype/<owner>.stellar-prototype` 파일 열기·저장은 텍스트 원본 codec·journal 작업 몫입니다.
