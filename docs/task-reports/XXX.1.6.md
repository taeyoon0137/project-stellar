# 작업 보고서: XXX.1.6 MotionSpec 생성·변경·삭제 command

- WBS: XXX.1.6
- 작업명: MotionSpec command와 대상 삭제 시 orphan 진단.
- 상태: 완료 (`[x]`)
- 결정: ADR-0094 결정 4
- 선행: XXX.1.1, Phase VII

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/motion/command.rs`(신규).
- `crates/document/src/motion/mod.rs`: module 등록.

## 구현 요약

- `MotionCommand::{Create, Replace, Delete, Restore}`가 검증 후 적용하고 정확한 inverse를 돌려줍니다. 실패하면 모음이 바뀌지 않습니다.
- 대상 node·속성이 사라져도 spec은 남고 `diagnose`가 `motion-orphan-target`을 냅니다. `Restore`는 대상 검사 없이 되살립니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 통과(lib 460개 포함 전체). |
| `cargo clippy -p stellar-document -p stellar-prototype --all-targets -- -D warnings` | 통과. |

시험:

- `motion::command::tests::apply_and_inverse_round_trip`, `invalid_commands_leave_set_unchanged`, `deleted_target_leaves_orphan_motion`.

## 차이(작업 지시와 다른 점)

- Phase VII `CommandKind`(영속 command journal) 등록은 영속 command registry(ADR-0079) 작업 몫입니다. document 계층 closed enum command로 구현했습니다(ADR-0094 결정 4).

## 남은 위험 / 한계

- CLI·MCP `CommandTransaction` 입력에서 MotionSpec command를 받는 연결이 없습니다.
