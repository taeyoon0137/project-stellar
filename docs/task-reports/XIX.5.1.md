# 작업 보고서: XIX.5.1 agent command proposal의 closed enum 한정

- WBS: XIX.5.1
- 작업명: agent command proposal은 일반 CommandTransaction closed enum만 사용하고 raw memory patch와 임의 script를 금지
- 상태: 완료 (`[x]`)
- 선행: XIX.2.3 `[x]`, VII.1.6 `[x]`

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/mcp_command_input.rs`(신규) — 형식 중립 command 접근자 `CommandFields`, 필드 형식 종류 표 `FIELD_SPECS`, canonical 종류 목록 `canonical_kinds()`, 해석 함수 `command_input`.
- `crates/ops/src/lib.rs` — 모듈 등록 한 줄.
- `crates/mcp-core/src/proposal.rs`(신규) — MCP tool 인자 `{base_revision, document, commands}`를 `ApplyRequest`로 해석.
- `crates/mcp-core/src/lib.rs` — 모듈 등록.

## 구현 요약

- MCP proposal의 command 하나는 `stellar apply` 입력 형식(XXIV.4.4)과 같은 object입니다. 필드 형식 11종(`set-visible` 등)은 표의 key만, 그 밖의 closed enum 종류는 `kind`+`canonical`(commands canonical bytes, VII.1.2)만 받습니다.
- 해석 결과는 `stellar_ops::apply`가 commands crate의 닫힌 enum `Command`로만 바꿉니다. `canonical`이 없는 모르는 종류는 해석 단계에서, `canonical`을 붙인 모르는 종류와 선언 종류·bytes 종류가 다른 입력은 적용 전 변환 단계에서 거부됩니다. raw memory patch·script를 나타내는 입력 형식 자체가 없습니다.
- 해석기는 JSON 구현과 무관한 `CommandFields`로 값을 읽습니다. 그래서 L2(mcp-core, 자체 JSON 사본)가 L1과 같은 규칙과 같은 `FIELD_SPECS` 표를 씁니다. 같은 표에서 XIX.9.2의 입력 schema를 생성합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib mcp_command_input` | 3 passed |
| `cargo test -p stellar-mcp-core --lib proposal` | 2 passed |

| 검사 | 결과 |
|---|---|
| unknown command 거부: `raw-patch`·`script`·`eval`·`memory-write`는 `unknown-command`, `canonical`을 붙인 모르는 종류는 적용 전 `UnknownCommand`(`unknown_command_without_canonical_is_rejected`, `proposal_uses_only_the_closed_enum`) | 통과 |
| 필드 표가 `apply::FIELD_KINDS`와 같고 canonical 종류는 closed enum에서 필드 종류를 뺀 나머지(`field_specs_cover_exactly_the_field_kinds`) | 통과 |
| 표 밖 key, 필드·canonical 혼용, 음수 너비, f32 범위 밖 숫자, 대문자 16진수 거부(`field_kinds_parse_like_stellar_apply`) | 통과 |

## 차이(작업 지시와 다른 점)

- `tools/stellar-cli/src/apply.rs`의 `command_input`은 이 lane 소유 밖이라 바꾸지 않았습니다. 지금은 CLI와 ops 해석기가 같은 규칙을 따로 구현합니다. CLI가 `stellar_ops::mcp_command_input::command_input`을 쓰도록 바꾸는 일은 CLI 소유 작업에 넘깁니다(아래 위험).

## 남은 위험 / 한계

- 위 이중 구현이 남아 있습니다. 한쪽 규칙만 바뀌면 CLI와 MCP 해석이 달라질 수 있습니다. 같은 입력에 대해 두 해석이 같은지는 이 행의 테스트가 직접 비교하지 않습니다.
- lease 범위 검사(XIX.5.2)와 user preview(XIX.5.3)는 다른 행입니다.
