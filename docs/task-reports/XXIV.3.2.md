# 작업 보고서: XXIV.3.2 redaction 기본 semantic diff 요약 artifact

- WBS: XXIV.3.2
- 작업명: semantic diff summary를 CI artifact로 생성하고 project text/name/path redaction을 기본으로 적용
- 상태: 완료 (`[x]`)
- 선행: XIV.3.8(`[x]`), XXIV.1.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/summary.rs`(신규) — `Redaction`(ProjectContent 기본 / None), `SummaryValue`·`SummaryEntry`·`DiffSummary`, `summarize`, `artifact_hash`.
- `crates/ops/src/lib.rs` — `summary` 모듈.
- `tools/stellar-cli/src/summary.rs`(신규) — `stellar diff-summary`, artifact JSON(`stellar-diff-summary` v1), 임시 파일 + rename 기록, 입력 프로젝트 안 기록 거부.
- `tools/stellar-cli/src/project_cmds.rs` — 비교 오류 분류 공유(`compare_error_public`).
- `tools/stellar-cli/src/cli.rs`, `src/lib.rs` — 명령 등록·도움말, 사용법 오류 test.
- `tools/stellar-cli/tests/output_fixtures.rs` — `diff_summary_redacts_project_text_names_and_paths_by_default`.
- `tools/stellar-cli/tests/fixtures/help.txt`, `tests/fixtures/results/diff-summary-redacted.json`(신규).
- `docs/task-reports/XXIV.3.2.md`(신규), 계획서(XXIV.3.2 `[ ]`→`[x]`).

## 구현 요약

- `stellar diff-summary <base> <target> [--output <파일>] [--include-content]`는 같은 프로젝트 두 상태의 semantic diff(XXIV.1.3 경로)를 요약합니다. 순서는 semantic diff canonical 순서(XIV.3.8)라 같은 입력은 같은 bytes입니다.
- **redaction 기본**: 문자열(`text`)·타입드(`typed`, 문자열·타이포그래피를 담을 수 있음)·불투명 bytes(`bytes`, 텍스트 payload 등) 값은 `{"type", "redacted": true}`로 종류만 남깁니다. bool·정수·float·StableId·속성 경로 표기·변경 종류·객체 종류는 남깁니다. 지운 값 수를 `redacted_values`로 보고합니다.
- **항상 넣지 않는 것**: 프로젝트 이름, 파일·프로젝트 경로, repository URL. `--include-content`를 명시해도 값만 포함되고 이 셋은 들어가지 않습니다.
- **artifact**: `--output`이 있으면 요약 JSON 한 줄을 같은 디렉터리의 임시 파일에 쓰고 `sync_all` 뒤 rename합니다. stdout 결과에는 형식·크기·content hash(`sha256:`)만 넣고 경로는 stderr 문맥에만 둡니다. 출력 위치가 입력 프로젝트 폴더 안이면 73(`output-inside-project`)으로 거부합니다. `--output`이 없으면 요약 object가 결과 `result`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops` | 35 passed, 0 failed |
| `cargo test -p stellar-cli` | unit 19, headless_help 6, json_copy_sync 2, output_fixtures 23 passed, 0 failed |
| `cargo clippy -p stellar-cli -p stellar-ops --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt --all -- --check` | 통과 |

| 검사(privacy fixture) | 결과 |
|---|---|
| 프로젝트 이름 `SECRET-PROJECT`, 경로 `summary-privacy-secret-dir`, 새 이름 `SECRET-RENAME`, 원래 표시 이름이 기본 stdout·artifact 어디에도 없음, golden `diff-summary-redacted.json` | 통과 |
| 지운 값 2(이전·이후 표시 이름), bool 변경 값은 남음 | 통과 |
| artifact 내용 = stdout 요약, stdout의 hash = 파일 bytes hash, stdout에 artifact 경로 없음 | 통과 |
| `--include-content`만 `SECRET-RENAME` 포함, 프로젝트 이름·경로는 계속 없음 | 통과 |
| 입력 프로젝트 안 `--output` 거부(73), 프로젝트 파일 변경 0 | 통과 |
| ops: 기본 redaction이 표시 이름을 지우고 `Redaction::None`은 유지(`text_values_are_redacted_by_default_and_kept_on_request`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 없음. redaction 해제는 `--include-content` 명시로만 가능합니다.

## 남은 위험 / 한계

- StableId와 OrderKey는 남깁니다. 식별자 자체는 프로젝트 텍스트가 아니지만, 같은 저장소를 가진 사람은 이를 실제 객체에 대응시킬 수 있습니다.
- float 값(크기·위치 등)은 남습니다. 디자인 수치까지 비공개가 필요하면 별도 정책이 필요합니다.
- 프로젝트 범위 변경(metadata·token·asset)은 현재 semantic snapshot 투영 대상이 아니라 요약에 나타나지 않습니다.
