# 작업 보고서: XIX.4.3 잠긴 subtree의 편집을 user command validator가 거부(조회·선택 허용)

- WBS: XIX.4.3
- 작업명: 잠긴 subtree의 편집을 user command validator가 거부(조회·선택 허용)
- 상태: 부분 완료 (`[~]`)
- 선행: XIX.4.2, Phase VII
- 결정: [ADR-0093](../adr/ADR-0093-agent-permission-and-edit-lease.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/lock/lease.rs` — `LeaseTable::judge`, `execute_leased`, `Actor::of`.
- `crates/commands/src/lock/lease_tests.rs` — operation matrix.

## 구현 요약

- 판정 함수는 `LeaseTable::judge` 하나이고 transaction 결과 `ChangeSet`(바뀐·추가·삭제 node, 속성 경로)을 봅니다. command 종류별 대상 계산은 없습니다.
- 사용자 출처 transaction은 바뀐 node가 적용 전 또는 후에 lease 안이면 거부합니다. 부모·순서가 바뀐 node가 lease root의 진조상이면(상위 이동) 거부하고, 상위 삭제는 lease root 자체가 삭제 집합에 있어 거부됩니다.
- `execute_leased`는 같은 실행 경로(`execute_transaction`)로 적용한 뒤 판정하고, 거부하면 역 transaction으로 되돌리고 revision을 실행 전 값으로 돌립니다. lease가 없는 문서의 사용자 편집은 판정 없이 실행합니다.
- 조회·선택은 command가 아니라 lease와 무관합니다. 기존 D8 잠금(`locked` 필드)과 lease는 별개입니다.

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
| operation matrix: root·자손 속성, 표시, 잠금 토글, 자식 추가(root·자손), 자손·root 삭제, 안팎 이동, root 이동, 상위 삭제·이동·순서 변경은 거부, 상위 이름·상위의 다른 자식 추가·밖 node 편집은 허용(`user_operation_matrix_on_leased_subtree`, 18 case) | 통과 |
| 거부 뒤 문서 레코드와 revision이 실행 전과 같음(같은 test의 공통 확인) | 통과 |
| 조회·선택 허용, D8과 분리(`reading_and_selection_are_not_blocked`) | 통과 |
| 시스템 출처는 거부하지 않고 겹친 lease를 알림(`system_origin_is_not_judged_but_reports_touched_leases`) | 통과 |

## 차이(작업 지시와 다른 점)

- 앱 runtime의 사용자 편집 경로와 `stellar apply`(ops)는 아직 `execute_leased`를 부르지 않습니다. 둘 다 이 lane 소유 밖입니다. 그래서 상태는 `[~]`입니다.

## 남은 위험 / 한계

- runtime의 사용자 command 경로와 ops::apply 쓰기 경로를 `execute_leased` + 조정자 lease 목록으로 바꿔야 실제 편집이 막힙니다(ADR-0093 Consequences).

## 근거 커밋

- `537dbbd7`
