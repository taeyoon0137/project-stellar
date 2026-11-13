# 작업 보고서: VII.2.3 MoveNode

- WBS: VII.2.3
- 작업명: `MoveNode` — parent와 OrderKey를 함께 변경, cycle·cross-view 검증
- 상태: 완료 (`[x]`)
- 선행: VII.2.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/node_ops.rs` — `MoveNode`(validate/invert/apply/codec), `decode_order_key`, 테스트 4건.
- `crates/commands/src/edit.rs` — composite primitive `replace_record`(레코드 전체 교체, 자손·stack 유지, journal 기록).
- `crates/commands/src/command.rs` — `Command::MoveNode`, `CommandError::Reparent`.
- `crates/commands/src/kind.rs`, `crates/commands/src/codec.rs` — `MoveNode = 0x0003` 등록·직렬화.
- `docs/task-reports/VII.2.3.md`(신규), 계획서(VII.2.3 `[ ]`→`[x]`).

## 구현 요약

- **입력**: node, 새 부모, 새 OrderKey(항상 함께). 같은 부모에 새 key면 재정렬입니다.
- **검증**: View root 이동 거부, document crate `validate_reparent`(VI.2.3)로 자기 부모·자손 부모(cycle)·다른 View tree
  이동 거부, 부모가 container인지, 새 형제(자기 제외)와 OrderKey 충돌, `새 부모 depth + 1 + subtree 높이 ≤ 64`.
- **apply**: `replace_record`로 부모·key만 바꾼 레코드로 교체합니다. `DocumentModel`이 부모·순서 제자리 변경을 공개하지
  않아 subtree를 자손부터 떼고 다시 넣으며(자손·modifier stack 유지), 모든 단계가 journal에 남아 실패 시 함께 되돌려집니다.
  같은 위치로의 이동은 변경 없음입니다.
- **inverse**: 원래 부모·key로의 `MoveNode`. change set은 node의 `PARENT`(부모가 바뀐 경우)·`ORDER_KEY` 속성과 이전·새 부모의
  구조 무효화입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 39 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 다른 부모로 이동→undo snapshot 동일, change set 속성·node 정확, 같은 부모 재정렬→undo, 자손·modifier stack 동반 이동(`move_and_reorder_round_trip`) | 통과 |
| 자기 부모·자손 부모·다른 View·View root·Text 부모·OrderKey 충돌·없는 node 거부, 문서 불변, 같은 위치 이동은 빈 change set(`invalid_moves_are_rejected`) | 통과 |
| subtree 높이 포함 깊이 65 거부, 64 허용(`move_depth_limit_counts_subtree_height`) | 통과 |
| random valid document 120 seed × 무작위 이동: 성공 시 문서 validator 통과, undo 후 레코드 동일, 성공 > 100(`random_moves_round_trip`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 부모·순서 변경이 subtree 재삽입이라 비용이 O(subtree × N)입니다. document crate에 공통 필드 제자리 setter가 생기면
  `replace_record`의 구조 경로를 O(1) primitive로 바꿀 수 있습니다.
