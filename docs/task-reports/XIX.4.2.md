# 작업 보고서: XIX.4.2 승인 후 target node와 전체 descendant를 잠그는 AgentEditLease 생성

- WBS: XIX.4.2
- 작업명: 승인 후 target node와 전체 descendant를 잠그는 AgentEditLease 생성
- 상태: 완료 (`[x]`)
- 선행: XIX.4.1, VI.2.3
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/lock/lease.rs`(신규) — `Lease`, `SubtreeLock`, `LeaseTable::{holder, snapshot, judge}`.
- `crates/commands/src/lock/lease_tests.rs`(신규).
- `crates/commands/src/lock.rs` — `lease`·`coordinator` module 등록.
- `crates/mcp-core/src/lease.rs` — `AgentEditLeases::{submit, approve, release}`.

## 구현 요약

- 제출(`submit`)은 승인 대기만 만들고 잠그지 않습니다. 사용자가 승인(`approve`)하면 그 시점 문서로 대상을 다시 해석해 프로젝트 조정자에서 lease를 획득합니다. 제출 뒤 대상이 옮겨졌으면 새 조상으로 잠그고, 사라졌으면 `missing-node`로 거부합니다(`approval_resolves_targets_against_the_current_document`).
- node 소속은 자신부터 조상을 따라 lease root를 찾아 정합니다. 자손 목록을 저장하지 않으므로 lease 뒤에 추가된 자손도 같은 lease입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --locked` | 185 passed, 5 ignored(자식 프로세스 전용 1건 포함) |
| `cargo test -p stellar-mcp-core --locked` | 모든 test target 통과(lib 82 passed) |
| `cargo clippy -p stellar-commands -p stellar-mcp-core --all-targets --locked -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base edfea9db` | 0 |
| `cargo xtask generate-ffi-header --check` | 0, drift 없음 |

| 검사 | 결과 |
|---|---|
| 새로 추가된 descendant 자동 잠금: 에이전트가 lease 안에 추가한 node가 같은 lease에 속하고 사용자 편집이 거부됨(`new_descendant_is_locked_automatically`) | 통과 |
| 승인 전에는 잠그지 않고 승인 뒤 subtree 전체가 잠김(`lease_is_created_only_after_approval`) | 통과 |
| 제출 뒤 대상이 옮겨지면 승인 시점 조상으로 잠그고 겹침도 그 조상으로 판정, 사라진 대상은 `missing-node`로 거부(`approval_resolves_targets_against_the_current_document`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- lease 판정 비용은 잠긴 subtree 크기에 비례합니다. 큰 subtree의 실측은 하지 않았습니다(ADR-0093 Performance).

## 근거 커밋

- `537dbbd7`
- `52aa2c2a`
