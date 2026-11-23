# 작업 보고서: XIX.5.3 user preview에 semantic diff와 validation 결과 제공, 승인 전 document state 불변

- WBS: XIX.5.3
- 작업명: user preview에 semantic diff와 validation 결과 제공, 승인 전 document state 불변
- 상태: 완료 (`[x]`)
- 선행: XIV.3.10, XIX.5.1
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/lock/lease.rs` — `preview`(문서 레코드 사본에 적용 + lease 판정), `LeasePreview`.
- `crates/mcp-core/src/attached/edit.rs` — `preview`(ops `diff_records`·`validate_records` 연결), `AgentPreview`.

## 구현 요약

- 미리보기는 문서 레코드로 사본을 만들어 같은 transaction을 적용합니다. 원본 `DocumentState`는 읽기만 하므로 문서와 revision이 바뀌지 않습니다.
- 결과에는 semantic diff(`stellar diff`와 같은 ops 함수), 문서 검증 진단(`stellar validate`와 같은 `validate_records`), change set, lease 판정이 들어갑니다. 범위 밖 proposal도 미리보기는 만들고 판정이 위반입니다.

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
| preview 전후 revision 동일: commands 미리보기(`preview_does_not_change_document_state`)와 attached 미리보기(`preview_has_semantic_diff_and_validation_without_changing_state`) 모두 revision·레코드 불변 | 통과 |
| 미리보기 diff가 바뀐 node 하나를 보고하고 검증 진단이 비어 있음 | 통과 |

## 차이(작업 지시와 다른 점)

- 렌더 프리뷰(이미지)는 이 행 검증 기준에 없고 UI 재설계 때 합니다.

## 남은 위험 / 한계

- 큰 문서는 미리보기마다 레코드 사본 비용(O(문서))이 듭니다. 실측은 하지 않았습니다.

## 근거 커밋

- `537dbbd7`
- `f8ad5150`
