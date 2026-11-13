# 작업 보고서: VI.7.4 빈 텍스트 node

- WBS: VI.7.4
- 작업명: 빈 텍스트도 유효 node로 유지하고 삭제와 구분
- 상태: 완료 (`[x]`)
- 선행: VI.7.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/text.rs` — `TextPayload::empty`, `TextPayload::is_empty`.
- `docs/task-reports/VI.7.4.md`(신규), 계획서(VI.7.4 `[ ]`→`[x]`).

## 구현 요약

- 빈 내용은 **유효한 Text node**입니다. 내용을 모두 지워도 node는 남고, 삭제는 node
  삭제 command로만 일어납니다(`is_empty`는 내용만 판단, node 존재와 무관).
- 빈 텍스트도 paragraph 하나(커서 위치)와 그 style을 유지합니다. `TextPayload::new`도
  빈 내용을 거부하지 않으며, 빈 내용에 대한 범위 밖 run은 여전히 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 119 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| empty content 저장·복원 + paragraph style 유지 + `new` 경로 동치(`empty_text_is_a_valid_node_and_round_trips`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 빈 텍스트의 hit test 영역(커서 높이)은 layout(Phase X)이 정합니다.
