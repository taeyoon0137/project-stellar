# 작업 보고서: XIV.12.2 해결은 candidate에만 적용

- WBS: XIV.12.2
- 작업명: resolution apply는 candidate merged snapshot에만 적용하고 WorkingTree/Index를 즉시 수정하지 않음
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록: 구현·검증과 선행 판정은 이미 `[x]` 조건을 충족했고 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 `[~]`에 머물렀습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XIV.12.1(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XIV.12.1
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 4·6

## 필수 실행 환경

- `DEV-MAC` (Rust, 시스템 Git 2.x, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/candidate/session.rs` — `ConflictSession::resolve`(C에만 적용, 역 command 반환)에 더해 `SessionConflict::{entry,
  allowed, resolution}`, `SessionUnit::status`, `UnitStatus`, `parse_resolution`.
- `tools/stellar-cli/src/merge_driver.rs` — `merge-driver resolve <프로젝트> [--resolve <번호>=<action>]...`(신뢰 프로젝트, preview).
- `tools/stellar-cli/src/cli.rs`(설명), `tests/fixtures/help.txt`, `tests/merge_driver.rs`.

## 구현 요약

- 해결 command는 `CandidateSnapshot::apply`(XXIII.2.3)로 메모리 C에만 적용합니다. 세션은 Git을 읽기 명령(`ls-files -u`,
  `cat-file`, `rev-parse --git-path`)으로만 부르고 파일을 쓰지 않습니다.
- `resolve`는 단위별 conflict(전역 번호, 허용 action, 적용한 해결), 미해결 수·위반 수·쓰기 가능 여부, raw conflict, 제외 경로,
  진행 중 Git 작업을 JSON으로 돌려줍니다. action 표기는 `base`·`ours`·`theirs`·`delete`·`custom:<값 canonical bytes 16진수>`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-cli --test merge_driver` | 5 passed |
| `cargo test -p stellar-cli --test headless_help` | 6 passed |
| `cargo clippy -p stellar-cli -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 시스템 Git merge conflict 뒤 `resolve --resolve 0=theirs`: 해결 theirs·쓰기 가능·허용 4개, 작업 트리 파일 digest와 `.git/index` bytes가 실행 전과 같음 | 통과 |
| 미해결 preview는 쓰기 불가로 보고, 허용하지 않은 `delete`는 65(`resolution`), 둘 다 작업 트리·index 동일 | 통과 |

## 차이(작업 지시와 다른 점)

- 해결의 메모리 적용 자체는 XXIII.2.3 커밋에 있고, 이 행은 Git 작업 트리 세션의 preview 경로와 disk·index 불변 검증입니다.

## 남은 위험 / 한계

- preview 결과는 실행마다 세션을 새로 엽니다. 여러 해결을 한 번에 주려면 `--resolve`를 반복합니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIV.12.2)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `e0adf084` Feat: conflict 해결 preview를 candidate에만 적용
