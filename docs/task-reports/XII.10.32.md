# 작업 보고서: XII.10.32 Text 내용·style 범위 편집 command

- WBS: XII.10.32
- 작업명: text range 교체와 범위 style 설정 command, 입력 종류·caret grouping key
- 상태: 완료 (`[x]`) — 코어 command·history grouping까지. 캔버스 텍스트 편집 연결은 XII.10.33의 몫입니다.
- 선행: VI.7.5, VII.4.3, VII.1.6
- 결정: ADR-0079 결정 1

## 필수 실행 환경

- `DEV-MAC` (Rust 단위·property 테스트).

## 변경 파일 목록

- `crates/commands/src/text_edit.rs` — `SetTextRangeStyle` command(범위 run 자르기·적용·지우기, VI.7.2 정규화),
  모듈 문서, 삽입·삭제·교체·한글 확정·범위 style fixture.
- `crates/commands/src/{kind,command,codec}.rs` — `set-text-range-style`(0x0051, codec 2) 등록.
- `crates/commands/src/test_support.rs`, `crates/commands/src/property_tests.rs` — 생성기와 VII.6.1 왕복 등록.
- `crates/history/src/entry.rs` — `TextInputKind::Replace`.
- `crates/history/src/grouping.rs` — `GroupingKey::text_edit`(입력 종류와 caret 계산), `Replace`는 합치지 않는 규칙,
  연속 입력 merge fixture.
- `crates/history/src/journal.rs` — grouping 텍스트 입력 종류 tag 4(`Replace`).
- `docs/task-reports/XII.10.32.md`(신규).

## 구현 요약

- 범위 교체는 기존 `replace-text`(0x0050, VI.7.5 `edit_text` helper)를 그대로 씁니다. ADR-0079 결정 1의
  "ReplaceTextRange"가 이 종류이며 같은 의미의 종류를 새로 만들지 않았습니다.
- `SetTextRangeStyle { node, range, style }`은 비어 있지 않은 scalar 범위의 문자 style을 하나로 바꾸거나(`Some`)
  기본 style로 둡니다(`None`). 범위와 겹치는 run은 범위 밖 조각만 남기고, 결과를 `normalize_style_runs`로
  정규화합니다(인접 같은 style 병합). style 표가 있는 문서는 표에 없는 번호를 거부합니다.
- 두 command의 역은 원래 레코드를 되돌리는 `RestoreNodeRecords`입니다.
- `GroupingKey::text_edit(&ReplaceText, TextInputKind)`가 caret 전·후를 계산해 VII.4.3 grouping key를 만듭니다.
  `Insert`·`Replace`는 (범위 끝 → 시작+확정 길이), `DeleteBackward`는 (끝 → 시작), `DeleteForward`는 (시작 → 시작)
  입니다. `Replace`(선택 교체)는 앞뒤 입력과 합치지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands text_edit` | 3 passed: 삽입·삭제·교체(줄바꿈 paragraph 추가)·한글 확정 음절·NFD 자모 보존 fixture, 범위 style 적용·지우기·병합·거부 fixture, canonical bytes |
| `cargo test -p stellar-commands property_tests` | `set_text_range_style_round_trip`(100건 이상) 포함 전체 통과 |
| `cargo test -p stellar-history grouping` | `continuous_text_input_merges_into_one_history_entry`, `text_merge_conditions` 통과 |
| `cargo test -p stellar-commands -p stellar-history` | commands 147, history 50 통과 |
| `cargo clippy -p stellar-commands -p stellar-history --all-targets -- -D warnings` | 경고 없음 |

| 검사 | 결과 |
|---|---|
| 삽입·삭제·교체·범위 style apply/inverse fixture | 통과(apply 결과 확인 → inverse 뒤 semantic snapshot 원본과 같음) |
| 한글 조합 확정 fixture | 통과(확정 음절 "한"·"글"만 command, 두 입력이 history 항목 하나로 합쳐지고 undo 한 번에 되돌아감) |
| 연속 입력 merge fixture | 통과(입력 3회 → 1항목, Backspace 2회 → 1항목, 선택 교체는 분리, 간격 초과는 새 항목) |

## 차이(작업 지시와 다른 점)

- 별도 `ReplaceTextRange` 종류를 만들지 않고 기존 `replace-text`를 범위 교체 command로 확인했습니다(ADR-0079).
- 입력 종류에 `Replace`를 더했습니다. 선택을 다른 문자열로 바꾸는 입력을 앞 타이핑과 합치지 않기 위해서입니다.

## 남은 위험 / 한계

- 캔버스 IME 조합 표시와 caret 이동(XII.10.33)은 셸·runtime 작업입니다.
- style 표가 없는 구형 문서는 문자 style 번호를 검사하지 않습니다.
