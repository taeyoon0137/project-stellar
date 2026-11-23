# 작업 보고서: XXX.7.8 Context command

- WBS: XXX.7.8
- 작업명: ContextKey 생성·rename·default 변경·삭제, provision 생성·값 변경·삭제, requirement·Slot 공개 목록 추가·삭제.
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XXX.7.5이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]` 후보)
- 결정: ADR-0099 결정 8
- 선행: XXX.7.5, VIII.8.1, Phase VII

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/commands/src/context.rs`(신규): `ContextCommand`, `ContextEdit`(tag 1\~12), validate·invert·apply·codec, 생성기.
- `crates/commands/src/{command,kind,codec,edit,lib,test_support,property_tests,fuzz}.rs`: `CommandKind::EditContext`(`0x00A0`,
  codec version 5), `Command::EditContext(Box<_>)`, `EditContext::replace_context` primitive와 `UndoOp::Context`, 생성기·등록.
- `crates/document/src/context/key.rs`: `type_spec_bytes`·`type_spec_from_bytes`(command codec이 같은 형식 사용).
- `tests/integration/tests/context_commands.rs`(신규).

## 구현 요약

- key 삭제는 그 key의 provision·requirement·공개 목록 항목도 지우고, 역(복원)이 모두 되살립니다.
- 소비가 있는 key를 지우면 binding은 지우지 않고 `ContextBindingWorld`로 orphan validator가 `OrphanedBinding`(`SourceMissing`)으로
  바꿉니다. provision만 지우면 바깥 provision 또는 default로 해석됩니다.
- 적용은 journal primitive라 transaction rollback과 Undo가 같은 경로입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands -p stellar-history` | 통과(property round-trip `edit_context_round_trip`, codec·kind golden, fuzz 포함) |
| `cargo test -p stellar-integration-tests --test context_commands` | 통과 |

시험:

- `context::tests::every_context_edit_applies_and_inverts`: 11개 연산 apply/inverse, codec round-trip.
- `context::tests::deleting_only_the_provision_falls_back_to_default_and_bad_edits_are_rejected`: default fallback, 잘못된 편집
  거부, rollback.
- `context_commands::deleting_a_consumed_key_orphans_only_its_consumers_and_inverse_restores_resolution`.

## 차이(작업 지시와 다른 점)

- command codec version을 5로 올렸습니다. 다른 lane이 같은 시기에 version을 올리면 통합 때 번호를 맞춰야 합니다.

## 남은 위험 / 한계

- 컴포넌트 owner provision의 범위·cycle은 command가 library를 몰라 validator(XXX.7.5)에 맡깁니다.
- CLI·MCP의 command JSON 입력(`ops::mcp_command_input`)에는 Context 편집을 더하지 않았습니다(소유 밖, 후속).
- XXX.2.4(포트 공개의 ContextRequirement plan)는 이 command로 연결할 수 있게 되었지만 연결하지 않았습니다(prototype lane 몫).

## 후속 보완 (2026-11-24, Context 제품 경로 연결)

- 결정: ADR-0114 결정 3.
- CLI·MCP command 입력(`ops::mcp_command_input`, `stellar apply`와 MCP proposal 공용)에 Context 연산마다 필드 종류 하나
  (`create-context-key`, `rename-context-key`, `set-context-key-default`, `delete-context-key`, `create-context-provision`,
  `set-context-provision-value`, `delete-context-provision`, `add-context-requirement`, `remove-context-requirement`,
  `add-slot-context-key`, `remove-slot-context-key`)를 더했습니다. 결과는 `edit-context` canonical 입력 하나입니다. scope·owner·
  값 타입·리터럴은 텍스트 원본 parser를 그대로 씁니다. command schema를 다시 생성했습니다.
- `stellar apply`가 Context가 있는 문서를 두 번째 저장할 때 ChunkId 중복으로 실패하던 결함을 고쳤습니다(base context-set
  pointer 재사용 안 함).
- 시험: `mcp_command_input::tests::context_field_kinds_become_one_edit_context_command`(11 종류 해석, 열린 식 default·잘못된
  식·타입·scope 거부), CLI 종단 `tools/stellar-cli/tests/context.rs`(apply dry-run diff, 저장, provision 삭제, 열린 식 거부).
