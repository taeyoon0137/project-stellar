# 작업 보고서: XXIV.3.1 CI mode

- WBS: XXIV.3.1
- 작업명: lockfile resolve check, project validation, migration-required check를 `stellar ci` 한 명령으로 제공
- 상태: 부분 완료 (`[~]`) — 구현과 작업 행의 검증 기준은 통과했으나, 선행 XXIV.1.3이 `[~]`입니다(선행 Phase XIV 미완료). 계획서 §1.2 규칙 4에 따라 XXIV.1.3이 `[x]`가 될 때까지 `[~]`로 둡니다.
- 선행: XXIV.1.3(이 lane에서 구현, `[~]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/ci.rs`(신규) — `run_ci`, `check_lockfile`, `CiReport`·`CheckResult`·`CheckFinding`·`CheckStatus`.
- `crates/ops/src/lib.rs` — `ci` 모듈.
- `tools/stellar-cli/src/project_cmds.rs` — `ci` 명령과 결과 JSON.
- `tools/stellar-cli/src/cli.rs` — `ci` 등록·도움말, 사용법 오류 test.
- `tools/stellar-cli/tests/output_fixtures.rs` — `ci_mode_fixtures`.
- `tools/stellar-cli/tests/fixtures/help.txt`, `tests/fixtures/results/ci-pass.json`·`ci-invalid.json`·`ci-lockfile.json`(신규).
- `docs/task-reports/XXIV.3.1.md`(신규), 계획서(XXIV.3.1 `[ ]`→`[x]`).

## 구현 요약

- `stellar ci <경로>`는 같은 snapshot 하나로 세 검사를 고정 순서로 실행합니다. 모두 통과하면 0, 하나라도 실패하면 1(`ci-failed`)이며 결과 object(`checks[]`: `name`, `status`, `findings`)와 진단을 함께 출력합니다. 프로젝트를 열 수 없으면(root 아님 1, 손상 매니페스트 65 등) 기존 열기 오류 분류대로 0이 아닌 코드로 끝납니다.
- **lockfile**: resolver·network·cache를 쓰지 않고 매니페스트와 `stellar.lock.yaml`이 맞물리는지만 봅니다. 잠금 파일은 loader 상한(1 MiB)까지만 읽고 제한 YAML + lock schema validator(XV.4.2)로 해석합니다. 실패 코드: `lockfile-missing`(의존성이 있는데 파일 없음), `lockfile-invalid`, `lockfile-read`, `lockfile-missing-entry`(최상위 entry 없음), `lockfile-stale-entry`(library id·source·repository·version 선택자 불일치, 어느 필드인지 표시), `lockfile-extra-entry`(매니페스트에 없는 최상위 entry), `lockfile-orphan-entry`(부모 경로가 없는 전이 entry). 메시지에는 repository URL을 넣지 않고 logical name만 씁니다.
- **validation**: XXIV.1.3 `validate`와 같은 검사에서 오류만 실패로 셉니다(경고는 통과).
- **migration**: generation 스키마가 이 도구의 스키마(1)보다 낮으면 `migration-required`, 높으면 `unsupported-schema`로 실패합니다. 저장 전 프로젝트는 통과입니다.
- 읽기 전용이며 파일을 쓰지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops` | 34 passed, 0 failed |
| `cargo test -p stellar-cli` | unit 19, headless_help 6, json_copy_sync 2, output_fixtures 22 passed, 0 failed |
| `cargo clippy -p stellar-cli -p stellar-ops --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt --all -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 유효 프로젝트 통과(0), 파일 변경 0, golden `ci-pass.json` | 통과 |
| invalid project CI failure: 손상 문서 + 새 스키마 → 종료 코드 1, `validation`·`migration` fail, golden `ci-invalid.json` | 통과 |
| 의존성 선언 + lock entry 없음 → 1, `lockfile-missing-entry`, stdout에 repository URL 없음, golden `ci-lockfile.json` | 통과 |
| 손상 매니페스트 → 65(열기 오류) | 통과 |
| ops: 맞는 lock 통과, 선택자 불일치 stale(`version` 표시), 부모 없는 전이 entry orphan, lock 없음·형식 오류(`invalid_projects_fail_the_matching_check`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- "lockfile resolve check"는 resolver 재실행이 아니라 매니페스트·잠금 파일 일치 검사로 구현했습니다. CI에서 fetch 없이 결정적으로 확인할 수 있는 범위이며, cache 무결성 검사(XV.4.4)와 실제 resolve는 cache·network가 필요해 포함하지 않았습니다.

## 남은 위험 / 한계

- **후속(`[x]`로 올리기 위한 남은 일)**: XXIV.1.3이 `[x]`가 된 뒤 이 보고서의 검증 명령을 다시 실행해 `[x]`로 바꿉니다.
- local override(`local_override`)가 있는 의존성은 lock 비교에서 따로 다루지 않습니다.
- lock entry가 가리키는 commit이 원격에 실제로 있는지, cache entry hash가 맞는지는 검사하지 않습니다.
