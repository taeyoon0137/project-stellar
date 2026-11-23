# 작업 보고서: XIX.3.1 MCP 권한 어휘 정의

- WBS: XIX.3.1
- 작업명: permission을 Read, Propose, Apply, Terminal, FileDelete, GitCommit, GitPush로 정의
- 상태: 완료 (`[x]`)
- 선행: XIX.2.4 `[x]`

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/agent-protocol/src/permission.rs`(신규) — `Permission` enum, `ALL`, `name`·`from_name`, `is_non_mutating`·`is_document`, enum golden test.
- `crates/agent-protocol/src/lib.rs` — `permission` 모듈 등록.
- `crates/mcp-core/src/catalog/tools.rs` — 자체 `RequiredGrant` enum을 지우고 `Permission`을 같은 이름으로 재노출(tool 정의의 필요 권한이 agent-protocol 어휘를 씀).

## 구현 요약

| variant | 이름 | 상태를 바꾸지 않음 | 문서 권한 |
|---|---|---|---|
| `Read` | `read` | 예 | 예 |
| `Propose` | `propose` | 예 | 예 |
| `Apply` | `apply` | 아니오 | 예 |
| `Terminal` | `terminal` | 아니오 | 아니오 |
| `FileDelete` | `file-delete` | 아니오 | 아니오 |
| `GitCommit` | `git-commit` | 아니오 | 아니오 |
| `GitPush` | `git-push` | 아니오 | 아니오 |

- 권한은 서로 독립입니다(§31.3). headless backend는 상태를 바꾸지 않는 권한(Read·Propose)이 필요한 tool만 냅니다(ADR-0005 §3, ADR-0085).
- 이름은 catalog JSON의 `grant` 값과 같은 kebab-case이고 대소문자를 구분합니다. 이전 `RequiredGrant`의 이름은 바뀌지 않아 catalog golden은 그대로입니다. `FileDelete`가 새로 생겼고 아직 이 권한을 요구하는 tool은 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-protocol` | 1 passed |
| `cargo test -p stellar-mcp-core` | 58 passed |

| 검사 | 결과 |
|---|---|
| enum golden: 일곱 권한의 순서·이름·분류, 이름 왕복, 모르는 이름·대소문자 다른 이름 거부(`permission_enum_golden`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 저장(XIX.3.2)과 승인 판정(XIX.3.3·XIX.3.4)은 다른 행입니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIX.3.1)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다. 이 어휘는 XIX.3.2\~XIX.3.4 권한 저장·판정(`stellar_mcp::permission`, ADR-0093)이 그대로 씁니다.

- `c3a7a29c` Feat: agent-protocol에 MCP 권한 어휘 일곱 개를 정의함 (XIX.3.1)
