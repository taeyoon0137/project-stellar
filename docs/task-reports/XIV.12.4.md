# 작업 보고서: XIV.12.4 검증한 snapshot을 WorkingTree에 저장하고 git add

- WBS: XIV.12.4
- 작업명: validated snapshot을 WorkingTree project files로 generation save하고 관련 path를 시스템 Git `git add`로 resolved 처리
- 상태: 완료 (`[x]`)
- 선행: XIV.12.3, V.5.4
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 6·7

## 필수 실행 환경

- `DEV-MAC` (Rust, 시스템 Git 2.x, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/candidate.rs` — `serialize_merged_generation`(ours 세대 위 다음 generation 파일, 기존 save API 경로).
- `crates/ops/src/candidate/session.rs` — `ConflictSession::plan`, `WritePlan`·`PlannedWrite`·`PlanError`.
- `crates/ops/src/candidate/apply.rs`·`apply/tests.rs`(신규) — `apply_session`, `TreeFiles`·`DiskFiles`, `ApplyError`.
- `tools/stellar-cli/src/merge_driver.rs` — `resolve --apply`; `merge_driver/system_git.rs`의 `add -A --`.
- `tools/stellar-cli/src/cli.rs`(설명), `tests/fixtures/help.txt`, `tests/merge_driver.rs`.

## 구현 요약

- `plan`은 모든 단위를 `validate`(XIV.12.3)하고 `ValidatedCandidate`로만 결과를 씁니다. 컴포넌트 원본은 v3 encoder(재해석 검증),
  generation은 문서 역투영 → `serialize_records` → `build_save_plan`(ours 세대 기준) → chunk·manifest·`design/current`입니다.
  레코드를 다시 snapshot으로 올린 값이 candidate와 다르면 거부합니다. 하나라도 막히면 아무것도 쓰지 않습니다.
- `apply_session`은 세션을 연 뒤 unmerged 목록·대상 파일 hash가 바뀌었으면(stale) 쓰지 않고, 계획한 파일을 한 번에 쓴 뒤(파일마다
  임시 파일 → rename) 관련 경로를 한 번의 `git add -A --`로 resolved 처리합니다. `git add`는 신뢰한 프로젝트에서만 실행됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops` | 63 passed(apply 2 포함) |
| `cargo test -p stellar-cli --test merge_driver --test headless_help` | 6 + 6 passed |
| `cargo clippy -p stellar-cli -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 시스템 Git: 컴포넌트 폭 conflict(driver nonzero) + `design/current` conflict(두 branch가 다른 node 이름 변경)를 한 merge에서 만든 뒤 preview: 단위 component-source·generation, generation 객체 수 > 4, 쓰기 가능 | 통과 |
| `resolve --resolve 0=theirs --apply`: added에 원본과 `design/current`, `git ls-files -u` 비어 있음, `stellar validate .` 0, 두 node 이름과 theirs 폭 모두 반영, `MERGE_HEAD` 유지, 사용자 `git commit` 뒤 status clean | 통과 |
| ops: 미해결이면 Blocked이고 add 호출 0, 해결 뒤 한 번 쓰고 add, 결과 원본을 codec이 읽음 | 통과 |
| ops: 세션 뒤 외부에서 파일이 바뀌면 Stale이고 파일·index 그대로 | 통과 |

## 차이(작업 지시와 다른 점)

- 문서 집합이 바뀌는 병합(문서 추가·삭제)은 Unsupported로 거부합니다(ADR-0081 Consequences).
- Git 실행은 CLI 시스템 Git adapter입니다(ADR-0081 결정 7).

## 남은 위험 / 한계

- 여러 파일 쓰기의 원자성은 다중 파일 journal(ADR-0078)이 들어오기 전까지 XIV.12.6의 되돌리기에 의존합니다.
